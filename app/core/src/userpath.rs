//! The user's PATH, resolved at startup and again on `refresh` (ADR-17 rule 7).
//!
//! A macOS app started from Finder, the Dock or `open` gets the minimal GUI PATH (`/usr/bin:/bin:/usr/sbin:/sbin`),
//! so `claude` and the tools a user installed with an installer, Homebrew or npm are not on it. `init` asks the
//! user's login shell for its PATH once, with a constant script, a deadline and a process-group kill, then adds the
//! process PATH and the usual install dirs. Preflight, the claude lookup, the Lead and every PTY spawn use the result.
//!
//! Safety: the shell gets a fixed `-c` script (no user data is ever put in a shell string), a scrubbed env, `HOME`
//! as its cwd (never a repo) and no stdin. Only absolute entries are kept, so a `.` or other relative entry in a
//! user's PATH can never make a spawn resolve a program from its cwd. Any failure falls back to the process PATH.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

/// How long the login shell gets before it is killed and the fallback is used.
pub const TIMEOUT: Duration = Duration::from_secs(3);

const BEGIN: &str = "__VAJRA_PATH_BEGIN__";
const END: &str = "__VAJRA_PATH_END__";
/// The only script the shell ever runs. `${PATH}` is expanded by the shell; nothing is interpolated by us.
const SCRIPT: &str = r#"printf '%s' "__VAJRA_PATH_BEGIN__${PATH}__VAJRA_PATH_END__""#;
/// The most stdout kept from the shell; rc files that print banners cannot grow it past this.
const OUT_CAP: usize = 64 * 1024;
/// The PATH the shell itself starts with, so `path_helper` and rc files that call system tools work.
const BASE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// How long a reader waits for a first resolution that runs in the background: the shell's own deadline (`TIMEOUT`,
/// unchanged) plus room for the kill and the install-dir probe.
pub const FIRST_WAIT: Duration = TIMEOUT.saturating_add(Duration::from_secs(2));

/// The resolved PATH and how it gets there. One static in the app; tests make their own.
pub struct State {
    current: RwLock<Option<Vec<PathBuf>>>,
    /// True from `start` until the first resolution has stored its answer.
    pending: Mutex<bool>,
    done: Condvar,
    refresh: Refresh,
    /// True once a refresh has stored an answer; the first resolution then no longer overwrites it.
    refreshed: AtomicBool,
}

