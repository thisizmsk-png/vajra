//! SPIKE: user terminals (a PTY running the user's login shell), separate from the agent session supervisor.
//!
//! Deliberately NOT built on `sessions::Supervisor`: that one is read-only, line based, sanitised and published on the
//! bus that agents and the control socket can read. A terminal is raw bytes, typed by the human, and never touches the
//! bus, the control service or the MCP surface, so no agent can read or write it by construction.
//!
//! Output goes to a `Sink` (the Tauri channel in main.rs; a collector in tests). Flow control is credit based: the
//! reader stops when more than `HIGH_WATER` bytes are unacknowledged and the UI acks what xterm has parsed, so a
//! runaway `cat` cannot grow the IPC queue without bound.

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

/// Unacknowledged output the reader allows before it waits for the UI.
pub const HIGH_WATER: usize = 1024 * 1024;
pub const LOW_WATER: usize = HIGH_WATER / 2;
const BATCH_WAIT: Duration = Duration::from_millis(4);
const CHUNK: usize = 64 * 1024;
/// After SIGHUP, how long the shell gets before SIGKILL.
pub const KILL_GRACE: Duration = Duration::from_millis(500);
pub const MAX_TERMINALS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermError {
    BadCwd,
    InvalidSize,
    TooMany,
    NotFound,
    ShuttingDown,
    Spawn(String),
    Io(String),
}

impl std::fmt::Display for TermError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadCwd => f.write_str("working directory is not an existing absolute directory"),
            Self::InvalidSize => f.write_str("rows and cols must be non-zero"),
            Self::TooMany => f.write_str("too many open terminals"),
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

#[derive(Debug, Clone)]
pub struct OpenSpec {
    pub cwd: PathBuf,
    pub rows: u16,
    pub cols: u16,
    /// The shell program. `None` is `$SHELL`, then `/bin/sh`. Never taken from UI data.
    pub shell: Option<PathBuf>,
    /// Arguments after the program. Default is `-l` (a login shell reads the user's profile, which sets PATH).
    pub args: Option<Vec<String>>,
    /// The HOME the shell sees (the app's own home; tests use a temp one). `None` keeps the process HOME.
    pub home: Option<PathBuf>,
}

/// The only variables a terminal gets from the app's own environment. A login shell rebuilds the rest (PATH, etc.)
/// from the user's profile; nothing else the app holds (nonces, tokens a launcher exported) is passed on.
pub const ENV_ALLOW: [&str; 5] = ["HOME", "USER", "LOGNAME", "LANG", "SHELL"];

struct Term {
    pid: u32,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    /// Bytes sent to the sink and not yet acknowledged.
    credit: Mutex<usize>,
    credit_cv: Condvar,
    exited: AtomicBool,
    reaped: Mutex<bool>,
    reader: Mutex<Option<JoinHandle<()>>>,
}

/// Every process whose session id is `sid`. An interactive shell gives each background job its own process group, so a
/// signal to the shell's group misses them; the session (the shell's pid, held while any member lives) catches all.
fn session_members(sid: u32) -> Vec<i32> {
    #[cfg(target_os = "macos")]
    {
        let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if n <= 0 {
            return Vec::new();
        }
        let mut pids = vec![0i32; n as usize + 64];
        let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * 4) as libc::c_int) };
        pids.truncate(got.max(0) as usize);
        pids.into_iter().filter(|&p| p > 1 && unsafe { libc::getsid(p) } == sid as i32).collect()
    }
    #[cfg(target_os = "linux")]
    {
        let Ok(rd) = std::fs::read_dir("/proc") else { return Vec::new() };
        rd.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<i32>().ok())
            .filter(|&p| p > 1 && unsafe { libc::getsid(p) } == sid as i32)
            .collect()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = sid;
        Vec::new()
    }
}

impl Term {
    fn signal_session(&self, sig: libc::c_int) {
        if self.pid > 1 {
            for p in session_members(self.pid) {
                unsafe { libc::kill(p, sig) };
            }
        }
    }

    fn signal(&self, sig: libc::c_int) {
        let reaped = lock(&self.reaped);
        if !*reaped {
            // The child is a session leader (portable-pty calls setsid), so -pid is its whole process group.
            unsafe {
                libc::kill(-(self.pid as i32), sig);
                libc::kill(self.pid as i32, sig);
            }
        }
    }
}

pub struct Terminals {
    map: Mutex<HashMap<u32, Arc<Term>>>,
    next: AtomicU32,
    closing: AtomicBool,
}

impl Default for Terminals {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminals {
    pub fn new() -> Self {
        Self { map: Mutex::new(HashMap::new()), next: AtomicU32::new(1), closing: AtomicBool::new(false) }
    }

    pub fn count(&self) -> usize {
        lock(&self.map).len()
    }

    fn get(&self, id: u32) -> Result<Arc<Term>, TermError> {
        lock(&self.map).get(&id).cloned().ok_or(TermError::NotFound)
    }

