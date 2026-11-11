//! The worker actions: list and inspect (read), message and redirect (queue), stop, confirm, cancel and resume.

use super::*;

impl Service {
    // ------------------------------------------------------------ read actions

    pub fn list_workers(&mut self, actor: Actor, run_id: Option<&str>) -> Result<Value> {
        let now = SystemTime::now();
        let workers: Vec<Value> = self
            .in_scope()
            .filter(|s| run_id.is_none_or(|r| s.run_id == r))
            .flat_map(|s| s.workers.iter().map(move |w| (s.id.as_str(), w)))
            .map(|(sid, w)| self.worker_json(sid, w))
            .collect();
        let a = self.new_action(Kind::List, actor, None, now)?;
        self.record(&a, State::Done, json!({}), now)?;
        Ok(json!({"workers": workers}))
    }

    pub(super) fn worker_json(&self, sid: &str, w: &WorkerInfo) -> Value {
        let stopped = self.sessions.get(sid).is_some_and(|s| s.dir.stop_path(w.key()).exists());
        let stop = self.last_stop.get(w.key());
        json!({
            "worker": w.key(),
            "role": ui_name(&w.worker.engine_id),
            "engine_id": w.worker.engine_id,
            "task_id": w.task_id,
            "title": w.title.as_deref().map(|t| untrusted(&self.redact(t))),
            "status": w.worker.status.as_str(),
            "provider": w.worker.provider,
            "round": w.round,
            "live": w.live,
            "last_activity": w.last_activity,
            "stopped": stopped,
            "stop_request": stop.map(|(_, s)| s.as_str()),
            "stop_action_id": stop.map(|(id, _)| id.clone()),
        })
    }

    pub fn inspect_worker(&mut self, actor: Actor, worker: &str, lines: u64) -> Result<Value> {
        if !(1..=MAX_LINES).contains(&lines) {
            return Err(ControlError::InvalidArgs(format!("lines must be 1..={MAX_LINES}")));
        }
        let now = SystemTime::now();
        let (sid, w) = self.resolve(worker)?;
        let red = self.redactor();
        let skip = w.lines.len().saturating_sub(lines as usize);
        let tail: Vec<Value> = w.lines[skip..].iter().map(|l| untrusted(&red.clean(l))).collect();
        let mut out = self.worker_json(&sid, &w);
        out["lines"] = Value::Array(tail);
        out["cwd"] = json!(w.cwd);
        let a = self.new_action(Kind::Inspect, actor, Some((&sid, &w)), now)?;
        self.record(&a, State::Done, json!({}), now)?;
        Ok(out)
    }

    // ------------------------------------------------------------ messages

    pub fn message_worker(&mut self, actor: Actor, worker: &str, text: &str) -> Result<Value> {
        self.enqueue(Kind::Message, actor, worker, text, None)
    }

    /// `message_worker` that also refuses when the worker it resolves to is not on attempt `expect`. Checked under the
    /// same `&mut self` as the resolution and the publication, so a refresh cannot swap the attempt in between (FG-11).
    pub fn message_worker_expecting(&mut self, actor: Actor, worker: &str, text: &str, expect: Option<u64>) -> Result<Value> {
        self.enqueue(Kind::Message, actor, worker, text, expect)
    }

    pub fn redirect_worker(&mut self, actor: Actor, worker: &str, text: &str) -> Result<Value> {
        self.enqueue(Kind::Redirect, actor, worker, text, None)
    }

