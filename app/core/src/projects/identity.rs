//! What a project folder is, canonically (ADR-15 section 1.2): the realpath, the device and inode (APFS is
//! case-insensitive, so a path compare is not enough), and the git common dir, which a worktree shares with its main
//! repository. Nothing here runs `git` or any repo script: the common dir is read from the `.git` entry itself, so a
//! hostile repository cannot execute anything by being added.

use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::contract::RootKey;

/// A `.git` file or a `commondir` file is one short line; anything longer is not one.
const GIT_FILE_CAP: usize = 4096;

#[derive(Debug)]
pub enum IdentityError {
    NotFound,
    NotADirectory,
    /// The folder is one a project must never be (the filesystem root, the home folder, the Claude config folder, the app's
    /// own data, or something that contains them). The reason is a short plain phrase for the UI.
    Refused(&'static str),
    Io(io::Error),
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "folder not found"),
            Self::NotADirectory => write!(f, "not a folder"),
            Self::Refused(why) => write!(f, "{why}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for IdentityError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::NotFound { Self::NotFound } else { Self::Io(e) }
    }
}

/// The folders that are never a project, whatever the owner picks.
#[derive(Debug, Clone)]
pub struct Guard {
    pub home: PathBuf,
    pub data: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub root: PathBuf,
    pub key: RootKey,
}

/// The canonical identity of the folder `picked`.
///
/// The project root is the folder picked, except for a git worktree: a worktree belongs to its main repository's
/// project, so its root is the main repository's folder (when the common dir is a `.git` directory beside it).
pub fn resolve(picked: &Path, guard: &Guard) -> Result<Resolved, IdentityError> {
    let canon = fs::canonicalize(picked)?;
    if !fs::metadata(&canon)?.is_dir() {
        return Err(IdentityError::NotADirectory);
    }
    let common = git_common_dir(&canon);
    // A worktree's project root is the main repo's folder, if that folder still exists.
    let root = match &common {
        Some(c) if c.file_name().is_some_and(|n| n == ".git") && c != &canon.join(".git") => {
            c.parent().filter(|p| p.is_dir()).map_or(canon.clone(), Path::to_path_buf)
        }
        _ => canon,
    };
    refuse(&root, guard)?;
    let meta = fs::metadata(&root)?;
    Ok(Resolved { key: RootKey { dev: meta.dev(), ino: meta.ino(), git_common_dir: common.map(|c| c.to_string_lossy().into_owned()) }, root })
}

fn refuse(root: &Path, g: &Guard) -> Result<(), IdentityError> {
    let claude = g.home.join(".claude");
    if root.parent().is_none() {
        return Err(IdentityError::Refused("the filesystem root cannot be a project"));
    }
    if root.starts_with(&g.home) && g.home.starts_with(root) {
        return Err(IdentityError::Refused("the home folder cannot be a project; pick a folder inside it"));
    }
    if g.home.starts_with(root) {
        return Err(IdentityError::Refused("a folder that contains your home folder cannot be a project"));
    }
    if root.starts_with(&claude) || claude.starts_with(root) {
        return Err(IdentityError::Refused("the Claude config folder cannot be a project"));
    }
    if root.starts_with(&g.data) || g.data.starts_with(root) {
        return Err(IdentityError::Refused("Vajra's own data folder cannot be a project"));
    }
    Ok(())
}

/// The git common dir of the repository whose root is `dir`, read from `dir/.git` without running anything:
/// a `.git` directory is its own common dir; a `.git` file (`gitdir: <path>`) names a worktree's git dir, whose
/// `commondir` file points at the shared one. Only a directory shaped like a git dir (`HEAD` and `objects`) counts.
pub fn git_common_dir(dir: &Path) -> Option<PathBuf> {
    let dotgit = dir.join(".git");
    let meta = fs::symlink_metadata(&dotgit).ok()?;
    let gitdir = if meta.is_dir() {
        fs::canonicalize(&dotgit).ok()?
    } else if meta.is_file() {
        let line = read_small(&dotgit)?;
        let target = line.trim().strip_prefix("gitdir:")?.trim();
        fs::canonicalize(dir.join(target)).ok()?
    } else if meta.file_type().is_symlink() {
        fs::canonicalize(&dotgit).ok().filter(|p| p.is_dir())?
    } else {
        return None;
    };
    let common = match read_small(&gitdir.join("commondir")) {
        Some(rel) => fs::canonicalize(gitdir.join(rel.trim())).ok()?,
        None => gitdir,
    };
    looks_like_git_dir(&common).then_some(common)
}

fn looks_like_git_dir(p: &Path) -> bool {
    p.join("HEAD").is_file() && p.join("objects").is_dir()
}

/// A small text file, read without following a final symlink and capped.
fn read_small(path: &Path) -> Option<String> {
    use std::os::unix::fs::OpenOptionsExt;
    let f = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).ok()?;
    if !f.metadata().ok()?.is_file() {
        return None;
    }
    let mut buf = Vec::new();
    f.take(GIT_FILE_CAP as u64 + 1).read_to_end(&mut buf).ok()?;
    if buf.len() > GIT_FILE_CAP {
        return None;
    }
    String::from_utf8(buf).ok()
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
