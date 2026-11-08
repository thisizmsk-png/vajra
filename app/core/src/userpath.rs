//! The user's PATH, resolved once at startup.
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
use std::sync::OnceLock;
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

static CURRENT: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// Resolve the user PATH once. Call from `main` before anything reads it; later calls keep the first answer.
pub fn init(home: &Path) {
    let login = login_shell().and_then(|sh| detect(&sh, home, TIMEOUT));
    let _ = CURRENT.set(effective(login, &process_path(), home));
}

/// The resolved dirs. Before `init` (unit tests, `--mcp`) this is the process PATH plus the install dirs; no shell
/// is ever started here.
pub fn current() -> Vec<PathBuf> {
    match CURRENT.get() {
        Some(d) => d.clone(),
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

/// Read `r` on this thread until the markers parse, EOF, `cap` bytes, or `deadline`, whichever comes first. Each read
/// waits in `poll` for at most the time left, so a shell (or an escaped grandchild) that holds the pipe open and
/// prints nothing cannot outlast the deadline; no reader thread or queue exists to leak or grow. `out` never holds
/// more than `cap` bytes: each read asks only for the room left.
fn collect<R: Read + std::os::fd::AsRawFd>(r: &mut R, deadline: Instant, cap: usize, out: &mut Vec<u8>) -> Option<Vec<PathBuf>> {
    let mut buf = [0u8; CHUNK];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let room = cap.saturating_sub(out.len());
        if left.is_zero() || room == 0 {
            return None;
        }
        let mut pfd = libc::pollfd { fd: r.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = left.as_millis().clamp(1, i32::MAX as u128) as libc::c_int;
        // SAFETY: one valid pollfd for an fd `r` owns for the whole call.
        let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
        if rc < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return None;
        }
        if rc == 0 {
            continue; // the deadline check above ends the loop
        }
        // poll said readable (data, EOF or error), so this read does not block.
        match r.read(&mut buf[..CHUNK.min(room)]) {
            Ok(0) => return None, // EOF before the markers
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if let Some(p) = parse(out) {
                    return Some(p);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
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
}
