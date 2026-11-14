//! The `vajra-run` tools on the app side (ADR-16 16.5 "Transport", PR 6d2): the tool table the `--mcp-run` server
//! lists, the per-run registry the socket routes those calls to, and the gate in front of it.
//!
//! - **Outside the Service lock.** A `vajra-run` request never takes the control `Service` mutex: it goes to the run's
//!   `RunProtocol`, whose `await_any` parks without any lock held, so two awaits run side by side.
//! - **The gate, in order.** The run is registered; the request's token hashes to the one the registry holds (only
//!   the hash is kept); the lead has started; and the socket peer descends from that lead process (pid and start
//!   stamp, at most 64 parent links: `procid::descends_from`). The peer is the process as it was at accept (pid and
//!   start stamp) and must still be that process when it is admitted. The token is not a boundary against the run's own
//!   workers (they can read files, T1): it keeps another run's client from calling by mistake. The peer check is
//!   the real gate.
//! - **Rotation.** Resume writes a new token file and the registry keeps only the new hash; the old token is refused.
//! - **The lead pid** is the harness process itself. `set_lead` refuses the app's own pid and any process the app
//!   descends from: everything would descend from those. The caller must also never start a worker under the lead's
//!   tree (the core spawns workers itself, 6c).
//! - **Bounds.** At most `MAX_RUN_CALLS` calls of one run are served at once; past it a call is answered `busy`.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::{fsx, nonce, sha256};
use crate::desktop::runproto::{self, PlanInput, Refusal, RunProtocol};

/// The token file's name inside a run's data dir (`<data>/runs/<run>/`).
pub const TOKEN_FILE: &str = "vajra-run.token";
/// The env the `--mcp-run` server reads: its run, and the token file's path (never the token).
pub const RUN_ENV: &str = "VAJRA_RUN";
pub const TOKEN_FILE_ENV: &str = "VAJRA_RUN_TOKEN_FILE";
/// The app socket's absolute path. `--mcp-run` never derives it from its own HOME or data dir: a lead may run with
/// another HOME, and the server must reach this app's socket or fail at once.
pub const SOCKET_ENV: &str = "VAJRA_RUN_SOCKET";
/// The request cap for `plan_tasks` (16.5); every other request keeps `socket::MAX_REQUEST`.
pub const MAX_PLAN_REQUEST: usize = 256 * 1024;
/// How long a call from a starting lead waits for its read-back to pass (`RunLead` polls it every 250 ms).
pub const CONFIRM_WAIT: Duration = Duration::from_secs(10);

/// A long wait gets `wait_s` plus this much before the client gives up.
pub const AWAIT_SLACK: Duration = Duration::from_secs(10);
/// Calls of one run served at once.
pub const MAX_RUN_CALLS: usize = 16;
/// The longest any client wait can be. A lead profile's harness tool timeout must be above it (Codex
/// `tool_timeout_sec = 130`), or the harness cuts a call the app is still serving.
pub const MAX_CLIENT_TIMEOUT: Duration = Duration::from_secs(120);

pub const TOOL_NAMES: [&str; 9] =
    ["plan_tasks", "dispatch_worker", "await_any", "await_worker", "cancel_worker", "integrate", "finish_run", "list_workers", "inspect_worker"];

pub fn is_run_tool(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

fn ticket_prop() -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": 200})
}

fn wait_prop() -> Value {
    json!({"type": "integer", "minimum": 0, "maximum": 110})
}

