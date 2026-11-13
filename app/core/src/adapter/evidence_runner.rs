//! 6i part B2: the core's evidence runner. The evidence check's commands (setup, criteria checks, the suite) run under
//! macOS Seatbelt (`/usr/bin/sandbox-exec -p <profile>`), never in an agent session (PR #135 review, codex).
//!
//! The profile (owner answers N19 to N23, the 6i brief, 6k's dir list):
//! - every read and write of the Vajra state dir (it holds the signing key) and the app's data dir is denied: the
//!   list is 6k's `vajra_deny_dirs`, the same one a Codex session gets (#216), plus the config's `denyRead` credential
//!   dirs;
//! - writes are allowed only in the evidence worktree, the run's own tmp dir and the build caches the toolchains need
//!   (cargo, npm, go, pip): never the repo's main checkout, never anywhere else in the home folder;
//! - the network follows the verifier seat: `false` keeps it to loopback (a test server can still bind);
//! - the run is killed at `evidence_run_max_s`.
//!
//! The launch-time probe (`key_probe`) reads the state dir under the same profile; the read must be refused by the
//! OS, or the run ends `not_verified`. The probe names the dir, never the key, and reads nothing from it.
//!
//! git inside the runner (S9): `write-tree` needs the object dir, which lives in the repo's common dir outside the
//! worktree. The 6i-e contract copies the index into the ticket tmp, and the runner gives git `GIT_INDEX_FILE` plus a
//! private object dir with the real one as an alternate (`git_env`), so no write ever reaches the shared repo.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// `evidence_run_max_s`: one evidence run (every command, in order) is killed at this.
pub const RUN_MAX: Duration = Duration::from_secs(30 * 60);
/// The launch-time key probe's own limit.
pub const PROBE_MAX: Duration = Duration::from_secs(10);
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// What a run may touch.
#[derive(Clone, Debug)]
pub struct Profile {
    /// The evidence worktree: the commands' cwd, readable and writable.
    pub worktree: PathBuf,
    /// The ticket's own tmp dir (TMPDIR, the index copy, git's private object dir): writable.
    pub tmp: PathBuf,
    /// Dirs nothing may read or write: `vajra_deny_dirs` plus the credential dirs.
    pub deny: Vec<PathBuf>,
    /// Build caches the toolchains write (cargo registry, npm cache, go build cache, pip cache).
    pub caches: Vec<PathBuf>,
    /// The verifier seat's network setting.
    pub network: bool,
}

