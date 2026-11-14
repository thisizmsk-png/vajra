//! Recorded ownership of user shells, run leads and workers across a crash (FG-24, ADR-9 item 6b'; 6d2d).
//!
//! Each terminal is recorded as (pid, start stamp) when it opens and forgotten when it ends cleanly. Every process the
//! supervisor starts (a run lead, a worker) is recorded the same way, with its kind and its run or ticket, before the
//! spawn returns, and forgotten when it is reaped; a spawn whose record cannot be written is refused. After a crash or a
//! SIGKILL of the app the file still lists the shells that were running. At launch `reconcile` checks each against the
//! live process table: a record is *owned* only if the pid is alive AND its start stamp is the recorded one. A pid that
//! is gone is dropped; a pid that is alive with another start stamp was reused by someone else and is dropped too,
//! untouched. `recover` signals only what `reconcile` proves is owned, re-proves it first, and never matches by name.
//! For shells nothing here runs by itself: the human chooses to recover. Leads and workers are ended at start-up
//! (`end_agents`), before any agent of this life starts, through the same proof, and only if the process itself carries
//! the record's random mark in its environment (`AGENT_MARK`): a record alone never proves who started a process.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::lock;
use crate::procid::{same_process, start_stamp};

/// Entries kept, and the longest line read back. A damaged or hostile file cannot grow memory without bound.
const MAX_ENTRIES: usize = 256;
const MAX_LINE: usize = 4096;
/// The most of the file that is read at all (a real ledger is a few tens of KiB), so a corrupted or oversized one cannot
/// take unbounded memory or hold up start-up.
const MAX_BYTES: u64 = 1024 * 1024;
pub const GRACE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Owner {
    pub pid: u32,
    pub stamp: u64,
    pub cwd: PathBuf,
    /// What the process is. A record from before kinds existed is a shell.
    #[serde(default)]
    pub kind: Kind,
    /// The run a lead or worker belongs to (or the supervisor session id when it has no run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// A worker's ticket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    /// Start-up reconciles that signalled it and could not end it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tries: u32,
    /// A lead's or worker's mark: the random value the supervisor put in its environment as `AGENT_MARK` when it
    /// started it. The start-up reconcile signals a recorded agent only if its environment still carries this value
    /// (`could_be_ours`), so a damaged or forged record naming another of the user's processes ends nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
}

/// The environment variable that carries a lead's or worker's mark (`Owner::mark`).
pub const AGENT_MARK: &str = "VAJRA_AGENT_MARK";

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// How many start-ups try to end a lead or worker that survives its kill before the record is dropped.
pub const MAX_TRIES: u32 = 3;

/// The kind of a recorded process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A user's terminal: recovered only when the human asks.
    #[default]
    Shell,
    /// A process the supervisor started that is not a worker: a run lead, the Director-General, a classic run.
    Lead,
    /// A core worker, with its ticket.
    Worker,
}

