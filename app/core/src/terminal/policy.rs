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
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_open: 8, max_per_root: 8, window: 1024 * 1024, max_write: 256 * 1024, write_queue: 1024 * 1024 }
    }
}

/// Where the user's login-shell environment comes from. The login shell rebuilds most of its environment from the
/// user's profile; this supplies the base it starts from. The app's implementation is `UserPathEnv`, which reads the
/// login environment `userpath` resolved once at startup, so a tab finds the same tools a Terminal window does.
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

/// The login environment `crate::userpath` resolved at startup: its PATH (the user's login-shell PATH, then the process
/// PATH, then the usual install dirs; absolute entries only) and its login shell. A terminal is interactive, so it gets
/// the USER PATH, unlike the scripts the app runs that hold keys, which stay on the system PATH.
pub struct UserPathEnv {
    home: Option<PathBuf>,
    /// Fixed dirs (tests); `None` reads `userpath::current()` live.
    dirs: Option<Vec<PathBuf>>,
    shell: Option<PathBuf>,
}

impl UserPathEnv {
    /// The app's: `userpath`'s resolved PATH and login shell, and this HOME.
    pub fn app(home: PathBuf) -> Self {
        Self { home: Some(home), dirs: None, shell: None }
    }

    /// With a fixed PATH, for tests that resolve one themselves (with a temp HOME).
    pub fn with_dirs(home: PathBuf, dirs: Vec<PathBuf>) -> Self {
        Self { home: Some(home), dirs: Some(dirs), shell: None }
    }

    /// Use this shell instead of the resolved login shell (tests, so they get `/bin/sh`).
    pub fn with_shell(mut self, shell: PathBuf) -> Self {
        self.shell = Some(shell);
        self
    }
}

impl Default for UserPathEnv {
    /// The process's own HOME.
    fn default() -> Self {
        Self { home: None, dirs: None, shell: None }
    }
}

impl LoginEnv for UserPathEnv {
    fn path(&self) -> Option<OsString> {
        match &self.dirs {
            Some(d) => std::env::join_paths(d).ok(),
            None => crate::userpath::joined(),
        }
    }
    fn shell(&self) -> Option<PathBuf> {
        self.shell.clone().or_else(|| pick_shell(std::env::var_os("SHELL").map(PathBuf::from), crate::userpath::login_shell()))
    }
    fn home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

/// The terminal's shell: the user's configured `$SHELL` when it is usable (absolute, an executable file), whatever it is
/// (fish, nu, elvish...). Only otherwise the resolved login shell. The POSIX-only list in `userpath` belongs to its PATH
/// probe script, not to which interactive shell the human gets.
pub fn pick_shell(configured: Option<PathBuf>, fallback: Option<PathBuf>) -> Option<PathBuf> {
    configured.filter(|p| usable_shell(p)).or(fallback)
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
    fn a_configured_executable_shell_outside_the_probe_list_is_kept() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("fish");
        let fish = d.join("fish");
        std::fs::write(&fish, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&fish, std::fs::Permissions::from_mode(0o755)).unwrap();
        let zsh = Some(PathBuf::from("/bin/zsh"));
        assert_eq!(pick_shell(Some(fish.clone()), zsh.clone()), Some(fish.clone()));
        // and it reaches the shell the terminal runs
        assert_eq!(choose_shell(&UserPathEnv::with_dirs(d.clone(), vec![]).with_shell(fish.clone())), fish);
    }

    #[test]
    fn an_unusable_configured_shell_falls_back_to_the_resolved_login_shell() {
        let d = dir("nofish");
        let plain = d.join("fish"); // exists but is not executable
        std::fs::write(&plain, "x").unwrap();
        let zsh = Some(PathBuf::from("/bin/zsh"));
        for bad in [Some(plain), Some(d.join("missing")), Some(PathBuf::from("fish")), None] {
            assert_eq!(pick_shell(bad, zsh.clone()), zsh);
        }
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
