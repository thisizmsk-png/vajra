//! 6i part B2: the core's evidence runner. The evidence check's commands (setup, criteria checks, the suite) run under
//! macOS Seatbelt (`/usr/bin/sandbox-exec -p <profile>`), never in an agent session (PR #135 review, codex).
//!
//! The profile (owner answers N19 to N23, the 6i brief, 6k's dir list):
//! - deny-default (#234 follow-up): only what a rule names is allowed (processes, reads, the writes and sockets
//!   below, and a short list of system Mach services), so a service or IPC kind nobody listed stays shut;
//! - every read and write of the Vajra state dir (it holds the signing key) and the app's data dir is denied: the
//!   list is 6k's `vajra_deny_dirs`, the same one a Codex session gets (#216), plus the config's `denyRead` credential
//!   dirs;
//! - writes are allowed only in the evidence worktree and the run's own fresh tmp dir, which also holds every build
//!   cache (`cache_env`): never the repo's main checkout, never a shared cache, never anywhere else in the home folder;
//! - no Mach service that starts or reaches a process outside the sandbox (LaunchServices, Apple Events, the security
//!   server, the pasteboard), and no new launchd job;
//! - unix sockets: only the system resolver's and those under the worktree and the tmp dir;
//! - the network follows the verifier seat: `false` keeps IP to loopback (a test server can still bind). Network off
//!   still allows name lookups, which can carry small amounts of data out, and loopback, which reaches any local
//!   service such as a proxy: it is not a data-loss boundary;
//! - the key must have exactly one link (a hard link would be a path no deny names);
//! - the engine script's own tools come only from the system dirs; the check commands use the user's PATH, so a
//!   check is only as good as the user's toolchain (a tool replaced in a folder the user can write runs as the check);
//! - the run is killed at `evidence_run_max_s`.
//!
//! The launch-time probe (`key_probe`) checks the key's link count, reads the state dir under the same profile (the
//! read must be refused by the OS) and asks LaunchServices for the front app (no answer may come), or the run ends
//! `not_verified`. The probe names the dir, never the key, and reads nothing from it.
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
    /// The run's own tmp dir, fresh for each run (TMPDIR, git's private object dir, every build cache): writable.
    pub tmp: PathBuf,
    /// Dirs nothing may read or write: `vajra_deny_dirs` plus the credential dirs.
    pub deny: Vec<PathBuf>,
    /// The verifier seat's network setting.
    pub network: bool,
}

/// The unix socket the system resolver answers on: the only socket outside the run a command may connect to.
const RESOLVER_SOCKET: &str = "/var/run/mDNSResponder";

/// Mach services that would let a command reach code outside the sandbox (bhishma M1 on #234). A process started
/// through any of them is not a descendant of the sandbox, so it could read the key and use the network.
/// - LaunchServices: `open` an app, a document or a `.command` file;
/// - Apple Events: script another app (`osascript`, `tell application ...`);
/// - the security server: ask for a Keychain item, such as a provider key the app stored;
/// - the pasteboard: read what the user copied.
///
/// launchd refuses `launchctl submit`, `bootstrap` and `load` from a sandboxed caller by itself; `job-creation` is denied
/// too, and the tests prove launchd's refusal. Under deny-default none of these is on the allow-list anyway; the named
/// denies stay so a later, broader allow can't re-open them by accident.
const DENIED_MACH: &[(&str, &str)] = &[
    ("global-name", "com.apple.coreservices.launchservicesd"),
    ("global-name-prefix", "com.apple.lsd."),
    ("global-name", "com.apple.coreservices.appleevents"),
    ("global-name", "com.apple.SecurityServer"),
    ("global-name-prefix", "com.apple.securityd"),
    ("global-name", "com.apple.secd"),
    ("global-name-prefix", "com.apple.pasteboard."),
];