/// The `vajra-run` tools, exactly: what `tools/list` returns.
pub fn tools() -> Value {
    let id = json!({"type": "string", "pattern": "^t[0-9]{1,2}$"});
    let ac = json!({"type": "string", "pattern": "^ac-[0-9]{1,2}$"});
    let short = |n: u32| json!({"type": "string", "maxLength": n});
    let criterion = json!({"type": "object", "additionalProperties": false, "required": ["id", "criterion", "check"],
        "properties": {"id": ac.clone(), "criterion": short(1000), "check": short(2000)}});
    let task = json!({"type": "object", "additionalProperties": false, "required": ["id", "title", "owner_role", "files", "brief"],
        "properties": {"id": id, "title": short(300), "owner_role": short(32),
            "files": {"type": "array", "maxItems": runproto::MAX_TASK_FILES, "items": short(300)},
            "deps": {"type": "array", "maxItems": runproto::MAX_TASK_DEPS, "items": id},
            "criteria": {"type": "array", "maxItems": runproto::MAX_CRITERIA, "items": ac},
            "brief": short(20_000), "touches_security": {"type": "boolean"}}});
    // Byte limits: a JSON Schema length counts characters, so an ASCII string at the limit passes both.
    let pair = |a: &str, an: usize, b: &str, bn: usize| json!({"type": "object", "additionalProperties": false, "required": [a, b],
        "properties": {a: short(an as u32), b: short(bn as u32)}});
    let obj = |required: Value, props: Value| json!({"type": "object", "additionalProperties": false, "required": required, "properties": props});
    json!([
        {"name": "plan_tasks",
         "description": "Submit the round's plan. Round 1 defines the criteria (each with a check); later rounds re-plan only the failed tasks.",
         "inputSchema": obj(json!(["round", "tasks"]), json!({"round": {"type": "integer", "minimum": 1, "maximum": 3},
            "criteria": {"type": "array", "minItems": 1, "maxItems": runproto::MAX_CRITERIA, "items": criterion},
            "tasks": {"type": "array", "minItems": 1, "maxItems": 8, "items": task},
            "decisions": {"type": "array", "maxItems": runproto::MAX_PLAN_DECISIONS, "items": pair("title", runproto::MAX_DECISION_TITLE, "rationale", runproto::MAX_RATIONALE)},
            "protected_actions": {"type": "array", "maxItems": runproto::MAX_PLAN_PROTECTED, "items": pair("action", runproto::MAX_ACTION, "why", runproto::MAX_ACTION_WHY)}}))},
        {"name": "dispatch_worker",
         "description": "Start a worker on one planned task. Returns a ticket, or not_ready with the reason.",
         "inputSchema": obj(json!(["task_id"]), json!({"task_id": {"type": "string", "pattern": "^t[0-9]{1,2}$"}}))},
        {"name": "await_any",
         "description": "Wait up to wait_s seconds (at most 110) for the first of these tickets to end; call it again until every ticket has ended.",
         "inputSchema": obj(json!(["tickets", "wait_s"]), json!({"tickets": {"type": "array", "minItems": 1, "maxItems": runproto::MAX_AWAIT_TICKETS, "items": ticket_prop()}, "wait_s": wait_prop()}))},
        {"name": "await_worker",
         "description": "Wait up to wait_s seconds (at most 110) for one ticket to end.",
         "inputSchema": obj(json!(["ticket", "wait_s"]), json!({"ticket": ticket_prop(), "wait_s": wait_prop()}))},
        {"name": "cancel_worker",
         "description": "Stop one of this run's workers.",
         "inputSchema": obj(json!(["ticket"]), json!({"ticket": ticket_prop()}))},
        {"name": "integrate",
         "description": "Merge the round's done tasks. Returns a round ticket; Vajra then runs the round's checks and reviews. Await the round ticket.",
         "inputSchema": obj(json!(["round"]), json!({"round": {"type": "integer", "minimum": 1, "maximum": 3}}))},
        {"name": "finish_run",
         "description": "End the run with your notes for the human. Vajra computes the run's status; you do not choose it.",
         "inputSchema": obj(json!(["report"]), json!({"report": short(8192)}))},
        {"name": "list_workers",
         "description": "This run's worker tickets with their task, round, seat and status, and the round tickets of the integrated rounds.",
         "inputSchema": obj(json!([]), json!({}))},
        {"name": "inspect_worker",
         "description": "One worker ticket of this run, with its task, round, seat and status.",
         "inputSchema": obj(json!(["ticket"]), json!({"ticket": ticket_prop()}))}
    ])
}

/// `integrate` merges in git before it answers: it gets this long, not the default.
pub const INTEGRATE_TIMEOUT: Duration = Duration::from_secs(120);

