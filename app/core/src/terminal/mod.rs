//! User terminals: a PTY running the user's login shell, typed by the human.
//!
//! This module is a wall, on purpose. It does not use the bus, the control service, the control socket, the MCP surface
//! or the agent session supervisor (`sessions`), so no agent tool can list, read or write a terminal; a test scans this
//! directory's source for those names. Output goes only to the tab's own `Sink` (a Tauri channel in main.rs).
//!
//! * Spawn is argv: the shell program is chosen by the core (`LoginEnv`, else `/bin/sh`), never taken from the UI, and
//!   nothing the UI sends is parsed by a shell. The UI sends a cwd, a size and typed bytes.
//! * The cwd must be a known root or a worker worktree (`policy::place`); the caller passes the roots.
//! * The environment is scrubbed (`policy::ENV_ALLOW` plus TERM, COLORTERM, the resolved PATH and SHELL).
//! * Output is coalesced (64 KiB or 4 ms) and credit flow controlled: the reader stops at `Limits::window` unacknowledged
//!   bytes and resumes at half, so a runaway `cat` cannot grow the IPC queue.
//! * Close hangs up the shell's group and session, kills what ignores that, reaps, and joins. App quit closes all.
//! * A terminal keeps running while its tab is hidden (ADR-15 D9): nothing here depends on a UI. If the UI goes away,
//!   `attach` binds a new sink; until then the reader simply waits at the window and the shell blocks on its output.

mod kill;
pub mod policy;

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::lock;
pub use policy::{Limits, LoginEnv, Placement, ProcessEnv};

const CHUNK: usize = 64 * 1024;
const BATCH_WAIT: Duration = Duration::from_millis(4);
/// After SIGHUP, how long the shell gets before SIGKILL.
pub const KILL_GRACE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermError {
    /// Not an absolute path to an existing directory.
    BadCwd,
    /// A real directory, but not a known root or worker worktree.
    NotAllowed,
    InvalidSize,
    /// The global or per-root cap of open terminals.
    TooMany,
    TooLarge,
    NotFound,
    ShuttingDown,
    Spawn(String),
    Io(String),
}

impl TermError {
    /// The code the IPC layer returns.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadCwd => "bad_cwd",
            Self::NotAllowed => "cwd_not_allowed",
            Self::InvalidSize => "invalid_size",
            Self::TooMany => "too_many_terminals",
            Self::TooLarge => "write_too_large",
            Self::NotFound => "not_found",
            Self::ShuttingDown => "shutting_down",
            Self::Spawn(_) => "spawn_failed",
            Self::Io(_) => "io_error",
        }
    }
}

impl std::fmt::Display for TermError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadCwd => f.write_str("working directory is not an existing absolute directory"),
            Self::NotAllowed => f.write_str("working directory is not a project root or one of its worktrees"),
            Self::InvalidSize => f.write_str("rows and cols must be non-zero"),
            Self::TooMany => f.write_str("too many open terminals; close one first"),
            Self::TooLarge => f.write_str("input is too large; send it in smaller pieces"),
            Self::NotFound => f.write_str("no such terminal"),
            Self::ShuttingDown => f.write_str("app is quitting"),
            Self::Spawn(e) => write!(f, "spawn failed: {e}"),
            Self::Io(e) => write!(f, "pty i/o failed: {e}"),
        }
    }
}

impl std::error::Error for TermError {}

pub enum Event {
    Data(Vec<u8>),
    /// The shell ended (exit code, or None when killed by a signal).
    Exit(Option<i32>),
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// What `list` and `open` say about a terminal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TermInfo {
    pub id: u32,
    pub cwd: PathBuf,
    pub root: PathBuf,
    pub rows: u16,
    pub cols: u16,
    /// False once the shell has ended (the tab can still be closed).
    pub alive: bool,
}

struct Term {
    pid: u32,
    place: Placement,
    size: Mutex<(u16, u16)>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    sink: Mutex<Sink>,
    /// Bytes sent to the sink and not yet acknowledged.
    credit: Mutex<usize>,
    credit_cv: Condvar,
    exited: AtomicBool,
    reaped: Mutex<bool>,
    reader: Mutex<Option<JoinHandle<()>>>,
}

impl Term {
    fn signal(&self, sig: libc::c_int) {
        let reaped = lock(&self.reaped);
        kill::signal_all(self.pid, !*reaped, sig);
    }

    fn send(&self, e: Event) {
        let sink = lock(&self.sink).clone();
        sink(e);
    }

    fn info(&self, id: u32) -> TermInfo {
        let (rows, cols) = *lock(&self.size);
        TermInfo { id, cwd: self.place.cwd.clone(), root: self.place.root.clone(), rows, cols, alive: !self.exited.load(Ordering::SeqCst) }
    }
}

pub struct Terminals {
    map: Mutex<HashMap<u32, Arc<Term>>>,
    next: AtomicU32,
    closing: AtomicBool,
    limits: Limits,
    env: Arc<dyn LoginEnv>,
}

