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

/// How long a queued message waits for the worker's next tool call.
pub const DEFAULT_TTL: Duration = Duration::from_secs(30 * 60);
/// An inflight claim with no ack after this is "delivery unknown" (5.4 step 6).
/// Cap for files a worker can write in the control dir (acks, stops).
pub const MAX_ACK_BYTES: u64 = 16 * 1024;
pub const INFLIGHT_UNKNOWN: Duration = Duration::from_secs(60);
pub const MAX_TEXT: usize = 4000;
pub const MAX_REASON: usize = 500;
pub const MAX_WORKER: usize = 128;
pub const MAX_LINES: u64 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    /// The human in the app UI.
    Human,
    /// The Lead chat session, through MCP and the socket.
    Lead,
}

impl Actor {
    /// The only place a source is chosen.
    pub fn source(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Lead => "lead-via-desktop",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Lead => "lead",
        }
    }
    fn via(self) -> &'static str {
        match self {
            Self::Human => "ui",
            Self::Lead => "mcp",
        }
    }
}

#[derive(Debug)]
pub enum ControlError {
    WorkerNotFound,
    WorkerFinished,
    WorkerAmbiguous,
    ControlDisabled,
    NotPermitted,
    NotStopped,
    ActionNotFound,
    UnknownTool,
    InvalidArgs(String),
    Io(io::Error),
}

impl ControlError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::WorkerNotFound => "worker_not_found",
            Self::WorkerFinished => "worker_finished",
            Self::WorkerAmbiguous => "worker_ambiguous",
            Self::ControlDisabled => "control_disabled",
            Self::NotPermitted => "not_permitted",
            Self::NotStopped => "not_stopped",
            Self::ActionNotFound => "action_not_found",
            Self::UnknownTool => "unknown_tool",
            Self::InvalidArgs(_) => "invalid_args",
            Self::Io(_) => "io_error",
        }
    }
    /// The error as a tool result. I/O detail stays out (it can carry paths).
    pub fn to_json(&self) -> Value {
        match self {
            Self::InvalidArgs(m) => json!({"error": self.code(), "detail": m}),
            _ => json!({"error": self.code()}),
        }
    }
}

impl From<io::Error> for ControlError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

type Result<T> = std::result::Result<T, ControlError>;

/// What the app knows about one worker: the crew record plus identity the
/// hook can match on. `title` and `lines` are agent text (untrusted).
#[derive(Clone, Debug)]
pub struct WorkerInfo {
    pub worker: Worker,
    pub task_id: Option<String>,
    pub title: Option<String>,
    pub round: Option<u32>,
    pub live: bool,
    pub last_activity: Option<String>,
    /// The subagent transcript cwd (its worktree).
    pub cwd: Option<String>,
    /// Recent transcript lines, newest last.
    pub lines: Vec<String>,
}

impl WorkerInfo {
    /// The app key: the agent id, else the cwd.
    pub fn key(&self) -> &str {
        if self.worker.agent_id.is_empty() { self.cwd.as_deref().unwrap_or("") } else { &self.worker.agent_id }
    }
    fn finished(&self) -> bool {
        !self.live || matches!(self.worker.status, Status::Done | Status::Failed)
    }
}

/// `task.<id>.r<n>`, the only steerable step label.
fn is_task_label(label: &str) -> bool {
    let Some(rest) = label.strip_prefix("task.") else { return false };
    let Some((id, round)) = rest.rsplit_once(".r") else { return false };
    !id.is_empty() && !id.contains('.') && !round.is_empty() && round.bytes().all(|b| b.is_ascii_digit())
}

/// `{untrusted:true, source:'agent-output', text}`
pub fn untrusted(text: &str) -> Value {
    json!({"untrusted": true, "source": "agent-output", "text": text})
}

#[derive(Clone, Debug)]
struct Action {
    id: String,
    kind: Kind,
    state: State,
    actor: Actor,
    session_id: String,
    run_id: String,
    worker: String,
    engine_id: String,
    task_id: Option<String>,
    created: SystemTime,
    reason: Option<String>,
    stop_match: Option<Value>,
    inflight_seen: Option<SystemTime>,
}

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
    pub ttl: Duration,
}

/// What an action does. `as_str` is the ledger and JSON name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    List,
    Inspect,
    Message,
    Redirect,
    Stop,
    Resume,
}

/// Where an action is. `as_str` is the ledger and JSON name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Requested,
    AwaitingHumanConfirm,
    Queued,
    Delivered,
    NotDelivered,
    DeliveryUnknown,
    Refused,
    Done,
}

