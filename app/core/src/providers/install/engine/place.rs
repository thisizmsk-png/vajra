//! Where the engine goes and whose copy is there (D7 design E3, E4; bhishma C2, C5, C6, C10).
//!
//! `~/.claude/skills/vajra` is replaced only when it is the app's own copy: no `.git` of any kind, a marker the app
//! wrote, and an app record in app data that agrees with the marker and the folder's `(dev, ino)`. Anything else is the
//! user's and is never touched. Staging, the previous copy and backups live in `~/.claude/.vajra-install` (0700),
//! never under `skills/`, where Claude Code would load a second `vajra` skill. Every write is through held directory
//! fds (`fsat`); a journal in app data, written before the first rename, is the only thing recovery acts on.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::bundle::Engine;
use super::fsat::{self, Dir, Kind, Rename};
use super::pack::sha256_hex;

/// The skill folder's name under `~/.claude/skills`.
pub const SKILL: &str = "vajra";
/// The app's work area under `~/.claude` (0700, same filesystem as `skills/`).
pub const WORK: &str = ".vajra-install";
/// The marker the app writes at the root of a copy it installed. Not in `manifest.json`, which is fine: the
/// integrity check only verifies the files it lists.
pub const MARKER: &str = ".vajra-desktop.json";
/// The app's own record of what it installed, in app data.
pub const RECORD: &str = "engine_install.json";
/// The journal of an install in progress, in app data.
pub const JOURNAL: &str = "engine_journal.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub version: String,
    pub tgz_sha256: String,
}

/// What the app installed: the bundle and the root folder's identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub version: String,
    pub tgz_sha256: String,
    pub dev: u64,
    pub ino: u64,
}

/// What is at `~/.claude/skills/vajra` now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// Nothing: a fresh install.
    Absent,
    /// The app's own copy (marker and record agree), with its version and root identity.
    Ours { version: String, dev: u64, ino: u64 },
    /// A git checkout, a link, a copy without the app's marker and record, or not a folder: never touched.
    UserCopy,
}

/// A journal: an install that got past its first rename. Only directories it names, with these identities, are ever
/// moved back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub phase: Phase,
    /// The previous copy's name in the work area and its identity (an update or Repair), if any.
    pub previous: Option<(String, u64, u64)>,
    /// The staged copy's name in the work area and its identity.
    pub staged: (String, u64, u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Staged, nothing swapped yet.
    Staged,
    /// The new copy is in place; the previous one (if any) is in the work area.
    Swapped,
}

/// The held directories an install works in: `~/.claude`, `skills/` and the work area.
pub struct Place {
    pub claude: Dir,
    pub skills: Dir,
    pub work: Dir,
    pub data: PathBuf,
}

/// Why the place can't be used. Each is a manual reason on the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsafe {
    /// `~/.claude`, `skills/` or the work area isn't a private folder of the user's (or is a link).
    NotPrivate,
    /// There isn't three times the engine's unpacked size free.
    NoSpace,
    /// Something is at the target that wasn't there when the plan was made.
    TargetChanged,
    /// A write failed.
    Io,
}

impl Unsafe {
    pub fn code(self) -> &'static str {
        match self {
            Unsafe::NotPrivate => "not_writable",
            Unsafe::NoSpace => "no_space",
            Unsafe::TargetChanged => "target_changed",
            Unsafe::Io => "io",
        }
    }
}

impl Place {
    /// Open (creating when absent) `home/.claude` (0700), its `skills/` (0755) and the work area (0700), each a real
    /// folder of `uid`'s, not group- or world-writable. `home` itself must be the user's and private the same way.
    pub fn open(home: &Path, data: &Path, uid: u32) -> Result<Place, Unsafe> {
        let home = Dir::open_path(home, uid).map_err(|_| Unsafe::NotPrivate)?;
        let claude = private(&home, ".claude", 0o700, uid)?;
        let skills = private(&claude, "skills", 0o755, uid)?;
        let work = private(&claude, WORK, 0o700, uid)?;
        if work.stat().map_err(|_| Unsafe::Io)?.mode & 0o077 != 0 {
            return Err(Unsafe::NotPrivate);
        }
        Ok(Place { claude, skills, work, data: data.to_path_buf() })
    }

