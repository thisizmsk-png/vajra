//! What a terminal may be opened on, and how many. Pure functions and plain data, so they are tested without a PTY.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use super::TermError;

/// Caps. `max_open` and `max_per_root` are counted over open terminals (an ended shell still holds its slot until its
/// tab is closed, like a real terminal). The ADR-15 target for projects is 8 per project and 24 in all; the default
/// here is 8 open in all until projects exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_open: usize,
    pub max_per_root: usize,
    /// Output bytes sent and not yet acknowledged before the reader stops (it resumes at half).
    pub window: usize,
    /// Largest single `write` (a paste is chunked by the caller).
    pub max_write: usize,
    /// Typed bytes that may wait for a shell that is not reading; more is answered `QueueFull`.
    pub write_queue: usize,
    /// Output the reader takes from one terminal per second, on average (a burst of `RATE_BURST` is free). Above it the
    /// reader waits, the PTY fills and the shell blocks on its write: a flood costs a few percent of a core instead of
    /// all of it, and nothing is dropped. No view can paint faster than this anyway; 0 turns it off.
    pub max_rate: usize,
    /// The same limit for a pane nobody is looking at, 32 MiB/s by default. No build or test log comes near that, so a
    /// background build is never held in practice; a pathological flood (`yes`) into a hidden pane costs about 29% of a core
    /// instead of 64% unpaced (hiding a pane must not make it dearer than showing it). 0 does not hold a hidden pane at all.
    pub hidden_rate: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_open: 8, max_per_root: 8, window: 1024 * 1024, max_write: 256 * 1024, write_queue: 1024 * 1024, max_rate: 8 * 1024 * 1024, hidden_rate: 32 * 1024 * 1024 }
    }
}

/// Where the user's login-shell environment comes from. The login shell rebuilds most of its environment from the
/// user's profile; this supplies the base it starts from. Swappable: #70 resolves the login-shell PATH once at startup
/// and implements this, so GUI launches find the same tools a Terminal window does.
pub trait LoginEnv: Send + Sync {
    /// The PATH to start the shell with, if one was resolved.
    fn path(&self) -> Option<OsString>;
    /// The user's login shell, if known. Must be an absolute path to an executable file or it is ignored.
    fn shell(&self) -> Option<PathBuf>;
    /// The HOME the shell sees. `None` keeps the app's own (production); tests give a temp HOME.
    fn home(&self) -> Option<PathBuf> {
        None
    }
}

/// Until #70 lands: no resolved PATH, and `$SHELL` from the app's own environment.
pub struct ProcessEnv;

impl LoginEnv for ProcessEnv {
    fn path(&self) -> Option<OsString> {
        None
    }
    fn shell(&self) -> Option<PathBuf> {
        std::env::var_os("SHELL").map(PathBuf::from)
    }
}

