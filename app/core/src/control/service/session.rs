//! Session lifecycle: start, resume and end a session, its nonce and redactor, the worker list and signals. Several run
//! sessions can be live at once (ADR-15 2.6); every call here names the one it is about.

use super::*;

impl Service {
    // ------------------------------------------------------------ session

    /// Take control of a run session: create its dirs and a fresh nonce. An existing nonce (app restart, or a resume
    /// into the same session) is kept. Other live sessions are untouched.
    pub fn start_session(&mut self, session_id: &str, run_id: &str) -> Result<&str> {
        if !queue::valid_session_id(session_id) {
            return Err(ControlError::InvalidArgs("session_id".into()));
        }
        let dir = SessionDir::create(&self.control_root, session_id)?;
        let n = match nonce::read(&dir.root) {
            Ok(n) => n,
            Err(_) => nonce::rotate(&dir.root)?,
        };
        dir.sweep_temp();
        self.seq = self.seq.max(dir.max_seq());
        let s = Session { id: session_id.into(), run_id: run_id.into(), dir, nonce: n, workers: Vec::new(), project: None };
        self.sessions.insert(session_id.into(), s);
        self.log("control_session_start", &[]);
        Ok(self.nonce(session_id).unwrap_or_default())
    }

    /// The run session was resumed (`claude --resume`): rotate its nonce. Only this session's pending actions retire.
    pub fn resume_session(&mut self, session_id: &str) -> Result<&str> {
        self.session(session_id)?;
        self.retire_pending(session_id);
        let s = self.sessions.get_mut(session_id).ok_or(ControlError::ControlDisabled)?;
        s.nonce = nonce::rotate(&s.dir.root)?;
        self.log("control_nonce_rotated", &[]);
        Ok(self.nonce(session_id).unwrap_or_default())
    }

    /// The run session belongs to `project` (ADR-15 2.3): that project's Director-General sees and steers it.
    pub fn set_project(&mut self, session_id: &str, project: &str) -> Result<()> {
        if !crate::ids::valid_project_ref(project) {
            return Err(ControlError::InvalidArgs("project".into()));
        }
        self.sessions.get_mut(session_id).ok_or(ControlError::ControlDisabled)?.project = Some(project.into());
        Ok(())
    }

    /// The project's Director-General is now on conversation `generation`; calls from any other generation are refused.
    pub fn set_generation(&mut self, project: &str, generation: u32) -> Result<()> {
        if !crate::ids::valid_project_ref(project) {
            return Err(ControlError::InvalidArgs("project".into()));
        }
        self.generations.insert(project.into(), generation);
        Ok(())
    }

    /// The live sessions, in no particular order.
    pub fn session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    /// Whether `session_id` is a live session.
    pub fn has_session(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    /// The run ended. Its queued messages will never be delivered; stops stay. Other sessions are untouched.
    pub fn end_session(&mut self, session_id: &str) {
        self.retire_pending(session_id);
        self.sessions.remove(session_id);
    }

    /// This session's queued messages expire and its unconfirmed Lead stops are refused: neither may reach a later
    /// session or the run after a nonce rotation. Confirmed stops stay (only resume_worker clears one).
    pub(super) fn retire_pending(&mut self, session_id: &str) {
        let now = SystemTime::now();
        let mine = |a: &&Action| a.session_id == session_id;
        let ids: Vec<String> = self.pending.values().filter(mine).filter(|a| a.kind.queued_text()).map(|a| a.id.clone()).collect();
        for id in ids {
            self.expire(&id, now, "session_ended");
        }
        let asks: Vec<Action> =
            self.pending.values().filter(mine).filter(|a| a.state == State::AwaitingHumanConfirm).cloned().collect();
        for a in asks {
            if self.record(&a, State::Refused, json!({"reason": "session_ended"}), now).is_ok() {
                self.pending.remove(&a.id);
            }
        }
    }

    /// The nonce for this session's worker brief. Never log or return it in a tool result.
    pub fn nonce(&self, session_id: &str) -> Option<&str> {
        self.sessions.get(session_id).map(|s| s.nonce.as_str())
    }

    /// A redactor for every live session's nonce: text bound for one session never carries another's either.
    pub fn redactor(&self) -> Redactor {
        let mut r = Redactor::default();
        for s in self.sessions.values() {
            r.add(Some(&s.nonce));
        }
        r
    }

    /// Discovery's view of one session's workers. A worker that is no longer live has its queued messages expired
    /// here, the one production path that learns of completion (AC-3.2.5); it is idempotent, so every refresh may
    /// repeat it. Workers of a session that is not live are dropped.
    pub fn set_workers(&mut self, session_id: &str, workers: Vec<WorkerInfo>) {
        let finished: Vec<String> = workers.iter().filter(|w| !w.live).map(|w| w.key().to_string()).collect();
        let Some(s) = self.sessions.get_mut(session_id) else { return };
        s.workers = workers;
        for k in finished {
            self.worker_finished(&k);
        }
    }

    /// What the crew tree overlays for one session: a worker with a stop in force (its stop file exists; a resume
    /// removes it) or a queued, not yet delivered message or redirect. Keyed by the worker key (agent id).
    pub fn signals(&self, session_id: &str) -> crate::crew::tree::Signals {
        let mut sig = crate::crew::tree::Signals::default();
        let Some(sess) = self.sessions.get(session_id) else { return sig };
        for w in &sess.workers {
            if fs::symlink_metadata(sess.dir.stop_path(w.key())).is_ok() {
                sig.stopped.insert(w.key().to_string());
            }
        }
        for a in self.pending.values() {
            if a.kind.queued_text() && a.state == State::Queued && a.session_id == session_id {
                sig.message_pending.insert(a.worker.clone());
            }
        }
        sig
    }

    /// The worker's transcript ended: its queued messages expire now so they
    /// are never applied later to a different worker. Its stop stays.
    pub fn worker_finished(&mut self, key: &str) {
        let now = SystemTime::now();
        let ids: Vec<String> = self
            .pending
            .values()
            .filter(|a| a.worker == key && a.kind.queued_text())
            .map(|a| a.id.clone())
            .collect();
        for id in ids {
            self.expire(&id, now, "worker_finished");
        }
        for s in self.sessions.values_mut() {
            for w in s.workers.iter_mut().filter(|w| w.key() == key) {
                w.live = false;
            }
        }
    }
}
