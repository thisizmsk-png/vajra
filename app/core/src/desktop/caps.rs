//! Run caps (ADR-15 2.6 step 5, owner decision D5): they replace the old "one run in the whole app" guard.
//!
//! - **1 active run per project.** Two workflows on one repo fight over its branches, worktrees and base. A project
//!   is identified by its git common dir, so a repo and its worktrees are one project (ADR-15 1.2).
//! - **At most `max_runs` active runs in the app**: 2 by default, a setting from 1 to 4.
//! - **A worker ceiling of 8 across concurrent runs**, on top of each run's own `maxInFlight`. A run started while no
//!   other run is active is never held by the ceiling, so one run keeps today's 1 to 16 range.
//!
//! A start over a cap is **held with a reason**: it is refused with `run_held`, a reason code and plain words. It is
//! never queued behind the user's back and nothing that is running is stopped.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Default and maximum for the app-wide run cap.
pub const DEFAULT_MAX_RUNS: u32 = 2;
pub const MAX_MAX_RUNS: u32 = 4;
/// Workers across all concurrent runs.
pub const WORKER_CEILING: u32 = 8;
/// What a resumed run counts against the ceiling: the workflow's own `maxInFlight` default.
pub const DEFAULT_RUN_WORKERS: u32 = 4;
/// The `.git` file and `commondir` are tiny; anything bigger is not git's.
/// The most of any Git metadata file (`.git`, `commondir`, `config`) that is ever read; a larger one is refused, not truncated.
pub(super) const GIT_META_CAP: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunLimits {
    pub max_runs: u32,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self { max_runs: DEFAULT_MAX_RUNS }
    }
}

impl RunLimits {
    pub fn with_max_runs(max_runs: u32) -> Result<Self, Value> {
        if (1..=MAX_MAX_RUNS).contains(&max_runs) {
            Ok(Self { max_runs })
        } else {
            Err(json!({"error": "invalid_input", "field": "max_runs", "detail": format!("must be 1 to {MAX_MAX_RUNS}")}))
        }
    }
    pub fn to_json(self) -> Value {
        json!({"max_runs": self.max_runs, "max_max_runs": MAX_MAX_RUNS, "runs_per_project": 1, "worker_ceiling": WORKER_CEILING})
    }
}

/// One active run, as the caps see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Active {
    pub project: PathBuf,
    pub workers: u32,
}

/// Why a start is held. `code` is stable for the UI; `detail` is shown as is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub code: &'static str,
    pub detail: String,
}

impl Held {
    pub fn to_json(&self) -> Value {
        json!({"error": "run_held", "reason": self.code, "detail": self.detail})
    }
}

/// May a run on `project` with `workers` start next to `active`? Checked in order: the project, the app, the ceiling.
pub fn check(limits: RunLimits, active: &[Active], project: &Path, workers: u32) -> Result<(), Held> {
    if active.iter().any(|a| a.project == project) {
        return Err(Held {
            code: "project_busy",
            detail: "This project already has a run going. It can start when that run finishes or is stopped.".into(),
        });
    }
    let n = active.len() as u32;
    if n >= limits.max_runs {
        return Err(Held {
            code: "app_runs_full",
            detail: format!("{n} runs are going, the most the app runs at once ({}). It can start when one finishes.", limits.max_runs),
        });
    }
    let used: u32 = active.iter().map(|a| a.workers).sum();
    if n > 0 && used.saturating_add(workers) > WORKER_CEILING {
        let free = WORKER_CEILING.saturating_sub(used);
        let detail = if free == 0 {
            format!("All {WORKER_CEILING} workers are in use by other runs. It can start when a run finishes.")
        } else {
            format!("{used} of {WORKER_CEILING} workers are in use by other runs. Lower this run's workers to {free} or less, or wait for a run to finish.")
        };
        return Err(Held { code: "workers_full", detail });
    }
    Ok(())
}

/// The project a run directory belongs to: its git common dir (canonical), so a repo and each of its worktrees map
/// to the same key. Found by reading `.git` (a dir, or a `gitdir:` file) and a worktree's `commondir` file, walking up
/// from `cwd`. Nothing is executed. A folder outside any repo is its own project (its canonical path).
#[cfg(test)]
pub fn project_key(cwd: &Path) -> PathBuf {
    let start = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let mut dir = start.as_path();
    loop {
        if let Some(key) = key_of_entry(dir, &start) {
            return key;
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return start,
        }
    }
}

