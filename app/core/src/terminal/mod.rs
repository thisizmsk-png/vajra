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

pub(crate) mod kill;
pub(crate) mod hidden;
pub mod policy;

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::lock;
pub use policy::{Limits, LoginEnv, Placement, UserPathEnv};

const CHUNK: usize = 64 * 1024;
/// Output that may be read ahead of the pace before the reader starts waiting (so a command's burst of output is free).
pub const RATE_BURST: usize = 256 * 1024;
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
    /// The terminal's input queue is full (the shell is not reading); retry later.
    QueueFull,
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
            Self::QueueFull => "input_queue_full",
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
            Self::QueueFull => f.write_str("the terminal is not reading its input right now"),
            Self::NotFound => f.write_str("no such terminal"),
            Self::ShuttingDown => f.write_str("app is quitting"),
            Self::Spawn(e) => write!(f, "spawn failed: {e}"),
            Self::Io(e) => write!(f, "pty i/o failed: {e}"),
        }
    }
}

impl std::error::Error for TermError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Data(Vec<u8>),
    /// A pane that was hidden is shown again: `bytes` of its output are not coming (only the newest are), and `alt` says
    /// the program entered (`Some(true)`) or left (`Some(false)`) the alternate screen while nobody was looking.
    Skipped { bytes: u64, alt: Option<bool> },
    /// New output arrived in a hidden pane (at most twice a second), for an unread mark on its tab.
    Activity,
    /// The shell ended (exit code, or None when killed by a signal).
    Exit(Option<i32>),
}

/// Where a terminal's output goes. Returns false when nobody is listening any more (the webview is gone).
pub type Sink = Arc<dyn Fn(Event) -> bool + Send + Sync>;

/// How often a hidden pane may report new output.
const ACTIVITY_EVERY: Duration = Duration::from_millis(500);

/// What a pane keeps while nobody is looking at it.
#[derive(Default)]
struct Hide {
    on: bool,
    ring: hidden::Ring,
    alt: hidden::AltTrack,
    /// Whether the view was on the alternate screen when it was hidden (it says so; the core does not watch visible output).
    alt_at_hide: bool,
    last_activity: Option<Instant>,
}

impl Hide {
    fn push(&mut self, data: &[u8]) {
        self.alt.feed(data);
        self.ring.push(data);
    }
}

/// What `list` and `open` say about a terminal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TermInfo {
    pub id: u32,
    /// The shell's process id, for diagnostics.
    pub pid: u32,
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
    /// Typed bytes wait here for the writer thread, so a shell that is not reading never blocks the caller.
    input: Arc<(Mutex<Input>, Condvar)>,
    writer: Mutex<Option<JoinHandle<()>>>,
    sink: Mutex<Sink>,
    /// Bytes sent to the sink and not yet acknowledged.
    credit: Mutex<usize>,
    credit_cv: Condvar,
    exited: AtomicBool,
    reaped: Mutex<bool>,
    /// The shell's handle. Whoever takes it (the reader at EOF, or `stop` when the reader cannot finish) reaps it.
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
    /// Output the reader has taken from the PTY and the flusher has not sent yet.
    pending: Arc<(Mutex<Pending>, Condvar)>,
    flusher: Mutex<Option<JoinHandle<()>>>,
    reader: Mutex<Option<JoinHandle<()>>>,
    /// Write end of the reader's wake pipe: a byte here makes a blocked reader return (None if the fds were too large for
    /// `select`, in which case the reader cannot be woken and `stop` falls back to detaching it).
    wake: Option<OwnedFd>,
    /// True while the pane is hidden (or its view is gone): the reader keeps the newest output in `hide` and sends nothing.
    hidden: AtomicBool,
    hide: Mutex<Hide>,
    /// Held while output is sent, and while a hidden pane is shown, so older output can never arrive after a newer tail.
    order: Mutex<()>,
}