/// The only Mach services a command may look up (deny-default, #234 follow-up). Each is a system lookup a toolchain
/// needs and none starts a process or hands back user data: user and group names, the notification and logging
/// daemons, the per-user temp dir, network configuration (DNS and proxies) and certificate trust (TLS with a network
/// seat). Everything else, LaunchServices, Apple Events, the security server and the pasteboard included, is refused.
const ALLOWED_MACH: &[&str] = &[
    "com.apple.system.opendirectoryd.libinfo",
    "com.apple.system.opendirectoryd.membership",
    "com.apple.system.notification_center",
    "com.apple.system.logger",
    "com.apple.logd",
    "com.apple.diagnosticd",
    "com.apple.bsd.dirhelper",
    "com.apple.SystemConfiguration.configd",
    "com.apple.SystemConfiguration.DNSConfiguration",
    "com.apple.trustd",
    "com.apple.trustd.agent",
];

/// Allowed only when the run has a network seat (bhishma's ruling B on #250): the agent that finishes a TLS chain check.
/// Without it curl's https fetch fails with "unable to get local issuer certificate". It evaluates trust; it is not the
/// security server, so Keychain items stay denied. A run with no seat has no use for it and never reaches it.
const SEAT_MACH: &str = "com.apple.TrustEvaluationAgent";

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
        // The writable roots exactly as given, never resolved here: a link swapped into a root's place can't add the
        // folder it points at (codex P1 on 3dc1a7bf). `run` requires them resolved already.
        let mut run_dirs = String::new();
        for p in [&self.worktree, &self.tmp] {
            let l = lit(p).ok_or_else(|| format!("a runner path isn't a plain absolute path: {}", p.display()))?;
            run_dirs.push_str(&format!(" (subpath {l})"));
        }
        let resolver = both(Path::new(RESOLVER_SOCKET)).iter().map(|p| lit(p).map(|l| format!(" (literal {l})"))).collect::<Option<String>>().ok_or("the resolver socket path")?;
        let deny = subpaths(&self.deny)?;
        // Deny-default (#234 follow-up): an operation is refused unless a rule below names it, so a Mach service, an
        // IPC kind or a device nobody listed stays shut, not open.
        let mut p = String::from("(version 1)\n(deny default)\n");
        // Processes: start and run programs; signal and inspect only processes inside this run's sandbox.
        p.push_str("(allow process-fork process-exec file-map-executable)\n");
        p.push_str("(allow signal process-info* (target same-sandbox))\n(allow process-info-pidinfo process-info-setcontrol (target self))\n");
        p.push_str("(allow sysctl-read)\n(allow ipc-posix-sem)\n");
        // Reads: anything (toolchains live anywhere the user put them), except the final deny below.
        p.push_str("(allow file-read*)\n");
        p.push_str(&format!("(allow mach-lookup{})\n", ALLOWED_MACH.iter().map(|n| format!(" (global-name \"{n}\")")).collect::<String>()));
        // Writes: only the worktree, the run's tmp dir (which holds every build cache), and the null and tty devices.
        p.push_str("(deny file-write*)\n");
        p.push_str(&format!("(allow file-write*{run_dirs} (literal \"/dev/null\") (literal \"/dev/tty\") (regex #\"^/dev/fd/\"))\n"));
        // Mach services that start or reach processes outside the sandbox (M1), and new launchd jobs.
        let mach: String = DENIED_MACH.iter().map(|(kind, name)| format!(" ({kind} \"{name}\")")).collect();
        p.push_str(&format!("(deny mach-lookup{mach})\n(deny job-creation)\n"));
        // Network. Unix sockets, either setting: only the system resolver's and those under the worktree and the tmp
        // dir, never a daemon's (Docker's would start a container with the home folder mounted; ssh and gpg agents
        // sign for the user) (M2). With no network seat, also no IP connection except to loopback: a test server
        // binds and is reached, names still resolve through the resolver socket, but no connection leaves the Mac.
        // A `network*` allow with `local ip` matches every outbound socket (it is bound locally), so allows name only
        // the remote end.
        // Binds and inbound connections: unix sockets only under the run's own dirs; IP on any address with a network
        // seat, else on loopback only (a server on every address would let a machine on the LAN connect in and read).
        if self.network {
            p.push_str("(allow network-outbound (remote ip))\n(allow network-bind network-inbound (local ip))\n");
            p.push_str(&format!("(allow mach-lookup (global-name \"{SEAT_MACH}\"))\n"));
        } else {
            p.push_str("(deny network-outbound)\n(allow network-outbound (remote ip \"localhost:*\"))\n(allow network-bind network-inbound (local ip \"localhost:*\"))\n");
        }
        p.push_str(&format!("(allow network-outbound{resolver}{run_dirs})\n(allow network-bind network-inbound{run_dirs})\n"));
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
        // The Codex sign-in folder (ChatGPT tokens or an API key), the Claude config file, cloud CLI credentials (F1).
        ".codex", ".claude.json", ".config/gcloud", ".azure",
    ] {
        let p = home.join(rel);
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// Every cache and package home a toolchain writes, pointed into the run's tmp through the toolchain's own variable
/// (M3): a check can't plant code in a shared cache that a later build outside the sandbox would compile or run. The
/// user's own caches in the home folder are never writable. Cargo's home and Go's module cache are the run's too, so no
/// build reads sources from a cache the user shares. A dependency then comes from the network seat, or from what the
/// worktree already holds (vendored crates, the N7 `node_modules` clone).
pub fn cache_env(tmp: &Path) -> Vec<(&'static str, PathBuf)> {
    let c = tmp.join("cache");
    vec![
        ("CARGO_HOME", c.join("cargo-home")),
        ("CARGO_TARGET_DIR", c.join("cargo-target")),
        ("GOMODCACHE", c.join("go-mod")),
        ("GOCACHE", c.join("go-build")),
        ("GOTMPDIR", tmp.join("go-tmp")),
        ("npm_config_cache", c.join("npm")),
        ("YARN_CACHE_FOLDER", c.join("yarn")),
        ("PIP_CACHE_DIR", c.join("pip")),
        ("PYTHONPYCACHEPREFIX", c.join("pycache")),
        ("XDG_CACHE_HOME", c.clone()),
    ]
}

impl Profile {
    /// Create the run's tmp dir, git's private object dir and every cache dir before the run.
    pub fn prepare(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.tmp.join("objects"))?;
        for (_, d) in cache_env(&self.tmp) {
            std::fs::create_dir_all(d)?;
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
    if !Path::new(SANDBOX_EXEC).is_file() || !state_dir.is_dir() || !crate::vajra::allowlist::key_has_one_link(state_dir) {
        return false;
    }
    let Some(target) = state_dir.to_str() else { return false };
    let under = |program: &str, args: &[String]| {
        let argv = profile.argv(program, args).ok()?;
        crate::vajra::bridge::run_process_full(Path::new(SANDBOX_EXEC), &argv, Some(&profile.worktree), PROBE_MAX, None, None).ok().filter(|o| !o.timed_out)
    };
    // The state dir: the OS refuses the read.
    let dir_refused = under("/bin/ls", &[target.to_string()]).is_some_and(|o| o.code == Some(1) && o.stdout.trim().is_empty() && o.stderr.contains("Operation not permitted"));
    // LaunchServices, the way out to an unsandboxed process (M1): asking it for the front app gets no answer.
    let launch_refused = under("/usr/bin/lsappinfo", &["front".to_string()]).is_some_and(|o| !o.stdout.contains("ASN:"));
    dir_refused && launch_refused
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
    /// The Vajra state dir: its key must have exactly one link before a run starts (M4).
    pub state_dir: &'a Path,
    /// Where the commands find their tools: the user's resolved PATH (`userpath::current()`, the same one every
    /// worker's spawn uses), so `cargo` from rustup or `node` from a version manager is found as in the verifier's
    /// run. Only absolute entries are kept; the system dirs follow. The sandbox applies whatever a tool is.
    pub tool_path: &'a [PathBuf],
    pub limit: Duration,
}

/// After the script exits, how long the core waits for its evidence channel to close. A background process of the
/// check, or one outside the run that inherited the write end, could hold it open; the run then fails, never waits.
const EOF_GRACE: Duration = Duration::from_secs(2);

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

/// The engine script's own PATH: root-owned, SIP-protected system dirs only (macOS ships bash, jq and git there).
const SCRIPT_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// Sourced by every bash in the run (BASH_ENV). The script's bash is level 0 and keeps `SCRIPT_PATH`; the bash it
/// starts for each check command is level 1 and switches to the user's PATH, which its children inherit. Constant
/// text: the PATH itself only ever travels in `VAJRA_CHECK_PATH`.
const CHECK_ENV: &str = "if [ \"${VAJRA_RUNNER_LEVEL:-0}\" = 1 ]; then PATH=\"$VAJRA_CHECK_PATH\"; export PATH; fi\nexport VAJRA_RUNNER_LEVEL=$(( ${VAJRA_RUNNER_LEVEL:-0} + 1 ))\n";

/// The folder holding the BASH_ENV file (codex P1 on d85a04a2). Every check command's bash sources that file before
/// its command, so a check that could rewrite it (`exit 0`) would pass every later check unrun. It lives next to the
/// run's tmp, never under the worktree or the tmp, so the profile allows no write to it, nor to the folder, which
/// is also made read-only before the run: no rename, no replace, no chmod, and Seatbelt refuses a hard link into a
/// writable folder. It must not exist yet, and is removed when the run ends.
struct StartupDir(PathBuf);

impl StartupDir {
    fn make(profile: &Profile) -> Result<Self, &'static str> {
        use std::os::unix::fs::PermissionsExt;
        let name = profile.tmp.file_name().and_then(|n| n.to_str()).ok_or("the run's temp folder has no name")?;
        let dir = profile.tmp.with_file_name(format!("{name}.bash-env"));
        // The containment check below compares paths as written, so every path must already be resolved (codex P1 on
        // d660bb62): `make_tmp` checked the worktree and the tmp's parent before making the tmp.
        let resolved = |p: &Path| p.canonicalize().ok().as_deref() == Some(p);
        if !resolved(&profile.worktree) || !resolved(&profile.tmp) {
            return Err("the run's folders aren't plain, resolved paths");
        }
        let writable = [&profile.worktree, &profile.tmp];
        if writable.iter().any(|w| dir.starts_with(w) || w.starts_with(&dir)) {
            return Err("the run's startup file would be writable by its checks");
        }
        std::fs::create_dir(&dir).map_err(|_| "the run's startup folder already exists; each run needs a fresh one")?;
        let me = Self(dir);
        let set = |p: &Path, mode| std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode));
        let ok = std::fs::write(me.file(), CHECK_ENV).is_ok() && set(&me.file(), 0o444).is_ok() && set(&me.0, 0o555).is_ok();
        if !ok {
            return Err("couldn't prepare the run's folders");
        }
        Ok(me)
    }
    fn file(&self) -> PathBuf {
        self.0.join("check-env.sh")
    }
}