/// How long the `--mcp-run` client waits for the app's answer to one call: `wait_s + 10` for an await, longer for
/// `integrate`. A call that times out may still have run, so the client answers `no_answer`, never "unavailable".
pub fn client_timeout(tool: &str, args: &Value, default: Duration) -> Duration {
    match tool {
        "await_any" | "await_worker" => Duration::from_secs(args.get("wait_s").and_then(Value::as_u64).unwrap_or(0).min(110)) + AWAIT_SLACK,
        "integrate" => INTEGRATE_TIMEOUT,
        _ => default,
    }
}

fn refusal(r: Refusal) -> Value {
    json!({"error": r.code, "reason": r.reason})
}

fn bad_args(what: &str) -> Value {
    json!({"error": "invalid_args", "reason": what})
}

fn field<'a>(args: &'a Value, k: &str) -> Result<&'a Value, Value> {
    args.get(k).ok_or_else(|| bad_args(&format!("{k} is required")))
}

fn text<'a>(args: &'a Value, k: &str) -> Result<&'a str, Value> {
    field(args, k)?.as_str().ok_or_else(|| bad_args(&format!("{k} must be a string")))
}

fn round(args: &Value) -> Result<u32, Value> {
    field(args, "round")?.as_u64().and_then(|r| u32::try_from(r).ok()).ok_or_else(|| bad_args("round must be a number"))
}

fn wait(args: &Value) -> Result<Duration, Value> {
    field(args, "wait_s")?.as_u64().map(Duration::from_secs).ok_or_else(|| bad_args("wait_s must be a number"))
}

fn ok<T: serde::Serialize>(r: Result<T, Refusal>) -> Result<Value, Value> {
    r.map(|v| json!(v)).map_err(refusal)
}

/// One `vajra-run` tool call on the run's protocol. `Ok` is the tool result, `Err` a tool error.
pub fn call(p: &RunProtocol, tool: &str, args: &Value) -> Result<Value, Value> {
    if !args.is_object() {
        return Err(bad_args("arguments must be an object"));
    }
    match tool {
        "plan_tasks" => {
            let input: PlanInput = serde_json::from_value(args.clone()).map_err(|e| bad_args(&format!("plan_tasks: {e}")))?;
            ok(p.plan_tasks(input))
        }
        "dispatch_worker" => ok(p.dispatch_worker(text(args, "task_id")?)),
        "await_any" => {
            let tickets: Vec<String> = serde_json::from_value(field(args, "tickets")?.clone()).map_err(|_| bad_args("tickets must be strings"))?;
            ok(p.await_any(&tickets, wait(args)?))
        }
        "await_worker" => ok(p.await_any(&[text(args, "ticket")?.to_string()], wait(args)?)),
        "cancel_worker" => ok(p.cancel_worker(text(args, "ticket")?).map(|c| json!({"cancelled": c}))),
        "integrate" => ok(p.integrate(round(args)?).map(|t| json!({"round_ticket": t}))),
        "finish_run" => ok(p.finish_run(text(args, "report")?).map(|s| json!({"status": s}))),
        "list_workers" => Ok(json!({"workers": p.workers(None), "rounds": p.rounds()})),
        "inspect_worker" => {
            let t = text(args, "ticket")?;
            p.workers(Some(t)).pop().ok_or_else(|| json!({"error": "invalid", "reason": "not a worker ticket of this run"}))
        }
        _ => Err(json!({"error": "unknown_tool"})),
    }
}

// ---------------------------------------------------------------- tokens

/// A fresh token, written to `<run_dir>/vajra-run.token` at 0600 (replacing any earlier one atomically). Returns the
/// file's path and the token's hash; the token itself is not kept.
pub fn write_token(run_dir: &Path) -> io::Result<(PathBuf, [u8; 32])> {
    fsx::private_dir(run_dir)?;
    let token = nonce::random_hex(32)?;
    let path = fsx::write_atomic(run_dir, TOKEN_FILE, token.as_bytes())?;
    Ok((path, sha256::digest(token.as_bytes())))
}

