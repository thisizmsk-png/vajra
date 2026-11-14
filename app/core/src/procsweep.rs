//! The descendant sweep (6c-3): a worker's process that left its process group (`setsid`, a daemon, a double fork) is
//! out of reach of the group kill, and would outlive its ticket and the app. A process is swept only when all of these
//! hold, read immediately before each signal (`owned::signal_verified`):
//!
//! - it runs as this user, and is not this app;
//! - it carries the worker's mark: it was seen holding the worker's own terminal while the worker lived (`holders`,
//!   polled during the attempt, since macOS revokes the terminal when the worker exits), or its `TMPDIR` is the
//!   ticket's;
//! - it started inside the window: at or after the worker's spawn (ticket end), or before this app started (startup);
//! - it still has the start stamp it had when it was matched.
//!
//! A process is never matched by name, cwd or path alone: a user's own shell opened in a worker's worktree has its own
//! terminal and the user's `TMPDIR`, so it never carries a mark. macOS hides the environment of Apple's platform binaries
//! (`sh`, `perl`, `sleep`), so for those only the terminal mark counts. A process that closed the terminal and cleared
//! its `TMPDIR` (or is a platform binary that closed the terminal, or was spawned and detached after the attempt's last
//! terminal poll) is missed: a false negative, never a wrong kill.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::owned::{ExitWatch, left, provably_gone, signal_verified};
use crate::procid::start_stamp;

/// What one sweep did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Swept {
    /// Matched and gone after SIGKILL.
    pub killed: usize,
    /// Matched, signalled, and still running after a second (an uninterruptible wait).
    pub survived: usize,
}

impl Swept {
    fn add(&mut self, o: Swept) {
        self.killed += o.killed;
        self.survived += o.survived;
    }
}

/// The start-stamp window a process must have started in. Units are `procid::start_stamp`'s.
#[derive(Debug, Clone, Copy)]
pub enum Window {
    /// At or after this stamp: the worker's spawn (`now_stamp` taken before it).
    Since(u64),
    /// Strictly before this stamp: this app's own start (an earlier app life's leftovers).
    Before(u64),
}

impl Window {
    fn holds(self, s: u64) -> bool {
        match self {
            Window::Since(f) => s >= f,
            Window::Before(c) => s < c,
        }
    }
}

/// What makes a process one worker's, read at sweep time.
enum Mark<'a> {
    /// The ticket's own `TMPDIR`.
    Ticket(&'a Path),
    /// Any ticket dir under this work root as its `TMPDIR` (`<work>/<project>/<run>/tmp/<leaf>`, exactly that depth).
    AnyTicketUnder(&'a Path),
}

impl Mark<'_> {
    fn matches(&self, pid: u32) -> bool {
        match self {
            Mark::Ticket(tmp) => env_var(pid, "TMPDIR").is_some_and(|t| t == tmp.as_os_str()),
            Mark::AnyTicketUnder(work) => env_var(pid, "TMPDIR").is_some_and(|t| is_ticket_tmp(work, Path::new(&t))),
        }
    }
}

/// The processes of this user, started at or after `since`, that hold `tty` open now: `(pid, start stamp)`. Read while
/// the worker lives: macOS revokes a terminal when its session leader exits, and its descriptors then no longer name it.
///
/// A holder must also descend from the worker (`worker`, its leader's pid) or have left every app process behind: its
/// parent chain passes through `worker`, or reaches launchd without passing through this app. A process this app
/// spawned while the terminal was open (git, another worker) can inherit it for an instant; it still has the app in its
/// chain, so it is never taken.
pub fn holders(tty: &Path, since: u64, worker: u32) -> Vec<(u32, u64)> {
    let me = std::process::id();
    all_pids()
        .into_iter()
        .filter(|&p| p != me && p > 1 && same_user(p))
        .filter_map(|p| Some((p, start_stamp(p)?)))
        .filter(|&(p, s)| s >= since && open_paths(p).iter().any(|o| o == tty) && detached_from_app(p, worker, me) && start_stamp(p) == Some(s))
        .collect()
}

