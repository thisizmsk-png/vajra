//! The ack watcher: settles queued actions from the hook's acks, retries failed checks and expires stale entries, plus the thread that ticks it.

use super::*;

impl Service {
    // ------------------------------------------------------------ ack watcher

    pub fn tick(&mut self) {
        self.tick_at(SystemTime::now());
    }

    /// One pass of the ack watcher: acks -> delivered, expired queue entries
    /// -> not_delivered, stale inflight claims -> delivery_unknown.
    pub fn tick_at(&mut self, now: SystemTime) {
        // Unconfirmed Lead stop requests are refused after the TTL.
        let stale: Vec<Action> = self
            .pending
            .values()
            .filter(|a| a.state == State::AwaitingHumanConfirm && now.duration_since(a.created).unwrap_or_default() >= self.ttl)
            .cloned()
            .collect();
        for a in stale {
            let _ = self.record(&a, State::Refused, json!({"reason": "expired"}), now);
            self.pending.remove(&a.id);
        }
        // Every unsettled message, in the session dir it was published to: a claimed message outlives its session.
        let ids: Vec<String> = self.pending.values().filter(|a| a.state == State::Queued).map(|a| a.id.clone()).collect();
        for id in ids {
            let Some(sid) = self.pending.get(&id).map(|a| a.session_id.clone()) else { continue };
            match self.check(&id, &SessionDir { root: self.control_root.join(sid) }, now) {
                Ok(()) => {
                    if let Some(a) = self.pending.get_mut(&id) {
                        a.check_failed = None;
                    }
                }
                Err(e) => self.check_failed(&id, &e, now),
            }
        }
    }

    /// A settlement check failed (an unreadable `queue/`, a full descriptor table, a ledger that will not append).
    /// The action stays pending and is retried, but never silently and never forever: the first failure and every
    /// change of error is logged (the OS error number only, no paths or text), and after `CHECK_FAIL_TICKS` failures in
    /// a row spanning `CHECK_FAIL_AFTER` the action is settled as `delivery_unknown` with the reason `scan_failed`, which
    /// the UI shows like any other unknown delivery. A good check in between starts the count again.
    pub(super) fn check_failed(&mut self, id: &str, err: &ControlError, now: SystemTime) {
        let errno = match err {
            ControlError::Io(e) => e.raw_os_error().map_or(-1, i64::from),
            _ => -2,
        };
        let Some(a) = self.pending.get_mut(id) else { return };
        let f = a.check_failed.get_or_insert(CheckFailure { first: now, count: 0, errno: i64::MIN });
        f.count += 1;
        let new_error = f.errno != errno;
        f.errno = errno;
        let (count, long_enough) = (f.count, now.duration_since(f.first).unwrap_or_default() >= CHECK_FAIL_AFTER);
        if new_error {
            self.log("control_check_failed", &[("errno", errno.unsigned_abs()), ("count", u64::from(count))]);
        }
        if count >= CHECK_FAIL_TICKS && long_enough {
            let Some(a) = self.pending.get(id).cloned() else { return };
            if self.record(&a, State::DeliveryUnknown, json!({"reason": "scan_failed"}), now).is_ok() {
                self.pending.remove(id);
                self.log("control_check_gave_up", &[("errno", errno.unsigned_abs()), ("count", u64::from(count))]);
            }
        }
    }