/// Ends the first resolution when dropped, so readers are woken even if it panicked or its thread never started.
struct Finish(&'static State);

impl Drop for Finish {
    fn drop(&mut self) {
        *self.0.pending.lock().unwrap_or_else(|e| e.into_inner()) = false;
        self.0.done.notify_all();
    }
}

impl State {
    pub const fn new() -> Self {
        Self { current: RwLock::new(None), pending: Mutex::new(false), done: Condvar::new(), refresh: Refresh::new(), refreshed: AtomicBool::new(false) }
    }

    fn set(&self, dirs: Vec<PathBuf>) {
        let mut cur = self.current.write().unwrap_or_else(|e| e.into_inner());
        *cur = Some(dirs);
        self.refreshed.store(true, Ordering::SeqCst);
    }

    /// Store the first resolution's answer unless a refresh already stored a newer one (a refresh whose wait for the
    /// first resolution timed out can finish before it). Checked under the same lock as `set`.
    fn set_first(&self, dirs: Vec<PathBuf>) {
        let mut cur = self.current.write().unwrap_or_else(|e| e.into_inner());
        if !self.refreshed.load(Ordering::SeqCst) {
            *cur = Some(dirs);
        }
    }

    /// Run the first resolution on its own thread and return at once. Until it has stored its answer, `current` and
    /// `refresh` wait for it (at most `wait`), so no reader ever acts on a PATH from before it.
    pub fn start(&'static self, resolve: impl FnOnce() -> Vec<PathBuf> + Send + 'static) {
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = true;
        let finish = Finish(self);
        // If the thread cannot start, the closure is dropped, `finish` with it, and readers get the fallback.
        let _ = std::thread::Builder::new().name("userpath".into()).spawn(move || {
            let finish = finish;
            finish.0.set_first(resolve());
        });
    }

    /// Block while the first resolution is in flight, at most `wait`.
    fn wait_first(&self, wait: Duration) {
        let deadline = Instant::now() + wait;
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        while *pending {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            pending = self.done.wait_timeout(pending, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// The resolved dirs, after any first resolution in flight. `None` when nothing was ever resolved.
    pub fn current(&self, wait: Duration) -> Option<Vec<PathBuf>> {
        self.wait_first(wait);
        self.current.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Resolve again, after any first resolution in flight, so the stored answer is never older than the first one.
    /// Single-flight and at most once per `REFRESH_GAP` (`Refresh::run`). `true` when it ran.
    pub fn refresh(&self, wait: Duration, now: Instant, resolve: impl FnOnce() -> Vec<PathBuf>) -> bool {
        self.wait_first(wait);
        self.refresh.run(now, resolve).map(|d| self.set(d)).is_some()
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

static STATE: State = State::new();

/// The shortest time between two refreshes: each one runs the user's login shell.
pub const REFRESH_GAP: Duration = Duration::from_secs(10);

fn resolve(home: &Path) -> Vec<PathBuf> {
    let login = login_shell().and_then(|sh| detect(&sh, home, TIMEOUT));
    effective(login, &process_path(), home)
}

/// Resolve the user PATH on this thread: the caller waits for the login shell (up to `TIMEOUT`).
pub fn init(home: &Path) {
    STATE.set(resolve(home));
}

/// Resolve the user PATH in the background, so the window does not wait for the user's login shell. Call from `main`.
/// The first reader (`current`, `joined`, `refresh`) waits for the answer instead, at most `FIRST_WAIT`.
pub fn start(home: &Path) {
    let home = home.to_path_buf();
    STATE.start(move || resolve(&home));
}

/// One login-shell run at a time, and at most one per `REFRESH_GAP`.
pub struct Refresh {
    last: Mutex<Option<Instant>>,
}

impl Refresh {
    pub const fn new() -> Self {
        Self { last: Mutex::new(None) }
    }

    /// `resolve()`'s answer, or `None` when a refresh is already running or the last one started under a gap ago.
    pub fn run(&self, now: Instant, resolve: impl FnOnce() -> Vec<PathBuf>) -> Option<Vec<PathBuf>> {
        let mut last = match self.last.try_lock() {
            Ok(g) => g,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        if last.is_some_and(|t| now.saturating_duration_since(t) < REFRESH_GAP) {
            return None;
        }
        *last = Some(now);
        Some(resolve())
    }
}

impl Default for Refresh {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve the user PATH again (ADR-17 rule 7), so a folder an install created after the app started is found without
/// a restart. Only the UI asks for it (Settings > Providers > Set up); it is single-flight and runs at most once per
/// `REFRESH_GAP`. The bridge's fixed PATH (`bridge::BASE_PATH`) never depends on it. `true` when it ran. A first
/// resolution still in flight is waited for first, so a read after this never sees a PATH from before the refresh.
pub fn refresh(home: &Path) -> bool {
    STATE.refresh(FIRST_WAIT, Instant::now(), || resolve(home))
}

/// The resolved dirs, waiting for a first resolution still in flight (`start`). With neither `init` nor `start`
/// (unit tests, `--mcp`) this is the process PATH plus the install dirs; no shell is ever started here.
pub fn current() -> Vec<PathBuf> {
    match STATE.current(FIRST_WAIT) {
        Some(d) => d,
        None => {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
            effective(None, &process_path(), &home)
        }
    }
}

/// `current()` joined into one PATH value.
pub fn joined() -> Option<OsString> {
    std::env::join_paths(current()).ok()
}

fn process_path() -> Vec<PathBuf> {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default()
}

/// The install dirs that hold `claude` and friends when no PATH names them, in lookup order.
pub fn known_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if home.is_absolute() {
        dirs.extend([".local/bin", ".claude/local"].map(|d| home.join(d)));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    if home.is_absolute() {
        dirs.extend([".npm-global/bin", ".volta/bin", ".bun/bin"].map(|d| home.join(d)));
        dirs.extend(nvm_bins(home));
    }
    dirs
}

/// `~/.nvm/versions/node/*/bin`, newest version first. Versions compare numerically (`v22.18.0` > `v22.9.0` >
/// `v9.11.2`); a name that is not `v<n>.<n>.<n>` sorts after every real version.
pub fn nvm_bins(home: &Path) -> Vec<PathBuf> {
    let mut nvm: Vec<(Option<Vec<u64>>, PathBuf)> = std::fs::read_dir(home.join(".nvm/versions/node"))
        .map(|r| r.filter_map(|e| e.ok()).map(|e| (node_version(&e.file_name().to_string_lossy()), e.path())).collect())
        .unwrap_or_default();
    // Some(..) > None, so descending puts real versions first, newest first; ties fall back to the name.
    nvm.sort_by(|a, b| b.cmp(a));
    nvm.into_iter().map(|(_, p)| p.join("bin")).collect()
}

fn node_version(name: &str) -> Option<Vec<u64>> {
    let v: Option<Vec<u64>> = name.strip_prefix('v')?.split('.').map(|c| c.parse().ok()).collect();
    v.filter(|v| !v.is_empty())
}

/// Login-shell PATH first (what the user's terminal sees), then the process PATH, then the install dirs that exist.
/// Relative and empty entries are dropped and duplicates keep their first place.
pub fn effective(login: Option<Vec<PathBuf>>, process: &[PathBuf], home: &Path) -> Vec<PathBuf> {
    let known = known_dirs(home).into_iter().filter(|d| d.is_dir());
    let mut out: Vec<PathBuf> = Vec::new();
    for d in login.unwrap_or_default().into_iter().chain(process.iter().cloned()).chain(known) {
        if d.is_absolute() && !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// `$SHELL` when it is an absolute path to an existing POSIX-style shell, else `/bin/zsh` (the macOS default).
pub fn login_shell() -> Option<PathBuf> {
    let ok = |p: &Path| {
        p.is_absolute()
            && p.is_file()
            && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| matches!(n, "zsh" | "bash" | "sh" | "ksh" | "dash"))
    };
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|p| ok(p))
        .or_else(|| Some(PathBuf::from("/bin/zsh")).filter(|p| ok(p)))
}

/// Run `shell -l -i -c SCRIPT` and return the PATH it prints, or `None` on any failure. Returns within `timeout`
/// (plus the kill): the shell runs in its own process group and the whole group is killed at the deadline.
pub fn detect(shell: &Path, home: &Path, timeout: Duration) -> Option<Vec<PathBuf>> {
    if !shell.is_absolute() || !home.is_absolute() || !home.is_dir() {
        return None;
    }
    let mut cmd = Command::new(shell);
    cmd.args(["-l", "-i", "-c", SCRIPT])
        .env_clear()
        .env("HOME", home)
        .env("PATH", BASE_PATH)
        .env("TERM", "dumb")
        .env("SHELL", shell)
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for k in ["USER", "LOGNAME", "LANG"] {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().ok()?;
    let pid = child.id() as libc::pid_t;
    let mut stdout = child.stdout.take()?;
    let mut out = Vec::new();
    let found = collect(&mut stdout, Instant::now() + timeout, OUT_CAP, &mut out);
    // SAFETY: killpg on the group we created for this child; a group that already exited just returns ESRCH. The
    // leader is not yet reaped, so its pid (the group id) cannot have been reused.
    unsafe {
        libc::killpg(pid, libc::SIGKILL);
    }
    drop(stdout); // closes our end, so anything still writing gets EPIPE rather than blocking
    let _ = child.wait();
    found
}

const CHUNK: usize = 4096;
/// After a complete marker pair, how much longer to read for a later one. Our `printf` is the shell's last command,
/// so the real pair is the last one written; a pair an rc file printed earlier must not win.
const LAST_PAIR_GRACE: Duration = Duration::from_millis(250);

/// Read `r` on this thread until EOF, `cap` bytes, or `deadline`, and return the **last** complete marker pair (see
/// `parse`); once a pair has parsed, the deadline shrinks to `LAST_PAIR_GRACE`, so a pipe held open after the shell's
/// own printf costs at most that. Each read
/// waits in `poll` for at most the time left, so a shell (or an escaped grandchild) that holds the pipe open and
/// prints nothing cannot outlast the deadline; no reader thread or queue exists to leak or grow. `out` never holds
/// more than `cap` bytes: each read asks only for the room left.
fn collect<R: Read + std::os::fd::AsRawFd>(r: &mut R, deadline: Instant, cap: usize, out: &mut Vec<u8>) -> Option<Vec<PathBuf>> {
    let mut buf = [0u8; CHUNK];
    // The newest complete pair so far, and the (shortened) time left to look for a later one.
    let mut found: Option<Vec<PathBuf>> = None;
    let mut deadline = deadline;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let room = cap.saturating_sub(out.len());
        if left.is_zero() || room == 0 {
            return found;
        }
        let mut pfd = libc::pollfd { fd: r.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = left.as_millis().clamp(1, i32::MAX as u128) as libc::c_int;
        // SAFETY: one valid pollfd for an fd `r` owns for the whole call.
        let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
        if rc < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return found;
        }
        if rc == 0 {
            continue; // the deadline check above ends the loop
        }
        // poll said readable (data, EOF or error), so this read does not block.
        match r.read(&mut buf[..CHUNK.min(room)]) {
            Ok(0) => return found, // EOF: the last pair written, if any
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if let Some(p) = parse(out) {
                    if found.is_none() {
                        deadline = deadline.min(Instant::now() + LAST_PAIR_GRACE);
                    }
                    found = Some(p);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return found,
        }
    }
}

/// The dirs between the last BEGIN marker and the END after it. Rc files may print anything before it.
fn parse(out: &[u8]) -> Option<Vec<PathBuf>> {
    let s = String::from_utf8_lossy(out);
    let start = s.rfind(BEGIN)? + BEGIN.len();
    let len = s[start..].find(END)?;
    let dirs: Vec<PathBuf> = std::env::split_paths(&s[start..start + len]).filter(|d| d.is_absolute()).collect();
    (!dirs.is_empty()).then_some(dirs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tmp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("vajra-userpath-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn exe(p: &Path, body: &str) {
        std::fs::write(p, body).unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A fake login shell that prints rc noise and then what the real script would print for `path`.
    fn fake_shell(dir: &Path, path: &str) -> PathBuf {
        let sh = dir.join("zsh");
        exe(&sh, &format!("#!/bin/sh\necho 'welcome banner'\nprintf '%s' '{BEGIN}{path}{END}'\n"));
        sh
    }

    #[test]
    fn a_minimal_path_launch_finds_claude_through_the_login_shell() {
        let home = tmp("home");
        let custom = home.join("custom/bin");
        std::fs::create_dir_all(&custom).unwrap();
        exe(&custom.join("claude"), "#!/bin/sh\necho '9.9.9 (Claude Code)'\n");
        let shells = tmp("shells");
        let sh = fake_shell(&shells, &format!("{}:/usr/bin:/bin", custom.display()));
        let minimal: Vec<PathBuf> = std::env::split_paths(BASE_PATH).collect();

        assert!(crate::vajra::preflight::find_on_path("claude", &minimal).is_none_or(|p| !p.starts_with(&home)), "the minimal PATH alone must not hold the fake claude");
        let login = detect(&sh, &home, Duration::from_secs(5));
        let dirs = effective(login, &minimal, &home);
        assert_eq!(dirs.first(), Some(&custom), "{dirs:?}");
        assert_eq!(crate::vajra::preflight::find_on_path("claude", &dirs), Some(custom.join("claude")));
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&shells);
    }

    #[test]
    fn install_dirs_are_probed_when_the_shell_gives_nothing() {
        let home = tmp("known");
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        exe(&home.join(".local/bin/claude"), "#!/bin/sh\n");
        let dirs = effective(None, &std::env::split_paths(BASE_PATH).collect::<Vec<_>>(), &home);
        assert_eq!(crate::vajra::preflight::find_on_path("claude", &dirs), Some(home.join(".local/bin/claude")));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_hanging_shell_is_killed_at_the_deadline() {
        let home = tmp("hang");
        let sh = home.join("zsh");
        // The child sleeper keeps stdout open too; the group kill must take it down with the shell.
        exe(&sh, "#!/bin/sh\nsleep 30 &\nsleep 30\n");
        let t = Instant::now();
        assert_eq!(detect(&sh, &home, Duration::from_millis(300)), None);
        assert!(t.elapsed() < Duration::from_secs(3), "took {:?}", t.elapsed());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_failing_or_silent_shell_falls_back() {
        let home = tmp("fail");
        let bad = home.join("bash");
        exe(&bad, "#!/bin/sh\necho 'no markers here'\nexit 1\n");
        assert_eq!(detect(&bad, &home, Duration::from_secs(5)), None);
        assert_eq!(detect(&home.join("missing-sh"), &home, Duration::from_secs(5)), None);
        assert_eq!(detect(Path::new("zsh"), &home, Duration::from_secs(5)), None, "a relative shell is never run");
        let process = vec![PathBuf::from("/usr/bin")];
        assert_eq!(effective(None, &process, &home).first(), Some(&PathBuf::from("/usr/bin")));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn relative_empty_and_duplicate_entries_are_dropped() {
        let home = tmp("junk");
        let sh = fake_shell(&home, ".::bin:/usr/bin:/usr/bin:./node_modules/.bin:/bin");
        let login = detect(&sh, &home, Duration::from_secs(5)).unwrap();
        assert_eq!(login, ["/usr/bin", "/usr/bin", "/bin"].map(PathBuf::from).to_vec(), "relative and empty entries dropped");
        let dirs = effective(Some(login), &[PathBuf::from("."), PathBuf::from("/bin"), PathBuf::from("/sbin")], &home);
        assert!(dirs.iter().all(|d| d.is_absolute()), "{dirs:?}");
        assert_eq!(&dirs[..3], &[PathBuf::from("/usr/bin"), PathBuf::from("/bin"), PathBuf::from("/sbin")]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_last_marker_pair_wins_over_rc_output() {
        let out = format!("{BEGIN}/evil{END}\nnoise {BEGIN}/usr/bin:/bin{END}");
        assert_eq!(parse(out.as_bytes()), Some(vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]));
        assert_eq!(parse(format!("{BEGIN}/usr/bin").as_bytes()), None, "an unterminated value is not trusted");
    }

    #[test]
    fn spawn_env_path_is_the_user_path_with_absolute_dirs_only() {
        let env = crate::sessions::user_env();
        let path = env.iter().find(|(k, _)| *k == "PATH").map(|(_, v)| v.clone()).unwrap();
        assert_eq!(Some(path.clone()), joined());
        assert!(std::env::split_paths(&path).all(|d| d.is_absolute()), "{path:?}");
    }

    #[test]
    fn nvm_versions_are_newest_first_by_number_not_by_text() {
        let home = tmp("nvm");
        for v in ["v9.11.2", "v22.9.0", "v22.18.0", "v10.0.0", "v8.17.0", "system"] {
            std::fs::create_dir_all(home.join(".nvm/versions/node").join(v).join("bin")).unwrap();
        }
        let names: Vec<String> = nvm_bins(&home)
            .iter()
            .map(|p| p.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["v22.18.0", "v22.9.0", "v10.0.0", "v9.11.2", "v8.17.0", "system"]);
        // The same order reaches the fallback PATH.
        let dirs = effective(None, &[], &home);
        let at = |v: &str| dirs.iter().position(|d| d.ends_with(format!("{v}/bin"))).unwrap();
        assert!(at("v22.18.0") < at("v22.9.0") && at("v22.9.0") < at("v9.11.2"), "{dirs:?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_reader_takes_the_last_marker_pair_not_the_first() {
        let (mut ours, mut theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        // An rc file prints a lookalike pair first; the shell's own printf comes after it.
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            theirs.write_all(format!("banner {BEGIN}/evil/bin{END}\n").as_bytes()).unwrap();
            std::thread::sleep(Duration::from_millis(50));
            theirs.write_all(format!("{BEGIN}/usr/bin:/bin{END}").as_bytes()).unwrap();
        });
        let mut out = Vec::new();
        let got = collect(&mut ours, Instant::now() + Duration::from_secs(5), OUT_CAP, &mut out);
        writer.join().unwrap();
        assert_eq!(got, Some(vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]));
        // A writer that keeps the pipe open after the pair costs only the grace, not the whole deadline.
        let (mut ours, mut theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        {
            use std::io::Write;
            theirs.write_all(format!("{BEGIN}/usr/bin{END}").as_bytes()).unwrap();
        }
        let t = Instant::now();
        assert_eq!(collect(&mut ours, Instant::now() + Duration::from_secs(10), OUT_CAP, &mut Vec::new()), Some(vec![PathBuf::from("/usr/bin")]));
        assert!(t.elapsed() < Duration::from_secs(2), "took {:?}", t.elapsed());
        drop(theirs);
    }

    #[test]
    fn the_reader_never_holds_more_than_the_cap_under_a_flood() {
        let (mut ours, mut theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        // 8 MiB of noise and then the markers; the writer is cut off with EPIPE once the reader is done.
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            let noise = vec![b'x'; 64 * 1024];
            for _ in 0..128 {
                if theirs.write_all(&noise).is_err() {
                    return;
                }
            }
            let _ = theirs.write_all(format!("{BEGIN}/usr/bin{END}").as_bytes());
        });
        let mut out = Vec::new();
        let t = Instant::now();
        assert_eq!(collect(&mut ours, Instant::now() + Duration::from_secs(10), OUT_CAP, &mut out), None);
        assert_eq!(out.len(), OUT_CAP, "exactly the cap is kept, never more");
        assert!(t.elapsed() < Duration::from_secs(5), "took {:?}", t.elapsed());
        drop(ours);
        writer.join().unwrap();
    }

    #[test]
    fn an_escaped_grandchild_holding_the_pipe_cannot_outlast_the_deadline() {
        let home = tmp("escape");
        let sh = home.join("zsh");
        // perl leaves the shell's process group (so killpg misses it) and keeps stdout open; the shell exits at once.
        exe(&sh, "#!/bin/sh\n/usr/bin/perl -e 'setpgrp(0, 0); sleep 3' &\nexit 0\n");
        let t = Instant::now();
        assert_eq!(detect(&sh, &home, Duration::from_millis(400)), None);
        assert!(t.elapsed() < Duration::from_secs(2), "took {:?}", t.elapsed());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_flooding_rc_file_gives_up_at_the_cap_not_the_deadline() {
        let home = tmp("flood");
        let sh = home.join("zsh");
        exe(&sh, &format!("#!/bin/sh\nhead -c 20000000 /dev/zero | tr '\\0' 'x'\nprintf '%s' '{BEGIN}/usr/bin{END}'\n"));
        let t = Instant::now();
        assert_eq!(detect(&sh, &home, Duration::from_secs(20)), None, "markers past the cap are not trusted");
        assert!(t.elapsed() < Duration::from_secs(10), "took {:?}", t.elapsed());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_script_is_constant_and_quotes_path() {
        assert!(SCRIPT.contains("${PATH}") && !SCRIPT.contains('`') && !SCRIPT.contains("$("));
    }

    #[test]
    fn a_folder_created_after_the_first_resolve_is_found_by_a_refresh() {
        let home = tmp("refresh");
        let before = effective(None, &[], &home);
        assert!(!before.contains(&home.join(".local/bin")));
        // An install creates the folder after the app started.
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        let gate = Refresh::new();
        let after = gate.run(Instant::now(), || effective(None, &[], &home)).expect("the first refresh runs");
        assert!(after.contains(&home.join(".local/bin")), "{after:?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_refresh_runs_at_most_once_per_gap() {
        let gate = Refresh::new();
        let t0 = Instant::now();
        let calls = std::cell::Cell::new(0);
        let resolve = || {
            calls.set(calls.get() + 1);
            vec![PathBuf::from("/usr/bin")]
        };
        assert!(gate.run(t0, resolve).is_some());
        assert!(gate.run(t0 + Duration::from_secs(3), resolve).is_none(), "too soon");
        assert!(gate.run(t0 + REFRESH_GAP, resolve).is_some());
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn a_refresh_while_one_is_running_does_not_start_another() {
        let gate = std::sync::Arc::new(Refresh::new());
        let (tx, rx) = std::sync::mpsc::channel();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let g = gate.clone();
        let h = std::thread::spawn(move || {
            g.run(Instant::now(), || {
                tx.send(()).unwrap();
                go_rx.recv().unwrap();
                vec![]
            })
        });
        rx.recv().unwrap();
        let called = std::cell::Cell::new(false);
        assert!(gate.run(Instant::now() + REFRESH_GAP * 2, || { called.set(true); vec![] }).is_none());
        assert!(!called.get(), "a second login shell never starts while one runs");
        go_tx.send(()).unwrap();
        assert!(h.join().unwrap().is_some());
    }

    fn state() -> &'static State {
        Box::leak(Box::new(State::new()))
    }

    #[test]
    fn a_background_start_returns_at_once_and_the_first_reader_waits_for_the_answer() {
        let st = state();
        assert_eq!(st.current(Duration::from_secs(5)), None, "nothing started: no wait and no answer");
        let t = Instant::now();
        st.start(|| {
            std::thread::sleep(Duration::from_millis(400));
            vec![PathBuf::from("/first/bin")]
        });
        assert!(t.elapsed() < Duration::from_millis(200), "start must not wait for the shell: {:?}", t.elapsed());
        assert_eq!(st.current(Duration::from_secs(5)), Some(vec![PathBuf::from("/first/bin")]), "the reader waited");
        assert!(t.elapsed() >= Duration::from_millis(400));
        let t = Instant::now();
        assert!(st.current(Duration::from_secs(5)).is_some());
        assert!(t.elapsed() < Duration::from_millis(200), "later readers do not wait");
    }

    #[test]
    fn a_refresh_during_a_slow_first_resolution_waits_and_the_next_read_sees_the_refreshed_path() {
        // The post-install check: an install links a binary while the first resolution is still running, then calls
        // refresh and reads the PATH. The read must hold the folder the refresh found, never an empty or older PATH.
        let st = state();
        let home = tmp("postinstall");
        let (h1, h2) = (home.clone(), home.clone());
        st.start(move || {
            let seen = effective(None, &[], &h1); // before the install: no ~/.local/bin
            std::thread::sleep(Duration::from_millis(400));
            seen
        });
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        assert!(st.refresh(Duration::from_secs(5), Instant::now(), move || effective(None, &[], &h2)), "the refresh ran");
        let dirs = st.current(Duration::from_secs(5)).expect("resolved");
        assert!(dirs.contains(&home.join(".local/bin")), "{dirs:?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_first_resolution_that_finishes_after_a_timed_out_refresh_does_not_overwrite_it() {
        let st = state();
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let (old, new) = (PathBuf::from("/old/first"), PathBuf::from("/new/refreshed"));
        let first = old.clone();
        st.start(move || {
            let _ = gate.recv(); // held until the refresh has stored its answer
            vec![first]
        });
        // The refresh gives up waiting for the first resolution, then resolves and stores.
        let fresh = new.clone();
        assert!(st.refresh(Duration::from_millis(50), Instant::now(), move || vec![fresh]), "the refresh ran");
        release.send(()).unwrap();
        // `current` waits for the first resolution to finish; its older answer must not replace the refresh.
        assert_eq!(st.current(Duration::from_secs(5)), Some(vec![new]));
    }

    #[test]
    fn a_first_resolution_that_panics_or_overruns_never_hangs_a_reader() {
        let st = state();
        st.start(|| panic!("resolve failed"));
        let t = Instant::now();
        assert_eq!(st.current(Duration::from_secs(5)), None, "the caller falls back");
        assert!(t.elapsed() < Duration::from_secs(2), "took {:?}", t.elapsed());
        // A resolution that outlasts the bound: the reader gives up at the bound.
        let st = state();
        st.start(|| {
            std::thread::sleep(Duration::from_secs(2));
            vec![]
        });
        let t = Instant::now();
        assert_eq!(st.current(Duration::from_millis(200)), None);
        assert!(t.elapsed() < Duration::from_secs(1), "took {:?}", t.elapsed());
    }

    #[test]
    fn the_reader_bound_covers_the_shell_deadline_which_is_unchanged() {
        assert_eq!(TIMEOUT, Duration::from_secs(3));
        assert!(FIRST_WAIT > TIMEOUT + LAST_PAIR_GRACE);
    }

    #[test]
    fn a_refresh_never_moves_the_fixed_bridge_path() {
        // The bridge PATH is a constant, not derived from the user PATH, so no refresh can move it.
        let gate = Refresh::new();
        assert!(gate.run(Instant::now(), || vec![PathBuf::from("/tmp/vajra-refresh-test/bin")]).is_some());
        assert_eq!(crate::vajra::bridge::bridge_path(), std::ffi::OsString::from(crate::vajra::bridge::BASE_PATH.join(":")));
    }
}