    pub(super) fn enqueue(&mut self, kind: Kind, actor: Actor, worker: &str, text: &str, expect: Option<u64>) -> Result<Value> {
        let text = self.text_arg(text, MAX_TEXT, "text", false)?;
        if self.in_scope().next().is_none() {
            return Err(ControlError::ControlDisabled);
        }
        // A core worker's profile decides (16.7): nothing is queued that no channel would deliver.
        if let Some(t) = worker.strip_prefix(CORE_PREFIX) {
            return self.enqueue_core(kind, actor, t, &text, expect);
        }
        let (sid, w) = self.resolve(worker)?;
        if let Some(want) = expect {
            let have = u64::from(w.round.unwrap_or(0));
            if have != want {
                return Err(ControlError::StaleGeneration(have));
            }
        }
        // design 5b: only Execute-phase task workers take steering; verify, review
        // and merge steps are the independent gate (they can still be stopped).
        if !is_task_label(&w.worker.label) {
            return Err(ControlError::ControlDisabled);
        }
        if w.finished() {
            return Err(ControlError::WorkerFinished);
        }
        let mut m = Map::new();
        if !w.worker.agent_id.is_empty() {
            m.insert("agent_id".into(), json!(w.worker.agent_id));
        }
        if let Some(c) = &w.cwd {
            m.insert("cwd".into(), json!(c));
        }
        // The hook matches within its own session's queue, so uniqueness is counted within that session only.
        let same_type = self.session(&sid)?.workers.iter().filter(|x| !x.finished() && x.worker.engine_id == w.worker.engine_id).count();
        if !w.worker.engine_id.is_empty() {
            m.insert("agent_type".into(), json!(w.worker.engine_id));
            if same_type == 1 {
                m.insert("unique".into(), json!(true));
            }
        }
        if let Some(t) = &w.task_id {
            m.insert("task_id".into(), json!(t));
        }
        if w.worker.agent_id.is_empty() && w.cwd.is_none() && same_type != 1 {
            return Err(ControlError::WorkerAmbiguous);
        }

        let now = SystemTime::now();
        let mut a = self.new_action(kind, actor, Some((&sid, &w)), now)?;
        let entry = json!({
            "v": 1, "action_id": a.id, "kind": kind.as_str(),
            "created": time::iso_secs(now), "expires": time::iso_secs(now + self.ttl),
            "source": actor.source(), "match": Value::Object(m), "text": text,
        });
        let dir = self.session(&sid)?.dir.dir("queue");
        self.seq += 1;
        let name = format!("{:06}-{}.json", self.seq, a.id);
        a.state = State::Queued;
        let extra = json!({"text": text, "text_sha256": sha256::hex_digest(text.as_bytes())});
        // Record first, publish second: the hook delivers whatever is in the queue the moment it appears, and a
        // delivery cannot be taken back, so the ledger must already know the action. If it cannot record, nothing is
        // published; if publishing then fails, that is recorded too.
        self.record(&a, State::Queued, extra, now)?;
        if let Err(e) = fsx::write_atomic(&dir, &name, entry.to_string().as_bytes()) {
            let _ = self.record(&a, State::NotDelivered, json!({"reason": "publish_failed"}), now);
            return Err(e.into());
        }
        self.pending.insert(a.id.clone(), a.clone());
        self.log("control_queued", &[("seq", self.seq)]);
        Ok(json!({"action_id": a.id, "state": State::Queued.as_str()}))
    }

    /// Message or Redirect to `core:<ticket>` (16.7). Its profile decides: Codex has no channel. A Claude worker's
    /// entry goes into its own control dir, matched exactly `{"session": <its session id>}`, the one match the control
    /// hook serves to a top-level session. The action's `session_id` is that dir, so the ack watcher settles it there.
    fn enqueue_core(&mut self, kind: Kind, actor: Actor, ticket: &str, text: &str, expect: Option<u64>) -> Result<Value> {
        let c = self.core_ticket(ticket)?;
        if expect.is_some_and(|want| want != u64::from(c.round)) {
            return Err(ControlError::StaleGeneration(u64::from(c.round)));
        }
        let sid = c.core.message_to(ticket).map_err(ControlError::NotSupportedByHarness)?;
        if !self.core_dirs.contains_key(&sid) {
            return Err(ControlError::NotSupportedByHarness(crate::adapter::profiles::NOT_STARTED));
        }
        let now = SystemTime::now();
        let mut a = self.new_action(kind, actor, None, now)?;
        (a.session_id, a.run_id, a.worker) = (sid.clone(), c.run, format!("{CORE_PREFIX}{ticket}"));
        let entry = json!({
            "v": 1, "action_id": a.id, "kind": kind.as_str(),
            "created": time::iso_secs(now), "expires": time::iso_secs(now + self.ttl),
            "source": actor.source(), "match": {"session": sid}, "text": text,
        });
        let dir = SessionDir { root: self.control_root.join(&sid) }.dir("queue");
        self.seq += 1;
        let name = format!("{:06}-{}.json", self.seq, a.id);
        a.state = State::Queued;
        let extra = json!({"text": text, "text_sha256": sha256::hex_digest(text.as_bytes())});
        // As for a hook worker: recorded first, published second.
        self.record(&a, State::Queued, extra, now)?;
        if let Err(e) = fsx::write_atomic(&dir, &name, entry.to_string().as_bytes()) {
            let _ = self.record(&a, State::NotDelivered, json!({"reason": "publish_failed"}), now);
            return Err(e.into());
        }
        self.pending.insert(a.id.clone(), a.clone());
        self.log("control_queued", &[("seq", self.seq)]);
        Ok(json!({"action_id": a.id, "state": State::Queued.as_str()}))
    }

    // ------------------------------------------------------------ stops

    /// Human: the stop file is written now. Lead: only a request that waits
    /// for the human's `confirm_stop`.
    pub fn stop_worker(&mut self, actor: Actor, worker: &str, reason: Option<&str>) -> Result<Value> {
        self.stop_worker_expecting(actor, worker, reason, None)
    }