impl Default for Terminals {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminals {
    pub fn new() -> Self {
        Self::with(Limits::default(), Arc::new(ProcessEnv))
    }

    pub fn with(limits: Limits, env: Arc<dyn LoginEnv>) -> Self {
        Self { map: Mutex::new(HashMap::new()), next: AtomicU32::new(1), closing: AtomicBool::new(false), limits, env }
    }

    pub fn count(&self) -> usize {
        lock(&self.map).len()
    }

    fn get(&self, id: u32) -> Result<Arc<Term>, TermError> {
        lock(&self.map).get(&id).cloned().ok_or(TermError::NotFound)
    }

    /// Open a terminal in `cwd`, which must be one of `roots` or a worker worktree of one.
    pub fn open(&self, cwd: &Path, roots: &[PathBuf], rows: u16, cols: u16, sink: Sink) -> Result<TermInfo, TermError> {
        if self.closing.load(Ordering::SeqCst) {
            return Err(TermError::ShuttingDown);
        }
        if rows == 0 || cols == 0 {
            return Err(TermError::InvalidSize);
        }
        let place = policy::place(cwd, roots)?;
        let pair = native_pty_system()
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| TermError::Io(e.to_string()))?;
        let shell = policy::choose_shell(self.env.as_ref());
        // argv, never a command line: nothing from the UI is ever parsed by a shell. `-l` makes it a login shell, which
        // reads the user's profile and so sets the rest of the environment.
        let mut cmd = CommandBuilder::from_argv(vec![shell.into_os_string(), OsString::from("-l")]);
        cmd.env_clear();
        for k in policy::ENV_ALLOW {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        if let Some(p) = self.env.path() {
            cmd.env("PATH", p);
        }
        if let Some(h) = self.env.home() {
            cmd.env("HOME", h);
        }
        cmd.env("SHELL", policy::choose_shell(self.env.as_ref()));
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.cwd(&place.cwd);
        // The cap is checked last and the slot taken under the same lock as the insert below, so two concurrent opens
        // cannot both pass the check.
        let mut map = lock(&self.map);
        if map.len() >= self.limits.max_open || map.values().filter(|t| t.place.root == place.root).count() >= self.limits.max_per_root {
            return Err(TermError::TooMany);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| TermError::Spawn(e.to_string()))?;
        drop(pair.slave); // so the reader sees EOF when the shell and its group are gone
        let pid = child.process_id().unwrap_or(0);
        let io = (pair.master.try_clone_reader(), pair.master.take_writer());
        let (reader, writer) = match io {
            (Ok(r), Ok(w)) => (r, w),
            (Err(e), _) | (_, Err(e)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(TermError::Io(e.to_string()));
            }
        };
        let term = Arc::new(Term {
            pid,
            place,
            size: Mutex::new((rows, cols)),
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            sink: Mutex::new(sink),
            credit: Mutex::new(0),
            credit_cv: Condvar::new(),
            exited: AtomicBool::new(false),
            reaped: Mutex::new(false),
            reader: Mutex::new(None),
        });
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let window = self.limits.window;
        let handle = {
            let term = term.clone();
            std::thread::Builder::new()
                .name(format!("term-{id}"))
                .spawn(move || read_loop(reader, child, &term, window))
                .map_err(|e| TermError::Io(e.to_string()))?
        };
        *lock(&term.reader) = Some(handle);
        let info = term.info(id);
        map.insert(id, term);
        Ok(info)
    }

    /// Every open terminal, oldest first.
    pub fn list(&self) -> Vec<TermInfo> {
        let mut v: Vec<TermInfo> = lock(&self.map).iter().map(|(id, t)| t.info(*id)).collect();
        v.sort_by_key(|i| i.id);
        v
    }

    /// Keystrokes or a paste typed by the human. The caller is the UI's own terminal view; there is no other path in.
    pub fn write(&self, id: u32, data: &[u8]) -> Result<(), TermError> {
        if data.len() > self.limits.max_write {
            return Err(TermError::TooLarge);
        }
        let t = self.get(id)?;
        let mut w = lock(&t.writer);
        w.write_all(data).and_then(|()| w.flush()).map_err(|e| TermError::Io(e.to_string()))
    }

    pub fn resize(&self, id: u32, rows: u16, cols: u16) -> Result<(), TermError> {
        if rows == 0 || cols == 0 {
            return Err(TermError::InvalidSize);
        }
        let t = self.get(id)?;
        lock(&t.master)
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| TermError::Io(e.to_string()))?;
        *lock(&t.size) = (rows, cols);
        Ok(())
    }

    /// The UI parsed `n` more bytes: give the reader that much credit back.
    pub fn ack(&self, id: u32, n: usize) -> Result<(), TermError> {
        let t = self.get(id)?;
        let mut c = lock(&t.credit);
        *c = c.saturating_sub(n);
        t.credit_cv.notify_all();
        Ok(())
    }

