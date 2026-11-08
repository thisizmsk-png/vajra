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

/// Is `pid` still the very process that started at `stamp`? False for a dead pid, a reused pid, or an unreadable one.
pub fn same_process(pid: u32, stamp: u64) -> bool {
    start_stamp(pid) == Some(stamp)
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
    fn given_pids_that_are_never_ours_when_stamped_then_there_is_none() {
        for pid in [0, 1, u32::MAX] {
            assert_eq!(start_stamp(pid), None);
        }
    }
}
