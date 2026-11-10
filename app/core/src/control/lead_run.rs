//! The Lead process (ADR-13 Decisions 3 to 5, 8): spawn one headless turn, scan stdout for the `system/init`
//! event, check the tool list and permission mode (fail closed), stream the rest to a sink, and kill the whole
//! process group on any violation, timeout or cancel.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::lead::{FramedAsk, LeadError};
use crate::lock;
use crate::sessions::argv::LEAD_TOOLS;

/// `init` must arrive within this many events and this long, else `lead_no_init`.
pub const INIT_MAX_EVENTS: usize = 64;
pub const INIT_TIMEOUT: Duration = Duration::from_secs(30);
/// One ask is bounded to ten minutes (Decision 8).
pub const ASK_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// A stream-json line longer than this is not an event.
const MAX_LINE: u64 = 4 * 1024 * 1024;
/// After the `result` event the process gets this long to exit before its group is killed.
const EXIT_GRACE: Duration = Duration::from_secs(5);

/// The only permission mode the Lead may report.
pub const EXPECTED_MODE: &str = "dontAsk";

/// The sorted tool names `init.tools` must equal exactly.
pub fn expected_tools() -> Vec<String> {
    let mut t: Vec<String> = LEAD_TOOLS.split(',').map(str::to_string).collect();
    t.sort();
    t
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitInfo {
    pub session_id: String,
}

/// Reads events until the first `system/init`. Hook and state events are skipped (Vajra's own SessionStart hook
/// puts `init` at event 6 of 10 in the spike); an `assistant`, `user` or `result` first means the process answered
/// without the check ever running.
#[derive(Debug, Default)]
pub struct InitScan {
    seen: usize,
}

impl InitScan {
    /// `Ok(None)` keeps scanning, `Ok(Some)` is a valid `init`, `Err` is fatal.
    pub fn feed(&mut self, ev: &Value) -> Result<Option<InitInfo>, LeadError> {
        self.seen += 1;
        if self.seen > INIT_MAX_EVENTS {
            return Err(LeadError::NoInit);
        }
        let ty = ev.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "assistant" | "user" | "result" => Err(LeadError::NoInit),
            "system" if ev.get("subtype").and_then(Value::as_str) == Some("init") => {
                let mut got: Vec<&str> = match ev.get("tools").and_then(Value::as_array) {
                    Some(a) if a.iter().all(Value::is_string) => a.iter().filter_map(Value::as_str).collect(),
                    _ => return Err(LeadError::ToolsMismatch),
                };
                got.sort_unstable();
                if got != expected_tools() {
                    return Err(LeadError::ToolsMismatch);
                }
                if ev.get("permissionMode").and_then(Value::as_str) != Some(EXPECTED_MODE) {
                    return Err(LeadError::ModeMismatch);
                }
                let session_id = ev.get("session_id").and_then(Value::as_str).unwrap_or("").to_string();
                Ok(Some(InitInfo { session_id }))
            }
            _ => Ok(None),
        }
    }
}

/// How a streamed line is tagged on the `lead-<run_id>` bus stream (`you` is added by the caller).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Lead,
    Tool,
    Error,
}

impl Tag {
    pub fn as_str(self) -> &'static str {
        match self {
            Tag::Lead => "lead",
            Tag::Tool => "tool",
            Tag::Error => "error",
        }
    }
}

/// Where a turn's output goes. Text is untrusted: the sink cleans it.
pub trait Sink {
    /// Every raw stdout line, before it is parsed (the self-test checks it for the canary).
    fn raw(&mut self, _line: &str) {}
    /// The `init` event passed the tool and mode check.
    fn init(&mut self, _info: &InitInfo) {}
    fn event(&mut self, tag: Tag, text: &str);
}

