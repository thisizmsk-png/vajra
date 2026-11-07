//! The control service (design.md 3, 5): resolves workers, writes queue and
//! stop files, records every action in the ledger, watches acks and expires
//! stale entries.
//!
//! Trust rules enforced here:
//! - `source` comes from the actor, never from a request: the Lead only ever
//!   produces `lead-via-desktop`; `human` is reachable only through the UI
//!   functions with `Actor::Human`.
//! - A Lead stop request stays `awaiting_human_confirm` until the human calls
//!   `confirm_stop`. Nothing is written to stops/ before that.
//! - A stop is cleared only by a human `resume_worker`. Transcript end, run
//!   end, ticks and the Lead leave it in force.
//! - The nonce is redacted from message text and agent output before either
//!   reaches the queue, ledger or a tool result.

// The service is split by seam; each child adds methods to `Service` (or defines the types it uses) and sees this
// module's private items. This file keeps the `Service` struct, `open`, `rebuild` and the shared helpers.
mod actions; // list, inspect, message, redirect, stop, confirm, cancel, resume
mod asks; // the Lead-ask and plan-decision records
mod session; // start, resume and end a session; the nonce, redactor and worker list
mod tools; // the Lead's tool surface (socket calls and their dispatch)
mod types; // actors, errors, the action model, the ask and decision records, the limits
mod watcher; // the ack watcher and the thread that ticks it
pub use types::*;
pub use watcher::{Watcher, spawn_watcher};

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::ledger::Ledger;
use super::queue::{self, SessionDir};
use super::{audit, fsx, nonce, sha256, time};
use crate::crew::names::ui_name;
use crate::crew::redact::Redactor;
use crate::crew::tree::{Status, Worker};
use crate::logging::Logger;

struct Session {
    id: String,
    run_id: String,
    dir: SessionDir,
    nonce: String,
}

pub struct Service {
    control_root: PathBuf,
    audit_log: PathBuf,
    ledger: Ledger,
    logger: Option<Logger>,
    session: Option<Session>,
    workers: Vec<WorkerInfo>,
    /// Actions not yet in a final state, by id.
    pending: HashMap<String, Action>,
    /// Latest stop action state per worker key, for `list_workers`.
    last_stop: HashMap<String, (String, State)>,
    seq: u64,
    /// The id of the newest ledger record `record` wrote: how `call_tool` names the action in its audit line.
    last_recorded: Option<String>,
    pub ttl: Duration,
}

impl Service {
    /// Open the service over `<data_dir>/control`. `audit_log` is Vajra's
    /// audit.log, passed explicitly. Pending actions are rebuilt from the
    /// ledger so acks that land while the app is down are still recorded.
    pub fn open(data_dir: &Path, audit_log: PathBuf, logger: Option<Logger>) -> io::Result<Self> {
        let control_root = data_dir.join("control");
        fsx::private_dir(data_dir)?;
        fsx::private_dir(&control_root)?;
        let path = control_root.join("ledger.jsonl");
        let records = Ledger::read_all(&path)?;
        let ledger = Ledger::open(&path)?;
        let mut svc = Self {
            control_root,
            audit_log,
            ledger,
            logger,
            session: None,
            workers: Vec::new(),
            pending: HashMap::new(),
            last_stop: HashMap::new(),
            seq: 0,
            last_recorded: None,
            ttl: DEFAULT_TTL,
        };
        svc.rebuild(&records);
        Ok(svc)
    }

    fn rebuild(&mut self, records: &[Value]) {
        for r in records {
            let (Some(kind), id) = (Kind::parse(str_field(r, "kind")), str_field(r, "action_id")) else { continue };
            let state = State::parse(str_field(r, "state"));
            let worker = str_field(r, "worker").to_string();
            if kind == Kind::Resume {
                // A resume clears the stop it names (and any still-pending record of it), exactly as resume_worker does live.
                let stopped = str_field(r, "stop_id");
                if self.last_stop.get(&worker).is_some_and(|(sid, _)| sid == stopped) {
                    self.last_stop.remove(&worker);
                }
                self.pending.remove(stopped);
                continue;
            }
            if kind == Kind::Stop {
                self.last_stop.insert(worker.clone(), (id.to_string(), state));
            }
            if !(kind.queued_text() || kind == Kind::Stop) {
                continue;
            }
            if matches!(state, State::Queued | State::AwaitingHumanConfirm) {
                let created = time::parse_iso(str_field(r, "ts"))
                    .map_or(UNIX_EPOCH, |s| UNIX_EPOCH + Duration::from_secs(s.max(0) as u64));
                let prev = self.pending.get(id).map(|a| a.created);
                self.pending.insert(
                    id.to_string(),
                    Action {
                        id: id.to_string(),
                        kind,
                        state,
                        actor: if str_field(r, "actor") == "lead" { Actor::Lead } else { Actor::Human },
                        session_id: str_field(r, "session_id").to_string(),
                        run_id: str_field(r, "run_id").to_string(),
                        worker,
                        engine_id: str_field(r, "engine_id").to_string(),
                        task_id: r.get("task_id").and_then(Value::as_str).map(str::to_string),
                        created: prev.unwrap_or(created),
                        reason: r.get("reason").and_then(Value::as_str).map(str::to_string),
                        stop_match: r.get("match").cloned(),
                        inflight_seen: None,
                        check_failed: None,
                    },
                );
            } else {
                self.pending.remove(id);
            }
        }
        self.seq = records.iter().filter(|r| Kind::parse(str_field(r, "kind")).is_some_and(Kind::queued_text)).count() as u64;
    }