    /// Whose copy is at the target: never touched unless it is provably the app's own.
    pub fn found(&self) -> Found {
        let Ok(Some(s)) = self.skills.stat_at(SKILL) else { return Found::Absent };
        if s.kind != Kind::Dir {
            return Found::UserCopy;
        }
        let Ok(root) = self.skills.open_dir(SKILL) else { return Found::UserCopy };
        // A `.git` of any kind (a worktree has a `.git` file) means a checkout.
        if !matches!(root.stat_at(".git"), Ok(None)) {
            return Found::UserCopy;
        }
        let marker: Option<Marker> = root.read(MARKER, 4096).ok().and_then(|b| serde_json::from_slice(&b).ok());
        match (marker, self.record()) {
            (Some(m), Some(r)) if m.version == r.version && m.tgz_sha256 == r.tgz_sha256 && (r.dev, r.ino) == (s.dev, s.ino) => Found::Ours { version: r.version, dev: s.dev, ino: s.ino },
            _ => Found::UserCopy,
        }
    }

    pub fn record(&self) -> Option<Record> {
        std::fs::read(self.data.join(RECORD)).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    pub fn journal(&self) -> Option<Journal> {
        std::fs::read(self.data.join(JOURNAL)).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    fn write_data(&self, name: &str, v: &impl Serialize) -> Result<(), Unsafe> {
        let tmp = self.data.join(format!("{name}.tmp"));
        let _ = std::fs::remove_file(&tmp);
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&tmp).map_err(|_| Unsafe::Io)?;
        std::io::Write::write_all(&mut f, &serde_json::to_vec(v).map_err(|_| Unsafe::Io)?).map_err(|_| Unsafe::Io)?;
        f.sync_all().map_err(|_| Unsafe::Io)?;
        std::fs::rename(&tmp, self.data.join(name)).map_err(|_| Unsafe::Io)
    }

    pub fn clear_journal(&self) {
        let _ = std::fs::remove_file(self.data.join(JOURNAL));
    }

    /// At least three times `unpacked` free (staging, the previous copy and a backup), C10.
    pub fn has_space(&self, unpacked: u64) -> bool {
        self.claude.free_bytes().is_ok_and(|b| b >= unpacked.saturating_mul(3))
    }

    /// Write every file of `engine` into a new staging folder in the work area, from the verified bytes, plus the
    /// marker. Directories are 0755 and files 0644 or 0755. On any failure the staging folder is removed.
    pub fn stage(&self, engine: &Engine, tgz_sha256: &str) -> Result<String, Unsafe> {
        let name = format!("staging-{}", random_hex());
        let root = self.work.make_dir(&name, 0o700).map_err(|_| Unsafe::Io)?;
        let r = (|| {
            for f in &engine.files {
                let mut dir_names: Vec<&str> = f.name.split('/').collect();
                let file = dir_names.pop().ok_or(Unsafe::Io)?;
                let mut cur = self.work.open_dir(&name).map_err(|_| Unsafe::Io)?;
                for d in dir_names {
                    cur = cur.open_or_make_dir(d, 0o755).map_err(|_| Unsafe::Io)?;
                }
                cur.write_new(file, &f.bytes, f.mode).map_err(|e| if e.raw_os_error() == Some(libc::ENOSPC) { Unsafe::NoSpace } else { Unsafe::Io })?;
            }
            let marker = serde_json::to_vec(&Marker { version: engine.version.clone(), tgz_sha256: tgz_sha256.to_string() }).map_err(|_| Unsafe::Io)?;
            root.write_new(MARKER, &marker, 0o644).map_err(|_| Unsafe::Io)?;
            // The root last: 0755, like any skill folder.
            // SAFETY: a valid fd.
            if unsafe { libc::fchmod(root.raw(), 0o755) } != 0 {
                return Err(Unsafe::Io);
            }
            self.work.sync().map_err(|_| Unsafe::Io)
        })();
        if r.is_err() {
            let _ = self.work.remove_tree(&name);
        }
        r.map(|_| name)
    }

    fn identity(&self, dir: &Dir, name: &str) -> Result<(u64, u64), Unsafe> {
        let s = dir.stat_at(name).map_err(|_| Unsafe::Io)?.ok_or(Unsafe::TargetChanged)?;
        Ok((s.dev, s.ino))
    }

    /// Put the staged copy in place. Fresh (`found` Absent): an exclusive rename, so anything that appeared at the
    /// target since the plan fails it (`target_changed`). Ours: an atomic swap, so `vajra` exists at every moment, then
    /// the old copy moves to `previous-<rand>` in the work area. The journal is written before the first rename.
    pub fn swap_in(&self, staged: &str, found: &Found) -> Result<Journal, Unsafe> {
        let (sd, si) = self.identity(&self.work, staged)?;
        match found {
            Found::Absent => {
                let j = Journal { phase: Phase::Staged, previous: None, staged: (staged.to_string(), sd, si) };
                self.write_data(JOURNAL, &j)?;
                self.work.rename(staged, &self.skills, SKILL, Rename::Exclusive).map_err(|e| if e.raw_os_error() == Some(libc::EEXIST) { Unsafe::TargetChanged } else { Unsafe::Io })?;
                let j = Journal { phase: Phase::Swapped, ..j };
                self.write_data(JOURNAL, &j)?;
                Ok(j)
            }
            Found::Ours { dev, ino, .. } => {
                // The target must still be the copy the plan saw.
                if self.identity(&self.skills, SKILL)? != (*dev, *ino) {
                    return Err(Unsafe::TargetChanged);
                }
                let prev = format!("previous-{}", random_hex());
                let j = Journal { phase: Phase::Staged, previous: Some((prev.clone(), *dev, *ino)), staged: (staged.to_string(), sd, si) };
                self.write_data(JOURNAL, &j)?;
                // After the swap the old copy has the staging name; then it gets its own name.
                self.work.rename(staged, &self.skills, SKILL, Rename::Swap).map_err(|_| Unsafe::Io)?;
                self.work.rename(staged, &self.work, &prev, Rename::Exclusive).map_err(|_| Unsafe::Io)?;
                let j = Journal { phase: Phase::Swapped, ..j };
                self.write_data(JOURNAL, &j)?;
                Ok(j)
            }
            Found::UserCopy => Err(Unsafe::TargetChanged),
        }
    }

    /// Undo a swap named by `j`: the new copy goes back to the work area and is removed, and the previous copy (if the
    /// journal names one with that identity) comes back as `vajra`. Only folders the journal names are touched.
    pub fn roll_back(&self, j: &Journal) -> Result<(), Unsafe> {
        let (staged, sd, si) = &j.staged;
        let at_target = self.skills.stat_at(SKILL).map_err(|_| Unsafe::Io)?;
        match &j.previous {
            Some((prev, pd, pi)) => {
                let prev_ok = self.work.stat_at(prev).map_err(|_| Unsafe::Io)?.is_some_and(|s| (s.dev, s.ino) == (*pd, *pi));
                if !prev_ok {
                    return Err(Unsafe::TargetChanged);
                }
                if at_target.is_some_and(|s| (s.dev, s.ino) == (*sd, *si)) {
                    self.work.rename(prev, &self.skills, SKILL, Rename::Swap).map_err(|_| Unsafe::Io)?;
                    self.work.remove_tree(prev).map_err(|_| Unsafe::Io)?;
                } else if at_target.is_none() {
                    self.work.rename(prev, &self.skills, SKILL, Rename::Exclusive).map_err(|_| Unsafe::Io)?;
                } else {
                    return Err(Unsafe::TargetChanged);
                }
            }
            None => {
                if at_target.is_some_and(|s| (s.dev, s.ino) == (*sd, *si)) {
                    let gone = format!("rolled-back-{}", random_hex());
                    self.skills.rename(SKILL, &self.work, &gone, Rename::Exclusive).map_err(|_| Unsafe::Io)?;
                    self.work.remove_tree(&gone).map_err(|_| Unsafe::Io)?;
                } else if at_target.is_some() {
                    return Err(Unsafe::TargetChanged);
                }
            }
        }
        if self.work.stat_at(staged).map_err(|_| Unsafe::Io)?.is_some_and(|s| (s.dev, s.ino) == (*sd, *si)) {
            let _ = self.work.remove_tree(staged);
        }
        self.clear_journal();
        Ok(())
    }

    /// After a verified install: record it, and deal with the previous copy. It is deleted only if it is byte for
    /// byte its own manifest (plus the marker); otherwise it is kept as `backup-<UTC>` and its name returned (C5).
    pub fn commit(&self, j: &Journal, version: &str, tgz_sha256: &str) -> Result<Option<String>, Unsafe> {
        let (dev, ino) = self.identity(&self.skills, SKILL)?;
        self.write_data(RECORD, &Record { version: version.to_string(), tgz_sha256: tgz_sha256.to_string(), dev, ino })?;
        let mut kept = None;
        if let Some((prev, pd, pi)) = &j.previous {
            if self.work.stat_at(prev).map_err(|_| Unsafe::Io)?.is_some_and(|s| (s.dev, s.ino) == (*pd, *pi)) {
                let d = self.work.open_dir(prev).map_err(|_| Unsafe::Io)?;
                if unmodified(&d) {
                    self.work.remove_tree(prev).map_err(|_| Unsafe::Io)?;
                } else {
                    let name = format!("backup-{}", utc_stamp());
                    self.work.rename(prev, &self.work, &name, Rename::Exclusive).map_err(|_| Unsafe::Io)?;
                    kept = Some(name);
                }
            }
        }
        self.clear_journal();
        Ok(kept)
    }

    /// Recover from a crash, under consent: only what the journal names, with those identities. A swapped install
    /// is rolled back (it was never verified); a staged one just loses its staging folder. Folders in the work area
    /// that the journal doesn't name are left alone.
    pub fn recover(&self) -> Result<bool, Unsafe> {
        let Some(j) = self.journal() else { return Ok(false) };
        self.roll_back(&j)?;
        Ok(true)
    }

    /// Folders in the work area that no journal names (left alone, reported).
    pub fn strays(&self) -> Vec<String> {
        let named: Vec<String> = self.journal().map(|j| j.previous.into_iter().map(|p| p.0).chain([j.staged.0]).collect()).unwrap_or_default();
        self.work.list().unwrap_or_default().into_iter().filter(|n| !n.starts_with("backup-") && n != "lock" && !named.contains(n)).collect()
    }
}

/// A folder of `uid`'s at `name` under `parent`, created with `mode` when absent; never a link.
fn private(parent: &Dir, name: &str, mode: u32, uid: u32) -> Result<Dir, Unsafe> {
    match parent.stat_at(name).map_err(|_| Unsafe::Io)? {
        None => parent.make_dir(name, mode).map_err(|_| Unsafe::Io),
        Some(s) if s.private_dir(uid) => parent.open_dir(name).map_err(|_| Unsafe::NotPrivate),
        Some(_) => Err(Unsafe::NotPrivate),
    }
}

/// Every file `manifest.json` lists is present with its hash, every file present is listed (or is the marker or the
/// manifest), and nothing is a link.
pub fn unmodified(root: &Dir) -> bool {
    let Ok(m) = root.read(super::pack::MANIFEST, 16 << 20) else { return false };
    let Ok(listed) = super::pack::manifest_files(&m) else { return false };
    let mut seen = 0usize;
    fn walk(d: &Dir, prefix: &str, listed: &std::collections::BTreeMap<String, String>, seen: &mut usize) -> bool {
        let Ok(names) = d.list() else { return false };
        for n in names {
            let path = if prefix.is_empty() { n.clone() } else { format!("{prefix}/{n}") };
            match d.stat_at(&n) {
                Ok(Some(s)) if s.kind == Kind::Dir => {
                    let Ok(sub) = d.open_dir(&n) else { return false };
                    if !walk(&sub, &path, listed, seen) {
                        return false;
                    }
                }
                Ok(Some(s)) if s.kind == Kind::File => {
                    if path == MARKER || path == super::pack::MANIFEST {
                        continue;
                    }
                    let Some(want) = listed.get(&path) else { return false };
                    let Ok(bytes) = d.read(&n, 64 << 20) else { return false };
                    if sha256_hex(&bytes) != *want {
                        return false;
                    }
                    *seen += 1;
                }
                _ => return false,
            }
        }
        true
    }
    walk(root, "", &listed, &mut seen) && seen == listed.len()
}

fn random_hex() -> String {
    let mut b = [0u8; 8];
    // SAFETY: arc4random_buf fills the buffer; it never fails.
    unsafe { libc::arc4random_buf(b.as_mut_ptr().cast(), b.len()) };
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{secs}-{}", random_hex())
}

/// The effective uid (re-exported for callers).
pub fn uid() -> u32 {
    fsat::euid()
}

#[cfg(test)]
#[path = "place_tests.rs"]
mod tests;