    /// `stop_worker` that also refuses when the worker it resolves to is not on attempt `expect`. The comparison
    /// happens here, under the same `&mut self` as the resolution and the stop publication, so a refresh cannot
    /// swap the attempt in between (action receipts, FG-11).
    pub fn stop_worker_expecting(&mut self, actor: Actor, worker: &str, reason: Option<&str>, expect: Option<u64>) -> Result<Value> {
        let reason = reason.map(|r| self.text_arg(r, MAX_REASON, "reason", true)).transpose()?;
        if self.in_scope().next().is_none() {
            return Err(ControlError::ControlDisabled);
        }
        if let Some(t) = worker.strip_prefix(CORE_PREFIX) {
            return self.stop_core(actor, t, reason, expect);
        }
        let (sid, w) = self.resolve(worker)?;
        if let Some(want) = expect {
            let have = u64::from(w.round.unwrap_or(0));
            if have != want {
                return Err(ControlError::StaleGeneration(have));
            }
        }
        // A stop matches only on agent_id or cwd, never on agent_type (5.3).
        let mut m = Map::new();
        if !w.worker.agent_id.is_empty() {
            m.insert("agent_id".into(), json!(w.worker.agent_id));
        } else if let Some(c) = &w.cwd {
            m.insert("cwd".into(), json!(c));
        } else {
            return Err(ControlError::WorkerAmbiguous);
        }
        let now = SystemTime::now();
        // One pending Lead request per worker: a looping Lead gets the same one back.
        if actor == Actor::Lead {
            let key = w.key();
            if let Some(p) = self.pending.values().find(|p| p.kind == Kind::Stop && p.state == State::AwaitingHumanConfirm && p.worker == key) {
                return Ok(json!({"action_id": p.id, "state": p.state.as_str()}));
            }
        }
        let mut a = self.new_action(Kind::Stop, actor, Some((&sid, &w)), now)?;
        a.reason = reason;
        a.stop_match = Some(Value::Object(m));
        match actor {
            Actor::Lead => {
                a.state = State::AwaitingHumanConfirm;
                self.record(&a, a.state, json!({}), now)?;
                self.pending.insert(a.id.clone(), a.clone());
                Ok(json!({"action_id": a.id, "state": a.state.as_str()}))
            }
            Actor::Human => self.write_stop(a, now),
        }
    }

    /// Stop on `core:<ticket>` (16.7): the human's stop runs the profile's stop; a Lead's waits for the human's confirm,
    /// as every Lead stop does. One ledger line per step, like a hook stop.
    fn stop_core(&mut self, actor: Actor, ticket: &str, reason: Option<String>, expect: Option<u64>) -> Result<Value> {
        let c = self.core_ticket(ticket)?;
        if expect.is_some_and(|want| want != u64::from(c.round)) {
            return Err(ControlError::StaleGeneration(u64::from(c.round)));
        }
        let key = format!("{CORE_PREFIX}{ticket}");
        let now = SystemTime::now();
        if actor == Actor::Lead {
            if let Some(p) = self.pending.values().find(|p| p.kind == Kind::Stop && p.state == State::AwaitingHumanConfirm && p.worker == key) {
                return Ok(json!({"action_id": p.id, "state": p.state.as_str()}));
            }
        }
        let mut a = self.new_action(Kind::Stop, actor, None, now)?;
        (a.session_id, a.run_id, a.worker, a.reason) = (c.session, c.run, key, reason);
        match actor {
            Actor::Lead => {
                a.state = State::AwaitingHumanConfirm;
                self.record(&a, a.state, json!({}), now)?;
                self.pending.insert(a.id.clone(), a.clone());
                Ok(json!({"action_id": a.id, "state": a.state.as_str()}))
            }
            Actor::Human => self.write_stop(a, now),
        }
    }