/// The project key of the `.git` entry directly in `dir` (one look, no walking up), or `None` if `dir` holds none.
/// `fallback` is the key of a `.git` file that names no git dir.
pub fn key_of_entry(dir: &Path, fallback: &Path) -> Option<PathBuf> {
    let dot = dir.join(".git");
    match std::fs::symlink_metadata(&dot) {
        Ok(m) if m.is_dir() => Some(canon(&dot)),
        Ok(m) if m.is_file() => {
            if let Some(gitdir) = read_small(&dot).and_then(|t| t.trim().strip_prefix("gitdir:").map(|g| g.trim().to_string())) {
                let gitdir = dir.join(gitdir);
                let common = match read_small(&gitdir.join("commondir")) {
                    Some(c) => gitdir.join(c.trim()),
                    None => gitdir,
                };
                return Some(canon(&common));
            }
            Some(fallback.to_path_buf())
        }
        _ => None,
    }
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn read_small(p: &Path) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    // O_NONBLOCK so a planted FIFO cannot hang the open; the type and size are checked on the descriptor, and at most cap+1 bytes are read.
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(p).ok()?;
    let m = f.metadata().ok()?;
    if !m.is_file() || m.len() > GIT_META_CAP {
        return None;
    }
    let mut buf = Vec::new();
    f.take(GIT_META_CAP + 1).read_to_end(&mut buf).ok()?;
    if buf.len() as u64 > GIT_META_CAP {
        return None;
    }
    String::from_utf8(buf).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vajra-caps-{tag}-{}-{}", std::process::id(), crate::control::nonce::random_hex(4).unwrap()));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn act(p: &str, w: u32) -> Active {
        Active { project: PathBuf::from(p), workers: w }
    }

    #[test]
    fn one_run_per_project() {
        let e = check(RunLimits::default(), &[act("/r/a/.git", 4)], Path::new("/r/a/.git"), 1).unwrap_err();
        assert_eq!(e.code, "project_busy");
        assert_eq!(e.to_json()["error"], "run_held");
        assert!(check(RunLimits::default(), &[act("/r/a/.git", 4)], Path::new("/r/b/.git"), 4).is_ok(), "another project may start");
    }

    #[test]
    fn two_runs_by_default_and_the_setting_goes_up_to_four() {
        let two = [act("/r/a/.git", 1), act("/r/b/.git", 1)];
        assert_eq!(check(RunLimits::default(), &two, Path::new("/r/c/.git"), 1).unwrap_err().code, "app_runs_full");
        let four = RunLimits::with_max_runs(4).unwrap();
        assert!(check(four, &two, Path::new("/r/c/.git"), 1).is_ok());
        let full = [act("/r/a/.git", 1), act("/r/b/.git", 1), act("/r/c/.git", 1), act("/r/d/.git", 1)];
        assert_eq!(check(four, &full, Path::new("/r/e/.git"), 1).unwrap_err().code, "app_runs_full");
        for bad in [0, 5, 99] {
            assert!(RunLimits::with_max_runs(bad).is_err(), "{bad}");
        }
        assert_eq!(RunLimits::with_max_runs(1).unwrap().max_runs, 1);
    }

    #[test]
    fn the_worker_ceiling_holds_a_second_run_but_never_a_lone_one() {
        assert!(check(RunLimits::default(), &[], Path::new("/r/a/.git"), 16).is_ok(), "a run alone keeps today's range");
        let e = check(RunLimits::default(), &[act("/r/a/.git", 6)], Path::new("/r/b/.git"), 4).unwrap_err();
        assert_eq!(e.code, "workers_full");
        assert!(e.detail.contains("to 2 or less"), "{}", e.detail);
        assert!(check(RunLimits::default(), &[act("/r/a/.git", 6)], Path::new("/r/b/.git"), 2).is_ok());
    }

    #[test]
    fn a_repo_and_its_worktrees_are_one_project() {
        let d = tmp("wt");
        let repo = d.join("repo");
        std::fs::create_dir_all(repo.join(".git/worktrees/w1")).unwrap();
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        std::fs::write(repo.join(".git/worktrees/w1/commondir"), "../..\n").unwrap();
        let wt = repo.join(".claude/worktrees/w1");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", repo.join(".git/worktrees/w1").display())).unwrap();
        let key = project_key(&repo);
        assert_eq!(key, repo.join(".git"));
        assert_eq!(project_key(&repo.join("src/deep")), key, "a subfolder is the same project");
        assert_eq!(project_key(&wt), key, "a worktree is its repo's project");
        let other = d.join("other");
        std::fs::create_dir_all(other.join(".git")).unwrap();
        assert_ne!(project_key(&other), key);
        let plain = d.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(project_key(&plain), plain, "outside a repo the folder is its own project");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_symlinked_path_to_the_same_repo_is_the_same_project() {
        let d = tmp("ln");
        std::fs::create_dir_all(d.join("repo/.git")).unwrap();
        std::os::unix::fs::symlink(d.join("repo"), d.join("alias")).unwrap();
        assert_eq!(project_key(&d.join("alias")), project_key(&d.join("repo")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