    /// Settle one queued action from file evidence alone. The hook moves queue/ -> inflight/, writes the ack, then
    /// removes inflight/. Reading in the order queue, inflight, ack means a hook that ran in between has left an ack, but
    /// "all absent" is still not proof of non-publication: after power loss an ack and a claim removal that were never
    /// made durable look the same. So absence settles as `delivery_unknown` (`no_evidence`), and only the app's own
    /// `expired/` rename is a firm `not_delivered`. Queue wins over everything: an honest hook cannot ack before it
    /// claims. A stop has no queue file; only its ack settles it.
    pub(super) fn check(&mut self, id: &str, dir: &SessionDir, now: SystemTime) -> Result<()> {
        let Some(a) = self.pending.get(id).cloned() else { return Ok(()) };
        let stop = a.kind == Kind::Stop;
        if !stop && dir.queued(id)?.is_some() {
            if now.duration_since(a.created).unwrap_or_default() >= self.ttl {
                self.expire(id, now, "expired");
            }
            return Ok(());
        }
        let claimed = dir.inflight(id)?.is_some();
        let (delivered, invalid) = match fsx::read_nofollow_capped(&dir.ack_path(id), MAX_ACK_BYTES) {
            Ok(b) => match serde_json::from_slice::<Value>(&b) {
                Ok(v) if v.is_object() && v.get("action_id").is_none_or(|x| x == id) => (Some(v), false),
                _ => (None, true),
            },
            Err(e) => (None, e.kind() != std::io::ErrorKind::NotFound),
        };
        if let Some(ack) = delivered {
            let audit_ref = json!({"kind": "desktop-control", "ts": str_field(&ack, "consumed_at"), "tool": str_field(&ack, "tool_name")});
            self.record(&a, State::Delivered, json!({"audit_ref": audit_ref}), now)?;
        } else if invalid {
            self.record(&a, State::DeliveryUnknown, json!({"reason": "ack_invalid"}), now)?;
        } else if stop {
            // A stop in force waits for the ack of the first tool call it blocks, however long that takes. A stop whose
            // file is gone (it never reached the disk, or was removed behind the app's back) has nothing in force and
            // nothing to wait for: after the TTL it settles as unknown instead of staying queued forever.
            if now.duration_since(a.created).unwrap_or_default() < self.ttl {
                return Ok(());
            }
            match fs::symlink_metadata(dir.stop_path(&a.worker)) {
                Ok(_) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    self.record(&a, State::DeliveryUnknown, json!({"reason": "stop_file_missing"}), now)?;
                }
                Err(e) => return Err(e.into()),
            }
        } else if claimed {
            let Some(p) = self.pending.get_mut(id) else { return Ok(()) };
            let seen = *p.inflight_seen.get_or_insert(now);
            if now.duration_since(seen).unwrap_or_default() < INFLIGHT_UNKNOWN {
                return Ok(());
            }
            self.record(&a, State::DeliveryUnknown, json!({"reason": "claim_stale"}), now)?;
        } else {
            // Nothing queued, claimed or acked. expired/ is the app's own rename, so that is a firm not_delivered. Plain
            // absence is not: after power loss an ack and a claim removal that were never made durable look exactly
            // like a message that was never published, so it settles as unknown, not as a terminal not_delivered.
            if dir.expired(id)?.is_some() {
                self.record(&a, State::NotDelivered, json!({"reason": "expired"}), now)?;
            } else {
                self.record(&a, State::DeliveryUnknown, json!({"reason": "no_evidence"}), now)?;
            }
        }
        self.pending.remove(id);
        Ok(())
    }

    /// Move a still-queued message to expired/ and record not_delivered. The rename is the referee against the hook:
    /// if it fails the hook claimed the file, and `check` settles it from the ack. A lost record is re-derived by
    /// `check` from expired/.
    pub(super) fn expire(&mut self, id: &str, now: SystemTime, reason: &str) {
        let Some(a) = self.pending.get(id).cloned() else { return };
        let dir = SessionDir { root: self.control_root.join(&a.session_id) };
        let Ok(Some(q)) = dir.queued(id) else { return };
        let dst = dir.dir("expired").join(q.file_name().unwrap_or_default());
        if fs::rename(&q, &dst).is_err() {
            return;
        }
        if self.record(&a, State::NotDelivered, json!({"reason": reason}), now).is_ok() {
            self.pending.remove(id);
        }
    }
}

/// Runs `tick` every `every` until dropped.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub fn spawn_watcher(svc: Arc<Mutex<Service>>, every: Duration) -> Watcher {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let handle = std::thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            crate::lock(&svc).tick();
            std::thread::sleep(every);
        }
    });
    Watcher { stop, handle: Some(handle) }
}
