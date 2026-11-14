//! Process identity that survives pid reuse: a pid is only ever trusted together with the time that process started.
//! Everything that signals a process it did not just spawn (crash recovery, FG-24) goes through `same_process`.

/// An opaque, stable start stamp of `pid` (microseconds since the epoch on macOS, clock ticks since boot on Linux), or
/// `None` when the process does not exist or cannot be read. Two reads of one process agree; a reused pid differs.
pub fn start_stamp(pid: u32) -> Option<u64> {
    if pid <= 1 || pid > i32::MAX as u32 {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let got = unsafe { libc::proc_pidinfo(pid as i32, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
        (got == size).then(|| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
    }
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // Field 2 (comm) may hold spaces and parentheses: fields restart after the last ')'. starttime is field 22.
        stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Has `pid` ended but not yet been reaped (a zombie)? It holds nothing and can run nothing, so liveness checks that ask
/// "is it still running" count it as gone. False when the process does not exist or cannot be read.
pub fn is_zombie(pid: u32) -> bool {
    zombie_stamp(pid).is_some()
}

/// If `pid` is a zombie, the start stamp it had (same units as `start_stamp`), so a caller can tell that the zombie is the
/// very process it recorded and not a later one on a reused pid; `None` if it is not a zombie, does not exist, or cannot
/// be read. On macOS `proc_pidinfo` cannot read a zombie (it fails with ESRCH while `kill(pid, 0)` still succeeds), so the
/// state and start time are read through `sysctl(KERN_PROC_PID)`, which still reports one.
pub fn zombie_stamp(pid: u32) -> Option<u64> {
    if pid <= 1 || pid > i32::MAX as u32 {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let k = kinfo(pid)?;
        let t = k.p_starttime;
        (k.p_stat == SZOMB).then(|| t.tv_sec as u64 * 1_000_000 + t.tv_usec as u64)
    }
    #[cfg(target_os = "linux")]
    {
        let z = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().and_then(|s| s.rsplit_once(')').map(|(_, r)| r.trim_start().starts_with('Z'))).unwrap_or(false);
        if z { start_stamp(pid) } else { None }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// The start stamp of `pid`, a child the caller spawned and has not reaped yet: the live stamp, or, if the child already
/// exited, the stamp of its zombie. On macOS `proc_pidinfo` cannot read a zombie, so a child that exits at once (`true`)
/// would otherwise look unreadable. Only for the caller's own unreaped child: its pid cannot be reused until it is reaped,
/// so the zombie is that very child. Not for pids read from a record (use `start_stamp` and `zombie_stamp` there).
pub fn child_stamp(pid: u32) -> Option<u64> {
    start_stamp(pid).or_else(|| zombie_stamp(pid))
}

/// `p_stat` of a zombie (sys/proc.h).
#[cfg(target_os = "macos")]
const SZOMB: libc::c_char = 5;

/// `sizeof(struct kinfo_proc)` on 64-bit macOS (arm64 and x86_64).
#[cfg(target_os = "macos")]
const KINFO_PROC_SIZE: usize = 648;

/// The head of `struct extern_proc` (sys/proc.h), the first member of `struct kinfo_proc`; the libc crate does not
/// define it for macOS. `p_starttime` is the `p_un` union's timeval member (the other is two pointers, also 16 bytes).
#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct ExternProcHead {
    p_starttime: libc::timeval,
    p_vmspace: *mut libc::c_void,
    p_sigacts: *mut libc::c_void,
    p_flag: libc::c_int,
    p_stat: libc::c_char,
    p_pid: libc::pid_t,
}

/// The head of `pid`'s kernel process record through `sysctl(KERN_PROC_PID)`, or `None` when there is no such process or
/// the record does not have the expected size and pid (so a changed layout claims nothing). Unlike `proc_pidinfo` this
/// also reads a zombie.
#[cfg(target_os = "macos")]
fn kinfo(pid: u32) -> Option<ExternProcHead> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid as libc::c_int];
    // Larger than the record and 8-byte aligned, so the head can be read in place.
    let mut buf = [0u64; 128];
    let mut len = std::mem::size_of_val(&buf);
    // SAFETY: `buf` is a writable buffer of `len` bytes; no new value is set.
    let rc = unsafe { libc::sysctl(mib.as_mut_ptr(), mib.len() as libc::c_uint, buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) };
    // A pid with no process succeeds with nothing written.
    if rc != 0 || len != KINFO_PROC_SIZE {
        return None;
    }
    // SAFETY: `buf` holds `KINFO_PROC_SIZE` initialised bytes, more than the head, and is aligned for it.
    let head = unsafe { *buf.as_ptr().cast::<ExternProcHead>() };
    (head.p_pid == pid as libc::pid_t).then_some(head)
}

/// Is `pid` still the very process that started at `stamp`? False for a dead pid, a reused pid, or an unreadable one.
pub fn same_process(pid: u32, stamp: u64) -> bool {
    start_stamp(pid) == Some(stamp)
}

/// `pid`'s parent pid and its own start stamp, read together, or `None` when the process does not exist or cannot be
/// read. Same units as `start_stamp`.
pub fn parent_and_stamp(pid: u32) -> Option<(u32, u64)> {
    if pid <= 1 || pid > i32::MAX as u32 {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let got = unsafe { libc::proc_pidinfo(pid as i32, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
        (got == size).then(|| (info.pbi_ppid, info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec))
    }
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // After the last ')': state, ppid (field 4), ..., starttime (field 22).
        let rest: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
        Some((rest.get(1)?.parse().ok()?, rest.get(19)?.parse().ok()?))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// The most parent links `descends_from` follows (ADR-16 16.5).
pub const MAX_HOPS: usize = 64;

/// Does `peer` descend from the process `root` that started at `root_stamp` (or is it that process)? It walks the
/// parent chain at most `MAX_HOPS` links. Each link must have started no earlier than its parent, so a reused pid on
/// the chain (a newer process under an old number) breaks it, and the root must match its recorded stamp exactly.
pub fn descends_from(peer: u32, root: u32, root_stamp: u64) -> bool {
    descends_with(peer, root, root_stamp, parent_and_stamp)
}

/// `descends_from` over any process table, for tests.
pub fn descends_with(peer: u32, root: u32, root_stamp: u64, read: impl Fn(u32) -> Option<(u32, u64)>) -> bool {
    let mut pid = peer;
    let mut child_stamp = u64::MAX;
    for _ in 0..=MAX_HOPS {
        let Some((ppid, stamp)) = read(pid) else { return false };
        if stamp > child_stamp {
            return false;
        }
        if pid == root {
            return stamp == root_stamp;
        }
        if ppid <= 1 {
            return false;
        }
        child_stamp = stamp;
        pid = ppid;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn given_this_process_when_stamped_twice_then_the_stamps_agree() {
        let me = std::process::id();
        assert!(start_stamp(me).is_some());
        assert_eq!(start_stamp(me), start_stamp(me));
        assert!(same_process(me, start_stamp(me).unwrap()));
    }

    #[test]
    fn given_a_dead_pid_when_stamped_then_there_is_none() {
        let mut c = Command::new("true").spawn().unwrap();
        let pid = c.id();
        c.wait().unwrap();
        assert_eq!(start_stamp(pid), None);
        assert!(!same_process(pid, 123));
    }

    #[test]
    fn given_a_live_pid_when_the_recorded_stamp_differs_then_it_is_not_the_same_process() {
        let mut c = Command::new("sleep").arg("30").spawn().unwrap();
        let real = start_stamp(c.id()).unwrap();
        assert!(same_process(c.id(), real));
        assert!(!same_process(c.id(), real + 1)); // what a reused pid looks like
        c.kill().ok();
        c.wait().ok();
    }

    #[test]
    fn given_a_child_of_this_process_when_walked_then_it_descends_from_us_and_not_from_a_stale_stamp() {
        let mut c = Command::new("sleep").arg("30").spawn().unwrap();
        let me = std::process::id();
        let stamp = start_stamp(me).unwrap();
        assert_eq!(parent_and_stamp(c.id()).map(|p| p.0), Some(me));
        assert!(descends_from(c.id(), me, stamp));
        assert!(descends_from(me, me, stamp), "the root itself counts");
        assert!(!descends_from(c.id(), me, stamp + 1), "a reused root pid has another stamp");
        assert!(!descends_from(me, c.id(), start_stamp(c.id()).unwrap()), "a parent never descends from its child");
        c.kill().ok();
        c.wait().ok();
    }

    #[test]
    fn given_a_fake_table_when_a_link_is_newer_than_its_child_or_the_chain_is_long_then_it_is_refused() {
        // 10 (stamp 50) -> 20 (40) -> 30 (30, the root).
        let table = |pid: u32| match pid {
            10 => Some((20, 50)),
            20 => Some((30, 40)),
            30 => Some((1, 30)),
            // 11's parent 21 started after 11 did: 21 is a reused pid.
            11 => Some((21, 35)),
            21 => Some((30, 45)),
            _ => None,
        };
        assert!(descends_with(10, 30, 30, table));
        assert!(!descends_with(11, 30, 30, table));
        assert!(!descends_with(10, 99, 30, table), "the chain ends at init without meeting the root");
        // A chain of MAX_HOPS + 2 processes, root at the top, all with one stamp.
        let top = 1000 + MAX_HOPS as u32 + 1;
        let long = |pid: u32| Some(if pid >= top { (1, 7) } else { (pid + 1, 7) });
        assert!(descends_with(1000 + 1, top, 7, long), "64 hops reach the root");
        assert!(!descends_with(1000, top, 7, long), "65 hops do not");
    }

    #[test]
    fn given_pids_that_are_never_ours_when_stamped_then_there_is_none() {
        for pid in [0, 1, u32::MAX] {
            assert_eq!(start_stamp(pid), None);
            assert_eq!(zombie_stamp(pid), None);
        }
    }

    #[test]
    fn given_a_killed_child_not_yet_reaped_when_read_then_it_is_a_zombie_with_its_own_stamp() {
        let mut c = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = c.id();
        let stamp = start_stamp(pid).unwrap();
        assert!(!is_zombie(pid) && zombie_stamp(pid).is_none(), "a running process is not a zombie");
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        let t0 = std::time::Instant::now();
        while !is_zombie(pid) && t0.elapsed() < std::time::Duration::from_secs(3) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Still holds its pid: kill(pid, 0) succeeds until the reap.
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, 0);
        assert!(is_zombie(pid), "the killed, unreaped child must read as a zombie");
        assert_eq!(zombie_stamp(pid), Some(stamp), "the zombie carries the start stamp it was recorded with");
        c.wait().unwrap();
        assert_eq!(zombie_stamp(pid), None, "a reaped process is no zombie");
    }

    /// A spawn that exits at once is a zombie before the spawner reads its stamp. The spawner's read of its own child
    /// must still give the stamp the child started with (the session start would otherwise refuse a fast `true`).
    #[test]
    fn given_our_child_already_exited_when_its_stamp_is_read_then_it_is_the_one_it_started_with() {
        let mut c = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = c.id();
        let stamp = start_stamp(pid).unwrap();
        assert_eq!(child_stamp(pid), Some(stamp), "a running child reads its live stamp");
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        let t0 = std::time::Instant::now();
        while !is_zombie(pid) && t0.elapsed() < std::time::Duration::from_secs(3) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(is_zombie(pid), "the killed, unreaped child must read as a zombie");
        #[cfg(target_os = "macos")]
        assert_eq!(start_stamp(pid), None, "the macOS live read cannot see a zombie (the cause of the refusal)");
        assert_eq!(child_stamp(pid), Some(stamp), "the exited child still reads the stamp it started with");
        c.wait().unwrap();
        assert_eq!(child_stamp(pid), None, "once reaped there is nothing to read");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn given_this_process_when_read_through_sysctl_then_its_start_time_matches_its_stamp() {
        let me = std::process::id();
        let k = kinfo(me).expect("sysctl reads this process with the expected record size and pid");
        let t = k.p_starttime;
        assert_eq!(Some(t.tv_sec as u64 * 1_000_000 + t.tv_usec as u64), start_stamp(me), "the extern_proc head layout matches the kernel's");
        assert_ne!(k.p_stat, SZOMB);
    }
}