/// A path as a Seatbelt string literal: absolute, no NUL, quotes and backslashes escaped. `None` refuses it.
fn lit(p: &Path) -> Option<String> {
    let s = p.to_str()?;
    if !p.is_absolute() || s.contains('\0') || s.chars().any(char::is_control) {
        return None;
    }
    Some(format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// Each path as given and as resolved (Seatbelt matches resolved paths: `/var` is `/private/var` on macOS).
fn both(p: &Path) -> Vec<PathBuf> {
    let mut out = vec![p.to_path_buf()];
    let real = p.canonicalize().ok().or_else(|| Some(p.parent()?.canonicalize().ok()?.join(p.file_name()?)));
    if let Some(r) = real.filter(|r| r != p) {
        out.push(r);
    }
    out
}

impl Profile {
    /// The profile text. Deny rules come last, so they win over every allow (Seatbelt: the last match decides).
    pub fn sbpl(&self) -> Result<String, String> {
        let subpaths = |ps: &[PathBuf]| -> Result<String, String> {
            let mut out = String::new();
            for p in ps.iter().flat_map(|p| both(p)) {
                let l = lit(&p).ok_or_else(|| format!("a runner path isn't a plain absolute path: {}", p.display()))?;
                out.push_str(&format!(" (subpath {l})"));
            }
            Ok(out)
        };
        if self.deny.is_empty() {
            return Err("the runner needs the Vajra dirs to deny".into());
        }
        let writable = subpaths(&[vec![self.worktree.clone(), self.tmp.clone()], self.caches.clone()].concat())?;
        let deny = subpaths(&self.deny)?;
        let mut p = String::from("(version 1)\n(allow default)\n");
        // Writes: only the worktree, the tmp dir, the caches, and the null and tty devices.
        p.push_str("(deny file-write*)\n");
        p.push_str(&format!("(allow file-write*{writable} (literal \"/dev/null\") (literal \"/dev/tty\") (regex #\"^/dev/fd/\"))\n"));
        if !self.network {
            // Outbound only to loopback and unix sockets (a test server binds and is reached; the system resolver's
            // socket still answers names, but no connection leaves the Mac). A `network*` allow with `local ip`
            // matches every outbound socket (it is bound locally), so the allow names only the remote end.
            p.push_str("(deny network-outbound)\n");
            p.push_str("(allow network-outbound (remote ip \"localhost:*\") (remote unix-socket))\n");
        }
        // Last, so nothing above can re-open them: the Vajra dirs and the credentials, neither read nor written.
        p.push_str(&format!("(deny file-read* file-write*{deny})\n"));
        Ok(p)
    }

    /// `sandbox-exec -p <profile> -- <program> <args...>`.
    pub fn argv(&self, program: &str, args: &[String]) -> Result<Vec<String>, String> {
        let mut v = vec!["-p".to_string(), self.sbpl()?, "--".into(), program.into()];
        v.extend(args.iter().cloned());
        Ok(v)
    }
}

/// The deny list: 6k's Vajra dirs, plus everything `config/sandbox.json` denies (its `denyRead` credential paths and
/// its `denyWrite` engine dirs: the runner denies both reads and writes of each), plus the Keychains and git and pip
/// credential files, each under `home`.
pub fn deny_dirs(home: &Path, data: &Path) -> Vec<PathBuf> {
    let mut out = super::profiles::vajra_deny_dirs(home, data);
    for rel in [
        ".aws", ".ssh", ".gnupg", ".netrc", ".npmrc", ".pgpass", ".config/gh", ".docker/config.json", ".kube",
        ".claude/skills/vajra", ".claude/hooks", ".pypirc", ".git-credentials", "Library/Keychains",
    ] {
        let p = home.join(rel);
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// The build caches the toolchains need to write, under `home`.
pub fn default_caches(home: &Path) -> Vec<PathBuf> {
    [".cargo/registry", ".cargo/git", ".npm", "Library/Caches/go-build", "go/pkg/mod", "Library/Caches/pip", ".cache"].iter().map(|r| home.join(r)).collect()
}

impl Profile {
    /// Create the worktree's tmp dir and every cache dir before the run: inside the profile a tool can't make a
    /// cache's parent (`~/Library`, say), since only the cache itself is writable.
    pub fn prepare(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.tmp)?;
        for c in &self.caches {
            std::fs::create_dir_all(c)?;
        }
        Ok(())
    }
}

/// git inside the runner (S9): the index copy and a private object dir in `tmp`, the repo's own objects as a
/// read-only alternate. `common` is the repo's common git dir (`git rev-parse --git-common-dir`).
pub fn git_env(tmp: &Path, common: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        ("GIT_INDEX_FILE", tmp.join("index")),
        ("GIT_OBJECT_DIRECTORY", tmp.join("objects")),
        ("GIT_ALTERNATE_OBJECT_DIRECTORIES", common.join("objects")),
    ]
}

/// The walkthrough base `verify-work --sign` gets: canonical (macOS's `/private` form), since sign resolves nothing and
/// refuses any link in the path (#214, 48436f20). `None` if it doesn't exist or isn't a directory.
pub fn canonical_base(base: &Path) -> Option<PathBuf> {
    base.canonicalize().ok().filter(|p| p.is_dir())
}

/// Whether a run's commands may start: `sandbox-exec` exists and, under `profile`, a read of the Vajra state dir is
/// refused by the OS. Any other answer (the read works, a different error, a timeout) is not proven. Run by the core
/// itself, never from inside another sandbox (macOS refuses a nested one, so the probe fails closed there).
pub fn key_probe(profile: &Profile, state_dir: &Path) -> bool {
    if !Path::new(SANDBOX_EXEC).is_file() || !state_dir.is_dir() {
        return false;
    }
    let Some(target) = state_dir.to_str() else { return false };
    let Ok(argv) = profile.argv("/bin/ls", &[target.to_string()]) else { return false };
    match crate::vajra::bridge::run_process_full(Path::new(SANDBOX_EXEC), &argv, Some(&profile.worktree), PROBE_MAX, None, None) {
        Ok(o) => o.code.is_some_and(|c| c != 0) && !o.timed_out && o.stdout.trim().is_empty() && o.stderr.contains("Operation not permitted"),
        Err(_) => false,
    }
}

/// The fd the unsigned evidence comes back on (6i-e's `--run-only` contract).
const EVIDENCE_FD: i32 = 3;
/// The evidence line is one compact JSON object; anything longer isn't a bundle.
const EVIDENCE_MAX: usize = 1 << 20;

/// One evidence run's inputs.
#[derive(Clone, Debug)]
pub struct EvidenceRun<'a> {
    /// `vajra-verify-work.sh` (the engine's own copy).
    pub script: &'a Path,
    pub slug: &'a str,
    /// The setup, check and suite commands, in order (exactly the approved list, for an approval run).
    pub commands: &'a [String],
    /// Canonical walkthrough base (`canonical_base`): the bundle lands in `<base>/<slug>/`.
    pub base: &'a Path,
    /// The repo's common git dir, for `git_env`.
    pub git_common: &'a Path,
    pub home: &'a Path,
    pub limit: Duration,
}

/// What a run gave back.
#[derive(Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// The commands ran; `passed` is the script's exit 0. `sha256` is of the evidence.json bytes the core wrote.
    Ran { passed: bool, sha256: String },
    /// No usable evidence: why, in plain words.
    Failed(String),
}