impl Term {
    fn signal(&self, sig: libc::c_int) {
        let reaped = lock(&self.reaped);
        kill::signal_all(self.pid, !*reaped, sig);
    }

    fn send(&self, e: Event) -> bool {
        let sink = lock(&self.sink).clone();
        sink(e)
    }

    /// Start keeping output instead of sending it. Output the flusher has not sent yet is older than anything the reader
    /// will keep from now on, so it goes into the ring first (those bytes were counted as unacknowledged when read, and
    /// will never be acknowledged: give them back). Needs `order` held by the caller.
    fn conceal(&self, h: &mut Hide, alt: bool) {
        if h.on {
            return;
        }
        h.ring = hidden::Ring::default();
        h.alt = hidden::AltTrack::new(alt);
        h.alt_at_hide = alt;
        h.last_activity = None;
        let old = std::mem::take(&mut lock(&self.pending.0).buf);
        if !old.is_empty() {
            h.push(&old);
            let mut c = lock(&self.credit);
            *c = c.saturating_sub(old.len());
        }
        h.on = true;
        self.hidden.store(true, Ordering::SeqCst);
        self.credit_cv.notify_all(); // a reader waiting for credit has no need of it any more
    }

    /// The reader's side: keep `data` if the pane is hidden. Returns (kept, report activity).
    fn keep(&self, data: &[u8]) -> (bool, bool) {
        let mut h = lock(&self.hide);
        if !h.on {
            return (false, false);
        }
        h.push(data);
        let due = h.last_activity.is_none_or(|t| t.elapsed() >= ACTIVITY_EVERY);
        if due {
            h.last_activity = Some(Instant::now());
        }
        (true, due)
    }

    /// Show a hidden pane: the older unsent output, then how much was left out, then the newest tail, once. A program on
    /// the alternate screen that had output while hidden is asked to redraw (its output is a screen, not a log).
    fn reveal(&self) {
        let _o = lock(&self.order);
        let mut h = lock(&self.hide);
        if !h.on {
            return;
        }
        let mut tail = h.ring.take();
        let (was, now) = (h.alt_at_hide, h.alt.on);
        let alt = (was != now).then_some(now);
        let missed = !tail.bytes.is_empty() || tail.skipped > 0 || alt.is_some();
        if now {
            // A screen is not a log: the tail of a full-screen program's output is half a frame. Send none (and no count);
            // the redraw below paints the whole screen.
            tail = hidden::Tail { bytes: Vec::new(), skipped: 0 };
        }
        h.on = false;
        self.hidden.store(false, Ordering::SeqCst);
        let mut ok = true;
        if tail.skipped > 0 || alt.is_some() {
            ok = self.send(Event::Skipped { bytes: tail.skipped, alt });
            if !ok {
                // The notice did not get through, so the tail was not offered either: keep it for the next view.
                h.ring.restore(std::mem::replace(&mut tail, hidden::Tail { bytes: Vec::new(), skipped: 0 }));
            }
        }
        if ok && !tail.bytes.is_empty() {
            let len = tail.bytes.len();
            *lock(&self.credit) += len;
            ok = ok && self.send(Event::Data(tail.bytes));
            if !ok {
                // Nobody took it, so nothing is owed an acknowledgement for it.
                let mut c = lock(&self.credit);
                *c = c.saturating_sub(len);
            }
        }
        if !ok {
            // The view is gone: keep hiding. What was in the tail is not kept again (it is the view's loss), but new output is.
            h.on = true;
            h.alt = hidden::AltTrack::new(now);
            h.alt_at_hide = false;
            self.hidden.store(true, Ordering::SeqCst);
            return;
        }
        drop(h);
        if now && missed {
            self.nudge();
        }
    }

    /// Make a full-screen program redraw: a size change is a SIGWINCH, and the size is put back at once.
    fn nudge(&self) {
        let (r, c) = *lock(&self.size);
        let (tr, tc) = if r > 1 { (r - 1, c) } else { (r, c.saturating_add(1)) };
        let m = lock(&self.master);
        let _ = m.resize(PtySize { rows: tr, cols: tc, pixel_width: 0, pixel_height: 0 });
        let _ = m.resize(PtySize { rows: r, cols: c, pixel_width: 0, pixel_height: 0 });
    }