/// A shell program the core may run: absolute, an executable regular file. Anything else falls back to `/bin/sh`.
pub fn usable_shell(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.is_absolute() && std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn choose_shell(env: &dyn LoginEnv) -> PathBuf {
    env.shell().filter(|p| usable_shell(p)).unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

/// The folder the app keeps a run's worker worktrees in, below a repository root.
const WORKTREES: [&str; 2] = [".claude", "worktrees"];

/// Where a terminal opened: the canonical directory and the root it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub cwd: PathBuf,
    pub root: PathBuf,
}

/// `cwd` may be a known root itself or a worker worktree directly below `<root>/.claude/worktrees/`. Both sides are
/// canonicalised (symlinks resolved), so a link inside a root that points elsewhere, a `..` climb and a sibling folder
/// with a shared prefix are all refused. The caller supplies the roots; the UI never does.
pub fn place(cwd: &Path, roots: &[PathBuf]) -> Result<Placement, TermError> {
    if !cwd.is_absolute() || cwd.components().any(|c| c == Component::ParentDir) {
        return Err(TermError::BadCwd);
    }
    let real = cwd.canonicalize().map_err(|_| TermError::BadCwd)?;
    if !real.is_dir() {
        return Err(TermError::BadCwd);
    }
    for root in roots {
        let Ok(root) = root.canonicalize() else { continue };
        if real == root {
            return Ok(Placement { cwd: real, root });
        }
        let wt: PathBuf = WORKTREES.iter().fold(root.clone(), |p, c| p.join(c));
        if real.parent() == Some(wt.as_path()) {
            return Ok(Placement { cwd: real, root });
        }
    }
    Err(TermError::NotAllowed)
}

/// Every environment variable a terminal gets from the app's own environment, by name. A login shell rebuilds the rest
/// from the user's profile; nothing else the app holds (steer nonces, tokens a launcher exported) is passed on.
pub const ENV_ALLOW: [&str; 4] = ["HOME", "USER", "LOGNAME", "LANG"];

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vajra-termpol-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    #[test]
    fn a_root_and_a_worker_worktree_are_allowed_and_nothing_else() {
        let d = dir("place");
        let root = d.join("repo");
        let wt = root.join(".claude/worktrees/w1");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(d.join("repo-other")).unwrap();
        let roots = vec![root.clone()];
        assert_eq!(place(&root, &roots).unwrap(), Placement { cwd: root.clone(), root: root.clone() });
        assert_eq!(place(&wt, &roots).unwrap().root, root);
        assert_eq!(place(&root.join("src"), &roots), Err(TermError::NotAllowed), "a subfolder is not a root or a worktree");
        assert_eq!(place(&d.join("repo-other"), &roots), Err(TermError::NotAllowed), "a shared name prefix is not inside");
        assert_eq!(place(&d, &roots), Err(TermError::NotAllowed), "the parent of a root is not allowed");
        assert_eq!(place(&wt.join("deep"), &roots), Err(TermError::BadCwd), "missing");
    }

    #[test]
    fn a_climb_a_relative_path_a_file_and_a_symlink_out_are_refused() {
        let d = dir("evil");
        let root = d.join("repo");
        std::fs::create_dir_all(root.join(".claude/worktrees")).unwrap();
        let outside = d.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(".claude/worktrees/escape")).unwrap();
        std::fs::write(d.join("file"), "x").unwrap();
        let roots = vec![root.clone()];
        assert_eq!(place(&root.join("../outside"), &roots), Err(TermError::BadCwd));
        assert_eq!(place(Path::new("repo"), &roots), Err(TermError::BadCwd));
        assert_eq!(place(&d.join("file"), &roots), Err(TermError::BadCwd));
        assert_eq!(place(&root.join(".claude/worktrees/escape"), &roots), Err(TermError::NotAllowed), "a link that leaves the root");
        // A root that no longer exists grants nothing.
        assert_eq!(place(&outside, &[d.join("gone")]), Err(TermError::NotAllowed));
        assert_eq!(place(&outside, &[]), Err(TermError::NotAllowed));
    }

    #[test]
    fn the_shell_is_an_absolute_executable_file_else_bin_sh() {
        struct E(Option<PathBuf>);
        impl LoginEnv for E {
            fn path(&self) -> Option<OsString> {
                None
            }
            fn shell(&self) -> Option<PathBuf> {
                self.0.clone()
            }
        }
        let d = dir("shell");
        let plain = d.join("plain");
        std::fs::write(&plain, "x").unwrap();
        assert_eq!(choose_shell(&E(Some("zsh".into()))), PathBuf::from("/bin/sh"), "relative");
        assert_eq!(choose_shell(&E(Some(plain))), PathBuf::from("/bin/sh"), "not executable");
        assert_eq!(choose_shell(&E(Some(d.clone()))), PathBuf::from("/bin/sh"), "a directory");
        assert_eq!(choose_shell(&E(Some("/bin/sh".into()))), PathBuf::from("/bin/sh"));
        assert_eq!(choose_shell(&E(None)), PathBuf::from("/bin/sh"));
    }
}
