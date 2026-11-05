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

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::bus::{self, Bus};
use crate::logging::Logger;

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
    NotFound,
    AlreadyRunning,
    /// Session ids are bus stream ids.
    InvalidId,
    EmptyArgv,
    NulInArgv,
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
            Self::NotFound => f.write_str("no such session"),
            Self::AlreadyRunning => f.write_str("session is already running"),
            Self::InvalidId => f.write_str("invalid session id"),
            Self::EmptyArgv => f.write_str("argv is empty"),
            Self::NulInArgv => f.write_str("argv contains NUL"),
            Self::BadCwd => f.write_str("working directory does not exist"),
            Self::InvalidSize => f.write_str("rows and cols must be non-zero"),
            Self::ShuttingDown => f.write_str("app is quitting"),
            Self::Spawn(e) => write!(f, "spawn failed: {e}"),
            Self::Io(e) => write!(f, "pty i/o failed: {e}"),
        }
    }
}

impl std::error::Error for SessionError {}

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    /// argv[0] is resolved on the scrubbed PATH.
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub rows: u16,
    pub cols: u16,
    /// Secrets (the steer nonce in the launch prompt, which the TUI echoes)
    /// replaced before a line reaches the bus, spill or IPC.
    pub redact: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitInfo {
    pub code: u32,
    /// Signal description when the child died from one.
    pub signal: Option<String>,
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

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
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
}

impl Session {
    fn signal(&self, sig: libc::c_int) {
        let exit = lock(&self.exit);
        if exit.is_none() {
            // The child is a session leader (setsid), so -pid is its group.
            unsafe {
                libc::kill(-(self.pid as i32), sig);
                libc::kill(self.pid as i32, sig);
            }
        }
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
}

impl Supervisor {
    pub fn new(bus: Arc<Bus>, log: Option<Arc<Logger>>) -> Self {
        Self {
            bus,
            log,
            sessions: Mutex::new(HashMap::new()),
            closing: AtomicBool::new(false),
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
        for (k, v) in scrubbed_env(|k| std::env::var_os(k)) {
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
        });
        let handle = {
            let session = session.clone();
            let bus = self.bus.clone();
            let log = self.log.clone();
            let stream = id.to_string();
            let redact: Vec<String> = spec.redact.iter().filter(|s| s.len() >= REDACT_MIN).cloned().collect();
            std::thread::Builder::new()
                .name(format!("pty-{id}"))
                .spawn(move || read_loop(&bus, &stream, &redact, reader, child, &session, log.as_deref()))
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
        if !s.take_over.load(Ordering::SeqCst) {
            self.log("session_write_refused", &[("pid", u64::from(s.pid))]);
            return Err(SessionError::ReadOnly);
        }
        let mut w = lock(&s.writer);
        w.write_all(data)
            .and_then(|_| w.flush())
            .map_err(|e| SessionError::Io(e.to_string()))
    }

    pub fn set_take_over(&self, id: &str, on: bool) -> Result<(), SessionError> {
        let s = self.get(id)?;
        s.take_over.store(on, Ordering::SeqCst);
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

    /// Exit status once reaped; `None` if still running after `timeout`.
    pub fn wait(&self, id: &str, timeout: Duration) -> Result<Option<ExitInfo>, SessionError> {
        Ok(self.get(id)?.wait_until(Instant::now() + timeout))
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

/// Shortest secret fragment redacted at a line wrap.
const REDACT_MIN: usize = 8;

/// Replace each secret, and a fragment of at least `REDACT_MIN` chars that a
/// terminal wrap left at the end (secret prefix) or start (secret suffix) of
/// the line.
fn redact_line(mut s: String, secrets: &[String]) -> String {
    const R: &str = "[redacted]";
    for n in secrets {
        if s.contains(n.as_str()) {
            s = s.replace(n.as_str(), R);
        }
        for k in (REDACT_MIN..n.len()).rev() {
            if s.ends_with(&n[..k]) {
                s.truncate(s.len() - k);
                s.push_str(R);
                break;
            }
        }
        for k in (REDACT_MIN..n.len()).rev() {
            if s.starts_with(&n[n.len() - k..]) {
                s.replace_range(..k, R);
                break;
            }
        }
    }
    s
}

fn push_line(bus: &Bus, stream: &str, secrets: &[String], raw: &[u8]) {
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    let text = String::from_utf8_lossy(raw);
    let clean = ansi::sanitize(&text);
    let clean = if secrets.is_empty() { clean } else { redact_line(clean, secrets) };
    // The id was validated at spawn, so push cannot fail.
    let _ = bus.push(stream, &clean);
}

fn read_loop(
    bus: &Bus,
    stream: &str,
    secrets: &[String],
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn Child + Send + Sync>,
    session: &Session,
    log: Option<&Logger>,
) {
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                let mut start = 0;
                while let Some(i) = pending[start..].iter().position(|&b| b == b'\n') {
                    push_line(bus, stream, secrets, &pending[start..start + i]);
                    start += i + 1;
                }
                pending.drain(..start);
                if pending.len() >= MAX_PENDING {
                    push_line(bus, stream, secrets, &pending);
                    pending.clear();
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    if !pending.is_empty() {
        push_line(bus, stream, secrets, &pending);
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
                *exit = Some(info);
                session.exited.notify_all();
                return;
            }
            Ok(None) => {}
            Err(_) => {
                *exit = Some(ExitInfo {
                    code: u32::MAX,
                    signal: None,
                });
                session.exited.notify_all();
                return;
            }
        }
        drop(exit);
        std::thread::sleep(Duration::from_millis(20));
    }
}