    fn info(&self, id: u32) -> TermInfo {
        let (rows, cols) = *lock(&self.size);
        TermInfo { id, pid: self.pid, cwd: self.place.cwd.clone(), root: self.place.root.clone(), rows, cols, alive: !self.exited.load(Ordering::SeqCst) }
    }
}

/// The open terminals, and the roots of opens in progress (their slot is taken before the shell is forked, so the fork
/// happens outside this lock and a slow spawn never stalls `list`, `ack` or `resize`).
#[derive(Default)]
struct Registry {
    terms: HashMap<u32, Arc<Term>>,
    opening: Vec<PathBuf>,
}

/// Releases an `open` slot on every path out of `open`.
struct Slot<'a>(&'a Mutex<Registry>, Option<PathBuf>);

impl Slot<'_> {
    /// Give the slot back under a lock the caller already holds (`Drop` would take it again).
    fn release(mut self, r: &mut Registry) {
        if let Some(root) = self.1.take() {
            r.forget_opening(&root);
        }
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        if let Some(root) = self.1.take() {
            lock(self.0).forget_opening(&root);
        }
    }
}

impl Registry {
    fn forget_opening(&mut self, root: &Path) {
        if let Some(i) = self.opening.iter().position(|p| p == root) {
            self.opening.swap_remove(i);
        }
    }
}