/// Read the token the `--mcp-run` server was pointed at: a regular file, not a link, owned by us, 0600 or tighter.
/// One open (`O_NOFOLLOW`), and the checks are on that descriptor, so the file read is the file checked.
pub fn read_token(path: &Path) -> io::Result<String> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path)?;
    let m = f.metadata()?;
    // SAFETY: getuid has no preconditions and cannot fail.
    let me = unsafe { libc::getuid() };
    if !m.file_type().is_file() || m.uid() != me || m.mode() & 0o077 != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the run token file must be ours and 0600"));
    }
    let mut b = Vec::new();
    f.take(256).read_to_end(&mut b)?;
    let t = String::from_utf8(b).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "the run token is not text"))?;
    let t = t.trim().to_string();
    if t.len() != 64 || !t.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the run token is malformed"));
    }
    Ok(t)
}

fn same_hash(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------------------------------------------------------------- the registry

/// The lead process the peer check walks to: the same pid and start stamp the 6d1 liveness lease watches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeadProc {
    pub pid: u32,
    pub stamp: u64,
}

/// The socket peer as it was when it connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    pub pid: u32,
    pub stamp: u64,
}

impl Peer {
    /// `pid` as it is now; `None` when it can't be read.
    pub fn now(pid: u32) -> Option<Peer> {
        Some(Peer { pid, stamp: crate::procid::start_stamp(pid)? })
    }
}

/// Why `set_lead` refused a pid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeadRefused {
    UnknownRun,
    /// The app itself, or a process the app descends from: every process would pass the peer check.
    TooWide,
}

