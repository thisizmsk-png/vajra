//! Session lifecycle: start, resume and end a session, its nonce and redactor, the worker list and signals.

use super::*;

impl Service {
    // ------------------------------------------------------------ session

    /// Take control of a run session: create its dirs and a fresh nonce. An
    /// existing nonce (app restart for the same session) is kept.
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
        self.session = Some(Session { id: session_id.into(), run_id: run_id.into(), dir, nonce: n });
        self.log("control_session_start", &[]);
        Ok(self.nonce().unwrap_or_default())
    }

    /// The run session was resumed (`claude --resume`): rotate the nonce.
    pub fn resume_session(&mut self) -> Result<&str> {
        self.session()?;
        self.retire_pending();
        let s = self.session.as_mut().ok_or(ControlError::ControlDisabled)?;
        s.nonce = nonce::rotate(&s.dir.root)?;
        self.log("control_nonce_rotated", &[]);
        Ok(self.nonce().unwrap_or_default())
    }

    /// The session the control service steers, if any.
    pub fn session_id(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.id.as_str())
    }

    /// The run ended. Queued messages will never be delivered; stops stay.
    pub fn end_session(&mut self) {
        self.retire_pending();
        self.session = None;
        self.workers.clear();
    }

    /// Queued messages expire and unconfirmed Lead stops are refused: neither
    /// may reach a later session or the run after a nonce rotation. Confirmed
    /// stops stay (only resume_worker clears one).
    pub(super) fn retire_pending(&mut self) {
        let now = SystemTime::now();
        let ids: Vec<String> =
            self.pending.values().filter(|a| a.kind.queued_text()).map(|a| a.id.clone()).collect();
        for id in ids {
            self.expire(&id, now, "session_ended");
        }
        let asks: Vec<Action> =
            self.pending.values().filter(|a| a.state == State::AwaitingHumanConfirm).cloned().collect();
        for a in asks {
            if self.record(&a, State::Refused, json!({"reason": "session_ended"}), now).is_ok() {
                self.pending.remove(&a.id);
            }
        }
    }

    /// The nonce for the worker brief. Never log or return it in a tool result.
    pub fn nonce(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.nonce.as_str())
    }

    pub fn redactor(&self) -> Redactor {
        Redactor::new(self.nonce())
    }

    /// Discovery's view of the run's workers. A worker that is no longer live has its queued messages expired here,
    /// the one production path that learns of completion (AC-3.2.5); it is idempotent, so every refresh may repeat it.
    pub fn set_workers(&mut self, workers: Vec<WorkerInfo>) {
        let finished: Vec<String> = workers.iter().filter(|w| !w.live).map(|w| w.key().to_string()).collect();
        self.workers = workers;
        for k in finished {
            self.worker_finished(&k);
        }
    }

    /// The worker's transcript ended: its queued messages expire now so they
    /// are never applied later to a different worker. Its stop stays.
    /// What the crew tree overlays: a worker with a stop in force (its stop file exists; a resume removes it) or a
    /// queued, not yet delivered message or redirect. Keyed by the worker key (agent id).
    pub fn signals(&self) -> crate::crew::tree::Signals {
        let mut sig = crate::crew::tree::Signals::default();
        if let Some(sess) = &self.session {
            for w in &self.workers {
                if fs::symlink_metadata(sess.dir.stop_path(w.key())).is_ok() {
                    sig.stopped.insert(w.key().to_string());
                }
            }
        }
        for a in self.pending.values() {
            if a.kind.queued_text() && a.state == State::Queued && self.session.as_ref().is_some_and(|s| s.id == a.session_id) {
                sig.message_pending.insert(a.worker.clone());
            }
        }
        sig
    }

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
        for w in self.workers.iter_mut().filter(|w| w.key() == key) {
            w.live = false;
        }
    }
}