/// What a finished turn looked like.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Done {
    /// The `result` event said `is_error`.
    pub is_error: bool,
    /// A tool call was denied: an erroring `tool_result`, or `permission_denials` in the result.
    pub denied: bool,
    /// A Read was refused by the permission rules themselves: the "denied by your permission settings" error, or a
    /// `Read` entry in `permission_denials`. Any other tool error (a bad path, an I/O failure) does not count.
    pub permission_denied: bool,
    /// The model called some tool.
    pub tool_attempted: bool,
    /// An erroring `result` that names a typed provider failure (an auth, rate-limit, overloaded or bad-request error in
    /// Claude's stream). Its text is not kept here: the sink gets it through `redact_provider_text`, before it is cut to length.
    pub provider_error: Option<crate::providers::errors::Classified>,
    /// The `file_path` of every Read that a permission rule refused (matched through the call's id, or taken from the
    /// result's `permission_denials`). The self-test accepts a denial only if it names the canary.
    pub denied_paths: Vec<String>,
    /// `tool_use` id to the Read's `file_path`, for matching a denied result back to its call.
    read_paths: std::collections::HashMap<String, String>,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub init: Duration,
    pub total: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self { init: INIT_TIMEOUT, total: ASK_TIMEOUT }
    }
}

/// The running process, shared between the thread that streams it and whoever cancels it. A kill only reaches the
/// group while the child has not been reaped, so it can never hit a recycled pid.
#[derive(Debug, Default)]
pub struct Handle {
    pid: AtomicU32,
    exited: Mutex<bool>,
    cancelled: AtomicBool,
    timed_out: AtomicBool,
    no_init: AtomicBool,
}

impl Handle {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Cancel the ask: the process group is killed, and the turn ends `cancelled`. Safe before spawn too.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.kill_group();
    }

    pub fn kill_group(&self) {
        let exited = lock(&self.exited);
        let pid = self.pid.load(Ordering::SeqCst);
        if !*exited && pid != 0 {
            // SAFETY: the child is a process-group leader (process_group(0)) and is not reaped while `exited`
            // is false and the lock is held, so -pid is its own group.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }

    /// Has the child exited, without reaping it? The zombie keeps the pid (and so the group id) from being
    /// recycled, which is what lets the caller kill leftover group members (its MCP server, hooks) safely.
    /// `None` means the pid is not a child of ours any more (already reaped): never signal it.
    fn exit_seen(&self) -> Option<bool> {
        let pid = self.pid.load(Ordering::SeqCst);
        if pid == 0 {
            return Some(false);
        }
        // SAFETY: waitid with WNOWAIT only inspects the child; `info` is a zeroed siginfo_t the call fills in.
        unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            let r = libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT);
            if r != 0 {
                return None;
            }
            Some(info.si_pid() != 0)
        }
    }

    /// Reap `child` once it has exited, and mark it so no later kill can reach its pid. The group is killed first,
    /// while the zombie still holds the group id and the lock is held, so no exit path reaps a leader and leaves
    /// its MCP server or asynchronous hooks running. The reap happens only after that non-reaping exit observation.
    fn try_reap(&self, child: &mut Child) -> bool {
        let mut exited = lock(&self.exited);
        if *exited {
            return true;
        }
        match self.exit_seen() {
            Some(false) => return false,
            Some(true) => {
                let pid = self.pid.load(Ordering::SeqCst);
                // SAFETY: the zombie leader is not reaped (we hold `exited` false and the lock), so -pid is its group.
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            None => {}
        }
        if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
            *exited = true;
        }
        *exited
    }
}

