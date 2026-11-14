//! needs_admin (ADR-17 rule 5, bhishma condition 2): before anything runs, every folder the install writes and
//! everything already in the package's folder must belong to the user and be writable. Otherwise the sheet shows the
//! command and why, and nothing is spawned. Vajra never asks for a password.

use std::path::{Path, PathBuf};

/// What an install writes.
#[derive(Debug, Clone, Default)]
pub struct Targets {
    /// Folders written into (prefix, lib/node_modules, bin, Cellar...): owned by the user and writable.
    pub dirs: Vec<PathBuf>,
    /// Existing trees replaced (the package's folder): every entry owned by the user, walked without following links.
    pub trees: Vec<PathBuf>,
    /// Existing files replaced (the bin link): owned by the user.
    pub files: Vec<PathBuf>,
}

/// The first thing that needs administrator rights, and why: `root_owned`, `not_owned`, `not_writable` or
/// `too_many_files` (a tree too large to check is treated as needing admin).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    pub path: PathBuf,
    pub reason: &'static str,
}

fn owner(path: &Path, m: &std::fs::Metadata, uid: u32) -> Option<Blocker> {
    use std::os::unix::fs::MetadataExt;
    match m.uid() {
        u if u == uid => None,
        0 => Some(Blocker { path: path.to_path_buf(), reason: "root_owned" }),
        _ => Some(Blocker { path: path.to_path_buf(), reason: "not_owned" }),
    }
}

fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: a valid NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// Check `t` as user `uid`, walking at most `limit` entries in all trees together.
pub fn check(t: &Targets, uid: u32, limit: usize) -> Option<Blocker> {
    for d in &t.dirs {
        let Ok(m) = std::fs::symlink_metadata(d) else { continue };
        // A link to nothing can't be checked: treat it as a folder the user can't write.
        let m = if m.file_type().is_symlink() {
            match std::fs::metadata(d) {
                Ok(m) => m,
                Err(_) => return Some(Blocker { path: d.clone(), reason: "not_writable" }),
            }
        } else {
            m
        };
        if let Some(b) = owner(d, &m, uid) {
            return Some(b);
        }
        if !writable(d) {
            return Some(Blocker { path: d.clone(), reason: "not_writable" });
        }
    }
    for f in &t.files {
        let Ok(m) = std::fs::symlink_metadata(f) else { continue };
        if let Some(b) = owner(f, &m, uid) {
            return Some(b);
        }
    }
    let mut seen = 0usize;
    for root in &t.trees {
        let mut stack = vec![root.clone()];
        while let Some(p) = stack.pop() {
            let Ok(m) = std::fs::symlink_metadata(&p) else { continue };
            seen += 1;
            if seen > limit {
                return Some(Blocker { path: root.clone(), reason: "too_many_files" });
            }
            if let Some(b) = owner(&p, &m, uid) {
                return Some(b);
            }
            if m.is_dir() {
                if !writable(&p) {
                    return Some(Blocker { path: p, reason: "not_writable" });
                }
                let Ok(rd) = std::fs::read_dir(&p) else { return Some(Blocker { path: p, reason: "not_writable" }) };
                stack.extend(rd.filter_map(|e| e.ok().map(|e| e.path())));
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "admin_tests.rs"]
mod tests;