impl Kind {
    const ALL: [Kind; 6] = [Kind::List, Kind::Inspect, Kind::Message, Kind::Redirect, Kind::Stop, Kind::Resume];
    fn as_str(self) -> &'static str {
        match self {
            Kind::List => "list",
            Kind::Inspect => "inspect",
            Kind::Message => "message",
            Kind::Redirect => "redirect",
            Kind::Stop => "stop",
            Kind::Resume => "resume",
        }
    }
    fn parse(k: &str) -> Option<Kind> {
        Self::ALL.into_iter().find(|x| x.as_str() == k)
    }
    /// Message and redirect go through the queue and can expire.
    fn queued_text(self) -> bool {
        matches!(self, Kind::Message | Kind::Redirect)
    }
}

impl State {
    const ALL: [State; 8] = [State::Requested, State::AwaitingHumanConfirm, State::Queued, State::Delivered, State::NotDelivered, State::DeliveryUnknown, State::Refused, State::Done];
    fn as_str(self) -> &'static str {
        match self {
            State::Requested => "requested",
            State::AwaitingHumanConfirm => "awaiting_human_confirm",
            State::Queued => "queued",
            State::Delivered => "delivered",
            State::NotDelivered => "not_delivered",
            State::DeliveryUnknown => "delivery_unknown",
            State::Refused => "refused",
            State::Done => "done",
        }
    }
    /// An unknown ledger state reads as final.
    fn parse(s: &str) -> State {
        Self::ALL.into_iter().find(|x| x.as_str() == s).unwrap_or(State::Done)
    }
}


