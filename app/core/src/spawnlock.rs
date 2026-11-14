//! The app-wide spawn lock (#234 follow-up, before key A is claimed).
//!
//! macOS has no `pipe2`, so a pipe the core makes with `libc::pipe` is inheritable until `fcntl` sets close-on-exec on
//! each end. A child that another thread forks in that window inherits both ends: for the evidence runner, a stray
//! writer on the evidence channel. Rust's own files, sockets and pipes are made close-on-exec atomically; only raw
//! `libc` calls have the window.
//!
//! So every process the app starts takes this lock **shared** for the `spawn` call itself, and code that makes an
//! inheritable fd takes it **exclusive** from that call until the close-on-exec flag is set. Spawns never wait for each
//! other, only for that window (a few syscalls). The shared hold ends when `spawn` returns: std returns only once the
//! child has exec'd (or `posix_spawn` has), so the child's fd table is final by then, and holding it for the child's
//! whole life would stall the runner behind a long `output()`.
//!
//! clippy's `disallowed-methods` (app/core/clippy.toml) refuses a bare `Command::spawn`, `output`, `status`, a PTY
//! `spawn_command`, `openpty` or `libc::pipe` outside this module, so a new spawn site can't skip the lock silently.
//! Test code is allowed to: test helpers aren't the app.

use std::io;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

static LOCK: RwLock<()> = RwLock::new(());

/// Held across one spawn. Shared: any number of spawns at once.
pub fn shared() -> RwLockReadGuard<'static, ()> {
    LOCK.read().unwrap_or_else(PoisonError::into_inner)
}

/// Held from making an inheritable fd until its close-on-exec flag is set: no process starts meanwhile. Never spawn,
/// and never take `shared`, while holding it (that would deadlock).
pub fn exclusive() -> RwLockWriteGuard<'static, ()> {
    LOCK.write().unwrap_or_else(PoisonError::into_inner)
}

/// `cmd.spawn()` under the shared lock.
#[allow(clippy::disallowed_methods)]
pub fn spawn(cmd: &mut Command) -> io::Result<Child> {
    let _g = shared();
    cmd.spawn()
}

/// `cmd.status()`: the spawn under the lock, the wait outside it. Same stdio as `status` (inherited unless set).
pub fn status(cmd: &mut Command) -> io::Result<ExitStatus> {
    spawn(cmd)?.wait()
}

/// `cmd.output()`: the spawn under the lock, the wait outside it. Like `output`, stdout and stderr are captured and
/// stdin is null; the caller can't override these here (none of the app's callers does).
pub fn output(cmd: &mut Command) -> io::Result<Output> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    spawn(cmd)?.wait_with_output()
}

/// A pipe whose two ends are close-on-exec before any process can start: `pipe` and both `fcntl`s run under the
/// exclusive lock. Returns `(read, write)`.
#[allow(clippy::disallowed_methods)]
pub fn cloexec_pipe() -> io::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::FromRawFd;
    let _g = exclusive();
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: a fresh pipe; both ends are owned right below.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the pipe just made; each fd is owned once.
    let ends = unsafe { (std::os::fd::OwnedFd::from_raw_fd(fds[0]), std::os::fd::OwnedFd::from_raw_fd(fds[1])) };
    #[cfg(test)]
    pipe_hook(fds);
    for fd in fds {
        // SAFETY: fcntl on an fd owned above.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(ends)
}

/// A native PTY pair, made under the exclusive lock: portable-pty calls `openpty` and then sets close-on-exec on each
/// end, the same window as a raw pipe, so a helper started meanwhile could hold a terminal's master open.
#[allow(clippy::disallowed_methods)]
pub fn openpty(size: portable_pty::PtySize) -> Result<portable_pty::PtyPair, String> {
    let _g = exclusive();
    portable_pty::native_pty_system().openpty(size).map_err(|e| e.to_string())
}

/// `slave.spawn_command(cmd)` under the shared lock.
#[allow(clippy::disallowed_methods)]
pub fn spawn_pty(slave: &dyn portable_pty::SlavePty, cmd: portable_pty::CommandBuilder) -> Result<Box<dyn portable_pty::Child + Send + Sync>, String> {
    let _g = shared();
    slave.spawn_command(cmd).map_err(|e| e.to_string())
}

#[cfg(test)]
type PipeHook = (std::thread::ThreadId, Box<dyn Fn([libc::c_int; 2]) + Send>);

/// Test-only: runs inside `cloexec_pipe`'s window (the pipe made, the flags not yet set), holding the exclusive lock,
/// only on the thread it names (other tests make pipes in parallel). Global, since the point is what *another* thread
/// can do meanwhile.
#[cfg(test)]
pub(crate) static PIPE_HOOK: std::sync::Mutex<Option<PipeHook>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn pipe_hook(fds: [libc::c_int; 2]) {
    let hook = PIPE_HOOK.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, f)) = hook.as_ref().filter(|(tid, _)| *tid == std::thread::current().id()) {
        f(fds)
    }
}

#[cfg(test)]
#[path = "spawnlock_tests.rs"]
mod tests;