pub struct Terminals {
    map: Mutex<Registry>,
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
        Self::with(Limits::default(), Arc::new(UserPathEnv::default()))
    }

    pub fn with(limits: Limits, env: Arc<dyn LoginEnv>) -> Self {
        Self { map: Mutex::new(Registry::default()), next: AtomicU32::new(1), closing: AtomicBool::new(false), limits, env }
    }

    pub fn count(&self) -> usize {
        lock(&self.map).terms.len()
    }

    fn get(&self, id: u32) -> Result<Arc<Term>, TermError> {
        lock(&self.map).terms.get(&id).cloned().ok_or(TermError::NotFound)
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
        let shell = policy::choose_shell(self.env.as_ref());
        // argv, never a command line: nothing from the UI is ever parsed by a shell. `-l` makes it a login shell, which
        // reads the user's profile and so sets the rest of the environment.
        self.spawn_in(place, vec![shell.into_os_string(), OsString::from("-l")], rows, cols, sink)
    }

    /// Open a tab running a CLI's own sign-in (ADR-17 rule 6): the program and args the catalog names, on the binary
    /// detection found, in its own fresh private folder. The command is built outside this module, only from the
    /// compiled catalog, so the UI can ask for a sign-in by harness id but never name a program. The env is the same as
    /// any terminal's, so no Vajra-owned CLI home goes with it.
    pub fn open_sign_in(&self, sign_in: &crate::providers::setup::SignInCommand, rows: u16, cols: u16, sink: Sink) -> Result<TermInfo, TermError> {
        if self.closing.load(Ordering::SeqCst) {
            return Err(TermError::ShuttingDown);
        }
        if rows == 0 || cols == 0 {
            return Err(TermError::InvalidSize);
        }
        let cwd = sign_in.cwd().to_path_buf();
        let place = Placement { cwd: cwd.clone(), root: cwd };
        let argv = std::iter::once(sign_in.program().as_os_str().to_owned()).chain(sign_in.args().iter().map(OsString::from)).collect();
        self.spawn_in(place, argv, rows, cols, sink)
    }

    fn spawn_in(&self, place: Placement, argv: Vec<OsString>, rows: u16, cols: u16, sink: Sink) -> Result<TermInfo, TermError> {
        let pair = native_pty_system()
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| TermError::Io(e.to_string()))?;
        let mut cmd = CommandBuilder::from_argv(argv);
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
        // The cap is checked and the slot taken under one lock, so two concurrent opens cannot both pass the check; the
        // fork below happens outside it.
        let slot = {
            let mut r = lock(&self.map);
            let under = |root: &Path| r.terms.values().filter(|t| t.place.root == root).count() + r.opening.iter().filter(|p| *p == root).count();
            if r.terms.len() + r.opening.len() >= self.limits.max_open || under(&place.root) >= self.limits.max_per_root {
                return Err(TermError::TooMany);
            }
            r.opening.push(place.root.clone());
            Slot(&self.map, Some(place.root.clone()))
        };
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
        let (reader, wake) = WakeReader::wrap(reader, pair.master.as_raw_fd());
        let input = Arc::new((Mutex::new(Input::default()), Condvar::new()));
        let term = Arc::new(Term {
            pid,
            place,
            size: Mutex::new((rows, cols)),
            master: Mutex::new(pair.master),
            input: input.clone(),
            writer: Mutex::new(None),
            sink: Mutex::new(sink),
            credit: Mutex::new(0),
            credit_cv: Condvar::new(),
            exited: AtomicBool::new(false),
            reaped: Mutex::new(false),
            child: Mutex::new(Some(child)),
            pending: Arc::new((Mutex::new(Pending::default()), Condvar::new())),
            flusher: Mutex::new(None),
            reader: Mutex::new(None),
            wake,
            hidden: AtomicBool::new(false),
            hide: Mutex::new(Hide::default()),
            order: Mutex::new(()),
        });
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (window, rate, hidden_rate) = (self.limits.window, self.limits.max_rate, self.limits.hidden_rate);
        // The three I/O threads are part of the terminal: if any cannot start, end the shell and what is started and
        // report the error, instead of publishing a terminal that cannot deliver output or accept input.
        let started = (|| -> std::io::Result<()> {
            let t = term.clone();
            *lock(&term.writer) = Some(std::thread::Builder::new().name(format!("term-in-{id}")).spawn(move || write_loop(writer, &t.input))?);
            let t = term.clone();
            *lock(&term.flusher) = Some(std::thread::Builder::new().name(format!("term-flush-{id}")).spawn(move || flush_loop(&t.pending, &t))?);
            let t = term.clone();
            *lock(&term.reader) = Some(std::thread::Builder::new().name(format!("term-{id}")).spawn(move || read_loop(reader, &t, window, rate, hidden_rate))?);
            Ok(())
        })();
        if let Err(e) = started {
            stop(&term);
            return Err(TermError::Io(e.to_string()));
        }
        let info = term.info(id);
        let mut r = lock(&self.map);
        if self.closing.load(Ordering::SeqCst) {
            drop(r);
            stop(&term);
            return Err(TermError::ShuttingDown);
        }
        r.terms.insert(id, term);
        // The terminal now counts as open; release the slot under this same lock (its Drop would take the lock again).
        slot.release(&mut r);
        Ok(info)
    }

    /// Every open terminal, oldest first.
    pub fn list(&self) -> Vec<TermInfo> {
        let mut v: Vec<TermInfo> = lock(&self.map).terms.iter().map(|(id, t)| t.info(*id)).collect();
        v.sort_by_key(|i| i.id);
        v
    }

    /// Ids of the terminals opened in `dir` or inside it (links resolved), for releasing a worker's worktree.
    pub fn ids_under(&self, dir: &Path) -> Vec<u32> {
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        self.list().into_iter().filter(|t| t.cwd.canonicalize().unwrap_or_else(|_| t.cwd.clone()).starts_with(&dir)).map(|t| t.id).collect()
    }

    /// Whether the terminal's shell has a child process (a dev server, an editor, a build): closing it would end that.
    /// Bounded to 2 s; a shell that cannot be asked in time, or at all, counts as busy. An ended shell is never busy.
    pub fn is_busy(&self, id: u32) -> bool {
        let Some(t) = self.list().into_iter().find(|t| t.id == id).filter(|t| t.alive) else { return false };
        let Ok(mut ask) = std::process::Command::new("/usr/bin/pgrep").args(["-P", &t.pid.to_string()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() else { return true };
        let start = std::time::Instant::now();
        loop {
            match ask.try_wait() {
                Ok(Some(s)) => return s.success(),
                Ok(None) if start.elapsed() < Duration::from_secs(2) => std::thread::sleep(Duration::from_millis(5)),
                _ => {
                    let _ = ask.kill();
                    let _ = ask.wait();
                    return true;
                }
            }
        }
    }

    /// Keystrokes or a paste typed by the human. The caller is the UI's own terminal view; there is no other path in.
    /// Never blocks: the bytes go to a bounded queue a writer thread drains into the PTY, and a full queue (the shell is
    /// not reading) answers `QueueFull` instead of waiting.
    pub fn write(&self, id: u32, data: &[u8]) -> Result<(), TermError> {
        if data.len() > self.limits.max_write {
            return Err(TermError::TooLarge);
        }
        let t = self.get(id)?;
        if t.exited.load(Ordering::SeqCst) {
            return Err(TermError::Io("the shell has ended".into()));
        }
        let (m, cv) = &*t.input;
        let mut q = lock(m);
        if q.closed {
            return Err(TermError::Io("the terminal's input is closed".into()));
        }
        if q.bytes + data.len() > self.limits.write_queue {
            return Err(TermError::QueueFull);
        }
        q.bytes += data.len();
        q.chunks.push_back(data.to_vec());
        cv.notify_one();
        Ok(())
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

    /// The view shows or hides this pane (the dock is hidden, or another tab is selected). While hidden the shell keeps
    /// running (not held to the visible rate): the core keeps the newest 256 KiB of its output and sends nothing, and showing the pane sends
    /// how much was left out, then that tail, once. `alt` says whether the view was on the alternate screen when it hid.
    /// An unknown id is an error and changes nothing.
    pub fn set_visible(&self, id: u32, visible: bool, alt: bool) -> Result<(), TermError> {
        let t = self.get(id)?;
        if visible {
            t.reveal();
        } else {
            let _o = lock(&t.order);
            let mut h = lock(&t.hide);
            t.conceal(&mut h, alt);
        }
        Ok(())
    }

    /// Bind a new sink to a terminal that outlived its view (the webview reloaded, or a hidden dock came back). The
    /// unacknowledged count restarts at zero. Output produced while no view was listening was kept (newest 256 KiB) and is
    /// sent to the new view now.
    pub fn attach(&self, id: u32, sink: Sink) -> Result<TermInfo, TermError> {
        let t = self.get(id)?;
        *lock(&t.sink) = sink;
        *lock(&t.credit) = 0;
        t.credit_cv.notify_all();
        t.reveal();
        Ok(t.info(id))
    }

    /// Close a tab: hang up the shell's group and session, kill what ignores that, reap, and join the reader.
    pub fn close(&self, id: u32) -> Result<(), TermError> {
        let t = lock(&self.map).terms.remove(&id).ok_or(TermError::NotFound)?;
        stop(&t);
        Ok(())
    }

    /// App quit: close every terminal. After this `open` is refused.
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let all: Vec<Arc<Term>> = lock(&self.map).terms.drain().map(|(_, t)| t).collect();
        for t in &all {
            t.signal(libc::SIGHUP);
        }
        // In parallel, so quitting with several stuck shells takes one grace period, not one each.
        std::thread::scope(|sc| {
            for t in &all {
                sc.spawn(move || stop(t));
            }
        });
    }

    /// Tests: the shell's pid.
    pub fn pid(&self, id: u32) -> Option<u32> {
        lock(&self.map).terms.get(&id).map(|t| t.pid)
    }
}

impl Drop for Terminals {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// How long `stop` waits for the reader to see EOF after the session is killed. Normally EOF is immediate; it is not if a
/// descendant escaped the session (`setsid`) and still holds the PTY.
const READER_GRACE: Duration = Duration::from_millis(400);

fn stop(t: &Arc<Term>) {
    t.signal(libc::SIGHUP);
    let deadline = Instant::now() + KILL_GRACE;
    while !t.exited.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !t.exited.load(Ordering::SeqCst) {
        t.signal(libc::SIGKILL);
    }
    // The shell and its session are gone, so a writer stuck on a full PTY gets an error and ends; wake an idle one.
    {
        let (m, cv) = &*t.input;
        let mut q = lock(m);
        q.closed = true;
        q.chunks.clear();
        cv.notify_all();
    }
    // Unblock a reader that is waiting for credit, then wait for it, but not for ever.
    *lock(&t.credit) = 0;
    t.credit_cv.notify_all();
    let reader = lock(&t.reader).take();
    let until = Instant::now() + READER_GRACE;
    while reader.as_ref().is_some_and(|h| !h.is_finished()) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    match reader {
        Some(h) if h.is_finished() => {
            let _ = h.join();
        }
        Some(h) => {
            // Something outside the session (a descendant that called `setsid`) still holds the PTY slave, so the
            // blocked read has no EOF coming. Reap the shell here without waiting for it, silence the sink, then wake
            // the reader through its wake pipe (no signals) so it returns, the thread ends, and the PTY descriptors are
            // released even though the slave stays open. Bounded: without a wake pipe, or if it does not end, the
            // thread is detached and stops at its next read (`exited` is set).
            *lock(&t.sink) = Arc::new(|_| true);
            if let Some(child) = lock(&t.child).take() {
                reap(t, child);
            }
            if wake_reader(t.wake.as_ref().map(|w| w.as_raw_fd()), &h, Duration::from_secs(1)) {
                let _ = h.join();
            }
        }
        None => {
            if let Some(child) = lock(&t.child).take() {
                reap(t, child); // the reader never started
            }
        }
    }
    // The reader (or this) is done with the pending buffer: let the flusher drain and end.
    {
        let (m, cv) = &*t.pending;
        lock(m).eof = true;
        cv.notify_all();
    }
    if let Some(h) = lock(&t.flusher).take() {
        let until = Instant::now() + Duration::from_millis(200);
        while !h.is_finished() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        if h.is_finished() {
            let _ = h.join();
        }
    }
    // Join the writer if it ends promptly. One stuck on a PTY that something outside the session still holds open must
    // not hang the close; it is detached and ends when that write fails.
    if let Some(h) = lock(&t.writer).take() {
        let until = Instant::now() + Duration::from_millis(200);
        while !h.is_finished() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        if h.is_finished() {
            let _ = h.join();
        }
    }
}

/// A reader that can be woken without a signal: it waits in `select` on the PTY master and on a pipe, and a byte on the
/// pipe makes `read` return 0 (end of input). `select` is used because `poll` is unreliable on macOS PTYs.
struct WakeReader {
    inner: Box<dyn Read + Send>,
    watch: RawFd,
    wake: OwnedFd,
}

impl WakeReader {
    /// Wrap `inner` (a clone of the master's reader). Returns the wake pipe's write end, or the reader unchanged and no
    /// wake end when a descriptor is too large for an `fd_set` or the pipe cannot be made.
    fn wrap(inner: Box<dyn Read + Send>, watch: Option<RawFd>) -> (Box<dyn Read + Send>, Option<OwnedFd>) {
        let Some(watch) = watch.filter(|fd| (*fd as usize) < libc::FD_SETSIZE) else { return (inner, None) };
        let mut fds = [0 as libc::c_int; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return (inner, None);
        }
        // Close-on-exec, so no other shell or child inherits the wake pipe.
        for fd in fds {
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        if r.as_raw_fd() as usize >= libc::FD_SETSIZE {
            return (inner, None);
        }
        (Box::new(Self { inner, watch, wake: r }), Some(w))
    }
}

impl Read for WakeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let (a, b) = (self.watch, self.wake.as_raw_fd());
            let mut set: libc::fd_set = unsafe { std::mem::zeroed() };
            unsafe {
                libc::FD_ZERO(&mut set);
                libc::FD_SET(a, &mut set);
                libc::FD_SET(b, &mut set);
            }
            let n = unsafe { libc::select(a.max(b) + 1, &mut set, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
            if n < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if unsafe { libc::FD_ISSET(b, &set) } {
                return Ok(0);
            }
            if unsafe { libc::FD_ISSET(a, &set) } {
                return self.inner.read(buf);
            }
        }
    }
}

/// Wake the reader through its pipe and wait for it to end, until `within` passes. Returns whether it ended.
fn wake_reader(wake: Option<RawFd>, h: &JoinHandle<()>, within: Duration) -> bool {
    if let Some(fd) = wake {
        unsafe { libc::write(fd, [1u8].as_ptr().cast(), 1) };
    }
    let until = Instant::now() + within;
    while !h.is_finished() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    h.is_finished()
}

/// End what is left of the session while the leader is still unreaped (an unreaped leader, even a zombie, holds its pid
/// and so its session id), reap it, and mark the terminal not signalable: after this nothing can signal this pid, which
/// may be recycled at once. Returns the shell's exit code, or None when a signal ended it.
fn reap(t: &Term, mut child: Box<dyn portable_pty::Child + Send + Sync>) -> Option<i32> {
    let mut reaped = lock(&t.reaped);
    kill::signal_all(t.pid, true, libc::SIGKILL);
    let status = child.wait();
    *reaped = true;
    t.exited.store(true, Ordering::SeqCst);
    // portable-pty reports a signal death as `signal() != None` with a synthetic exit code of 1.
    status.ok().filter(|s| s.signal().is_none()).map(|s| s.exit_code() as i32)
}

/// Typed bytes not yet written to the PTY.
#[derive(Default)]
struct Input {
    chunks: std::collections::VecDeque<Vec<u8>>,
    bytes: usize,
    closed: bool,
}

/// The only thread that writes to the PTY master. A `write_all` here may block for as long as the shell does not read;
/// callers never wait on it. It ends when the input is closed or the PTY write fails (the shell is gone).
fn write_loop(mut w: Box<dyn Write + Send>, input: &(Mutex<Input>, Condvar)) {
    let (m, cv) = input;
    loop {
        let mut q = lock(m);
        while q.chunks.is_empty() && !q.closed {
            q = cv.wait(q).unwrap_or_else(|e| e.into_inner());
        }
        if q.closed {
            return;
        }
        let Some(chunk) = q.chunks.pop_front() else { continue };
        drop(q);
        let ok = w.write_all(&chunk).and_then(|()| w.flush()).is_ok();
        let mut q = lock(m);
        q.bytes = q.bytes.saturating_sub(chunk.len());
        if !ok {
            q.closed = true;
            q.chunks.clear();
            return;
        }
    }
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
        drop(p);
        // `order` first, so a pane being hidden or shown cannot slip its bytes in between this take and this send.
        let _o = lock(&t.order);
        let mut p = lock(m);
        let take = p.buf.len().min(CHUNK * 4);
        if take == 0 {
            continue; // a hide or show took it
        }
        let data: Vec<u8> = p.buf.drain(..take).collect();
        drop(p);
        let len = data.len();
        {
            // The reader checks `hidden` and then appends to `pending` without `order`, so a hide can land between the two
            // and leave bytes here that are newer than the ring's. They must not reach a hidden view (they would arrive
            // before the older tail, and the alternate-screen tracker would miss them): keep them with the rest.
            let mut h = lock(&t.hide);
            if h.on {
                h.push(&data);
                drop(h);
                let mut c = lock(&t.credit);
                *c = c.saturating_sub(len);
                continue;
            }
        }
        if !t.send(Event::Data(data)) {
            // The view is gone (the webview reloaded or closed): what could not be sent was never read by anyone, so it is
            // not owed an acknowledgement; keep what comes next for the view that attaches.
            let mut c = lock(&t.credit);
            *c = c.saturating_sub(len);
            drop(c);
            let mut h = lock(&t.hide);
            t.conceal(&mut h, false);
        }
    }
}

/// Token bucket as a clock: `due` is when the bytes read so far have been paid for at `rate`. It may lag `now` by up to
/// one burst, so time lost to a sleep that overshoots (small reads) is repaid instead of lowering the rate.
struct Pace {
    rate: usize,
    ahead: Duration,
    due: Instant,
}

impl Pace {
    fn new(rate: usize) -> Self {
        let ahead = if rate > 0 { Duration::from_secs_f64(RATE_BURST as f64 / rate as f64) } else { Duration::ZERO };
        Self { rate, ahead, due: Instant::now() }
    }

    fn pay(&mut self, n: usize) {
        if self.rate > 0 {
            let now = Instant::now();
            self.due = self.due.max(now.checked_sub(self.ahead).unwrap_or(now)) + Duration::from_secs_f64(n as f64 / self.rate as f64);
        }
    }
}

fn read_loop(mut reader: Box<dyn Read + Send>, t: &Arc<Term>, window: usize, rate: usize, hidden_rate: usize) {
    let low = window / 2;
    // A visible pane is held to `rate`; a hidden one to `hidden_rate` (32 MiB/s by default, far above any real build
    // log; 0 means not held). Each has its own clock, so switching does not carry a debt over.
    let (mut seen, mut unseen) = (Pace::new(rate), Pace::new(hidden_rate));
    let mut buf = vec![0u8; CHUNK];
    loop {
        {
            // Stop at the window and resume at half of it, so acks do not turn into stop-and-go.
            let mut c = lock(&t.credit);
            if *c >= window {
                while *c > low && !t.exited.load(Ordering::SeqCst) && !t.hidden.load(Ordering::SeqCst) {
                    c = t.credit_cv.wait_timeout(c, Duration::from_millis(100)).unwrap_or_else(|e| e.into_inner()).0;
                }
            }
        }
        // Ahead of the pace: wait, in slices so a close is never held up by it (and a hide or show is noticed).
        let was_hidden = t.hidden.load(Ordering::SeqCst);
        let pace = if was_hidden { &unseen } else { &seen };
        if pace.rate > 0 {
            while pace.due > Instant::now() && !t.exited.load(Ordering::SeqCst) && t.hidden.load(Ordering::SeqCst) == was_hidden {
                std::thread::sleep((pace.due - Instant::now()).min(Duration::from_millis(10)));
            }
        }
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) if t.exited.load(Ordering::SeqCst) => break, // `stop` reaped the shell while this read was blocked (defensive: the tty is normally hung up when the leader exits, but a holder that keeps the PTY open must not make a closed terminal buffer output)
            Ok(n) => {
                if t.hidden.load(Ordering::SeqCst) {
                    let (kept, report) = t.keep(&buf[..n]);
                    if kept {
                        unseen.pay(n);
                        if report {
                            t.send(Event::Activity);
                        }
                        continue;
                    }
                }
                seen.pay(n);
                *lock(&t.credit) += n;
                let (m, cv) = &*t.pending;
                lock(m).buf.extend_from_slice(&buf[..n]);
                cv.notify_one();
            }
        }
    }
    {
        let (m, cv) = &*t.pending;
        lock(m).eof = true;
        cv.notify_all();
    }
    if let Some(f) = lock(&t.flusher).take() {
        let _ = f.join();
    }
    // The PTY closed. If `stop` already reaped the shell (this thread was blocked past its grace), there is nothing left
    // to report; otherwise reap it here and say how it ended.
    if let Some(child) = lock(&t.child).take() {
        let code = reap(t, child);
        t.send(Event::Exit(code));
    }
}

#[cfg(test)]
mod harness;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_hidden;