/// Start the Lead: argv exactly as built, the scrubbed env, cwd = the run's repo, the framed ask on stdin and
/// nothing else. It takes a `FramedAsk` and no other text type, so no raw string can reach the Lead's stdin:
///
/// ```no_run
/// use std::path::Path;
/// use vajra_desktop::control::lead::frame;
/// use vajra_desktop::control::lead_run::{Handle, spawn};
/// use vajra_desktop::crew::redact::Redactor;
///
/// let framed = frame("01ARZ3NDEKTSV4RRFFQ69G5FAV", "20261106-0911", "hi", &Redactor::default()).unwrap();
/// let _child = spawn(&["/bin/cat".to_string()], Path::new("/"), &[], &framed, &Handle::new());
/// ```
///
/// ```compile_fail,E0308
/// use std::path::Path;
/// use vajra_desktop::control::lead_run::{Handle, spawn};
///
/// let _ = spawn(&["/bin/cat".to_string()], Path::new("/"), &[], "raw text", &Handle::new());
/// ```
///
/// ```compile_fail,E0308
/// use std::path::Path;
/// use vajra_desktop::control::lead_run::{Handle, spawn};
///
/// let _ = spawn(&["/bin/cat".to_string()], Path::new("/"), &[], &String::from("raw text"), &Handle::new());
/// ```
pub fn spawn(
    argv: &[String],
    cwd: &Path,
    env: &[(&'static str, OsString)],
    stdin: &FramedAsk,
    handle: &Arc<Handle>,
) -> Result<Child, LeadError> {
    let (prog, args) = argv.split_first().ok_or_else(|| LeadError::Invalid("argv: empty".into()))?;
    if handle.is_cancelled() {
        return Err(LeadError::Cancelled);
    }
    let mut cmd = Command::new(prog);
    cmd.args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (*k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Its own process group, so one signal reaches the CLI, its MCP server and its hooks.
        .process_group(0);
    let mut child = cmd.spawn()?;
    // Hold the lock across publishing the pid, so a cancel that raced the spawn kills the group right here.
    {
        // A handle can drive several processes in a row (the self-test turns, then the ask): re-arm it.
        let mut exited = lock(&handle.exited);
        *exited = false;
        handle.timed_out.store(false, Ordering::SeqCst);
        handle.no_init.store(false, Ordering::SeqCst);
        handle.pid.store(child.id(), Ordering::SeqCst);
    }
    if handle.is_cancelled() {
        handle.kill_group();
    }
    if let Some(mut si) = child.stdin.take() {
        // Under the pipe buffer (4000 chars of text), so this does not block on a Lead that is not reading yet.
        let _ = si.write_all(stdin.as_bytes());
    }
    Ok(child)
}

fn short(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|x| x.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Map one post-`init` event onto sink lines. Tool inputs are shown, tool results are not (they quote files and
/// worker output); an erroring result shows its error.
fn dispatch(ev: &Value, sink: &mut dyn Sink, done: &mut Done, said: &mut bool) -> bool {
    match ev.get("type").and_then(Value::as_str).unwrap_or("") {
        "assistant" => {
            for c in ev["message"]["content"].as_array().into_iter().flatten() {
                match c.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        for l in c.get("text").and_then(Value::as_str).unwrap_or("").lines() {
                            *said = true;
                            sink.event(Tag::Lead, l);
                        }
                    }
                    Some("tool_use") => {
                        done.tool_attempted = true;
                        let name = c.get("name").and_then(Value::as_str).unwrap_or("?");
                        if let (Some(id), Some(path)) = (c.get("id").and_then(Value::as_str), c["input"].get("file_path").and_then(Value::as_str)) {
                            done.read_paths.insert(id.to_string(), path.to_string());
                        }
                        sink.event(Tag::Tool, &format!("{name} {}", short(&c.get("input").map(Value::to_string).unwrap_or_default(), 200)));
                    }
                    _ => {}
                }
            }
            false
        }
        "user" => {
            for c in ev["message"]["content"].as_array().into_iter().flatten() {
                if c.get("type").and_then(Value::as_str) == Some("tool_result") {
                    if c.get("is_error").and_then(Value::as_bool) == Some(true) {
                        done.denied = true;
                        let msg = text_of(&c["content"]);
                        if msg.contains("denied by your permission settings") {
                            done.permission_denied = true;
                            if let Some(p) = c.get("tool_use_id").and_then(Value::as_str).and_then(|id| done.read_paths.get(id)).cloned() {
                                done.denied_paths.push(p);
                            }
                        }
                        sink.event(Tag::Error, &format!("tool error: {}", short(&crate::crew::redact::redact_provider_text(&msg), 200)));
                    } else {
                        sink.event(Tag::Tool, "result ok");
                    }
                }
            }
            false
        }
        "result" => {
            done.is_error = ev.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            if let Some(a) = ev.get("permission_denials").and_then(Value::as_array).filter(|a| !a.is_empty()) {
                done.denied = true;
                for d in a.iter().filter(|d| d.get("tool_name").and_then(Value::as_str) == Some("Read")) {
                    done.permission_denied = true;
                    if let Some(p) = d["tool_input"].get("file_path").and_then(Value::as_str) {
                        done.denied_paths.push(p.to_string());
                    }
                }
            }
            let res = ev.get("result").and_then(Value::as_str).unwrap_or("");
            if done.is_error {
                done.provider_error = Some(crate::providers::errors::classify(res)).filter(|c| c.kind != crate::providers::errors::Kind::UnexplainedExit);
                sink.event(Tag::Error, &short(&crate::crew::redact::redact_provider_text(res), 300));
            } else if !*said && !res.is_empty() {
                // A turn the CLI handled without a model call (a slash command) still shows its reply.
                for l in res.lines() {
                    sink.event(Tag::Lead, l);
                }
            }
            true
        }
        _ => false,
    }
}

/// Stream one turn to `sink`. Any failure kills the process group first: a mismatched tool list or mode, no
/// `init` in time, a timeout, a cancel, an oversized line, or a stream that ends early.
pub fn drive(mut child: Child, handle: &Arc<Handle>, limits: Limits, sink: &mut dyn Sink) -> Result<Done, LeadError> {
    let Some(stdout) = child.stdout.take() else {
        handle.kill_group();
        return Err(LeadError::Io("no stdout".into()));
    };
    let started = Instant::now();
    let init_seen = Arc::new(AtomicBool::new(false));
    let over = Arc::new(AtomicBool::new(false));
    let dog = {
        let (h, init_seen, over) = (handle.clone(), init_seen.clone(), over.clone());
        std::thread::spawn(move || {
            while !over.load(Ordering::SeqCst) {
                let up = started.elapsed();
                if up >= limits.total {
                    h.timed_out.store(true, Ordering::SeqCst);
                    h.kill_group();
                } else if up >= limits.init && !init_seen.load(Ordering::SeqCst) {
                    h.no_init.store(true, Ordering::SeqCst);
                    h.kill_group();
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        })
    };

    let mut r = BufReader::new(stdout);
    let mut scan = InitScan::default();
    let mut done = Done::default();
    let mut said = false;
    let mut finished = false;
    let mut fatal: Option<LeadError> = None;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = r.by_ref().take(MAX_LINE).read_until(b'\n', &mut buf);
        match n {
            Ok(0) | Err(_) => break,
            Ok(_) if buf.last() != Some(&b'\n') && buf.len() as u64 >= MAX_LINE => {
                fatal = Some(LeadError::Io("line too long".into()));
                break;
            }
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end_matches(['\n', '\r']);
        sink.raw(line);
        let Ok(ev) = serde_json::from_str::<Value>(line) else {
            // Not an event. Before `init` it still counts toward the bound.
            if !init_seen.load(Ordering::SeqCst) {
                if let Err(e) = scan.feed(&Value::Null) {
                    fatal = Some(e);
                    break;
                }
            }
            continue;
        };
        if !init_seen.load(Ordering::SeqCst) {
            match scan.feed(&ev) {
                Ok(Some(info)) => {
                    init_seen.store(true, Ordering::SeqCst);
                    sink.init(&info);
                }
                Ok(None) => {}
                Err(e) => {
                    fatal = Some(e);
                    break;
                }
            }
            continue;
        }
        if dispatch(&ev, sink, &mut done, &mut said) {
            finished = true;
            break;
        }
    }

    if fatal.is_some() || !finished {
        handle.kill_group();
    }
    // Let a finished turn exit by itself, then take the group down with it (MCP server, hooks).
    let grace = Instant::now();
    loop {
        if fatal.is_some() || grace.elapsed() >= EXIT_GRACE {
            handle.kill_group();
        }
        // `try_reap` takes down whatever the CLI left running before it reaps the CLI.
        if handle.try_reap(&mut child) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    over.store(true, Ordering::SeqCst);
    let _ = dog.join();

    if handle.is_cancelled() {
        return Err(LeadError::Cancelled);
    }
    if handle.timed_out.load(Ordering::SeqCst) {
        return Err(LeadError::Timeout);
    }
    if let Some(e) = fatal {
        return Err(e);
    }
    if handle.no_init.load(Ordering::SeqCst) || !init_seen.load(Ordering::SeqCst) {
        return Err(LeadError::NoInit);
    }
    if !finished {
        return Err(LeadError::Io("stream ended before the result".into()));
    }
    Ok(done)
}

#[cfg(test)]
#[path = "lead_run_tests.rs"]
mod tests;
