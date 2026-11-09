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
mod proposals; // ADR-15 2.4: run proposals from a Director-General, started or dismissed by a human
mod session; // start, resume and end a session; the nonce, redactor and worker list
mod tools; // the Lead's tool surface (socket calls and their dispatch)
mod types; // actors, errors, the action model, the ask and decision records, the limits
mod watcher; // the ack watcher and the thread that ticks it
pub use proposals::{DEFAULT_PROPOSAL_BASE, DEFAULT_PROPOSAL_ROUNDS, MAX_OPEN_PROPOSALS, Proposal, ProposalDecision, proposal_sha, valid_proposal_item};
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
use super::queue::{self, Listing, SessionDir};
use super::{audit, fsx, nonce, sha256, time};
use crate::crew::names::ui_name;
use crate::crew::redact::Redactor;
use crate::crew::tree::{Status, Worker};
use crate::logging::Logger;

/// One run session the service steers. Several can be live at once (ADR-15 2.6): each keeps its own nonce, its own
/// queue and stop dirs and its own workers, so nothing addressed to one session can reach another.
struct Session {
    id: String,
    run_id: String,
    dir: SessionDir,
    nonce: String,
    workers: Vec<WorkerInfo>,
    /// The project the run belongs to (ADR-15 2.3), set by the app at launch. Its Director-General sees this session.
    project: Option<String>,
}

pub struct Service {
    control_root: PathBuf,
    audit_log: PathBuf,
    ledger: Ledger,
    logger: Option<Logger>,
    /// Live run sessions by session id.
    sessions: HashMap<String, Session>,
    /// While a Lead tool call runs: the sessions it is bound to (one run, or every run of one project). `None` for human
    /// calls, which see every session.
    scope: Option<Vec<String>>,
    /// The current conversation generation of each project's Director-General. A tool call with another one is refused.
    generations: HashMap<String, u32>,
    /// While a Director-General's tool call runs: its (project, generation). Only such a call may propose a run.
    bound_project: Option<(String, u32)>,
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
            sessions: HashMap::new(),
            scope: None,
            generations: HashMap::new(),
            bound_project: None,
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

    fn session(&self, sid: &str) -> Result<&Session> {
        self.sessions.get(sid).ok_or(ControlError::ControlDisabled)
    }

    /// The sessions the current call may see: the bound session for a Lead call, every session for a human one.
    fn in_scope(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values().filter(|s| self.scope.as_ref().is_none_or(|b| b.contains(&s.id)))
    }

    /// A worker's generation (its attempt number) for the action receipt guard. Read only: no ledger line. A name
    /// that is ambiguous across sessions has no generation (the caller must use the agent id).
    pub fn worker_generation(&self, arg: &str) -> Option<u64> {
        self.resolve(arg).ok().map(|(_, w)| u64::from(w.round.unwrap_or(0)))
    }

    /// The one worker of a campaign's run(s) on task `task` (the bare id in `tasks.json`; the workflow namespaces worker
    /// task ids as `<run_id>.<task>`). Matches only those exact ids, so a worker of another campaign with the same task
    /// name is never found. `Ok(None)`: no such worker. `Err`: more than one live worker matches. Read only.
    pub fn task_worker(&self, runs: &[String], task: &str) -> std::result::Result<Option<String>, ControlError> {
        let hit: Vec<&WorkerInfo> = self
            .in_scope()
            .flat_map(|s| s.workers.iter())
            .filter(|w| {
                w.task_id.as_deref().is_some_and(|t| {
                    // The workflow writes tasks.json ids already namespaced (`<run>.t1`), and so are worker task ids: an
                    // exact match whose run is one of the campaign's. A bare id (`t1`) matches the remainder after `<run>.`.
                    runs.iter().any(|r| t.strip_prefix(r.as_str()).and_then(|x| x.strip_prefix('.')).is_some_and(|rest| t == task || rest == task))
                })
            })
            .collect();
        let live: Vec<&&WorkerInfo> = hit.iter().filter(|w| !w.finished()).collect();
        match (hit.as_slice(), live.as_slice()) {
            ([], _) => Ok(None),
            (_, [w]) => Ok(Some(w.key().to_string())),
            (_, []) => Ok(hit.last().map(|w| w.key().to_string())),
            _ => Err(ControlError::WorkerAmbiguous),
        }
    }

    /// The workers as `list_workers` shows them (every session in scope), without recording a list action (for
    /// `project_state`).
    pub fn workers_view(&self) -> Vec<Value> {
        self.in_scope()
            .flat_map(|s| s.workers.iter().map(move |w| (s.id.as_str(), w)))
            .map(|(sid, w)| self.worker_json(sid, w))
            .collect()
    }

    /// The one worker `arg` names among the sessions in scope, with its session id. A name that matches live workers
    /// in two sessions (each run has a `t1`) is ambiguous: the caller must use the agent id.
    fn resolve(&self, arg: &str) -> Result<(String, WorkerInfo)> {
        if arg.is_empty() || arg.chars().count() > MAX_WORKER {
            return Err(ControlError::InvalidArgs("worker".into()));
        }
        let key = arg.strip_prefix("agent:").unwrap_or(arg);
        let all: Vec<(&str, &WorkerInfo)> = self.in_scope().flat_map(|s| s.workers.iter().map(move |w| (s.id.as_str(), w))).collect();
        let exact: Vec<(&str, &WorkerInfo)> = all.iter().copied().filter(|(_, w)| !key.is_empty() && w.key() == key).collect();
        let found = if exact.is_empty() {
            let suffix = format!(".{arg}");
            all.iter()
                .copied()
                .filter(|(_, w)| {
                    w.task_id.as_deref().is_some_and(|t| t == arg || t.ends_with(&suffix))
                        || w.worker.engine_id == arg
                        || ui_name(&w.worker.engine_id).eq_ignore_ascii_case(arg)
                })
                .collect()
        } else {
            exact
        };
        let one = match found.as_slice() {
            [] => return Err(ControlError::WorkerNotFound),
            [w] => *w,
            many => {
                let live: Vec<&(&str, &WorkerInfo)> = many.iter().filter(|(_, w)| !w.finished()).collect();
                match live.as_slice() {
                    [w] => **w,
                    _ => return Err(ControlError::WorkerAmbiguous),
                }
            }
        };
        Ok((one.0.to_string(), one.1.clone()))
    }

    /// The session an action without a worker is about: the bound one, or the only one.
    fn default_session(&self) -> Option<&Session> {
        let mut it = self.in_scope();
        match (it.next(), it.next()) {
            (Some(s), None) => Some(s),
            _ => None,
        }
    }

    fn new_action(&self, kind: Kind, actor: Actor, w: Option<(&str, &WorkerInfo)>, now: SystemTime) -> Result<Action> {
        let s = match w {
            Some((sid, _)) => self.sessions.get(sid),
            None => self.default_session(),
        };
        let (sid, rid) = s.map_or((String::new(), String::new()), |s| (s.id.clone(), s.run_id.clone()));
        let w = w.map(|(_, w)| w);
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

#[cfg(test)]
#[path = "service_sessions_tests.rs"]
mod sessions_tests;