    pub(super) fn write_stop(&mut self, mut a: Action, now: SystemTime) -> Result<Value> {
        if let Some(t) = a.worker.strip_prefix(CORE_PREFIX).map(str::to_string) {
            // Still live (a confirm may come after the ticket ended): recorded first, then stopped.
            let c = self.core_ticket(&t)?;
            self.pending.remove(&a.id);
            a.actor = Actor::Human;
            a.state = State::Queued;
            self.record(&a, State::Queued, json!({}), now)?;
            c.core.stop(&t);
            return Ok(json!({"action_id": a.id, "state": State::Queued.as_str()}));
        }
        // Into the session the stop was asked for, never whichever session happens to be live now.
        let s = self.session(&a.session_id)?;
        let mut entry = json!({
            "v": 1, "action_id": a.id, "kind": Kind::Stop.as_str(), "created": time::iso_secs(now),
            "source": Actor::Human.source(), "match": a.stop_match.clone().unwrap_or(json!({})),
        });
        if let Some(r) = &a.reason {
            entry["reason"] = json!(r);
        }
        let path = s.dir.stop_path(&a.worker);
        // A stop already in force stays as it is: rewriting it would let a failed ledger write roll the file back
        // away, lifting the stop with no resume record. The human sees the existing action.
        if fs::symlink_metadata(&path).is_ok() {
            let existing = fsx::read_nofollow_capped(&path, MAX_ACK_BYTES).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            let id = existing.as_ref().map_or("", |v| str_field(v, "action_id")).to_string();
            return Ok(json!({"action_id": id, "state": State::Queued.as_str(), "already_stopped": true}));
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        let stops = s.dir.dir("stops");
        // A confirmed stop is the human's action from here on.
        a.actor = Actor::Human;
        a.state = State::Queued;
        // Same rule as a message: record first, then make the stop visible to the hook (which enforces it at once).
        self.record(&a, State::Queued, json!({}), now)?;
        if let Err(e) = fsx::write_atomic(&stops, &name, entry.to_string().as_bytes()) {
            let _ = self.record(&a, State::Refused, json!({"reason": "publish_failed"}), now);
            return Err(e.into());
        }
        self.pending.insert(a.id.clone(), a.clone());
        Ok(json!({"action_id": a.id, "state": State::Queued.as_str()}))
    }

    pub(super) fn awaiting(&self, action_id: &str) -> Result<Action> {
        match self.pending.get(action_id) {
            Some(a)
                if a.kind == Kind::Stop
                    && a.state == State::AwaitingHumanConfirm
                    && self.sessions.contains_key(&a.session_id) =>
            {
                Ok(a.clone())
            }
            _ => Err(ControlError::ActionNotFound),
        }
    }

    /// The human confirmed a Lead stop request in the dialog.
    pub fn confirm_stop(&mut self, actor: Actor, action_id: &str) -> Result<Value> {
        if actor != Actor::Human {
            return Err(ControlError::NotPermitted);
        }
        let a = self.awaiting(action_id)?;
        self.write_stop(a, SystemTime::now())
    }

    /// The human cancelled a Lead stop request.
    pub fn cancel_stop(&mut self, actor: Actor, action_id: &str) -> Result<Value> {
        if actor != Actor::Human {
            return Err(ControlError::NotPermitted);
        }
        let mut a = self.awaiting(action_id)?;
        a.actor = Actor::Human;
        self.record(&a, State::Refused, json!({}), SystemTime::now())?;
        self.pending.remove(action_id);
        Ok(json!({"action_id": a.id, "state": State::Refused.as_str()}))
    }

    /// A `core:` ticket ended: a Lead's stop on it that still waits for the human is refused (`worker_finished`), so
    /// the dialog never offers to stop a worker that is gone.
    pub fn core_ticket_ended(&mut self, ticket: &str) {
        let key = format!("{CORE_PREFIX}{ticket}");
        let now = SystemTime::now();
        // Its undelivered messages expire too: no later session may take them.
        let queued: Vec<String> = self.pending.values().filter(|a| a.worker == key && a.kind.queued_text() && a.state == State::Queued).map(|a| a.id.clone()).collect();
        for id in queued {
            self.expire(&id, now, "worker_finished");
        }
        let asks: Vec<Action> = self.pending.values().filter(|a| a.kind == Kind::Stop && a.state == State::AwaitingHumanConfirm && a.worker == key).cloned().collect();
        for a in asks {
            if self.record(&a, State::Refused, json!({"reason": "worker_finished"}), now).is_ok() {
                self.pending.remove(&a.id);
            }
        }
    }

    /// The only way a stop is cleared. Appends a ledger line and the audit
    /// line `TS ALLOW desktop-control reason="resume id=<action_id>"`.
    pub fn resume_worker(&mut self, actor: Actor, worker: &str) -> Result<Value> {
        if actor != Actor::Human {
            return Err(ControlError::NotPermitted);
        }
        let (sid, w) = self.resolve(worker)?;
        let path = self.session(&sid)?.dir.stop_path(w.key());
        let stop: Value = match fsx::read_nofollow_capped(&path, MAX_ACK_BYTES) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or(Value::Null),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(ControlError::NotStopped),
            Err(e) => return Err(e.into()),
        };
        let now = SystemTime::now();
        let a = self.new_action(Kind::Resume, Actor::Human, Some((&sid, &w)), now)?;
        let stop_id = str_field(&stop, "action_id").to_string();
        // Records first, stop file last: a failure leaves the stop in force (a retry may add a duplicate
        // resume record), never a cleared stop without its ledger and audit lines.
        self.record(&a, State::Done, json!({"stop_id": stop_id}), now)?;
        audit::append_resume(&self.audit_log, &a.id, now)?;
        fs::remove_file(&path)?;
        self.pending.remove(&stop_id);
        self.last_stop.remove(w.key());
        Ok(json!({"action_id": a.id, "state": State::Done.as_str(), "stop_id": stop_id}))
    }
}