    // ------------------------------------------------------------ helpers

    fn log(&self, event: &'static str, fields: &[(&'static str, u64)]) {
        if let Some(l) = &self.logger {
            let _ = l.log(event, fields);
        }
    }

    fn redact(&self, text: &str) -> String {
        self.redactor().clean(text)
    }

    fn session(&self) -> Result<&Session> {
        self.session.as_ref().ok_or(ControlError::ControlDisabled)
    }

    fn resolve(&self, arg: &str) -> Result<&WorkerInfo> {
        if arg.is_empty() || arg.chars().count() > MAX_WORKER {
            return Err(ControlError::InvalidArgs("worker".into()));
        }
        let key = arg.strip_prefix("agent:").unwrap_or(arg);
        let exact: Vec<&WorkerInfo> = self.workers.iter().filter(|w| !key.is_empty() && w.key() == key).collect();
        let found = if exact.is_empty() {
            let suffix = format!(".{arg}");
            self.workers
                .iter()
                .filter(|w| {
                    w.task_id.as_deref().is_some_and(|t| t == arg || t.ends_with(&suffix))
                        || w.worker.engine_id == arg
                        || ui_name(&w.worker.engine_id).eq_ignore_ascii_case(arg)
                })
                .collect()
        } else {
            exact
        };
        match found.as_slice() {
            [] => Err(ControlError::WorkerNotFound),
            [w] => Ok(w),
            many => {
                let live: Vec<&&WorkerInfo> = many.iter().filter(|w| !w.finished()).collect();
                match live.as_slice() {
                    [w] => Ok(w),
                    _ => Err(ControlError::WorkerAmbiguous),
                }
            }
        }
    }

    fn new_action(&self, kind: Kind, actor: Actor, w: Option<&WorkerInfo>, now: SystemTime) -> Result<Action> {
        let (sid, rid) = self.session.as_ref().map_or((String::new(), String::new()), |s| (s.id.clone(), s.run_id.clone()));
        Ok(Action {
            id: new_action_id(now)?,
            kind,
            state: State::Requested,
            actor,
            session_id: sid,
            run_id: rid,
            worker: w.map(|w| w.key().to_string()).unwrap_or_default(),
            engine_id: w.map(|w| w.worker.engine_id.clone()).unwrap_or_default(),
            task_id: w.and_then(|w| w.task_id.clone()),
            created: now,
            reason: None,
            stop_match: None,
            inflight_seen: None,
                        check_failed: None,
        })
    }

    /// Append one ledger line for `a` in `state`, plus `extra` fields.
    fn record(&mut self, a: &Action, state: State, extra: Value, now: SystemTime) -> Result<()> {
        let mut r = Map::new();
        r.insert("v".into(), json!(1));
        r.insert("ts".into(), json!(time::iso_ms(now)));
        r.insert("action_id".into(), json!(a.id));
        r.insert("kind".into(), json!(a.kind.as_str()));
        r.insert("state".into(), json!(state.as_str()));
        r.insert("actor".into(), json!(a.actor.name()));
        r.insert("via".into(), json!(a.actor.via()));
        r.insert("source".into(), json!(a.actor.source()));
        for (k, v) in [("run_id", &a.run_id), ("session_id", &a.session_id), ("worker", &a.worker), ("engine_id", &a.engine_id)] {
            if !v.is_empty() {
                r.insert(k.into(), json!(v));
            }
        }
        if let Some(t) = &a.task_id {
            r.insert("task_id".into(), json!(t));
        }
        if let Some(t) = &a.reason {
            r.insert("reason".into(), json!(t));
        }
        if let Some(m) = &a.stop_match {
            r.insert("match".into(), m.clone());
        }
        if let Value::Object(x) = extra {
            r.extend(x);
        }
        self.ledger.append(&Value::Object(r))?;
        self.last_recorded = Some(a.id.clone());
        if a.kind == Kind::Stop {
            self.last_stop.insert(a.worker.clone(), (a.id.clone(), state));
        }
        self.log("control_ledger", &[]);
        Ok(())
    }

    fn text_arg(&self, text: &str, max: usize, name: &str, allow_empty: bool) -> Result<String> {
        let n = text.chars().count();
        if (!allow_empty && n == 0) || n > max {
            return Err(ControlError::InvalidArgs(format!("{name} must be {}..={max} chars", u8::from(!allow_empty))));
        }
        // Human and Lead text keeps its lines; only the nonce is replaced.
        Ok(self.redactor().redact(text.to_string()))
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