/// Why signing didn't happen.
#[derive(Debug, PartialEq, Eq)]
pub enum SignOutcome {
    Signed,
    /// Exit 3: one of 6i-e's fixed reasons (the first stderr line, `sign refused: <reason>`).
    Refused(String),
    /// Exit 4: no usable key.
    NoKey,
    /// Anything else (a different exit, a timeout, a first line outside the contract): treated as refused.
    Unknown(String),
}

/// 6i-e's refusal reasons. A first line outside this list is `Unknown` (fail closed).
fn known_refusal(reason: &str, slug: &str) -> bool {
    matches!(reason, "sha256 mismatch" | "head mismatch" | "already signed" | "no evidence.json" | "not JSON" | "a symlinked path") || reason == format!("not a bundle for {slug}")
}

/// `<script> --run-only <slug> <cmd>...` under `profile`, the evidence read from fd 3. The core writes the line it got
/// to `<base>/<slug>/evidence.json` itself (no command can then change it: fd 3 is closed for every command, and the
/// core never re-reads a file a command could rewrite) and returns its sha256 for `sign`.
pub fn run(profile: &Profile, r: &EvidenceRun) -> RunOutcome {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    if !crate::bus::valid_stream_id(r.slug) || r.commands.is_empty() {
        return RunOutcome::Failed("not a runnable evidence check".into());
    }
    if canonical_base(r.base).as_deref() != Some(r.base) {
        return RunOutcome::Failed("the walkthrough folder isn't a plain, resolved path".into());
    }
    if let Err(e) = profile.prepare() {
        return RunOutcome::Failed(format!("couldn't prepare the run's folders: {e}"));
    }
    let Some(script) = r.script.to_str() else { return RunOutcome::Failed("the evidence script's path isn't text".into()) };
    let mut args = vec!["--run-only".to_string(), r.slug.to_string()];
    args.extend(r.commands.iter().cloned());
    let argv = match profile.argv("/bin/bash", &[vec![script.to_string()], args].concat()) {
        Ok(a) => a,
        Err(e) => return RunOutcome::Failed(e),
    };
    let mut fds = [0i32; 2];
    // SAFETY: a fresh pipe; both ends are owned below.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return RunOutcome::Failed("couldn't open the evidence pipe".into());
    }
    // SAFETY: the pipe just made; each fd is owned once.
    let (rd, wr) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    // Close-on-exec on both ends, so no other child the app spawns meanwhile inherits the write end (which would hold
    // the read open past this run). The child gets only fd 3, made inheritable below.
    for fd in fds {
        // SAFETY: fcntl on an fd this function owns.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            return RunOutcome::Failed("couldn't open the evidence pipe".into());
        }
    }
    let wfd = wr.as_raw_fd();
    let mut cmd = std::process::Command::new(SANDBOX_EXEC);
    cmd.args(&argv).current_dir(&profile.worktree).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    cmd.env_clear().env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin").env("HOME", r.home).env("TMPDIR", &profile.tmp).env("LC_ALL", "C.UTF-8");
    cmd.env("VAJRA_WALKTHROUGH_DIR", r.base).env("GIT_TERMINAL_PROMPT", "0").env("GIT_CONFIG_NOSYSTEM", "1");
    for (k, v) in git_env(&profile.tmp, r.git_common) {
        cmd.env(k, v);
    }
    cmd.process_group(0);
    // SAFETY: async-signal-safe calls only (dup2, fcntl) between fork and exec. The write end becomes an inheritable fd
    // 3 (dup2 clears close-on-exec on the copy; if the write end already is fd 3, dup2 would be a no-op, so its flag is
    // cleared instead). The read end, close-on-exec, is gone at exec, or was replaced by the dup2 if it sat on fd 3.
    unsafe {
        cmd.pre_exec(move || {
            let r = if wfd == EVIDENCE_FD { libc::fcntl(wfd, libc::F_SETFD, 0) } else { libc::dup2(wfd, EVIDENCE_FD) };
            if r < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return RunOutcome::Failed(format!("the evidence runner didn't start: {e}")),
    };
    drop(wr); // only the child holds the write end now: EOF comes when it's done with fd 3
    let pid = child.id();
    let reader = std::thread::spawn(move || {
        let mut f = std::fs::File::from(rd);
        let mut buf = Vec::new();
        let _ = (&mut f).take(EVIDENCE_MAX as u64 + 1).read_to_end(&mut buf);
        buf
    });
    let start = std::time::Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) if start.elapsed() >= r.limit => break None,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    // SAFETY: the group this child leads (`process_group(0)`); ESRCH is fine.
    unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
    let _ = child.wait();
    let line = reader.join().unwrap_or_default();
    let Some(code) = code else { return RunOutcome::Failed("the evidence check ran past its time limit".into()) };
    if code == 2 {
        return RunOutcome::Failed("the evidence script refused the run (usage)".into());
    }
    // Exactly one line: compact JSON, a newline, nothing after it.
    let ok_line = line.len() <= EVIDENCE_MAX && line.last() == Some(&b'\n') && !line[..line.len() - 1].contains(&b'\n');
    let Some(json) = ok_line.then(|| &line[..line.len() - 1]) else { return RunOutcome::Failed("the evidence script returned no single evidence line".into()) };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(json) else { return RunOutcome::Failed("the evidence isn't JSON".into()) };
    if v.get("hmac").is_some() || v["slug"].as_str() != Some(r.slug) {
        return RunOutcome::Failed("the evidence isn't an unsigned bundle for this check".into());
    }
    let dir = r.base.join(r.slug);
    if let Err(e) = crate::control::fsx::write_atomic(&dir, "evidence.json", json) {
        return RunOutcome::Failed(format!("couldn't save the evidence: {e}"));
    }
    RunOutcome::Ran { passed: code == 0, sha256: crate::control::sha256::hex_digest(json) }
}