impl Drop for StartupDir {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::remove_file(self.file());
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// The system dirs every check command's PATH ends with.
const SYSTEM_PATH: [&str; 6] = ["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/opt/homebrew/bin", "/usr/local/bin"];

/// The run's PATH: the user's tool dirs that are absolute plain paths (no `:`, no control character), then the system
/// dirs, each once.
fn run_path(tool_path: &[PathBuf]) -> String {
    let mut out: Vec<String> = Vec::new();
    let user = tool_path.iter().filter(|p| p.is_absolute()).filter_map(|p| p.to_str()).filter(|s| !s.contains(':') && !s.chars().any(char::is_control));
    for s in user.map(str::to_string).chain(SYSTEM_PATH.iter().map(|s| s.to_string())) {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out.join(":")
}

/// The evidence script's text: a plain file (no link), at most 256 KiB, UTF-8 with no NUL (it travels in argv).
fn script_text(script: &Path) -> Result<String, &'static str> {
    let bad = "the evidence script isn't a plain, readable script";
    let m = std::fs::symlink_metadata(script).map_err(|_| bad)?;
    if !m.file_type().is_file() || m.len() > 256 * 1024 {
        return Err(bad);
    }
    let text = String::from_utf8(std::fs::read(script).map_err(|_| bad)?).map_err(|_| bad)?;
    if text.contains('\0') {
        return Err(bad);
    }
    Ok(text)
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
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    if !crate::bus::valid_stream_id(r.slug) || r.commands.is_empty() {
        return RunOutcome::Failed("not a runnable evidence check".into());
    }
    if canonical_base(r.base).as_deref() != Some(r.base) {
        return RunOutcome::Failed("the walkthrough folder isn't a plain, resolved path".into());
    }
    if !crate::vajra::allowlist::key_has_one_link(r.state_dir) {
        return RunOutcome::Failed("Vajra's signing key has a second name somewhere, so the runner can't keep it out of reach".into());
    }
    if let Err(why) = make_tmp(profile, r.git_common) {
        return RunOutcome::Failed(why.into());
    }
    // bash gets the script's text, read here once, never its path: the engine folder is denied inside the profile,
    // and a copy anywhere a command can write could be rewritten while bash reads it, so a command could print a
    // passing line of its own on fd 3. With `-c` bash holds the whole script before any command runs.
    let text = match script_text(r.script) {
        Ok(t) => t,
        Err(why) => return RunOutcome::Failed(why.into()),
    };
    let mut args = vec!["-c".to_string(), text, "vajra-verify-work.sh".into(), "--run-only".into(), r.slug.to_string()];
    args.extend(r.commands.iter().cloned());
    let argv = match profile.argv("/bin/bash", &args) {
        Ok(a) => a,
        Err(e) => return RunOutcome::Failed(e),
    };
    // Both ends close-on-exec before any process can start (L1): macOS has no pipe2, so `cloexec_pipe` holds the
    // app-wide spawn lock exclusive from pipe() until both flags are set, and every spawn in the app takes it shared.
    // No child another thread starts can inherit either end. The runner's own child gets only fd 3, made inheritable
    // below. A held write end is still bounded by EOF_GRACE and the deadline (a background process of a check).
    let Ok((rd, wr)) = crate::spawnlock::cloexec_pipe() else { return RunOutcome::Failed("couldn't open the evidence pipe".into()) };
    let wfd = wr.as_raw_fd();
    let mut cmd = std::process::Command::new(SANDBOX_EXEC);
    cmd.args(&argv).current_dir(&profile.worktree).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // Two PATHs (bhishma F1 on 2e7e304f). The script's own tools (bash, jq, git, mktemp, ...) come only from the
    // root-owned system dirs: from a folder the user can write, a planted jq or bash could forge a passing bundle that
    // sign would accept. The check commands get the user's PATH: `CHECK_ENV`, sourced by every bash through
    // BASH_ENV, switches to it one level down, in the `bash -c "$cmd"` the script starts for each command.
    let env_dir = match StartupDir::make(profile) {
        Ok(d) => d,
        Err(why) => return RunOutcome::Failed(why.into()),
    };
    let check_env = env_dir.file();
    cmd.env_clear().env("PATH", SCRIPT_PATH).env("VAJRA_CHECK_PATH", run_path(r.tool_path)).env("BASH_ENV", &check_env).env("HOME", r.home).env("TMPDIR", &profile.tmp).env("LC_ALL", "C.UTF-8");
    cmd.env("VAJRA_WALKTHROUGH_DIR", r.base).env("GIT_TERMINAL_PROMPT", "0").env("GIT_CONFIG_NOSYSTEM", "1");
    for (k, v) in git_env(&profile.tmp, r.git_common).into_iter().chain(cache_env(&profile.tmp)) {
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
    let mut child = match crate::spawnlock::spawn(&mut cmd) {
        Ok(c) => c,
        Err(e) => return RunOutcome::Failed(format!("the evidence runner didn't start: {e}")),
    };
    drop(wr); // only the child holds the write end now: EOF comes when it's done with fd 3
    let pid = child.id();
    // The reader runs in this loop, non-blocking, bounded by the run's deadline (L1): a write end held open by a
    // process outside the run can't hang it, and more than EVIDENCE_MAX bytes ends it.
    // SAFETY: fcntl on the read end this function owns; O_NONBLOCK applies to this end only.
    let nonblocking = unsafe { libc::fcntl(rd.as_raw_fd(), libc::F_SETFL, libc::fcntl(rd.as_raw_fd(), libc::F_GETFL) | libc::O_NONBLOCK) } == 0;
    let mut f = std::fs::File::from(rd);
    let (mut line, mut chunk) = (Vec::new(), [0u8; 8192]);
    let start = std::time::Instant::now();
    let (mut exited, mut eof) = (None::<(Option<i32>, std::time::Instant)>, false);
    let ended = loop {
        while !eof && line.len() <= EVIDENCE_MAX {
            match f.read(&mut chunk) {
                Ok(0) => eof = true,
                Ok(n) => line.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => eof = true,
            }
        }
        if exited.is_none() {
            match child.try_wait() {
                Ok(Some(s)) => exited = Some((s.code(), std::time::Instant::now())),
                Ok(None) => {}
                Err(_) => exited = Some((None, std::time::Instant::now())),
            }
        }
        match exited {
            _ if !nonblocking => break Err("couldn't read the evidence pipe"),
            _ if line.len() > EVIDENCE_MAX => break Err("the evidence script returned no single evidence line"),
            Some((code, _)) if eof => break Ok(code),
            Some((_, at)) if at.elapsed() >= EOF_GRACE => break Err("the evidence channel stayed open after the script ended"),
            _ if start.elapsed() >= r.limit => break Err("the evidence check ran past its time limit"),
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    // SAFETY: the group this child leads (`process_group(0)`); ESRCH is fine.
    unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
    let _ = child.wait();
    let code = match ended {
        Ok(Some(c)) => c,
        Ok(None) => return RunOutcome::Failed("the evidence script was killed".into()),
        Err(why) => return RunOutcome::Failed(why.into()),
    };
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
    if save_evidence(r.base, r.slug, json).is_err() {
        return RunOutcome::Failed("the evidence folder was replaced during the run".into());
    }
    RunOutcome::Ran { passed: code == 0, sha256: crate::control::sha256::hex_digest(json) }
}

#[cfg(test)]
thread_local! {
    /// Test-only: runs after the bundle folder is pinned and before the write, to swap the path under the core.
    pub(super) static AFTER_PIN: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

/// Write `<base>/<slug>/evidence.json` through pinned folders. The base is under the worktree, which a check's
/// commands can write, and a process of theirs can outlive the kill (`setsid`), so any name on the way could be
/// swapped for a link at any moment. The walk opens every component from `/` with O_NOFOLLOW, holds the bundle
/// folder's descriptor, and does the whole atomic write (`openat`, `renameat`) relative to it: a swap after the open
/// can't redirect the write, and a link on the way fails it.
/// Open `path`'s folder by walking from `/`, every component with O_NOFOLLOW, each relative to the one above: a link
/// anywhere on the way fails it, and the descriptor stays on that folder whatever is renamed afterwards. With `create`,
/// missing components below `from` (0700) are made the same way.
fn walk(from: Option<&crate::projects::pinned::Pinned>, path: &Path, create: bool) -> std::io::Result<crate::projects::pinned::Pinned> {
    use crate::projects::pinned::Pinned;
    use std::path::Component;
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a plain path");
    let mut dir: Option<Pinned> = None;
    for c in path.components() {
        let next = match (c, dir.as_ref().or(from)) {
            (Component::RootDir, None) => Pinned::open(Path::new("/"), false)?,
            (Component::Normal(n), Some(d)) => d.child(n.to_str().ok_or_else(bad)?, create)?,
            _ => return Err(bad()),
        };
        dir = Some(next);
    }
    dir.ok_or_else(bad)
}

#[cfg(test)]
type StageHook = Option<Box<dyn Fn(&str)>>;

#[cfg(test)]
thread_local! {
    /// Test-only: runs at each stage of making the run's tmp ("before-create", "after-create"), to swap it for a link.
    pub(super) static TMP_HOOK: std::cell::RefCell<StageHook> = const { std::cell::RefCell::new(None) };
}

fn tmp_hook(_stage: &str) {
    #[cfg(test)]
    TMP_HOOK.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f(_stage)
        }
    });
}

/// Make the run's tmp and its folders without ever following a link (codex P1 on 3dc1a7bf, bhishma Low). The worktree
/// and the tmp's parent must be resolved paths already, so the profile's literal roots are exact. The tmp is made by
/// one exclusive `mkdir` (a name already there, a link included, refuses the run), then pinned by a walk from `/` that
/// refuses a link anywhere on the way, and checked to be a plain folder at its resolved path; every folder in it is
/// made relative to that descriptor. A swap afterwards can't widen anything: the profile names only the literal tmp,
/// and Seatbelt judges a write by the path it resolves to.
fn make_tmp(profile: &Profile, git_common: &Path) -> Result<(), &'static str> {
    let resolved = |p: &Path| p.canonicalize().ok().as_deref() == Some(p);
    if !resolved(&profile.worktree) || !profile.tmp.parent().is_some_and(resolved) {
        return Err("the run's folders aren't plain, resolved paths");
    }
    tmp_hook("before-create");
    // A fresh tmp for every run (L2): a process an earlier run left behind can't have prepared this one's files.
    if std::fs::create_dir(&profile.tmp).is_err() {
        return Err("the run's temp folder already exists; each run needs a fresh one");
    }
    tmp_hook("after-create");
    let replaced = "the run's temp folder was replaced as it was made";
    let tmp = walk(None, &profile.tmp, false).map_err(|_| replaced)?;
    let plain = std::fs::symlink_metadata(&profile.tmp).is_ok_and(|m| m.file_type().is_dir()) && resolved(&profile.tmp);
    if !plain {
        return Err(replaced);
    }
    let rel = |d: &Path| d.strip_prefix(&profile.tmp).map(Path::to_path_buf).map_err(|_| replaced);
    for d in std::iter::once(profile.tmp.join("objects")).chain(cache_env(&profile.tmp).into_iter().map(|(_, d)| d)) {
        walk(Some(&tmp), &rel(&d)?, true).map_err(|_| "couldn't prepare the run's folders")?;
    }
    // git's private index (`git_env`) starts as a copy of the worktree's own (codex P2 on f95df103): a missing one
    // reads as empty, so every tracked file would look deleted and untracked. Written through the pinned tmp.
    if let Some(index) = worktree_index(&profile.worktree, git_common)? {
        tmp.write_atomic("index", &index).map_err(|_| "couldn't copy the worktree's git index")?;
    }
    Ok(())
}

/// The evidence worktree's git index, found from protected data only (bhishma F1 on b04316cb). The worktree's own
/// `.git` file is writable, so it could point anywhere; its content, and git's answer from it, are never used. The
/// index is the one in the run's git common dir (`common`, which must be resolved): for a main checkout, `common` is
/// the worktree's `.git` folder; for a linked worktree, it's the admin folder `<common>/worktrees/<name>` whose own
/// `gitdir` back-link (kept in `common`, outside the worktree) names exactly this worktree's `.git`. It is read through
/// a pinned walk that follows no link, and must be a plain file with exactly one link, at most 512 MiB. `None` when
/// the worktree isn't in git, or nothing was ever staged.
fn worktree_index(worktree: &Path, common: &Path) -> Result<Option<Vec<u8>>, &'static str> {
    const CAP: u64 = 512 << 20;
    let Some(admin) = git_admin(worktree, common)? else { return Ok(None) };
    // A regular file only (#234 follow-up): the pinned open uses O_NOFOLLOW (a symlink fails), O_NONBLOCK (a FIFO
    // can't stall the run) and checks the opened file is regular (a directory, FIFO or device fails).
    match admin.read_single_link("index", CAP) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("the worktree's git index isn't a plain file with one link"),
    }
}

