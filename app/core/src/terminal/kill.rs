//! Ending a terminal's processes. Killing the shell's process group is not enough: an interactive shell gives every
//! background job its own group, so `sleep 300 &` would outlive its tab. The shell is a session leader (portable-pty
//! calls setsid), so every process whose session id is the shell's pid belongs to the terminal, and the session id is
//! held for as long as any member lives, so it cannot be recycled under us.
//!
//! TODO(windows): none of this exists there. Use a Job Object created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, assign
//! the ConPTY child to it right after spawn (before it can fork), and `TerminateJobObject` on close and on app quit.
//! Until then the crate does not build for Windows anyway (`sessions` and this module use libc signals).

/// Every process whose session id is `sid`.
pub fn session_members(sid: u32) -> Vec<i32> {
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

/// Signal the shell's process group and the shell itself (only while it has not been reaped, so a recycled pid is never
/// hit), and every other member of its session (always; the session id outlives the shell while any member lives).
pub fn signal_all(pid: u32, shell_alive: bool, sig: libc::c_int) {
    if pid <= 1 {
        return;
    }
    if shell_alive {
        unsafe {
            libc::kill(-(pid as i32), sig);
            libc::kill(pid as i32, sig);
        }
    }
    for p in session_members(pid) {
        unsafe { libc::kill(p, sig) };
    }
}