/// `<script> --sign <slug> --expect-sha256 <sha>`, outside the sandbox (it holds the key), in the evidence worktree
/// (sign checks git.head against this checkout's HEAD).
pub fn sign(script: &Path, slug: &str, sha256: &str, base: &Path, worktree: &Path, home: &Path) -> SignOutcome {
    if canonical_base(base).as_deref() != Some(base) {
        return SignOutcome::Unknown("the walkthrough folder isn't a plain, resolved path".into());
    }
    let argv = vec![script.to_string_lossy().into_owned(), "--sign".into(), slug.into(), "--expect-sha256".into(), sha256.into()];
    let o = match crate::vajra::bridge::run_process_env(Path::new("/bin/bash"), &argv, Some(worktree), PROBE_MAX * 6, Some(home), None, &[("VAJRA_WALKTHROUGH_DIR", base)]) {
        Ok(o) => o,
        Err(e) => return SignOutcome::Unknown(format!("signing didn't start: {e}")),
    };
    let first = o.stderr.lines().next().unwrap_or("").trim();
    match (o.timed_out, o.code) {
        (false, Some(0)) => SignOutcome::Signed,
        (false, Some(4)) => SignOutcome::NoKey,
        (false, Some(3)) => match first.strip_prefix("sign refused: ") {
            Some(why) if known_refusal(why, slug) => SignOutcome::Refused(why.to_string()),
            _ => SignOutcome::Unknown("signing refused for a reason Vajra doesn't know".into()),
        },
        _ => SignOutcome::Unknown("signing ended unexpectedly".into()),
    }
}

#[cfg(test)]
#[path = "evidence_runner_tests.rs"]
mod tests;