/// The commit `worktree` has checked out, read from protected data only, through the same pinned walk as its index
/// (bhishma M1 on the approval wiring): the HEAD in the worktree's admin folder inside the run's git common dir
/// `common`, which also proves `common` is this worktree's own repository. A symbolic HEAD is followed one level, to a
/// loose ref under `common`'s refs folder or a line of its packed-refs. Never the worktree's own `.git` file or git.
pub(crate) fn worktree_head(worktree: &Path, common: &Path) -> Result<String, &'static str> {
    const CAP: u64 = 4096;
    let not_ours = "the worktree's git folder isn't the run's repository";
    let unreadable = "the worktree's checked-out commit can't be read";
    let admin = git_admin(worktree, common)?.ok_or(not_ours)?;
    let text = |b: Vec<u8>| String::from_utf8(b).ok().map(|s| s.trim_end_matches('\n').to_string());
    let head = admin.read_single_link("HEAD", CAP).ok().and_then(text).ok_or(unreadable)?;
    let is_sha = |s: &str| matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    let Some(name) = head.strip_prefix("ref: ") else {
        return if is_sha(&head) { Ok(head) } else { Err(unreadable) };
    };
    // Only a branch or tag-like ref under refs, with plain components: no `..`, no empty part.
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() < 2 || parts[0] != "refs" || parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return Err(unreadable);
    }
    let common_dir = walk(None, common, false).map_err(|_| not_ours)?;
    let mut dir = common_dir.child(parts[0], false).ok();
    for p in &parts[1..parts.len() - 1] {
        dir = dir.and_then(|d| d.child(p, false).ok());
    }
    if let Some(sha) = dir.and_then(|d| d.read_single_link(parts[parts.len() - 1], CAP).ok()).and_then(text) {
        return if is_sha(&sha) { Ok(sha) } else { Err(unreadable) };
    }
    let packed = common_dir.read_single_link("packed-refs", 64 << 20).ok().and_then(text).ok_or(unreadable)?;
    packed
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with('^'))
        .find_map(|l| l.split_once(' ').filter(|(_, r)| *r == name).map(|(s, _)| s.to_string()))
        .filter(|s| is_sha(s))
        .ok_or(unreadable)
}