struct Entry {
    proto: RunProtocol,
    token: [u8; 32],
    lead: Option<LeadProc>,
    /// A launched lead whose read-back has not passed yet: its calls wait (`CONFIRM_WAIT`), and none passes until
    /// `confirm_lead`.
    starting: Option<LeadProc>,
    /// The current lead's call slots. Each `set_lead` starts a fresh count: a dead lead's parked calls release the old
    /// one and never crowd out its successor.
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

/// Holds one of a run's call slots until the call returns.
struct CallSlot(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for CallSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The live runs the socket serves `vajra-run` calls for.
pub struct RunRegistry {
    runs: Mutex<HashMap<String, Entry>>,
    /// Leads no longer current (cleared, replaced or their run removed) whose process may still run: their trees stay
    /// held to vajra-run tools until the process is gone. Pruned as each one goes.
    fenced: Mutex<Vec<LeadProc>>,
    /// The processes the app started to use the Service route: the Director-General's harness for each ask and a
    /// classic run's harness. Only they, their descendants and the app itself pass `service_allowed`; a worker the app
    /// spawns is never one, so neither it nor anything it starts can steer another session. Pruned as each one goes.
    service_roots: Mutex<Vec<LeadProc>>,
    /// The peer check; `procid::descends_from` outside tests.
    descends: Box<dyn Fn(u32, LeadProc) -> bool + Send + Sync>,
}

impl Default for RunRegistry {
    fn default() -> Self {
        Self::with_peer_check(Box::new(|peer, lead| crate::procid::descends_from(peer, lead.pid, lead.stamp)))
    }
}

impl RunRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn with_peer_check(descends: Box<dyn Fn(u32, LeadProc) -> bool + Send + Sync>) -> Self {
        RunRegistry { runs: Mutex::new(HashMap::new()), fenced: Mutex::new(Vec::new()), service_roots: Mutex::new(Vec::new()), descends }
    }

    /// Record `pid`, a process the app just started to use the Service route, as one of its roots. Called right after
    /// the spawn, before the process can have made a tool call; a call that came earlier is refused, never let through.
    /// A pid whose stamp cannot be read (it is already gone) records nothing. The app itself and its ancestors are
    /// refused: they would admit everything.
    pub fn allow_service(&self, pid: u32) -> bool {
        let me = std::process::id();
        let Some(stamp) = crate::procid::start_stamp(pid) else { return false };
        if pid == me || crate::procid::descends_from(me, pid, stamp) {
            return false;
        }
        let mut roots = crate::lock(&self.service_roots);
        roots.retain(|r| crate::procid::same_process(r.pid, r.stamp));
        let root = LeadProc { pid, stamp };
        if !roots.contains(&root) {
            roots.push(root);
        }
        true
    }

    /// Serve `run` with `proto` behind the token whose hash is `token`. No call passes until `set_lead`.
    pub fn register(&self, run: &str, proto: RunProtocol, token: [u8; 32]) {
        crate::lock(&self.runs).insert(run.to_string(), Entry { proto, token, lead: None, starting: None, calls: Default::default() });
    }

    /// The lead process that started (or restarted) for `run`: the harness process itself. Calls must come from its
    /// tree. The app's own pid, or one of its ancestors, is refused.
    pub fn set_lead(&self, run: &str, lead: LeadProc) -> Result<(), LeadRefused> {
        let me = std::process::id();
        if lead.pid == me || crate::procid::descends_from(me, lead.pid, lead.stamp) {
            return Err(LeadRefused::TooWide);
        }
        let mut runs = crate::lock(&self.runs);
        let e = runs.get_mut(run).ok_or(LeadRefused::UnknownRun)?;
        if let Some(old) = e.lead.replace(lead) {
            self.fence(old);
        }
        e.calls = Default::default();
        Ok(())
    }

    /// A lead that launched but is not yet confirmed confined (its read-back has not passed). It replaces the current
    /// lead, so nothing passes until `confirm_lead`; a call from its tree waits up to `CONFIRM_WAIT` for that.
    pub fn start_lead(&self, run: &str, lead: LeadProc) -> Result<(), LeadRefused> {
        let me = std::process::id();
        if lead.pid == me || crate::procid::descends_from(me, lead.pid, lead.stamp) {
            return Err(LeadRefused::TooWide);
        }
        let mut runs = crate::lock(&self.runs);
        let e = runs.get_mut(run).ok_or(LeadRefused::UnknownRun)?;
        // The lead it replaces, confirmed or still starting, stays fenced while its process runs, as in `set_lead`.
        for old in [e.lead.take(), e.starting.replace(lead)].into_iter().flatten().filter(|o| *o != lead) {
            self.fence(old);
        }
        e.calls = Default::default();
        Ok(())
    }

    /// The starting lead's read-back passed: from now on its calls pass. False when `lead` is no longer the one
    /// starting (it was cleared, or a successor started).
    pub fn confirm_lead(&self, run: &str, lead: LeadProc) -> bool {
        match crate::lock(&self.runs).get_mut(run) {
            Some(e) if e.starting == Some(lead) => {
                e.starting = None;
                e.lead = Some(lead);
                true
            }
            _ => false,
        }
    }

    /// Is `peer` (still the same process) in the tree of any registered lead? Such a process may call only
    /// `vajra-run` tools, and only it may send a request past `socket::MAX_REQUEST`.
    pub fn in_lead_tree(&self, peer: Option<Peer>) -> bool {
        let Some(p) = peer else { return false };
        // A starting lead counts: its tree is held to vajra-run tools from the first call. So does a fenced one.
        let mut leads: Vec<LeadProc> = crate::lock(&self.runs).values().flat_map(|e| e.lead.into_iter().chain(e.starting)).collect();
        leads.extend(crate::lock(&self.fenced).iter().copied().filter(|l| crate::procid::same_process(l.pid, l.stamp)));
        !leads.is_empty() && crate::procid::same_process(p.pid, p.stamp) && leads.into_iter().any(|l| (self.descends)(p.pid, l))
    }

    /// The lead process `lead` exited: nothing passes until the next `set_lead`. A late exit of an earlier lead never
    /// clears its successor. Returns whether it cleared.
    pub fn clear_lead(&self, run: &str, lead: LeadProc) -> bool {
        match crate::lock(&self.runs).get_mut(run) {
            Some(e) if e.lead == Some(lead) || e.starting == Some(lead) => {
                e.lead = None;
                e.starting = None;
                self.fence(lead);
                true
            }
            _ => false,
        }
    }

    /// Keep `lead`'s tree held to vajra-run tools while its process still runs.
    fn fence(&self, lead: LeadProc) {
        let mut f = crate::lock(&self.fenced);
        f.retain(|l| crate::procid::same_process(l.pid, l.stamp));
        if !f.contains(&lead) {
            f.push(lead);
        }
    }

    /// May `peer` use the Service route? Only by positive proof, an allowlist: it is still the process that connected,
    /// it is the app itself or descends from a recorded Service root (`allow_service`: the Director-General's and a
    /// classic run's harness, whose `--mcp` server is its child), and it is in no lead's tree, current or fenced. Being
    /// the app's descendant is not enough: a worker the app spawns, and anything it starts, is refused. A descendant
    /// that outlived its root is reparented away from it, so it can never prove it, on any connection.
    pub fn service_allowed(&self, peer: Option<Peer>) -> bool {
        let Some(p) = peer else { return false };
        if !crate::procid::same_process(p.pid, p.stamp) || self.in_lead_tree(peer) {
            return false;
        }
        let me = std::process::id();
        if p.pid == me {
            return crate::procid::start_stamp(me) == Some(p.stamp);
        }
        let roots: Vec<LeadProc> = crate::lock(&self.service_roots).clone();
        roots.into_iter().any(|r| crate::procid::descends_from(p.pid, r.pid, r.stamp))
    }

    /// Resume: only the new token's hash is kept.
    pub fn rotate_token(&self, run: &str, token: [u8; 32]) -> bool {
        crate::lock(&self.runs).get_mut(run).map(|e| e.token = token).is_some()
    }

    pub fn remove(&self, run: &str) {
        if let Some(e) = crate::lock(&self.runs).remove(run) {
            for l in [e.lead, e.starting].into_iter().flatten() {
                self.fence(l);
            }
        }
    }

    /// Gate one request and find its protocol. The registry lock is released before the call runs.
    fn admit(&self, run: &str, token: &str, peer: Option<Peer>) -> Result<(RunProtocol, CallSlot), Value> {
        use std::sync::atomic::Ordering;
        let mut runs = crate::lock(&self.runs);
        // A lead still starting: wait (lock released) for its read-back, so its first call is not refused.
        let until = std::time::Instant::now() + CONFIRM_WAIT;
        while runs.get(run).is_some_and(|e| e.lead.is_none() && e.starting.is_some()) && std::time::Instant::now() < until {
            drop(runs);
            std::thread::sleep(Duration::from_millis(50));
            runs = crate::lock(&self.runs);
        }
        // An unknown run has ended (or was never this app's): a terminal answer, so the lead stops instead of retrying.
        let e = runs.get(run).ok_or_else(|| json!({"error": "run_gone", "reason": "this run has ended; stop and exit"}))?;
        if !same_hash(&sha256::digest(token.as_bytes()), &e.token) {
            return Err(json!({"error": "run_token"}));
        }
        let lead = e.lead.ok_or_else(|| json!({"error": "lead_not_running"}))?;
        let (proto, calls) = (e.proto.clone(), e.calls.clone());
        drop(runs);
        // The peer must still be the process that connected (a reused pid has another stamp), and descend from the lead.
        match peer {
            Some(p) if crate::procid::same_process(p.pid, p.stamp) && (self.descends)(p.pid, lead) => {}
            _ => return Err(json!({"error": "peer_not_lead"})),
        }
        if calls.fetch_add(1, Ordering::SeqCst) >= MAX_RUN_CALLS {
            calls.fetch_sub(1, Ordering::SeqCst);
            return Err(json!({"error": "busy", "reason": "too many calls of this run in flight"}));
        }
        Ok((proto, CallSlot(calls)))
    }

    /// One `vajra-run` request from the socket: `{"vajra_run": run, "token": t, "tool": name, "args": {...}}`, from the
    /// peer process `peer`. The reply is `{"ok": result}` or an error object.
    pub fn serve(&self, req: &Value, peer: Option<Peer>) -> Value {
        let (Some(run), Some(token), Some(tool)) =
            (req.get("vajra_run").and_then(Value::as_str), req.get("token").and_then(Value::as_str), req.get("tool").and_then(Value::as_str))
        else {
            return json!({"error": "bad_request"});
        };
        if !is_run_tool(tool) {
            return json!({"error": "unknown_tool"});
        }
        let (proto, _slot) = match self.admit(run, token, peer) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let args = req.get("args").cloned().unwrap_or_else(|| json!({}));
        match call(&proto, tool, &args) {
            Ok(v) => json!({"ok": v}),
            Err(e) => json!({"tool_error": e}),
        }
    }
}

#[cfg(test)]
#[path = "runreg_tests.rs"]
mod tests;
