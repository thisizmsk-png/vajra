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

#[cfg(test)]
#[path = "evidence_runner_tests.rs"]
mod tests;
