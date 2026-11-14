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
    if pid <= 1 || pid > i32::MAX as u32 {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let got = unsafe { libc::proc_pidinfo(pid as i32, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
        got == size && info.pbi_status == 5 // SZOMB
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).ok().and_then(|s| s.rsplit_once(')').map(|(_, r)| r.trim_start().starts_with('Z'))).unwrap_or(false)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
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
        }
    }
}
