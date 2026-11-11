//! C3 session supervisor.
//!
//! Runs the run session and the Lead chat session in PTYs. Spawn takes an
//! argv array and never a shell (NFR-SEC-4); the child sees only the
//! `ENV_ALLOW` variables. One reader thread per PTY splits output into lines,
//! sanitizes them (`ansi::sanitize`) and pushes them to the C2 bus under the
//! session id; once the PTY closes it reaps the child. Sessions are
//! read-only: `write` is refused unless "Take over" is on (AC-2.2.4).
//!
//! Lines, not bytes: output with no trailing newline is held until the next
//! newline, 64 KiB, or exit.

pub mod ansi;
pub mod argv;

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::bus::{self, Bus};
use crate::crew::redact::Redactor;
use crate::providers::install::env::SetupEnv;
use crate::logging::Logger;
use crate::providers::errors::{self, Ident, ProviderError};

#[cfg(test)]
mod tests;

/// The only environment variables a session sees.
pub const ENV_ALLOW: [&str; 6] = ["PATH", "HOME", "LANG", "TERM", "USER", "SHELL"];
/// TERM when the app itself has none (a GUI launch often does not).
pub const DEFAULT_TERM: &str = "xterm-256color";
/// After SIGHUP, how long a session gets before SIGKILL.
pub const KILL_GRACE: Duration = Duration::from_millis(1000);
/// Longest line the reader holds before flushing it unterminated.
const MAX_PENDING: usize = bus::MAX_LINE_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// `write` without "Take over" (AC-2.2.4).
    ReadOnly,
    /// `submit_fixed_turn` while the human has the keyboard (Take over is on).
    TakeOverActive,
    NotFound,
    AlreadyRunning,
    /// Session ids are bus stream ids.
    InvalidId,
    EmptyArgv,
    NulInArgv,
    /// `extra_env` names a variable outside `EXTRA_ENV_ALLOW`, twice, or holds a NUL.
    BadEnv,
    /// cwd is missing or not a directory. (portable-pty would silently fall
    /// back to HOME, which would launch the run in the wrong repo.)
    BadCwd,
    InvalidSize,
    ShuttingDown,
    Spawn(String),
    Io(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadOnly => f.write_str("session is read-only; press Take over to type"),
            Self::TakeOverActive => f.write_str("Take over is on; turn it off, then approve"),
            Self::NotFound => f.write_str("no such session"),
            Self::AlreadyRunning => f.write_str("session is already running"),
            Self::InvalidId => f.write_str("invalid session id"),
            Self::EmptyArgv => f.write_str("argv is empty"),
            Self::NulInArgv => f.write_str("argv contains NUL"),
            Self::BadEnv => f.write_str("extra environment is not allowed"),
            Self::BadCwd => f.write_str("working directory does not exist"),
            Self::InvalidSize => f.write_str("rows and cols must be non-zero"),
            Self::ShuttingDown => f.write_str("app is quitting"),
            Self::Spawn(e) => write!(f, "spawn failed: {e}"),
            Self::Io(e) => write!(f, "pty i/o failed: {e}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// `S.Action` of `workflows/vajra-deliver.js`: the only words a plan's `action` (and so an approval turn) can hold.
pub const PLAN_ACTIONS: [&str; 9] = ["push", "pull_request", "deploy", "migration", "data_deletion", "secrets", "spend", "external_message", "other"];

/// `Execute approved plan <run_id>.<index> (<action>)`: the only text `submit_fixed_turn` can type. It can be built only
/// from a run id that matches `YYYYMMDD-HHMM`, a small index and one of the fixed `PLAN_ACTIONS`, so it holds no free text.
///
/// The action is the one in the entry the human approved. The session executes the plan it holds at that index, which
/// may differ from what the inbox showed (the result file is worker-writable); naming the action makes such a mismatch
/// visible to the session and in its terminal instead of silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedPlanTurn(String);

impl ApprovedPlanTurn {
    pub const MAX_INDEX: usize = 99;

    pub fn new(run_id: &str, index: usize, action: &str) -> Option<Self> {
        (crate::ids::valid_run_id(run_id) && index <= Self::MAX_INDEX && PLAN_ACTIONS.contains(&action))
            .then(|| Self(format!("Execute approved plan {run_id}.{index} ({action})")))
    }

    pub fn text(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    /// argv[0] is resolved on the scrubbed PATH.
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub rows: u16,
    pub cols: u16,
    /// The steer nonce in the launch prompt (the TUI echoes it): replaced,
    /// with the C5 redactor, before a line reaches the bus, spill or IPC.
    pub redact: Redactor,
    /// Variables added to `ENV_ALLOW` for this launch: only names in `EXTRA_ENV_ALLOW` (a Vajra-owned `CODEX_HOME` or
    /// `CLAUDE_CONFIG_DIR`, and the per-ticket `TMPDIR`), never a key. Any other name refuses the spawn.
    pub extra_env: Vec<(&'static str, OsString)>,
}

/// The bus stream prefix reserved for one-click installs (`Supervisor::spawn_setup`).
pub const SETUP_STREAM: &str = "setup-";

/// The only names `SpawnSpec::extra_env` may set (ADR-16 16.8).
pub const EXTRA_ENV_ALLOW: [&str; 3] = ["CODEX_HOME", "CLAUDE_CONFIG_DIR", "TMPDIR"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitInfo {
    pub code: u32,
    /// Signal description when the child died from one.
    pub signal: Option<String>,
}

/// `scrubbed_env` of this process with `PATH` replaced by the user's resolved PATH (`crate::userpath`), so a GUI
/// launch spawns `claude` from where the user's terminal would find it. Absolute dirs only: never the cwd.
pub fn user_env() -> Vec<(&'static str, OsString)> {
    let path = crate::userpath::joined();
    scrubbed_env(|k| if k == "PATH" { path.clone() } else { std::env::var_os(k) })
}

/// The allowed variables present in `get`, in `ENV_ALLOW` order, with TERM
/// defaulted.
pub fn scrubbed_env(get: impl Fn(&str) -> Option<OsString>) -> Vec<(&'static str, OsString)> {
    ENV_ALLOW
        .iter()
        .filter_map(|&k| match get(k) {
            Some(v) => Some((k, v)),
            None if k == "TERM" => Some((k, OsString::from(DEFAULT_TERM))),
            None => None,
        })
        .collect()
}

use crate::lock;

/// Each name at most once, each in `EXTRA_ENV_ALLOW`, each value non-empty with no NUL.
fn extra_env_ok(env: &[(&'static str, OsString)]) -> bool {
    use std::os::unix::ffi::OsStrExt;
    env.iter().enumerate().all(|(i, (k, v))| {
        EXTRA_ENV_ALLOW.contains(k) && !v.is_empty() && !v.as_bytes().contains(&0) && !env[..i].iter().any(|(o, _)| o == k)
    })
}

struct Session {
    pid: u32,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    take_over: AtomicBool,
    /// `Some` once reaped. The reaper only reaps while holding this lock and
    /// `signal` only signals while holding it, so a signal never reaches a
    /// recycled pid.
    exit: Mutex<Option<ExitInfo>>,
    exited: Condvar,
    reader: Mutex<Option<JoinHandle<()>>>,
    /// Vajra asked this process to stop (`kill`, `shutdown`): its exit is ours, not a failure to report.
    stop_asked: AtomicBool,
    /// The last `TAIL_LINES` lines (sanitized and redacted), kept once the process is gone for the core's retry rule.
    tail: Mutex<Vec<String>>,
}

/// Whether a process shows as stopped (`SSTOP`), from the kernel's process info; `None` where it can't be read.
#[cfg(target_os = "macos")]
fn leader_stopped(pid: u32) -> Option<bool> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n = unsafe { libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
    (n == size).then_some(info.pbi_status == 4)
}

#[cfg(not(target_os = "macos"))]
fn leader_stopped(_pid: u32) -> Option<bool> {
    None
}

impl Session {
    fn signal(&self, sig: libc::c_int) {
        self.stop_asked.store(true, Ordering::SeqCst);
        let exit = lock(&self.exit);
        if exit.is_none() {
            // The child is a session leader (setsid), so -pid is its group.
            unsafe {
                libc::kill(-(self.pid as i32), sig);
                libc::kill(self.pid as i32, sig);
            }
        }
    }

    /// Job control (`SIGSTOP`/`SIGCONT`) for a live session's whole group, under the exit lock like `signal`. It never
    /// marks the session as asked to stop. True when the session was live and signalled.
    fn job_signal(&self, sig: libc::c_int) -> bool {
        let exit = lock(&self.exit);
        if exit.is_some() {
            return false;
        }
        unsafe {
            libc::kill(-(self.pid as i32), sig);
            libc::kill(self.pid as i32, sig);
        }
        true
    }

    /// `SIGSTOP` the live session's group and confirm the leader shows as stopped. A stop sent while the child is still
    /// starting (between fork and exec) can be lost, so it is sent again, for up to about 200 ms (#146 review: a
    /// session spawned during a core call is paused too). True when the session was live.
    fn stop(&self) -> bool {
        let exit = lock(&self.exit);
        if exit.is_some() {
            return false;
        }
        for _ in 0..40 {
            unsafe {
                libc::kill(-(self.pid as i32), libc::SIGSTOP);
                libc::kill(self.pid as i32, libc::SIGSTOP);
            }
            if leader_stopped(self.pid) != Some(false) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }

    fn wait_until(&self, deadline: Instant) -> Option<ExitInfo> {
        let mut exit = lock(&self.exit);
        while exit.is_none() {
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            exit = self
                .exited
                .wait_timeout(exit, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        exit.clone()
    }

    fn join_reader_if_done(&self) {
        let mut r = lock(&self.reader);
        if r.as_ref().is_some_and(|h| h.is_finished()) {
            let _ = r.take().map(JoinHandle::join);
        }
    }
}

pub struct Supervisor {
    bus: Arc<Bus>,
    log: Option<Arc<Logger>>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    closing: AtomicBool,
    /// Where a worker's failed exit is reported (H9). Unset: nothing is reported.
    errors: Mutex<Option<ErrorSink>>,
}

/// Receives the `provider_error` event of a worker that died. The app installs one that tells the lead and the DG.
pub type ErrorSink = Arc<dyn Fn(&ProviderError) + Send + Sync>;

/// What the reader thread does when its child is gone.
struct ExitHooks {
    log: Option<Arc<Logger>>,
    report: Option<(ProviderTag, ErrorSink)>,
}

/// Who a spawned worker is, for its `provider_error` event. Owned by the session for its whole life: nothing here can be
/// changed after spawn, so a failure is always reported against the account it ran on (H7).
#[derive(Debug, Clone)]
pub struct ProviderTag {
    pub slot: String,
    pub session_id: String,
    pub account: String,
    /// 1 for the first launch of a ticket, 2 and 3 for the core's retries (6c).
    pub attempt: u32,
    /// The core reports this worker's failures itself (6c), after it has decided whether to retry: `report_exit` stays
    /// quiet, so the card never says "retrying" for a ticket the core ends.
    pub core_reports: bool,
}

impl Supervisor {
    pub fn new(bus: Arc<Bus>, log: Option<Arc<Logger>>) -> Self {
        Self {
            bus,
            log,
            sessions: Mutex::new(HashMap::new()),
            closing: AtomicBool::new(false),
            errors: Mutex::new(None),
        }
    }

    pub fn set_error_sink(&self, sink: ErrorSink) {
        *lock(&self.errors) = Some(sink);
    }

    /// A `provider_error` the core decided itself (a `core_reports` worker), to the same sink.
    pub fn report(&self, e: &ProviderError) {
        let sink = lock(&self.errors).clone();
        if let Some(sink) = sink {
            sink(e);
        }
    }

    fn log(&self, event: &'static str, fields: &[(&'static str, u64)]) {
        if let Some(l) = &self.log {
            let _ = l.log(event, fields);
        }
    }

    fn get(&self, id: &str) -> Result<Arc<Session>, SessionError> {
        lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or(SessionError::NotFound)
    }

    /// Start `spec.argv` in a new PTY; its output goes to bus stream `id`.
    /// An exited session's id can be reused.
    pub fn spawn(&self, id: &str, spec: SpawnSpec) -> Result<(), SessionError> {
        self.spawn_tagged(id, spec, None)
    }

    /// `spawn` for a provider worker: a failed exit is reported as a `provider_error` against `tag`.
    pub fn spawn_tagged(&self, id: &str, spec: SpawnSpec, tag: Option<ProviderTag>) -> Result<(), SessionError> {
        if id.starts_with(SETUP_STREAM) {
            return Err(SessionError::InvalidId);
        }
        if !extra_env_ok(&spec.extra_env) {
            return Err(SessionError::BadEnv);
        }
        let mut env = user_env();
        env.extend(spec.extra_env.iter().cloned());
        self.start(id, &spec, &env, tag)
    }

    /// A one-click install (ADR-17, bhishma condition 8) on bus stream `setup-<id>`: exactly the Setup env profile,
    /// never `user_env`, and the profile's proxy value and login are redacted from every line. Only `setup-` streams.
    pub fn spawn_setup(&self, id: &str, argv: Vec<String>, cwd: &Path, env: &SetupEnv, redact: Redactor) -> Result<(), SessionError> {
        if !id.starts_with(SETUP_STREAM) {
            return Err(SessionError::InvalidId);
        }
        let mut r = crate::providers::install::env::redactor(env);
        r.absorb(&redact);
        let spec = SpawnSpec { argv, cwd: cwd.to_path_buf(), rows: 24, cols: 100, redact: r, extra_env: Vec::new() };
        self.start(id, &spec, env.vars(), None)
    }

    fn start(&self, id: &str, spec: &SpawnSpec, env: &[(&'static str, OsString)], tag: Option<ProviderTag>) -> Result<(), SessionError> {
        if self.closing.load(Ordering::SeqCst) {
            return Err(SessionError::ShuttingDown);
        }
        if !bus::valid_stream_id(id) {
            return Err(SessionError::InvalidId);
        }
        if spec.argv.is_empty() {
            return Err(SessionError::EmptyArgv);
        }
        if spec.argv.iter().any(|a| a.contains('\0')) {
            return Err(SessionError::NulInArgv);
        }
        if !spec.cwd.is_dir() {
            return Err(SessionError::BadCwd);
        }
        if spec.rows == 0 || spec.cols == 0 {
            return Err(SessionError::InvalidSize);
        }
        let mut sessions = lock(&self.sessions);
        if let Some(old) = sessions.get(id) {
            if lock(&old.exit).is_none() {
                return Err(SessionError::AlreadyRunning);
            }
            old.join_reader_if_done();
        }

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: spec.rows,
                cols: spec.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| SessionError::Io(e.to_string()))?;

        // CommandBuilder::from_argv runs argv directly; only new_default_prog
        // would start a shell, and we never call it.
        let mut cmd = CommandBuilder::from_argv(spec.argv.iter().map(OsString::from).collect());
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.cwd(&spec.cwd);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| SessionError::Spawn(e.to_string()))?;
        // Drop our slave fd so the reader sees EOF when the child is gone.
        drop(pair.slave);
        let pid = child.process_id().unwrap_or(0);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| SessionError::Io(e.to_string()));
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| SessionError::Io(e.to_string()));
        let (reader, writer) = match (reader, writer) {
            (Ok(r), Ok(w)) => (r, w),
            (Err(e), _) | (_, Err(e)) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };

        let session = Arc::new(Session {
            pid,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            take_over: AtomicBool::new(false),
            exit: Mutex::new(None),
            exited: Condvar::new(),
            reader: Mutex::new(None),
            stop_asked: AtomicBool::new(false),
            tail: Mutex::new(Vec::new()),
        });
        let handle = {
            let session = session.clone();
            let bus = self.bus.clone();
            let log = self.log.clone();
            let stream = id.to_string();
            let redact = spec.redact.clone();
            let report = tag.and_then(|t| lock(&self.errors).clone().map(|sink| (t, sink)));
            std::thread::Builder::new()
                .name(format!("pty-{id}"))
                .spawn(move || read_loop(&bus, &stream, &redact, reader, child, &session, ExitHooks { log, report }))
                .map_err(|e| SessionError::Io(e.to_string()))?
        };
        *lock(&session.reader) = Some(handle);
        sessions.insert(id.to_string(), session);
        self.log("session_spawn", &[("pid", u64::from(pid))]);
        Ok(())
    }

    /// Send keystrokes. Refused unless take_over is on.
    pub fn write(&self, id: &str, data: &[u8]) -> Result<(), SessionError> {
        let s = self.get(id)?;
        // The mode is checked under the writer lock, and `set_take_over` changes it under the same lock, so a write
        // that was allowed cannot stay queued across the mode turning off.
        let mut w = lock(&s.writer);
        if !s.take_over.load(Ordering::SeqCst) {
            self.log("session_write_refused", &[("pid", u64::from(s.pid))]);
            return Err(SessionError::ReadOnly);
        }
        w.write_all(data)
            .and_then(|_| w.flush())
            .map_err(|e| SessionError::Io(e.to_string()))
    }

    /// Type one fixed turn into a running session that is NOT in Take over (ADR-14). The bytes are the turn's own
    /// fixed text plus Enter; the caller has no way to pass text, so a plan, a reason or any stream text can never be
    /// typed here. The take-over check runs under the writer lock, so it cannot interleave with the human typing.
    pub fn submit_fixed_turn(&self, id: &str, turn: &ApprovedPlanTurn) -> Result<(), SessionError> {
        let s = self.get(id)?;
        let mut w = lock(&s.writer);
        if s.take_over.load(Ordering::SeqCst) {
            return Err(SessionError::TakeOverActive);
        }
        w.write_all(turn.text().as_bytes())
            .and_then(|_| w.write_all(b"\r"))
            .and_then(|_| w.flush())
            .map_err(|e| SessionError::Io(e.to_string()))
    }

    pub fn set_take_over(&self, id: &str, on: bool) -> Result<(), SessionError> {
        let s = self.get(id)?;
        {
            // Serialised with every delivery: a write or fixed turn in progress finishes first, and none that starts
            // later sees the old mode.
            let _w = lock(&s.writer);
            s.take_over.store(on, Ordering::SeqCst);
        }
        self.log(
            "session_take_over",
            &[("pid", u64::from(s.pid)), ("on", u64::from(on))],
        );
        Ok(())
    }

    pub fn take_over(&self, id: &str) -> Result<bool, SessionError> {
        Ok(self.get(id)?.take_over.load(Ordering::SeqCst))
    }

    pub fn resize(&self, id: &str, rows: u16, cols: u16) -> Result<(), SessionError> {
        if rows == 0 || cols == 0 {
            return Err(SessionError::InvalidSize);
        }
        let s = self.get(id)?;
        lock(&s.master)
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| SessionError::Io(e.to_string()))
    }

    /// (rows, cols) of the PTY.
    pub fn size(&self, id: &str) -> Result<(u16, u16), SessionError> {
        let s = self.get(id)?;
        let size = lock(&s.master)
            .get_size()
            .map_err(|e| SessionError::Io(e.to_string()))?;
        Ok((size.rows, size.cols))
    }

    pub fn pid(&self, id: &str) -> Result<u32, SessionError> {
        Ok(self.get(id)?.pid)
    }

    pub fn is_running(&self, id: &str) -> Result<bool, SessionError> {
        Ok(lock(&self.get(id)?.exit).is_none())
    }

    /// The session's last lines, sanitized and redacted, once its process is gone (empty before).
    pub fn last_lines(&self, id: &str) -> Result<Vec<String>, SessionError> {
        Ok(lock(&self.get(id)?.tail).clone())
    }

    /// Whether Vajra asked this session to stop (`kill`): its exit is ours, not the worker's failure.
    pub fn stop_asked(&self, id: &str) -> Result<bool, SessionError> {
        Ok(self.get(id)?.stop_asked.load(Ordering::SeqCst))
    }

    /// Exit status once reaped; `None` if still running after `timeout`.
    pub fn wait(&self, id: &str, timeout: Duration) -> Result<Option<ExitInfo>, SessionError> {
        Ok(self.get(id)?.wait_until(Instant::now() + timeout))
    }

    /// Stop the process groups of these live sessions (`SIGSTOP`) while the core's git touches a repo they share
    /// (ADR-16 16.6). Returns the ids it stopped; unknown or exited ids are skipped. Pair with `resume`.
    pub fn pause(&self, ids: &[String]) -> Vec<String> {
        ids.iter().filter(|id| self.get(id).is_ok_and(|s| s.stop())).cloned().collect()
    }

    /// `SIGSTOP` these sessions again, without waiting: the kernel continues an orphaned process group when one of its
    /// members exits while it is stopped (POSIX), and every session leader's group is orphaned. A pause holder calls
    /// this every few milliseconds while it holds the pause.
    pub fn restop(&self, ids: &[String]) {
        for id in ids {
            if let Ok(s) = self.get(id) {
                s.job_signal(libc::SIGSTOP);
            }
        }
    }

    /// `SIGCONT` the sessions `pause` stopped.
    pub fn resume(&self, ids: &[String]) {
        for id in ids {
            if let Ok(s) = self.get(id) {
                s.job_signal(libc::SIGCONT);
            }
        }
    }

    /// SIGHUP the session's process group, SIGKILL after `KILL_GRACE`.
    /// A no-op on an exited session.
    pub fn kill(&self, id: &str) -> Result<(), SessionError> {
        let s = self.get(id)?;
        self.log("session_kill", &[("pid", u64::from(s.pid))]);
        s.signal(libc::SIGHUP);
        if s.wait_until(Instant::now() + KILL_GRACE).is_none() {
            s.signal(libc::SIGKILL);
        }
        Ok(())
    }

    /// App quit: kill every session, wait for the reapers, join the readers.
    /// Later spawns fail with `ShuttingDown`.
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let all: Vec<Arc<Session>> = lock(&self.sessions).values().cloned().collect();
        for s in &all {
            s.signal(libc::SIGHUP);
        }
        let grace = Instant::now() + KILL_GRACE;
        for s in &all {
            if s.wait_until(grace).is_none() {
                s.signal(libc::SIGKILL);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        for s in &all {
            if s.wait_until(deadline).is_none() {
                self.log("session_reap_timeout", &[("pid", u64::from(s.pid))]);
            }
            s.join_reader_if_done();
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn push_line(bus: &Bus, stream: &str, redactor: &Redactor, tail: &mut VecDeque<String>, raw: &[u8]) {
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    let text = String::from_utf8_lossy(raw);
    let clean = redactor.redact(ansi::sanitize(&text));
    if tail.len() == errors::TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(clean.clone());
    // The id was validated at spawn, so push cannot fail.
    let _ = bus.push(stream, &clean);
}

fn read_loop(
    bus: &Bus,
    stream: &str,
    redactor: &Redactor,
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn Child + Send + Sync>,
    session: &Session,
    exit: ExitHooks,
) {
    let ExitHooks { log, report } = exit;
    let log = log.as_deref();
    let mut tail: VecDeque<String> = VecDeque::with_capacity(errors::TAIL_LINES);
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                let mut start = 0;
                while let Some(i) = pending[start..].iter().position(|&b| b == b'\n') {
                    push_line(bus, stream, redactor, &mut tail, &pending[start..start + i]);
                    start += i + 1;
                }
                pending.drain(..start);
                if pending.len() >= MAX_PENDING {
                    push_line(bus, stream, redactor, &mut tail, &pending);
                    pending.clear();
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    if !pending.is_empty() {
        push_line(bus, stream, redactor, &mut tail, &pending);
    }

    // Reap. Poll under the exit lock so `signal` never races the reap.
    loop {
        let mut exit = lock(&session.exit);
        match child.try_wait() {
            Ok(Some(status)) => {
                let info = ExitInfo {
                    code: status.exit_code(),
                    signal: status.signal().map(str::to_string),
                };
                if let Some(l) = log {
                    let _ = l.log(
                        "session_exit",
                        &[
                            ("pid", u64::from(session.pid)),
                            ("code", u64::from(info.code)),
                            ("signaled", u64::from(info.signal.is_some())),
                        ],
                    );
                }
                let failed = (info.code != 0 || info.signal.is_some()) && !session.stop_asked.load(Ordering::SeqCst);
                let (code, signal) = (info.code, info.signal.clone());
                *lock(&session.tail) = tail.iter().cloned().collect();
                *exit = Some(info);
                session.exited.notify_all();
                drop(exit);
                if let (true, Some((tag, sink))) = (failed, report) {
                    report_exit(&tag, &sink, Some(code), signal.as_deref(), tail.make_contiguous());
                }
                return;
            }
            Ok(None) => {}
            Err(_) => {
                *lock(&session.tail) = tail.iter().cloned().collect();
                *exit = Some(ExitInfo {
                    code: u32::MAX,
                    signal: None,
                });
                session.exited.notify_all();
                drop(exit);
                if let (Some((tag, sink)), false) = (report, session.stop_asked.load(Ordering::SeqCst)) {
                    report_exit(&tag, &sink, None, None, tail.make_contiguous());
                }
                return;
            }
        }
        drop(exit);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A worker died: report its own typed error if its last lines held one, else `unexplained_exit` with the exit code and
/// those lines. The action comes from the policy; the account is the one the worker was spawned with. A worker never
/// just disappears.
fn report_exit(tag: &ProviderTag, sink: &ErrorSink, code: Option<u32>, signal: Option<&str>, tail: &[String]) {
    if tag.core_reports {
        return;
    }
    let (c, text) = errors::classify_exit(code, signal, tail);
    let who = Ident { slot: &tag.slot, session_id: &tag.session_id, account: &tag.account };
    let n = tag.attempt.max(1);
    sink(&ProviderError::new(&who, &c, n, &text, errors::decide(c.kind, n, c.retry_after_s).action));
}