impl Owner {
    /// A shell record.
    pub fn shell(pid: u32, stamp: u64, cwd: PathBuf) -> Self {
        Self { pid, stamp, cwd, kind: Kind::Shell, run: None, ticket: None, tries: 0, mark: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Alive, and the start stamp matches: provably ours. `members` counts its session (the shell and its jobs).
    Owned { members: usize },
    /// No such process any more: nothing to do.
    Gone,
    /// The pid is alive but started at another time: someone else's. Never touched.
    Reused,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Found {
    #[serde(flatten)]
    pub owner: Owner,
    #[serde(flatten)]
    pub verdict: Verdict,
}

/// At most `MAX_BYTES` of the file as text (bytes that are not UTF-8 become U+FFFD, so one bad line costs only itself). A
/// line cut by the cap fails to parse and is skipped.
fn read_capped(path: &Path) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    // A link planted at the path is not followed (O_NOFOLLOW), and a FIFO with no writer cannot block the open
    // (O_NONBLOCK); anything that is not a regular file is refused, so a hostile name costs an empty ledger, not a hang.
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).ok()?;
    if !f.metadata().ok()?.is_file() {
        return None;
    }
    let mut buf = Vec::new();
    f.take(MAX_BYTES).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

pub struct Ledger {
    path: PathBuf,
    guard: Mutex<()>,
    /// Which owner (pid and start stamp) each open terminal recorded, so closing a tab forgets exactly that one.
    ids: Mutex<std::collections::HashMap<u32, (u32, u64)>>,
}

impl Ledger {
    pub fn new(path: PathBuf) -> Self {
        Self { path, guard: Mutex::new(()), ids: Mutex::new(std::collections::HashMap::new()) }
    }

    /// Every readable entry. A bad line is skipped, not fatal.
    pub fn load(&self) -> Vec<Owner> {
        let _g = lock(&self.guard);
        self.read()
    }

    fn read(&self) -> Vec<Owner> {
        let Some(text) = read_capped(&self.path) else { return Vec::new() };
        text.lines().filter(|l| l.len() <= MAX_LINE).filter_map(|l| serde_json::from_str::<Owner>(l).ok()).filter(|o| o.pid > 1).take(MAX_ENTRIES).collect()
    }

    /// Replace the file with `all`: a new file created exclusively under a name nobody can have planted (`create_new` is
    /// O_EXCL, which never follows a symlink, and O_NOFOLLOW is set as well), then renamed into place. Nothing a worker
    /// leaves in the directory is ever opened for writing, so a planted link cannot make this truncate its target.
    fn write(&self, all: &[Owner]) -> std::io::Result<()> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let tmp = self.path.with_file_name(format!("{name}.{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let put = || -> std::io::Result<()> {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).custom_flags(libc::O_NOFOLLOW).mode(0o600).open(&tmp)?;
            for o in all {
                writeln!(f, "{}", serde_json::to_string(o).map_err(std::io::Error::other)?)?;
            }
            f.sync_all()?;
            std::fs::rename(&tmp, &self.path)
        };
        let r = put();
        if r.is_err() {
            let _ = std::fs::remove_file(&tmp); // ours (created exclusively above), or absent
        }
        r
    }

    /// Record a process the moment it starts. For a shell this is best effort (a failure only means a crash cannot be
    /// recovered). For a lead or worker a failure refuses the spawn, so a full ledger refuses rather than dropping the
    /// oldest record: no lead or worker runs without one.
    pub fn record(&self, o: Owner) -> std::io::Result<()> {
        // A line `read` would skip is a record that is not there after a crash: refused, so the spawn is too.
        if serde_json::to_string(&o).map_err(std::io::Error::other)?.len() > MAX_LINE {
            return Err(std::io::Error::other("the ownership record is too long"));
        }
        let _g = lock(&self.guard);
        let mut all = self.read();
        all.retain(|x| !(x.pid == o.pid && x.stamp == o.stamp));
        all.push(o);
        // Over the cap, the oldest shells go first; a lead or worker record is never dropped to make room, so a record
        // that would need one is refused.
        while all.len() > MAX_ENTRIES {
            match all.iter().position(|x| x.kind == Kind::Shell) {
                Some(i) => {
                    all.remove(i);
                }
                None => return Err(std::io::Error::other("the ownership ledger is full")),
            }
        }
        self.write(&all)
    }

    pub fn forget(&self, pid: u32, stamp: u64) -> std::io::Result<()> {
        let _g = lock(&self.guard);
        let mut all = self.read();
        let n = all.len();
        all.retain(|x| !(x.pid == pid && x.stamp == stamp));
        if all.len() == n {
            return Ok(());
        }
        self.write(&all)
    }

    /// Record the shell of terminal `id`, remembering which record is its own.
    pub fn record_for(&self, id: u32, o: Owner) -> std::io::Result<()> {
        lock(&self.ids).insert(id, (o.pid, o.stamp));
        self.record(o)
    }

    /// The human closed terminal `id`: forget the record that terminal made, and only that one (an ended tab can outlive
    /// its shell while the OS gives the pid to a newer shell, whose record must stay). Removing our own record signals nothing.
    pub fn forget_for(&self, id: u32) -> std::io::Result<()> {
        let own = lock(&self.ids).remove(&id);
        match own {
            Some((pid, stamp)) => self.forget(pid, stamp),
            None => Ok(()),
        }
    }

    /// Close terminal `id` with `close`, and only then forget its record, whatever `close` returned. The record is what lets
    /// the next launch find a shell that survived a crash during the close, so it must outlive the stop; once the shell is
    /// stopped (or was never there) a stale record would only reconcile as gone.
    pub fn close_then_forget<T, E>(&self, id: u32, close: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        let r = close();
        let _ = self.forget_for(id);
        r
    }

    /// At start-up, before any agent of this life starts: end every lead and worker an earlier life recorded that is
    /// still provably ours (pid and start stamp, re-proved before each signal), and forget the ones that are gone or
    /// whose pid another process now holds (never signalled). Shells are left for the human. Returns what was found,
    /// with what happened to each, so the caller can write the tickets and runs they belonged to.
    pub fn end_agents(&self) -> Vec<(Owner, Ended)> {
        let entries: Vec<Owner> = self.load().into_iter().filter(|o| o.kind != Kind::Shell).collect();
        let mut out = Vec::new();
        for o in entries {
            let mut ended = match judge(&o) {
                Verdict::Gone => Ended::Gone,
                Verdict::Reused => Ended::Reused,
                // The stamp proves the record names a live process, not that this app started it: one that could not
                // be a supervisor child of ours is never signalled.
                Verdict::Owned { .. } if !could_be_ours(&o) => Ended::Reused,
                Verdict::Owned { .. } => end_session(&o),
            };
            let _ = self.forget(o.pid, o.stamp);
            if ended == Ended::Survived {
                // Tried again at the next start, a few times at most: a record that never ends must not fill the ledger.
                // Saturating: the count is read from the file.
                ended = self.keep_survivor(&o);
            }
            out.push((o, ended));
        }
        out
    }

    /// A clean quit ended every shell: nothing is left to recover. A lead or worker record still here (one whose
    /// process outlived the quit) stays, so the next start ends it.
    /// A record whose session outlived the kill: written back with one more try, or given up after `MAX_TRIES`. One
    /// that cannot be written back is `Unrecorded`, so the reconcile report says the next start cannot try it again.
    pub(crate) fn keep_survivor(&self, o: &Owner) -> Ended {
        if o.tries.saturating_add(1) >= MAX_TRIES {
            Ended::GivenUp
        } else if self.record(Owner { tries: o.tries.saturating_add(1), ..o.clone() }).is_err() {
            Ended::Unrecorded
        } else {
            Ended::Survived
        }
    }

    pub fn clear(&self) {
        let _g = lock(&self.guard);
        let agents: Vec<Owner> = self.read().into_iter().filter(|o| o.kind != Kind::Shell).collect();
        if agents.is_empty() {
            let _ = std::fs::remove_file(&self.path);
        } else {
            let _ = self.write(&agents);
        }
    }

    /// Check every record against the live process table, drop the ones that are not ours any more, and return what is
    /// left to report. Kept records are those still `Owned`.
    pub fn settle(&self) -> Vec<Found> {
        let _g = lock(&self.guard);
        let (shells, agents): (Vec<Owner>, Vec<Owner>) = self.read().into_iter().partition(|o| o.kind == Kind::Shell);
        let found = reconcile(&shells);
        // Lead and worker records are the start-up reconcile's: kept as they are, never offered to the human.
        let keep: Vec<Owner> = found.iter().filter(|f| matches!(f.verdict, Verdict::Owned { .. })).map(|f| f.owner.clone()).chain(agents).collect();
        let _ = self.write(&keep);
        found
    }
}

pub fn judge(o: &Owner) -> Verdict {
    match start_stamp(o.pid) {
        None => Verdict::Gone,
        Some(s) if s == o.stamp => Verdict::Owned { members: 1 + crate::terminal::kill::session_members(o.pid).into_iter().filter(|&p| p as u32 != o.pid).count() },
        Some(_) => Verdict::Reused,
    }
}

pub fn reconcile(entries: &[Owner]) -> Vec<Found> {
    entries.iter().map(|o| Found { owner: o.clone(), verdict: judge(o) }).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    /// Every process of the shell's session is gone (the leader and each job it had).
    Ended,
    /// Some processes of the session are still running after SIGKILL; `remaining` counts them.
    Survived { remaining: usize },
    /// Not provably ours (gone, or the pid was reused): nothing was signalled.
    NotOwned,
}

/// Settle one recovery: `Ended` and `NotOwned` forget the record (the ledger entry and the list the UI shows); `Survived`
/// keeps both, with the number of processes that are really left, so the human can try again and a restart can still find it.
pub fn apply_recovery(ledger: &Ledger, found: &mut Vec<Found>, owner: &Owner, result: &Recovery) {
    match result {
        Recovery::Survived { remaining } => {
            if let Some(f) = found.iter_mut().find(|f| f.owner == *owner) {
                f.verdict = Verdict::Owned { members: *remaining };
            }
        }
        Recovery::Ended | Recovery::NotOwned => {
            let _ = ledger.forget(owner.pid, owner.stamp);
            found.retain(|f| f.owner != *owner);
        }
    }
}

/// Is this exact process (pid and start stamp) still running? A zombie is not.
pub(crate) fn running(pid: u32, stamp: u64) -> bool {
    same_process(pid, stamp) && !crate::procid::is_zombie(pid)
}

/// What can be read about a recorded process when deciding whether it is gone.
#[derive(Debug, Clone, Copy, Default)]
struct Seen {
    /// Its exit watch reported NOTE_EXIT (registered before its stamp was verified, so the exit is that process's).
    exited: bool,
    /// `kill(pid, 0)` did not fail with ESRCH: something holds the pid (live, a zombie, or another user's process).
    exists: bool,
    /// Its start stamp now, if it can be read.
    stamp: Option<u64>,
    /// Its start stamp if it is a zombie (`procid::zombie_stamp`).
    zombie: Option<u64>,
    /// It is our own child and `waitid(WNOWAIT)` reports it exited (not reaped here: whoever spawned it still reaps it).
    child_exited: bool,
}

impl Seen {
    fn read(w: &mut ExitWatch, pid: u32) -> Self {
        // SAFETY: kill with signal 0 only checks.
        let exists = unsafe { libc::kill(pid as i32, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
        Seen { exited: w.exited(pid), exists, stamp: start_stamp(pid), zombie: crate::procid::zombie_stamp(pid), child_exited: child_exited(pid) }
    }
}

/// Is `pid` our child that has exited? Reads without reaping (`WNOWAIT`), so the code that spawned it still owns the reap
/// and its `Child` handle cannot later signal a reused pid.
fn child_exited(pid: u32) -> bool {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a writable siginfo_t; WNOHANG never blocks, WNOWAIT leaves the child waitable.
    let rc = unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) };
    // SAFETY: si_pid is set by waitid when it reports a child; zeroed (0) otherwise.
    rc == 0 && unsafe { info.si_pid() } == pid as libc::pid_t
}

/// Fail closed: the process recorded at `stamp` is gone only when that is proven, by its exit watch, by the kernel saying
/// the pid is free, by the pid now holding another process (a different stamp), or by it being a zombie of that very
/// process. Anything else, a live pid whose stamp cannot be read included, still runs.
fn gone_from(s: Seen, stamp: u64) -> bool {
    if s.exited || !s.exists {
        return true;
    }
    match s.stamp {
        Some(now) if now != stamp => true,
        Some(_) => s.zombie == Some(stamp),
        None => s.zombie == Some(stamp) || s.child_exited,
    }
}

/// Is the process recorded at (`pid`, `stamp`) provably gone? See `gone_from`.
pub(crate) fn provably_gone(w: &mut ExitWatch, pid: u32, stamp: u64) -> bool {
    gone_from(Seen::read(w, pid), stamp)
}

/// How many of `members` are not provably gone.
pub(crate) fn left(w: &mut ExitWatch, members: &[(u32, u64)]) -> usize {
    members.iter().filter(|&&(p, s)| !provably_gone(w, p, s)).count()
}

/// Reports which processes have exited, so a signal is never sent to a pid after its process is gone. A process is
/// registered BEFORE its identity is read: if the stamp then matches, the registered pid is the right process, and its
/// exit from then on is delivered here (an exited process that is not reaped still holds its pid). macOS only; elsewhere
/// (or if no kqueue can be made) it reports nothing and the stamp check alone applies.
pub(crate) struct ExitWatch {
    kq: i32,
    gone: std::collections::HashSet<u32>,
}

impl ExitWatch {
    pub(crate) fn new() -> Self {
        #[cfg(target_os = "macos")]
        let kq = unsafe { libc::kqueue() };
        #[cfg(not(target_os = "macos"))]
        let kq = -1;
        Self { kq, gone: Default::default() }
    }

    /// Watch `pid` for exit. False if it does not exist (already gone). True without a watch if there is no kqueue.
    pub(crate) fn add(&mut self, pid: u32) -> bool {
        #[cfg(target_os = "macos")]
        if self.kq >= 0 {
            let ev = libc::kevent { ident: pid as usize, filter: libc::EVFILT_PROC, flags: libc::EV_ADD | libc::EV_ONESHOT, fflags: libc::NOTE_EXIT, data: 0, udata: std::ptr::null_mut() };
            return unsafe { libc::kevent(self.kq, &ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) } >= 0;
        }
        let _ = pid;
        true
    }

    /// Has `pid` exited since it was added? Reads the pending events without waiting.
    pub(crate) fn exited(&mut self, pid: u32) -> bool {
        #[cfg(target_os = "macos")]
        if self.kq >= 0 {
            let none = libc::timespec { tv_sec: 0, tv_nsec: 0 };
            let mut out: [libc::kevent; 16] = unsafe { std::mem::zeroed() };
            loop {
                let n = unsafe { libc::kevent(self.kq, std::ptr::null(), 0, out.as_mut_ptr(), out.len() as i32, &none) };
                if n <= 0 {
                    break;
                }
                for e in &out[..n as usize] {
                    if e.fflags & libc::NOTE_EXIT != 0 {
                        self.gone.insert(e.ident as u32);
                    }
                }
            }
        }
        self.gone.contains(&pid)
    }
}

impl Drop for ExitWatch {
    fn drop(&mut self) {
        if self.kq >= 0 {
            unsafe { libc::close(self.kq) };
        }
    }
}

/// Signal `pid` only if it has not exited since it was watched and is still the process with this start stamp, both read
/// immediately before the call. Returns whether a signal was sent. Not atomic (macOS cannot signal by handle): what is
/// left is the gap between this check and the `kill` syscall for this one pid, microseconds, and harm needs the pid to be
/// reaped and reallocated to another process inside it, which needs the pid counter to wrap.
pub(crate) fn signal_verified(w: &mut ExitWatch, pid: u32, stamp: u64, sig: libc::c_int) -> bool {
    if w.exited(pid) || !running(pid, stamp) {
        return false;
    }
    unsafe { libc::kill(pid as i32, sig) == 0 }
}

/// The leader and every other member of its session, each watched and stamped, or None if the leader is not provably
/// ours. The leader is watched and verified first, then the session is listed, then the leader is verified again: if it
/// held its pid across the listing, "session id equals its pid" really is its session. Each member is watched before its
/// stamp is read and must still be in that session.
fn snapshot(w: &mut ExitWatch, o: &Owner) -> Option<Vec<(u32, u64)>> {
    if !w.add(o.pid) || !running(o.pid, o.stamp) {
        return None;
    }
    let mut out = vec![(o.pid, o.stamp)];
    for p in crate::terminal::kill::session_members(o.pid) {
        let p = p as u32;
        if p == o.pid || !w.add(p) {
            continue;
        }
        let Some(stamp) = start_stamp(p) else { continue };
        if unsafe { libc::getsid(p as i32) } == o.pid as i32 && !out.iter().any(|&(q, _)| q == p) {
            out.push((p, stamp));
        }
    }
    (running(o.pid, o.stamp) && !w.exited(o.pid)).then_some(out)
}

/// End a recorded shell's session, but only if the record still proves it is ours.
///
/// The shell is watched for exit and its stamp checked, then its session is listed (`snapshot`), each member with its own
/// stamp: once a leader that died on SIGHUP is reaped its pid (and with it the session id) can be reused, so members that
/// ignored SIGHUP (`nohup`, `trap '' HUP`) could no longer be found. Every signal goes to one pid, and only after that
/// pid is checked not to have exited and to still match its stamp (`signal_verified`); there is no signal to a group or
/// to "everything in the session". The steps: SIGHUP each member; wait up to `GRACE`; list the session again while the
/// leader still proves itself (members that started since) and SIGKILL each member that still matches. `Ended` means each
/// of them is provably gone (`provably_gone`); otherwise `Survived` says how many are not.
pub fn recover(o: &Owner) -> Recovery {
    let mut w = ExitWatch::new();
    let Some(mut members) = snapshot(&mut w, o) else { return Recovery::NotOwned };
    for &(p, s) in &members {
        signal_verified(&mut w, p, s, libc::SIGHUP);
    }
    let t0 = Instant::now();
    while t0.elapsed() < GRACE && left(&mut w, &members) > 0 {
        std::thread::sleep(Duration::from_millis(25));
    }
    if let Some(now) = snapshot(&mut w, o) {
        for m in now {
            if !members.contains(&m) {
                members.push(m); // started after the first listing
            }
        }
    }
    for &(p, s) in &members {
        signal_verified(&mut w, p, s, libc::SIGKILL);
    }
    let t1 = Instant::now();
    while t1.elapsed() < Duration::from_secs(1) && left(&mut w, &members) > 0 {
        std::thread::sleep(Duration::from_millis(20));
    }
    // Fail closed: a member counts as ended only when it is provably gone (`gone_from`), never because its stamp went
    // unreadable while its pid is still held.
    match left(&mut w, &members) {
        0 => Recovery::Ended,
        remaining => Recovery::Survived { remaining },
    }
}

/// What `end_agents` did with one record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ended {
    /// It was ours and is gone now: killed here.
    Killed,
    /// It was ours, was signalled, and some of its session still runs (kept recorded, so the next start tries again).
    Survived,
    /// It had already exited.
    Gone,
    /// Its pid belongs to another process now, or one that could not be ours: left untouched.
    Reused,
    /// It survived `MAX_TRIES` start-ups' kills: the record is dropped.
    GivenUp,
    /// It survived, and its record could not be written back: the next start cannot try again.
    Unrecorded,
}

/// Is the recorded process a lead or worker this app's supervisor started? Each is its own session's leader (the PTY
/// child), runs as this user, is not this app, and carries the record's mark in its environment (`AGENT_MARK`), read
/// from the process itself. A record says nothing about provenance on its own: one without a mark, or whose process
/// does not carry it (another of the user's sessions, or a program whose environment macOS hides), is never signalled.
fn could_be_ours(o: &Owner) -> bool {
    let pid = o.pid;
    // SAFETY: getsid has no preconditions.
    let sid = unsafe { libc::getsid(pid as i32) };
    let Some(mark) = o.mark.as_deref().filter(|m| !m.is_empty()) else { return false };
    pid > 1
        && pid != std::process::id()
        && sid == pid as i32
        && crate::procsweep::is_same_user(pid)
        && crate::procsweep::env_var(pid, AGENT_MARK).is_some_and(|v| v.as_encoded_bytes() == mark.as_bytes())
}

/// SIGKILL a recorded agent's whole session through the 6c-3 sweep's proof (`procsweep::kill_proved`): the leader and
/// each member are listed with their own stamps while the leader proves itself (`snapshot`), and each is re-proved
/// immediately before its signal. An agent has no work to save, so there is no SIGHUP grace.
fn end_session(o: &Owner) -> Ended {
    let mut w = ExitWatch::new();
    let Some(members) = snapshot(&mut w, o) else { return Ended::Gone };
    drop(w);
    if crate::procsweep::kill_proved(members).survived == 0 { Ended::Killed } else { Ended::Survived }
}

pub fn ledger_path(data: &Path) -> PathBuf {
    data.join("owned.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};

    /// A session leader with a background job in its session, like a login shell with `sleep 300 &`.
    fn leader() -> (Child, Owner) {
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 300 & wait"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let ch = c.spawn().unwrap();
        // wait for the job to exist so the session has two members
        let t0 = Instant::now();
        while crate::terminal::kill::session_members(ch.id()).len() < 2 && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let o = Owner::shell(ch.id(), start_stamp(ch.id()).unwrap(), "/tmp".into());
        (ch, o)
    }
    fn alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    fn gone(mut c: Child) {
        c.kill().ok();
        c.wait().ok();
    }
    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vajra-owned-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join("owned.jsonl")
    }

    #[test]
    fn given_a_held_pid_whose_stamp_cannot_be_read_when_judged_then_it_still_runs() {
        // Today's rule (`!running`) called this gone and recover reported Ended for a process it could not prove dead.
        let held = Seen { exists: true, ..Seen::default() };
        assert!(!gone_from(held, 42), "an unreadable stamp on a held pid is SURVIVING, never ended");
        let reused_zombie = Seen { exists: true, zombie: Some(7), ..Seen::default() };
        assert!(!gone_from(reused_zombie, 42), "a zombie on a reused pid proves nothing about the recorded process");
    }

    #[test]
    fn given_each_proof_of_exit_when_judged_then_only_those_are_gone() {
        let s = 42;
        let base = Seen { exists: true, ..Seen::default() };
        assert!(gone_from(Seen { exited: true, ..base }, s), "the exit watch saw it exit");
        assert!(gone_from(Seen { exists: false, ..base }, s), "ESRCH: the pid is free");
        assert!(gone_from(Seen { stamp: Some(43), ..base }, s), "the pid holds another process now");
        assert!(gone_from(Seen { zombie: Some(s), ..base }, s), "a zombie of this very process");
        assert!(gone_from(Seen { stamp: Some(s), zombie: Some(s), ..base }, s), "a zombie whose stamp still reads (Linux)");
        assert!(gone_from(Seen { child_exited: true, ..base }, s), "our own child, exited and waiting to be reaped");
        assert!(!gone_from(Seen { stamp: Some(s), ..base }, s), "the same process, running");
        assert!(!gone_from(Seen { stamp: Some(s), child_exited: true, ..base }, s), "a readable live stamp outranks a child report");
    }

    #[test]
    fn given_a_live_child_then_a_killed_unreaped_one_when_read_then_only_the_killed_one_is_gone() {
        let mut c = Command::new("sleep").arg("30").spawn().unwrap();
        let (pid, stamp) = (c.id(), start_stamp(c.id()).unwrap());
        let mut w = ExitWatch::new();
        assert!(w.add(pid));
        assert!(!provably_gone(&mut w, pid, stamp), "a running child is not gone");
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        let t0 = Instant::now();
        while !provably_gone(&mut w, pid, stamp) && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(alive(pid), "not reaped yet: the pid is still held");
        assert!(provably_gone(&mut w, pid, stamp), "killed and unreaped is provably gone");
        assert!(child_exited(pid) && child_exited(pid), "WNOWAIT reads it without reaping it");
        c.wait().unwrap();
    }

    #[test]
    fn given_a_live_shell_with_its_recorded_stamp_when_reconciled_then_it_is_owned_with_its_session() {
        let (c, o) = leader();
        assert_eq!(judge(&o), Verdict::Owned { members: 2 });
        gone(c);
    }

    #[test]
    fn given_a_dead_pid_when_reconciled_then_it_is_gone() {
        let (mut c, o) = leader();
        c.kill().unwrap();
        c.wait().unwrap();
        assert_eq!(judge(&o), Verdict::Gone);
    }

    #[test]
    fn given_a_live_pid_with_another_start_stamp_when_reconciled_then_it_is_reused_and_not_ours() {
        let (c, mut o) = leader();
        o.stamp += 7;
        assert_eq!(judge(&o), Verdict::Reused);
        gone(c);
    }

    #[test]
    fn given_a_reused_pid_when_recovery_is_asked_then_nothing_is_signalled() {
        let (c, mut o) = leader();
        o.stamp += 7;
        assert_eq!(recover(&o), Recovery::NotOwned);
        std::thread::sleep(Duration::from_millis(100));
        assert!(alive(c.id()), "an unowned process must survive");
        assert!(crate::terminal::kill::session_members(c.id()).len() >= 2, "and so must its session");
        gone(c);
    }

    #[test]
    fn given_an_owned_shell_when_recovered_then_it_and_its_background_job_end() {
        let (mut c, o) = leader();
        let before = crate::terminal::kill::session_members(o.pid);
        assert_eq!(recover(&o), Recovery::Ended);
        c.wait().ok();
        for p in before {
            let t0 = Instant::now();
            while alive(p as u32) && t0.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(!alive(p as u32), "member {p} must have ended");
        }
    }

    /// A leader that dies on SIGHUP with a background job that ignores it (`nohup`, `trap '' HUP`): the leader's pid is
    /// then free to be reused, and the job is only findable by the snapshot taken before the hangup.
    fn hup_ignoring_job() -> (Child, Owner) {
        let mut c = Command::new("sh");
        c.args(["-c", "(trap '' HUP; exec sleep 300) & wait"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let ch = c.spawn().unwrap();
        let t0 = Instant::now();
        while crate::terminal::kill::session_members(ch.id()).len() < 2 && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(150)); // the subshell has exec'd sleep with HUP ignored
        let o = Owner::shell(ch.id(), start_stamp(ch.id()).unwrap(), "/tmp".into());
        (ch, o)
    }

    #[test]
    fn given_a_job_that_ignores_hangup_when_the_leader_exits_then_the_job_is_killed_and_ended_is_honest() {
        let (c, o) = hup_ignoring_job();
        let job: Vec<i32> = crate::terminal::kill::session_members(o.pid).into_iter().filter(|&p| p as u32 != o.pid).collect();
        assert!(!job.is_empty(), "the background job exists");
        // After a crash the leader is not our child: something else reaps it at once.
        let reaper = std::thread::spawn(move || {
            let mut c = c;
            c.wait().ok();
        });
        assert_eq!(recover(&o), Recovery::Ended);
        reaper.join().unwrap();
        for p in job {
            // `kill(pid, 0)` succeeds for an exited job its new parent has not reaped yet; that is not running.
            assert!(!alive(p as u32) || crate::procid::is_zombie(p as u32), "the HUP-ignoring job {p} must be gone, not reported ended while running");
        }
    }

    #[test]
    fn a_member_that_survives_is_reported_as_survived_not_ended() {
        let v = serde_json::to_value(Recovery::Survived { remaining: 2 }).unwrap();
        assert_eq!(v, serde_json::json!({"survived": {"remaining": 2}}));
        assert_eq!(serde_json::to_value(Recovery::Ended).unwrap(), serde_json::json!("ended"));
    }

    #[test]
    fn given_a_gone_pid_when_recovery_is_asked_then_it_is_not_owned() {
        let (mut c, o) = leader();
        c.kill().unwrap();
        c.wait().unwrap();
        assert_eq!(recover(&o), Recovery::NotOwned);
    }

    #[test]
    fn given_records_when_settled_then_only_owned_ones_stay_in_the_file() {
        let (c1, o1) = leader();
        let (c2, mut o2) = leader();
        o2.stamp += 1;
        let (mut c3, o3) = leader();
        c3.kill().unwrap();
        c3.wait().unwrap();
        let l = Ledger::new(tmp("settle"));
        for o in [&o1, &o2, &o3] {
            l.record(o.clone()).unwrap();
        }
        let found = l.settle();
        assert_eq!(found.len(), 3);
        assert_eq!(l.load(), vec![o1]);
        gone(c1);
        gone(c2);
    }

    #[test]
    fn the_ledger_round_trips_forgets_and_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let l = Ledger::new(tmp("rt"));
        let a = Owner::shell(4242, 1, "/a".into());
        let b = Owner::shell(4243, 2, "/b".into());
        l.record(a.clone()).unwrap();
        l.record(b.clone()).unwrap();
        l.record(a.clone()).unwrap(); // no duplicate
        assert_eq!(l.load(), vec![b.clone(), a.clone()]);
        l.forget(4243, 99).unwrap(); // a stamp mismatch forgets nothing
        assert_eq!(l.load().len(), 2);
        l.forget(4243, 2).unwrap();
        assert_eq!(l.load(), vec![a]);
        assert_eq!(std::fs::metadata(&l.path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn closing_a_tab_forgets_only_the_owner_it_recorded_even_when_the_pid_was_reused() {
        let l = Ledger::new(tmp("reuse"));
        // An ended terminal (id 1) that is still in the tab list, and a newer shell (id 2) that got the same pid.
        l.record_for(1, Owner::shell(4242, 10, "/old".into())).unwrap();
        l.record_for(2, Owner::shell(4242, 20, "/new".into())).unwrap();
        l.forget_for(1).unwrap();
        assert_eq!(l.load(), vec![Owner::shell(4242, 20, "/new".into())], "the newer shell stays recoverable");
        l.forget_for(1).unwrap(); // closing it again, or a terminal that was never recorded, changes nothing
        l.forget_for(99).unwrap();
        assert_eq!(l.load().len(), 1);
        l.forget_for(2).unwrap();
        assert!(l.load().is_empty());
    }

    #[test]
    fn an_oversized_ledger_is_read_only_up_to_the_byte_cap() {
        use std::io::Write as _;
        let p = tmp("huge");
        let first = "{\"pid\":4242,\"stamp\":7,\"cwd\":\"/a\"}\n";
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(first.as_bytes()).unwrap();
        f.set_len(256 * 1024 * 1024).unwrap(); // a sparse 256 MiB file: reading it whole would allocate that much
        drop(f);
        let text = read_capped(&p).unwrap();
        assert!(text.len() as u64 <= MAX_BYTES, "read {} bytes", text.len());
        assert_eq!(Ledger::new(p).load(), vec![Owner::shell(4242, 7, "/a".into())], "what is inside the cap still loads");
    }

    #[test]
    fn a_ledger_with_bytes_that_are_not_utf8_still_loads_the_good_lines() {
        let p = tmp("nonutf8");
        let mut b = b"\xff\xfe garbage\n".to_vec();
        b.extend_from_slice(b"{\"pid\":4242,\"stamp\":7,\"cwd\":\"/a\"}\n");
        std::fs::write(&p, b).unwrap();
        assert_eq!(Ledger::new(p).load().len(), 1);
    }

    #[test]
    fn a_recovery_that_left_members_running_keeps_the_record_and_the_retry_and_only_an_end_or_not_owned_settles_it() {
        let l = Ledger::new(tmp("survived"));
        let o = Owner::shell(4242, 7, "/a".into());
        let other = Owner::shell(4243, 8, "/b".into());
        l.record(o.clone()).unwrap();
        l.record(other.clone()).unwrap();
        let mut found = vec![Found { owner: o.clone(), verdict: Verdict::Owned { members: 3 } }, Found { owner: other.clone(), verdict: Verdict::Owned { members: 1 } }];
        // Survived: still on disk (a restart can find it again) and still offered, with the count that is really left.
        apply_recovery(&l, &mut found, &o, &Recovery::Survived { remaining: 2 });
        assert!(l.load().contains(&o), "the durable record stays");
        assert_eq!(found[0], Found { owner: o.clone(), verdict: Verdict::Owned { members: 2 } }, "still offered for another attempt, with what is left");
        // Ended and NotOwned settle it: gone from the ledger and from the list.
        apply_recovery(&l, &mut found, &o, &Recovery::Ended);
        assert_eq!(l.load(), vec![other.clone()]);
        assert_eq!(found.len(), 1);
        apply_recovery(&l, &mut found, &other, &Recovery::NotOwned);
        assert!(l.load().is_empty() && found.is_empty());
    }

    #[test]
    fn a_damaged_or_hostile_file_is_bounded_and_does_not_panic() {
        let p = tmp("bad");
        let mut s = String::from("not json\n{\"pid\":0,\"stamp\":1,\"cwd\":\"/\"}\n");
        s.push_str(&format!("{{\"pid\":5,\"stamp\":1,\"cwd\":\"{}\"}}\n", "x".repeat(MAX_LINE)));
        for i in 0..(MAX_ENTRIES + 50) {
            s.push_str(&format!("{{\"pid\":{},\"stamp\":1,\"cwd\":\"/\"}}\n", i + 2));
        }
        std::fs::write(&p, s).unwrap();
        let l = Ledger::new(p);
        assert_eq!(l.load().len(), MAX_ENTRIES);
    }

    #[test]
    fn no_signal_is_ever_sent_by_name() {
        let src = include_str!("owned.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for bad in ["killall", "pkill", "pgrep", "\"ps\""] {
            assert!(!code.contains(bad), "{bad}");
        }
    }

    #[test]
    fn given_a_process_that_exited_when_watched_then_it_is_reported_and_nothing_is_signalled_to_it() {
        let mut c = Command::new("sleep").arg("300").spawn().unwrap();
        let stamp = start_stamp(c.id()).unwrap();
        let mut w = ExitWatch::new();
        assert!(w.add(c.id()));
        assert!(!w.exited(c.id()), "alive: no exit event");
        c.kill().unwrap(); // dead but not reaped: its pid is still held, as for an orphan's zombie
        let t0 = Instant::now();
        while !w.exited(c.id()) && t0.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(w.exited(c.id()), "the exit is reported");
        assert!(!signal_verified(&mut w, c.id(), stamp, libc::SIGTERM), "an exited process is not signalled");
        c.wait().unwrap();
    }

    #[test]
    fn given_a_live_process_with_another_stamp_when_signalled_then_it_is_skipped_and_keeps_running() {
        let c = Command::new("sleep").arg("300").spawn().unwrap();
        let stamp = start_stamp(c.id()).unwrap();
        let mut w = ExitWatch::new();
        assert!(w.add(c.id()));
        assert!(!signal_verified(&mut w, c.id(), stamp + 3, libc::SIGKILL), "not the recorded process");
        std::thread::sleep(Duration::from_millis(100));
        assert!(alive(c.id()), "the bystander was not touched");
        assert!(signal_verified(&mut w, c.id(), stamp, libc::SIGTERM), "the matching process is signalled");
        gone(c);
    }

    #[test]
    fn given_a_pid_that_does_not_exist_when_watched_then_it_is_refused() {
        let c = Command::new("true").spawn().unwrap();
        let pid = c.id();
        let mut c = c;
        c.wait().unwrap();
        let mut w = ExitWatch::new();
        assert!(!w.add(pid), "a reaped pid cannot be watched, so recovery stops at NotOwned");
    }

    #[test]
    fn given_a_member_that_exits_during_the_loop_when_the_loop_reaches_it_then_it_is_skipped() {
        let (c, o) = leader();
        let mut w = ExitWatch::new();
        let members = snapshot(&mut w, &o).unwrap();
        assert_eq!(members.len(), 2, "the shell and its job");
        let (job, job_stamp) = *members.iter().find(|&&(p, _)| p != o.pid).unwrap();
        unsafe { libc::kill(job as i32, libc::SIGKILL) }; // the job dies on its own before the loop reaches it
        let t0 = Instant::now();
        while !w.exited(job) && t0.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!signal_verified(&mut w, job, job_stamp, libc::SIGHUP), "skipped: it exited");
        gone(c);
    }

    #[test]
    fn given_a_symlink_planted_at_the_old_temp_name_when_the_ledger_is_written_then_its_target_is_untouched() {
        use std::os::unix::fs::symlink;
        let path = tmp("symlink");
        let dir = path.parent().unwrap().to_path_buf();
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, "precious").unwrap();
        for name in ["owned.tmp", "owned.jsonl.tmp"] {
            let _ = std::fs::remove_file(dir.join(name));
            symlink(&victim, dir.join(name)).unwrap();
        }
        let l = Ledger::new(path.clone());
        l.record(Owner::shell(std::process::id(), start_stamp(std::process::id()).unwrap(), "/tmp".into())).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious", "the link's target was not truncated or written");
        assert_eq!(l.load().len(), 1, "the ledger itself was written");
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".tmp") && n.starts_with("owned.jsonl.") && n != "owned.jsonl.tmp").collect();
        assert!(left.is_empty(), "no temp file is left behind: {left:?}");
    }