    /// Bind a new sink to a terminal that outlived its view (the webview reloaded, or a hidden dock came back). The
    /// unacknowledged count restarts at zero; output produced while no view was attached is not replayed.
    pub fn attach(&self, id: u32, sink: Sink) -> Result<TermInfo, TermError> {
        let t = self.get(id)?;
        *lock(&t.sink) = sink;
        *lock(&t.credit) = 0;
        t.credit_cv.notify_all();
        Ok(t.info(id))
    }

    /// Close a tab: hang up the shell's group and session, kill what ignores that, reap, and join the reader.
    pub fn close(&self, id: u32) -> Result<(), TermError> {
        let t = lock(&self.map).remove(&id).ok_or(TermError::NotFound)?;
        stop(&t);
        Ok(())
    }

    /// App quit: close every terminal. After this `open` is refused.
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let all: Vec<Arc<Term>> = lock(&self.map).drain().map(|(_, t)| t).collect();
        for t in &all {
            t.signal(libc::SIGHUP);
        }
        for t in &all {
            stop(t);
        }
    }

    /// Tests: the shell's pid.
    pub fn pid(&self, id: u32) -> Option<u32> {
        lock(&self.map).get(&id).map(|t| t.pid)
    }
}

impl Drop for Terminals {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn stop(t: &Arc<Term>) {
    t.signal(libc::SIGHUP);
    let deadline = Instant::now() + KILL_GRACE;
    while !t.exited.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !t.exited.load(Ordering::SeqCst) {
        t.signal(libc::SIGKILL);
    }
    // Unblock a reader that is waiting for credit, then wait for it.
    *lock(&t.credit) = 0;
    t.credit_cv.notify_all();
    if let Some(h) = lock(&t.reader).take() {
        let _ = h.join();
    }
    // Anything the shell left in its session (a `sleep &` in its own job group) goes with it.
    t.signal(libc::SIGKILL);
}

/// Output the reader has taken from the PTY and the flusher has not sent yet.
#[derive(Default)]
struct Pending {
    buf: Vec<u8>,
    eof: bool,
}

/// A PTY hands out ~1 KiB per read (measured), so a `cat` of 50 MB would be ~50,000 IPC messages. The reader appends to
/// `pending`; this thread sends it when it holds CHUNK bytes or BATCH_WAIT has passed since the first byte, so typing
/// echo is still immediate (one small message) and a flood travels in large messages. (Polling the PTY fd to batch
/// instead is unreliable on macOS: every poll waited its full timeout.)
fn flush_loop(pending: &(Mutex<Pending>, Condvar), t: &Term) {
    let (m, cv) = pending;
    loop {
        let mut p = lock(m);
        while p.buf.is_empty() && !p.eof {
            p = cv.wait(p).unwrap_or_else(|e| e.into_inner());
        }
        if p.buf.is_empty() {
            return; // eof and drained
        }
        let first = Instant::now();
        while p.buf.len() < CHUNK && !p.eof && first.elapsed() < BATCH_WAIT {
            p = cv.wait_timeout(p, BATCH_WAIT.saturating_sub(first.elapsed())).unwrap_or_else(|e| e.into_inner()).0;
        }
        let take = p.buf.len().min(CHUNK * 4);
        let data: Vec<u8> = p.buf.drain(..take).collect();
        drop(p);
        t.send(Event::Data(data));
    }
}

fn read_loop(mut reader: Box<dyn Read + Send>, mut child: Box<dyn portable_pty::Child + Send + Sync>, t: &Arc<Term>, window: usize) {
    let pending = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
    let flusher = {
        let (pending, t) = (pending.clone(), t.clone());
        std::thread::Builder::new().name("term-flush".into()).spawn(move || flush_loop(&pending, &t)).ok()
    };
    let low = window / 2;
    let mut buf = vec![0u8; CHUNK];
    loop {
        {
            // Stop at the window and resume at half of it, so acks do not turn into stop-and-go.
            let mut c = lock(&t.credit);
            if *c >= window {
                while *c > low && !t.exited.load(Ordering::SeqCst) {
                    c = t.credit_cv.wait_timeout(c, Duration::from_millis(100)).unwrap_or_else(|e| e.into_inner()).0;
                }
            }
        }
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                *lock(&t.credit) += n;
                let (m, cv) = &*pending;
                lock(m).buf.extend_from_slice(&buf[..n]);
                cv.notify_one();
            }
        }
    }
    {
        let (m, cv) = &*pending;
        lock(m).eof = true;
        cv.notify_all();
    }
    if let Some(f) = flusher {
        let _ = f.join();
    }
    // The PTY closed. Reap under the lock `signal` takes, so a signal never reaches a recycled pid.
    let status = child.wait();
    t.exited.store(true, Ordering::SeqCst);
    *lock(&t.reaped) = true;
    let code = status.ok().map(|s| s.exit_code() as i32);
    t.send(Event::Exit(code));
}

#[cfg(test)]
mod harness;
#[cfg(test)]
mod tests;