/// Walk `pid`'s parents: true if `worker` is met before `app`, or launchd is reached without meeting `app`.
fn detached_from_app(pid: u32, worker: u32, app: u32) -> bool {
    if pid == worker {
        return true;
    }
    let mut p = pid;
    for _ in 0..64 {
        match ppid_of(p) {
            Some(q) if q == worker => return true,
            Some(q) if q == app => return false,
            Some(q) if q <= 1 => return true,
            Some(q) => p = q,
            None => return false,
        }
    }
    false
}

fn ppid_of(pid: u32) -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let got = unsafe { libc::proc_pidinfo(pid as i32, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
        (got == size).then_some(info.pbi_ppid)
    }
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

/// A stamp for "now" in `procid::start_stamp`'s units, a little early, so a process spawned right after it is never
/// before it.
pub fn now_stamp() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let us = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_micros() as u64;
        Some(us.saturating_sub(1_000))
    }
    #[cfg(target_os = "linux")]
    {
        let up: f64 = std::fs::read_to_string("/proc/uptime").ok()?.split_whitespace().next()?.parse().ok()?;
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        (hz > 0).then(|| ((up * hz as f64) as u64).saturating_sub(1))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Sweep one ticket's leftovers after its attempt ended: the terminal holders `seen` while it ran (each still the same
/// process), and every process with its `TMPDIR`, started at or after `since`.
pub fn sweep_ticket(seen: &[(u32, u64)], tmp: &Path, since: u64) -> Swept {
    let mut total = kill_proved(seen.iter().copied().filter(|&(_, s)| s >= since).collect());
    total.add(sweep(&Mark::Ticket(tmp), Window::Since(since)));
    total
}

/// SIGKILL each `(pid, stamp)` that is still that process, then wait up to a second for them to go. Fail closed: one is
/// counted killed only once it is provably gone (`owned::provably_gone`); a held pid whose stamp cannot be read survives.
pub(crate) fn kill_proved(hits: Vec<(u32, u64)>) -> Swept {
    let mut watch = ExitWatch::new();
    let hits: Vec<(u32, u64)> = hits.into_iter().filter(|&(p, s)| watch.add(p) && !provably_gone(&mut watch, p, s)).collect();
    for &(p, s) in &hits {
        signal_verified(&mut watch, p, s, libc::SIGKILL);
    }
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(1) && left(&mut watch, &hits) > 0 {
        std::thread::sleep(Duration::from_millis(20));
    }
    let survived = left(&mut watch, &hits);
    Swept { killed: hits.len() - survived, survived }
}

/// At startup, before any agent: an earlier app life's worker processes, by their ticket `TMPDIR` under `work`.
pub fn sweep_earlier_lives(work: &Path) -> Swept {
    let Some(me) = start_stamp(std::process::id()) else { return Swept::default() };
    sweep_under(work, me)
}

/// `sweep_earlier_lives` with its cutoff given: processes started before `before`.
fn sweep_under(work: &Path, before: u64) -> Swept {
    sweep(&Mark::AnyTicketUnder(work), Window::Before(before))
}

/// `<work>/<project>/<run>/tmp/<leaf>`, with plain components only.
fn is_ticket_tmp(work: &Path, t: &Path) -> bool {
    use std::path::Component;
    let Ok(rest) = t.strip_prefix(work) else { return false };
    let parts: Vec<_> = rest.components().collect();
    parts.len() == 4 && parts.iter().all(|c| matches!(c, Component::Normal(_))) && parts[2] == Component::Normal("tmp".as_ref())
}

/// Match, then SIGKILL each match through the stamp re-check. Repeats (at most 3 passes) while a pass still kills
/// something, so a child forked during the sweep is found by the next pass.
fn sweep(mark: &Mark, w: Window) -> Swept {
    let mut total = Swept::default();
    let me = std::process::id();
    for _ in 0..3 {
        let mut watch = ExitWatch::new();
        let mut hits: Vec<(u32, u64)> = Vec::new();
        for pid in all_pids() {
            if pid == me || pid <= 1 || !same_user(pid) || !watch.add(pid) {
                continue;
            }
            let Some(stamp) = start_stamp(pid) else { continue };
            if w.holds(stamp) && mark.matches(pid) && start_stamp(pid) == Some(stamp) {
                hits.push((pid, stamp));
            }
        }
        if hits.is_empty() {
            break;
        }
        let n = hits.len();
        let s = kill_proved(hits);
        total.add(s);
        if s.survived == n {
            break; // nothing more will die by trying again
        }
    }
    total
}

/// Does `pid` run as this user?
pub(crate) fn is_same_user(pid: u32) -> bool {
    same_user(pid)
}

fn same_user(pid: u32) -> bool {
    uid_of(pid) == Some(unsafe { libc::getuid() })
}

fn uid_of(pid: u32) -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let got = unsafe { libc::proc_pidinfo(pid as i32, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
        (got == size).then_some(info.pbi_uid)
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(format!("/proc/{pid}")).ok().map(|m| m.uid())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

fn all_pids() -> Vec<u32> {
    #[cfg(target_os = "macos")]
    {
        let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if n <= 0 {
            return Vec::new();
        }
        // Room for processes that start between the two calls.
        let mut buf = vec![0 as libc::pid_t; n as usize + 64];
        let bytes = (buf.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        let got = unsafe { libc::proc_listallpids(buf.as_mut_ptr().cast(), bytes) };
        buf.truncate(got.max(0) as usize);
        buf.into_iter().filter(|&p| p > 0).map(|p| p as u32).collect()
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/proc").into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Vec::new()
    }
}

/// The most file descriptors of one process that are looked at.
const MAX_FDS: usize = 4096;

/// XNU's `struct vnode_fdinfowithpath` (`sys/proc_info.h`): a `proc_fileinfo` (24 bytes), a `vnode_info` (152), then
/// the path (`MAXPATHLEN`). Only the path is read. Not in `libc`.
#[cfg(target_os = "macos")]
#[repr(C)]
struct VnodeFdInfoWithPath {
    _fi: [u8; 24],
    _vi: [u8; 152],
    path: [u8; 1024],
}

#[cfg(target_os = "macos")]
const PROC_PIDFDVNODEPATHINFO: libc::c_int = 2;

/// The paths of `pid`'s open vnode descriptors (files, devices), at most `MAX_FDS` of them.
fn open_paths(pid: u32) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let fd_size = std::mem::size_of::<libc::proc_fdinfo>();
        let mut fds: Vec<libc::proc_fdinfo> = vec![unsafe { std::mem::zeroed() }; MAX_FDS];
        let bytes = (fds.len() * fd_size) as libc::c_int;
        let got = unsafe { libc::proc_pidinfo(pid as i32, libc::PROC_PIDLISTFDS, 0, fds.as_mut_ptr().cast(), bytes) };
        if got <= 0 {
            return Vec::new();
        }
        fds.truncate(got as usize / fd_size);
        let mut out = Vec::new();
        for f in fds.iter().filter(|f| f.proc_fdtype == libc::PROX_FDTYPE_VNODE as u32) {
            let mut v = VnodeFdInfoWithPath { _fi: [0; 24], _vi: [0; 152], path: [0; 1024] };
            let size = std::mem::size_of::<VnodeFdInfoWithPath>() as libc::c_int;
            let r = unsafe { libc::proc_pidfdinfo(pid as i32, f.proc_fd, PROC_PIDFDVNODEPATHINFO, (&mut v as *mut VnodeFdInfoWithPath).cast(), size) };
            if r == size {
                let raw: Vec<u8> = v.path.iter().take_while(|&&c| c != 0).copied().collect();
                out.push(PathBuf::from(OsStr::from_bytes(&raw)));
            }
        }
        out
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir(format!("/proc/{pid}/fd")).into_iter().flatten().flatten().take(MAX_FDS).filter_map(|e| std::fs::read_link(e.path()).ok()).collect()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        Vec::new()
    }
}

/// The most of one process's argument and environment block that is read (macOS's own limit is 1 MiB).
const MAX_ARGS: usize = 1 << 20;

/// One variable of `pid`'s environment as it was at exec, or `None` (hidden, gone, or unset).
pub(crate) fn env_var(pid: u32, name: &str) -> Option<OsString> {
    use std::os::unix::ffi::OsStrExt;
    let block = env_block(pid)?;
    let key = format!("{name}=");
    block.split(|&b| b == 0).find_map(|kv| kv.strip_prefix(key.as_bytes())).map(|v| OsStr::from_bytes(v).to_owned())
}

/// The `NAME=value` strings, NUL-separated.
fn env_block(pid: u32) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        // KERN_PROCARGS2: argc (i32), the exec path, NUL padding, argc arguments, then the environment.
        // One buffer per thread, reused for every process a sweep looks at (a sweep reads hundreds).
        thread_local!(static BUF: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(vec![0u8; MAX_ARGS]));
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
        let buf: Vec<u8> = BUF.with_borrow_mut(|b| {
            let mut len = b.len();
            let r = unsafe { libc::sysctl(mib.as_mut_ptr(), 3, b.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) };
            (r == 0 && len >= 4).then(|| b[..len].to_vec())
        })?;
        let argc = i32::from_ne_bytes(buf[..4].try_into().ok()?).max(0) as usize;
        let mut i = 4;
        while i < buf.len() && buf[i] != 0 {
            i += 1; // the exec path
        }
        while i < buf.len() && buf[i] == 0 {
            i += 1; // its padding
        }
        for _ in 0..argc {
            while i < buf.len() && buf[i] != 0 {
                i += 1;
            }
            i += 1;
        }
        Some(buf.get(i..)?.to_vec())
    }
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        let mut v = Vec::new();
        std::fs::File::open(format!("/proc/{pid}/environ")).ok()?.take(MAX_ARGS as u64).read_to_end(&mut v).ok()?;
        Some(v)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vajra-sweep-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    /// A sleeper whose environment is readable (this test binary is not an Apple platform binary), with `TMPDIR` set.
    fn sleeper(tmpdir: &Path) -> Child {
        Command::new(std::env::current_exe().unwrap()).args(["--exact", "procsweep::tests::sleeper_body", "--ignored", "--nocapture"]).env("TMPDIR", tmpdir).env("VAJRA_SWEEP_SLEEP", "1").stdin(Stdio::null()).stdout(Stdio::null()).spawn().unwrap()
    }

    /// The body `sleeper` runs; ignored unless asked for.
    #[test]
    #[ignore]
    fn sleeper_body() {
        if std::env::var_os("VAJRA_SWEEP_SLEEP").is_some() {
            std::thread::sleep(Duration::from_secs(600));
        }
    }

    fn alive(c: &mut Child) -> bool {
        c.try_wait().unwrap().is_none()
    }

    fn wait_readable(c: &Child, d: &Path) {
        let t0 = Instant::now();
        while !env_var(c.id(), "TMPDIR").is_some_and(|t| t == d.as_os_str()) {
            assert!(t0.elapsed() < Duration::from_secs(10), "its TMPDIR never became readable");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_process_with_the_tickets_tmpdir_started_after_the_spawn_is_killed() {
        let d = tmp("hit");
        let floor = now_stamp().unwrap();
        let mut c = sleeper(&d);
        wait_readable(&c, &d);
        assert_eq!(sweep_ticket(&[], &d, floor), Swept { killed: 1, survived: 0 });
        std::thread::sleep(Duration::from_millis(20));
        assert!(!alive(&mut c));
    }

    #[test]
    fn a_platform_binary_holding_the_workers_terminal_is_killed_by_the_terminal_mark() {
        let d = tmp("tty");
        let floor = now_stamp().unwrap();
        // A plain file stands in for the terminal device: the mark is "holds this path open".
        let dev = d.join("tty");
        std::fs::write(&dev, "").unwrap();
        let f = std::fs::File::open(&dev).unwrap();
        let mut c = Command::new("/bin/sleep").arg("600").stdin(f).spawn().unwrap();
        let t0 = Instant::now();
        while !open_paths(c.id()).iter().any(|p| p == &dev) {
            assert!(t0.elapsed() < Duration::from_secs(10), "its open file never became visible");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(env_var(c.id(), "TMPDIR"), None, "a platform binary's environment is hidden on macOS");
        let seen = holders(&dev, floor, c.id());
        assert_eq!(seen.iter().map(|h| h.0).collect::<Vec<_>>(), vec![c.id()], "the one holder, this user's, after the floor");
        // Named as anyone but itself, a process this app spawned (its parent is this test process) is never a holder:
        // an app child that inherited the descriptor for an instant is not a worker's descendant.
        assert!(holders(&dev, floor, 0).is_empty());
        assert_eq!(sweep_ticket(&seen,&d.join("not-its-tmp"), floor), Swept { killed: 1, survived: 0 });
        std::thread::sleep(Duration::from_millis(20));
        assert!(!alive(&mut c));
    }

    #[test]
    fn another_tmpdir_or_a_process_older_than_the_spawn_is_untouched() {
        let (d, other) = (tmp("mine"), tmp("other"));
        let mut older = sleeper(&d);
        let mut stranger = sleeper(&other);
        wait_readable(&older, &d);
        wait_readable(&stranger, &other);
        let after = now_stamp().unwrap() + 5_000_000;
        assert_eq!(sweep_ticket(&[], &d, after), Swept::default(), "started before the worker: not its descendant");
        assert_eq!(sweep_ticket(&holders(&d.join("no-such-tty"), 0, 0), &d.join("sub"), 0), Swept::default(), "another TMPDIR never matches");
        assert!(alive(&mut older) && alive(&mut stranger));
        for c in [&mut older, &mut stranger] {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Cost of one startup sweep over this machine's process table (run with `--ignored`; prints, asserts a ceiling).
    #[test]
    #[ignore]
    fn a_startup_sweep_is_cheap() {
        let w = tmp("cost");
        let t0 = Instant::now();
        let n = all_pids().len();
        sweep_earlier_lives(&w);
        let e = t0.elapsed();
        println!("startup sweep: {n} processes in {e:?}");
        assert!(e < Duration::from_millis(500), "{n} processes took {e:?}");
    }

    #[test]
    fn a_startup_sweep_takes_only_an_earlier_lives_ticket_dirs_under_the_work_root() {
        let w = tmp("work");
        assert!(is_ticket_tmp(&w, &w.join("p/r1/tmp/t1-r1")));
        for p in ["p/r1/tmp", "p/r1/t1-r1/tmp/x", "p/r1/tmp/t1-r1/deeper", "p/r1/tmp/../tmp/t1-r1"] {
            assert!(!is_ticket_tmp(&w, &w.join(p)), "{p}");
        }
        assert!(!is_ticket_tmp(&w, Path::new("/tmp/p/r1/tmp/t1-r1")));
        // A worker of this app life (started after this test process) is not an earlier life's.
        let t = w.join("p/r1/tmp/t1-r1");
        std::fs::create_dir_all(&t).unwrap();
        let mut c = sleeper(&t);
        wait_readable(&c, &t);
        assert_eq!(sweep_earlier_lives(&w), Swept::default());
        assert!(alive(&mut c));
        // Seen from a later app life (a cutoff after it started), the same worker is an earlier life's: killed. One
        // whose TMPDIR is the run's shared `tmp`, not a ticket's, is not.
        let mut shared = sleeper(&w.join("p/r1/tmp"));
        wait_readable(&shared, &w.join("p/r1/tmp"));
        let later = now_stamp().unwrap() + 5_000_000;
        assert_eq!(sweep_under(&w, later), Swept { killed: 1, survived: 0 });
        std::thread::sleep(Duration::from_millis(20));
        assert!(!alive(&mut c) && alive(&mut shared));
        let _ = shared.kill();
        let _ = shared.wait();
    }
}