    #[test]
    fn given_a_fifo_or_a_symlink_at_the_ledger_path_when_read_then_it_is_empty_and_never_blocks() {
        use std::os::unix::ffi::OsStrExt;
        let path = tmp("fifo");
        let dir = path.parent().unwrap().to_path_buf();
        // a FIFO with no writer: a plain open would block for ever
        let _ = std::fs::remove_file(&path);
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let p2 = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Ledger::new(p2).load().len());
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).expect("reading a FIFO must not block"), 0);
        // a symlink to a real ledger is not followed
        let real = dir.join("real.jsonl");
        std::fs::write(&real, format!("{}\n", serde_json::to_string(&Owner::shell(4242, 1, "/tmp".into())).unwrap())).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();
        assert_eq!(Ledger::new(path.clone()).load().len(), 0, "the link is refused");
        // a regular file still reads
        std::fs::remove_file(&path).unwrap();
        std::fs::copy(&real, &path).unwrap();
        assert_eq!(Ledger::new(path).load().len(), 1);
    }

    #[test]
    fn given_a_close_in_progress_when_it_succeeds_or_fails_then_the_record_is_there_during_it_and_gone_after() {
        for outcome in [Ok(()), Err("not_found")] {
            let l = Ledger::new(tmp(&format!("close-{}", outcome.is_ok())));
            let me = std::process::id();
            l.record_for(7, Owner::shell(me, start_stamp(me).unwrap(), "/tmp".into())).unwrap();
            let r = l.close_then_forget(7, || {
                assert_eq!(l.load().len(), 1, "the record is still recoverable while the shell is being stopped");
                outcome
            });
            assert_eq!(r, outcome, "the close result is returned as it was");
            assert!(l.load().is_empty(), "forgotten once the close is over, whatever it returned");
        }
    }
}