    pub fn open(&self, spec: OpenSpec, sink: Sink) -> Result<u32, TermError> {
        if self.closing.load(Ordering::SeqCst) {
            return Err(TermError::ShuttingDown);
        }
        if !spec.cwd.is_absolute() || !spec.cwd.is_dir() {
            return Err(TermError::BadCwd);
        }
        if spec.rows == 0 || spec.cols == 0 {
            return Err(TermError::InvalidSize);
        }
        if self.count() >= MAX_TERMINALS {
            return Err(TermError::TooMany);
        }
        let pair = native_pty_system()
            .openpty(PtySize { rows: spec.rows, cols: spec.cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| TermError::Io(e.to_string()))?;
        let shell = spec
            .shell
            .clone()
            .or_else(|| std::env::var_os("SHELL").map(PathBuf::from).filter(|p| p.is_absolute()))
            .unwrap_or_else(|| PathBuf::from("/bin/sh"));
        let mut argv: Vec<OsString> = vec![shell.into_os_string()];
        argv.extend(spec.args.clone().unwrap_or_else(|| vec!["-l".into()]).into_iter().map(OsString::from));
        // argv, never a shell command line: nothing from the UI is ever parsed by a shell.
        let mut cmd = CommandBuilder::from_argv(argv);
        cmd.env_clear();
        for k in ENV_ALLOW {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        if let Some(h) = &spec.home {
            cmd.env("HOME", h);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.cwd(&spec.cwd);
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| TermError::Spawn(e.to_string()))?;
        drop(pair.slave); // so the reader sees EOF when the shell and its group are gone
        let pid = child.process_id().unwrap_or(0);
        let io = (pair.master.try_clone_reader(), pair.master.take_writer());
        let (reader, writer) = match io {
            (Ok(r), Ok(w)) => (r, w),
            (Err(e), _) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(TermError::Io(e.to_string()));
            }
            (_, Err(e)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(TermError::Io(e.to_string()));
            }
        };
        let term = Arc::new(Term {
            pid,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            credit: Mutex::new(0),
            credit_cv: Condvar::new(),
            exited: AtomicBool::new(false),
            reaped: Mutex::new(false),
            reader: Mutex::new(None),
        });
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let handle = {
            let term = term.clone();
            std::thread::Builder::new()
                .name(format!("term-{id}"))
                .spawn(move || read_loop(reader, child, &term, &sink))
                .map_err(|e| TermError::Io(e.to_string()))?
        };
        *lock(&term.reader) = Some(handle);
        lock(&self.map).insert(id, term);
        Ok(id)
    }

    /// Keystrokes typed by the human. The caller is the UI's own terminal view; there is no other path in.
    pub fn write(&self, id: u32, data: &[u8]) -> Result<(), TermError> {
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
            .map_err(|e| TermError::Io(e.to_string()))
    }

    /// The UI parsed `n` more bytes: give the reader that much credit back.
    pub fn ack(&self, id: u32, n: usize) -> Result<(), TermError> {
        let t = self.get(id)?;
        let mut c = lock(&t.credit);
        *c = c.saturating_sub(n);
        t.credit_cv.notify_all();
        Ok(())
    }

    /// Close a tab: hang up the shell's process group, kill what ignores that, reap, and join the reader.
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

    /// Tests: whether the shell of terminal `id` has been reaped.
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
    t.signal_session(libc::SIGHUP);
    let deadline = Instant::now() + KILL_GRACE;
    while !t.exited.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !t.exited.load(Ordering::SeqCst) {
        t.signal(libc::SIGKILL);
        t.signal_session(libc::SIGKILL);
    }
    // Unblock a reader that is waiting for credit, then wait for it.
    *lock(&t.credit) = 0;
    t.credit_cv.notify_all();
    if let Some(h) = lock(&t.reader).take() {
        let _ = h.join();
    }
    // Anything the shell left in its session (a `sleep &` in its own job group) goes with it.
    t.signal_session(libc::SIGKILL);
}

/// Output the reader has taken from the PTY and the flusher has not sent yet.
#[derive(Default)]
struct Pending {
    buf: Vec<u8>,
    eof: bool,
}

/// A PTY hands out ~1 KiB per read (measured), so a `cat` of 50 MB would be ~50,000 IPC messages. The reader appends
/// to `pending`; this thread sends it when it holds CHUNK bytes or BATCH_WAIT has passed since the first byte, so
/// typing echo is still immediate (one small message) and a flood travels in large messages.
fn flush_loop(pending: &(Mutex<Pending>, Condvar), sink: &Sink) {
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
            p = cv.wait_timeout(p, BATCH_WAIT - first.elapsed()).unwrap_or_else(|e| e.into_inner()).0;
        }
        let take = p.buf.len().min(CHUNK * 4);
        let data: Vec<u8> = p.buf.drain(..take).collect();
        drop(p);
        sink(Event::Data(data));
    }
}

fn read_loop(mut reader: Box<dyn Read + Send>, mut child: Box<dyn portable_pty::Child + Send + Sync>, t: &Term, sink: &Sink) {
    let pending = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
    let flusher = {
        let (pending, sink) = (pending.clone(), sink.clone());
        std::thread::Builder::new().name("term-flush".into()).spawn(move || flush_loop(&pending, &sink)).ok()
    };
    let mut buf = vec![0u8; CHUNK];
    loop {
        {
            // Stop at the high-water mark and resume at the low-water mark, so acks do not turn into stop-and-go.
            let mut c = lock(&t.credit);
            if *c >= HIGH_WATER {
                while *c > LOW_WATER && !t.exited.load(Ordering::SeqCst) {
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
    sink(Event::Exit(code));
}

/// `cwd` as the UI may give it: an absolute path to an existing directory (production: also one of the app's known
/// project or worktree dirs).
pub fn is_project_dir(p: &Path) -> bool {
    p.is_absolute() && p.is_dir()
}

#[cfg(test)]
mod tests;