fn str_field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn new_action_id(now: SystemTime) -> io::Result<String> {
    // Time-ordered like a ULID: 12 hex of ms, then 64 random bits.
    Ok(format!("a{:012x}{}", time::unix_ms(now), nonce::random_hex(8)?))
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
                    },
                );
            } else {
                self.pending.remove(id);
            }
        }
        self.seq = records.iter().filter(|r| Kind::parse(str_field(r, "kind")).is_some_and(Kind::queued_text)).count() as u64;
    }

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
    fn retire_pending(&mut self) {
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

    // ------------------------------------------------------------ read actions

    pub fn list_workers(&mut self, actor: Actor, run_id: Option<&str>) -> Result<Value> {
        let now = SystemTime::now();
        let workers: Vec<Value> = match (&self.session, run_id) {
            (Some(s), Some(r)) if s.run_id != r => Vec::new(),
            _ => self.workers.iter().map(|w| self.worker_json(w)).collect(),
        };
        let a = self.new_action(Kind::List, actor, None, now)?;
        self.record(&a, State::Done, json!({}), now)?;
        Ok(json!({"workers": workers}))
    }

    fn worker_json(&self, w: &WorkerInfo) -> Value {
        let stopped = self.session.as_ref().is_some_and(|s| s.dir.stop_path(w.key()).exists());
        let stop = self.last_stop.get(w.key());
        json!({
            "worker": w.key(),
            "role": ui_name(&w.worker.engine_id),
            "engine_id": w.worker.engine_id,
            "task_id": w.task_id,
            "title": w.title.as_deref().map(|t| untrusted(&self.redact(t))),
            "status": w.worker.status.as_str(),
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
        let w = self.resolve(worker)?.clone();
        let red = self.redactor();
        let skip = w.lines.len().saturating_sub(lines as usize);
        let tail: Vec<Value> = w.lines[skip..].iter().map(|l| untrusted(&red.clean(l))).collect();
        let mut out = self.worker_json(&w);
        out["lines"] = Value::Array(tail);
        out["cwd"] = json!(w.cwd);
        let a = self.new_action(Kind::Inspect, actor, Some(&w), now)?;
        self.record(&a, State::Done, json!({}), now)?;
        Ok(out)
    }

    // ------------------------------------------------------------ messages

    pub fn message_worker(&mut self, actor: Actor, worker: &str, text: &str) -> Result<Value> {
        self.enqueue(Kind::Message, actor, worker, text)
    }

    pub fn redirect_worker(&mut self, actor: Actor, worker: &str, text: &str) -> Result<Value> {
        self.enqueue(Kind::Redirect, actor, worker, text)
    }

    fn enqueue(&mut self, kind: Kind, actor: Actor, worker: &str, text: &str) -> Result<Value> {
        let text = self.text_arg(text, MAX_TEXT, "text", false)?;
        self.session()?;
        let w = self.resolve(worker)?.clone();
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
        let same_type = self.workers.iter().filter(|x| !x.finished() && x.worker.engine_id == w.worker.engine_id).count();
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
        let mut a = self.new_action(kind, actor, Some(&w), now)?;
        let entry = json!({
            "v": 1, "action_id": a.id, "kind": kind.as_str(),
            "created": time::iso_secs(now), "expires": time::iso_secs(now + self.ttl),
            "source": actor.source(), "match": Value::Object(m), "text": text,
        });
        let dir = self.session()?.dir.dir("queue");
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

    // ------------------------------------------------------------ stops

    /// Human: the stop file is written now. Lead: only a request that waits
    /// for the human's `confirm_stop`.
    pub fn stop_worker(&mut self, actor: Actor, worker: &str, reason: Option<&str>) -> Result<Value> {
        let reason = reason.map(|r| self.text_arg(r, MAX_REASON, "reason", true)).transpose()?;
        self.session()?;
        let w = self.resolve(worker)?.clone();
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
        let mut a = self.new_action(Kind::Stop, actor, Some(&w), now)?;
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

    fn write_stop(&mut self, mut a: Action, now: SystemTime) -> Result<Value> {
        let s = self.session()?;
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

    fn awaiting(&self, action_id: &str) -> Result<Action> {
        match self.pending.get(action_id) {
            Some(a)
                if a.kind == Kind::Stop
                    && a.state == State::AwaitingHumanConfirm
                    && self.session.as_ref().is_some_and(|s| s.id == a.session_id) =>
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

    /// The only way a stop is cleared. Appends a ledger line and the audit
    /// line `TS ALLOW desktop-control reason="resume id=<action_id>"`.
    pub fn resume_worker(&mut self, actor: Actor, worker: &str) -> Result<Value> {
        if actor != Actor::Human {
            return Err(ControlError::NotPermitted);
        }
        let w = self.resolve(worker)?.clone();
        let path = self.session()?.dir.stop_path(w.key());
        let stop: Value = match fsx::read_nofollow_capped(&path, MAX_ACK_BYTES) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or(Value::Null),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(ControlError::NotStopped),
            Err(e) => return Err(e.into()),
        };
        let now = SystemTime::now();
        let a = self.new_action(Kind::Resume, Actor::Human, Some(&w), now)?;
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
            let _ = self.check(&id, &SessionDir { root: self.control_root.join(sid) }, now);
        }
    }

    /// Settle one queued action from file evidence alone. The hook moves queue/ -> inflight/, writes the ack, then
    /// removes inflight/, so reading in the order queue, inflight, ack makes "all absent" proof that the file was never
    /// published (a hook that ran in between would have left an ack). Queue wins over everything: an honest hook cannot
    /// ack before it claims. A stop has no queue file; only its ack settles it.
    fn check(&mut self, id: &str, dir: &SessionDir, now: SystemTime) -> Result<()> {
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
            return Ok(());
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
    fn expire(&mut self, id: &str, now: SystemTime, reason: &str) {
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

    // ------------------------------------------------------------ the Lead's tool surface

    /// A tool call from the socket. The actor is always the Lead; arguments
    /// are checked against the tool's schema (no extra properties).
    pub fn call_tool(&mut self, tool: &str, args: &Value) -> Result<Value> {
        let empty = Map::new();
        let args = match args {
            Value::Object(m) => m,
            Value::Null => &empty,
            _ => return Err(ControlError::InvalidArgs("arguments must be an object".into())),
        };
        let allowed: &[&str] = match tool {
            "list_workers" => &["run_id"],
            "inspect_worker" => &["worker", "lines"],
            "message_worker" | "redirect_worker" => &["worker", "text"],
            "stop_worker" => &["worker", "reason"],
            _ => return Err(ControlError::UnknownTool),
        };
        if let Some(k) = args.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(ControlError::InvalidArgs(format!("unknown property {k}")));
        }
        let s = |k: &str, req: bool| -> Result<Option<&str>> {
            match args.get(k) {
                None if !req => Ok(None),
                Some(Value::String(v)) => Ok(Some(v.as_str())),
                _ => Err(ControlError::InvalidArgs(format!("{k} must be a string"))),
            }
        };
        let lead = Actor::Lead;
        match tool {
            "list_workers" => {
                let run = s("run_id", false)?;
                if let Some(r) = run {
                    if !crate::ids::valid_run_id(r) {
                        return Err(ControlError::InvalidArgs("run_id".into()));
                    }
                }
                self.list_workers(lead, run)
            }
            "inspect_worker" => {
                let lines = match args.get("lines") {
                    None => 50,
                    Some(v) => v.as_u64().ok_or_else(|| ControlError::InvalidArgs("lines must be an integer".into()))?,
                };
                self.inspect_worker(lead, s("worker", true)?.unwrap_or(""), lines)
            }
            "message_worker" => self.message_worker(lead, s("worker", true)?.unwrap_or(""), s("text", true)?.unwrap_or("")),
            "redirect_worker" => self.redirect_worker(lead, s("worker", true)?.unwrap_or(""), s("text", true)?.unwrap_or("")),
            _ => self.stop_worker(lead, s("worker", true)?.unwrap_or(""), s("reason", false)?),
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

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
