//! Find git repositories already on this Mac, so a new user can add them as projects in one step. Generic: it looks in
//! the common code folders under the home folder and in any folder the person names, and knows nothing about any other
//! tool. Read only and bounded: it never runs git or a repo script (identity reads `.git` itself), never follows a
//! symlink, never enters hidden or build folders, and writes nothing. A linked worktree is counted under its main
//! repository, so each repository is offered once.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::identity::{self, Guard};

/// Folders under home where code usually lives. Never Documents, Desktop or Downloads: macOS protects them (TCC) and an
/// automatic scan there would show a privacy prompt before the person has clicked anything. Those are scanned only when
/// the person names them (`extra`).
pub const COMMON_FOLDERS: &[&str] = &["Development", "Developer", "Projects", "Code", "code", "src", "repos", "dev", "work", "workspace", "GitHub"];

const MAX_DEPTH: usize = 3;
const MAX_VISITED: usize = 20_000;
pub const MAX_FOUND: usize = 200;
const SKIP: &[&str] = &["node_modules", "target", "build", "dist", "vendor", "Library"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The repository's own folder (a worktree is folded into this).
    pub root: PathBuf,
    pub name: String,
    /// Linked worktrees found for it, not counting the main folder.
    pub worktrees: usize,
}

#[derive(Debug, Default)]
pub struct Discovery {
    pub found: Vec<Found>,
    /// The scan stopped early (too many folders or repositories); more may exist.
    pub truncated: bool,
}

/// Repositories under the common folders and `extra` that `owned` does not already hold. `owned` answers "is this
/// root (or a worktree of it) already a project?".
pub fn discover(guard: &Guard, extra: &[PathBuf], owned: &dyn Fn(&Path) -> bool) -> Discovery {
    // An automatic root must itself be a real folder: `~/Code -> ~/Documents` would otherwise walk a protected folder
    // with no user action. Folders the person names (`extra`) may be symlinks; they chose them.
    let mut roots: Vec<PathBuf> = COMMON_FOLDERS
        .iter()
        .map(|f| guard.home.join(f))
        .filter(|p| p.symlink_metadata().is_ok_and(|m| m.is_dir()))
        .collect();
    roots.extend(extra.iter().cloned());
    let mut out = Discovery::default();
    let mut seen_dirs = std::collections::HashSet::new();
    let mut repos: BTreeMap<PathBuf, usize> = BTreeMap::new();
    let mut visited = 0usize;
    for r in roots {
        let Ok(canon) = fs::canonicalize(&r) else { continue };
        if !seen_dirs.insert(canon.clone()) {
            continue;
        }
        walk(&canon, 0, guard, &mut repos, &mut visited, &mut out.truncated);
    }
    for (root, worktrees) in repos {
        if owned(&root) {
            continue;
        }
        if out.found.len() >= MAX_FOUND {
            out.truncated = true;
            break;
        }
        let name = root.file_name().map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned());
        out.found.push(Found { root, name, worktrees });
    }
    out
}

fn walk(dir: &Path, depth: usize, guard: &Guard, repos: &mut BTreeMap<PathBuf, usize>, visited: &mut usize, truncated: &mut bool) {
    *visited += 1;
    if *visited > MAX_VISITED {
        *truncated = true;
        return;
    }
    if dir.join(".git").symlink_metadata().is_ok() {
        // A repository or a linked worktree. `resolve` folds a worktree into its main repository and refuses the
        // home folder, the Claude folder and the app's data.
        if let Ok(r) = identity::resolve(dir, guard) {
            let is_worktree = r.root != dir;
            let n = repos.entry(r.root).or_insert(0);
            if is_worktree {
                *n += 1;
            }
        }
        return; // never look inside a repository
    }
    if depth >= MAX_DEPTH {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut kids: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) // a symlink is not a dir here: not followed
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            !n.starts_with('.') && !SKIP.contains(&n.as_ref())
        })
        .map(|e| e.path())
        .collect();
    kids.sort();
    for k in kids {
        if *truncated {
            return;
        }
        walk(&k, depth + 1, guard, repos, visited, truncated);
    }
}

#[cfg(test)]
#[path = "discover_tests.rs"]
mod tests;