/// The worktree's admin folder inside the run's git common dir, pinned; `None` when the worktree isn't in git. See
/// `worktree_index` for how it is found without trusting anything the worktree can write.
fn git_admin(worktree: &Path, common: &Path) -> Result<Option<crate::projects::pinned::Pinned>, &'static str> {
    let dotgit = worktree.join(".git");
    let Ok(kind) = std::fs::symlink_metadata(&dotgit).map(|m| m.file_type()) else { return Ok(None) };
    let not_ours = "the worktree's git folder isn't the run's repository";
    if common.canonicalize().ok().as_deref() != Some(common) {
        return Err(not_ours);
    }
    let common_dir = walk(None, common, false).map_err(|_| not_ours)?;
    let admin = if kind.is_dir() && dotgit == common {
        common_dir
    } else if kind.is_file() {
        let worktrees = common_dir.child("worktrees", false).map_err(|_| not_ours)?;
        let want = dotgit.to_str().ok_or(not_ours)?;
        let mut found = None;
        for name in worktrees.names().map_err(|_| not_ours)? {
            let Ok(dir) = worktrees.child(&name, false) else { continue };
            let back = dir.read_single_link("gitdir", 4096).ok().and_then(|b| String::from_utf8(b).ok());
            if back.as_deref().map(|s| s.trim_end_matches('\n')) == Some(want) {
                if found.is_some() {
                    return Err(not_ours);
                }
                found = Some(dir);
            }
        }
        found.ok_or(not_ours)?
    } else {
        return Err(not_ours);
    };
    Ok(Some(admin))
}

fn save_evidence(base: &Path, slug: &str, json: &[u8]) -> std::io::Result<()> {
    let dir = walk(None, base, false)?.child(slug, true)?;
    #[cfg(test)]
    AFTER_PIN.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f()
        }
    });
    dir.write_atomic("evidence.json", json)
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
