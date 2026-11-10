//! The project store (ADR-15 D1: JSON files). `<data>/projects/<id>/project.json` is one record, rewritten atomically at
//! 0600; `<data>/projects/<id>/sessions.jsonl` is append-only (the newest line per session id wins). Nothing here
//! moves, rewrites or deletes campaigns, the run index, the ledger or the control dirs: a project record only refers
//! to them. Every read goes through the contract's validation, and a record whose id does not match its folder name is
//! skipped, so one file cannot impersonate another project.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use super::contract::{self, ContractError, Project, Session, SessionLog};
use super::identity::{self, Guard, IdentityError};
use super::pinned::Pinned;
use crate::control::time;

/// Only the tail of a sessions file is read.
const SESSIONS_READ_CAP: u64 = 1024 * 1024;
/// No more projects than this are kept (a store with more is not one a person made).
const MAX_PROJECTS: usize = 500;
/// No more directory entries than this are examined in one scan.
const MAX_ENTRIES: usize = 2000;

#[derive(Debug)]
pub enum StoreError {
    NotFound,
    /// The record changed since the caller read it (`expect_rev` did not match).
    Conflict { current_rev: u64 },
    /// The folder overlaps another project's folder; pass `confirm_nested` to add it anyway.
    Nested { with_id: String, with_name: String },
    /// The store has too many entries to scan completely, or no room for another project; nothing was written.
    Full,
    Identity(IdentityError),
    Invalid(ContractError),
    Io(io::Error),
    /// `<data>/projects` is not a folder the store will use (#77 follow-up): every call refuses, nothing is written.
    Unavailable(Unavailable),
}

/// Why the projects folder is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    /// A symbolic link: the app's records could be pointed somewhere else, so it is never followed.
    Symlink,
    /// Something other than a folder is in its place.
    NotAFolder,
    /// It could not be created or opened (permissions, a full disk).
    CannotOpen,
}

/// The projects folder the store refused at open, and why. The app still opens; the person is told what to do.
#[derive(Clone, Debug)]
pub struct Unavailable {
    pub path: PathBuf,
    pub reason: UnavailableReason,
}

impl Unavailable {
    /// What is wrong, in words (no path: that is its own field).
    pub fn what(&self) -> String {
        let why = match self.reason {
            UnavailableReason::Symlink => "is a symbolic link, which the app never follows (it could point the app's records somewhere else)",
            UnavailableReason::NotAFolder => "is not a folder",
            UnavailableReason::CannotOpen => "could not be opened",
        };
        format!("The app's projects folder {why}, so projects are unavailable until it is fixed.")
    }

    /// What to do, naming the path.
    pub fn fix(&self) -> String {
        let p = self.path.display();
        match self.reason {
            UnavailableReason::Symlink | UnavailableReason::NotAFolder => format!("Move {p} aside (for example: mv \"{p}\" \"{p}.old\"), then restart the app. A new projects folder is made at that path."),
            UnavailableReason::CannotOpen => format!("Check that {p} is a folder you own and can write, then restart the app."),
        }
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "project not found"),
            Self::Conflict { current_rev } => write!(f, "the project changed (now at revision {current_rev}); reload and try again"),
            Self::Nested { with_name, .. } => write!(f, "this folder overlaps the project {with_name}"),
            Self::Full => write!(f, "there are too many projects (or stray entries) to add another"),
            Self::Identity(e) => write!(f, "{e}"),
            Self::Invalid(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
            Self::Unavailable(u) => write!(f, "{}", u.what()),
        }
    }
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<IdentityError> for StoreError {
    fn from(e: IdentityError) -> Self {
        Self::Identity(e)
    }
}

impl From<ContractError> for StoreError {
    fn from(e: ContractError) -> Self {
        Self::Invalid(e)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// What `add` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    Created(Project),
    /// The folder (or its main repository) already was a project; nothing changed.
    Existing(Project),
    /// The folder was an archived project; it is active again.
    Restored(Project),
}

impl Added {
    pub fn project(&self) -> &Project {
        match self {
            Self::Created(p) | Self::Existing(p) | Self::Restored(p) => p,
        }
    }

    pub fn outcome(&self) -> &'static str {
        match self {
            Self::Created(_) => "created",
            Self::Existing(_) => "existing",
            Self::Restored(_) => "restored",
        }
    }
}

#[derive(Debug, Default)]
pub struct Listing {
    pub projects: Vec<Project>,
    /// Entries that were not a valid project record (damaged, foreign, or from another major version).
    pub skipped: usize,
    /// The scan stopped at a limit: this listing is NOT the whole store.
    pub truncated: bool,
}

type Clock = Box<dyn Fn() -> SystemTime + Send + Sync>;

pub struct Store {
    /// The projects folder, opened once; every project folder is opened relative to it, so replacing `<data>/projects`
    /// afterwards (a rename plus a symlink) cannot redirect anything. `Err` when the folder was refused at open.
    root: std::result::Result<Pinned, Unavailable>,
    limits: (usize, usize),
    guard: Guard,
    /// Serialises every read-modify-write of a record within the process.
    lock: Mutex<()>,
    /// Held by a whole migration confirm (see `migrate::confirm`), so two confirms never interleave.
    migrating: Mutex<()>,
    now: Clock,
}

/// Open (creating at 0700) the projects folder, or say why it is refused. A symbolic link or a non-folder is refused
/// before anything is created or chmod-ed, and the open itself never follows a link (O_NOFOLLOW), so a link planted
/// between the check and the open is refused too.
fn open_root(dir: &Path) -> std::result::Result<Pinned, Unavailable> {
    let refuse = |reason| Unavailable { path: dir.to_path_buf(), reason };
    match fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_symlink() => return Err(refuse(UnavailableReason::Symlink)),
        Ok(m) if !m.is_dir() => return Err(refuse(UnavailableReason::NotAFolder)),
        _ => {}
    }
    pin_root(dir)
}

/// Create (0700) and open the projects folder with no path operation that follows a link: `mkdir` never follows one,
/// the open is O_DIRECTORY | O_NOFOLLOW, and the mode is set through the opened descriptor (`fchmod`). A symlink swapped in
/// after the check in `open_root` is refused, and its target is never created in or chmod-ed (#106 review). The parent
/// (the app's data folder) is only created if missing, never chmod-ed here.
fn pin_root(dir: &Path) -> std::result::Result<Pinned, Unavailable> {
    let refuse = |reason| Unavailable { path: dir.to_path_buf(), reason };
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent).map_err(|_| refuse(UnavailableReason::CannotOpen))?;
    }
    Pinned::open(dir, true).map_err(|e| {
        refuse(match e.raw_os_error() {
            Some(libc::ELOOP) => UnavailableReason::Symlink,
            // macOS answers a symlink under O_DIRECTORY | O_NOFOLLOW with ENOTDIR: say which it is (only the message).
            Some(libc::ENOTDIR) if fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) => UnavailableReason::Symlink,
            Some(libc::ENOTDIR) => UnavailableReason::NotAFolder,
            _ => UnavailableReason::CannotOpen,
        })
    })
}

impl Store {
    /// Open (creating at 0700) `<data>/projects`.
    pub fn open(data: &Path, home: &Path) -> io::Result<Self> {
        Self::with_clock(data, home, Box::new(SystemTime::now))
    }

    /// Smaller limits, for tests: (projects kept, entries examined).
    #[cfg(test)]
    fn with_limits(mut self, projects: usize, entries: usize) -> Self {
        self.limits = (projects, entries);
        self
    }

    pub fn with_clock(data: &Path, home: &Path, now: Clock) -> io::Result<Self> {
        let root = open_root(&data.join("projects"));
        // The guard compares against canonical paths; a data dir that is a symlink is compared by its target.
        let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        Ok(Self { root, limits: (MAX_PROJECTS, MAX_ENTRIES), guard: Guard { home: canon(home), data: canon(data) }, lock: Mutex::new(()), migrating: Mutex::new(()), now })
    }

    /// The projects folder refused at open, if it was: the app shows this and every call refuses.
    pub fn unavailable(&self) -> Option<&Unavailable> {
        self.root.as_ref().err()
    }

    fn root(&self) -> Result<&Pinned> {
        self.root.as_ref().map_err(|u| StoreError::Unavailable(u.clone()))
    }

    /// The folders that can never be a project.
    pub fn guard(&self) -> &Guard {
        &self.guard
    }

    /// Serialise one whole migration: taken before the done-check and held until the marker is written.
    pub fn migration_lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.migrating.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Has the owner confirmed the migration? The marker is read through the pinned projects folder; anything but a
    /// clean "not there" counts as done, so a surprise never lets a second migration run.
    pub fn migration_done(&self) -> bool {
        let Ok(root) = self.root.as_ref() else { return true };
        match root.read(".migrated-v1", 16) {
            Ok(_) => true,
            Err(e) => e.kind() != io::ErrorKind::NotFound,
        }
    }

    pub fn mark_migrated(&self) -> io::Result<()> {
        match &self.root {
            Ok(root) => root.write_atomic(".migrated-v1", b"1\n"),
            Err(u) => Err(io::Error::other(u.what())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn stamp(&self) -> String {
        time::iso_secs((self.now)())
    }

    // ------------------------------------------------------------------------------------------------- reads

    /// Every valid, active project (and archived ones when asked), oldest first.
    pub fn list(&self, include_archived: bool) -> Result<Listing> {
        let _g = self.lock();
        let mut all = self.read_all()?;
        if !include_archived {
            all.projects.retain(|p| p.archived.is_none());
        }
        Ok(all)
    }

    pub fn get(&self, id: &str) -> Result<Project> {
        let _g = self.lock();
        self.read_one(id)
    }

    /// The project a folder belongs to, archived ones included (FG-02c): the folder itself or the nearest folder above
    /// it that is a project's recorded root, matched by device and inode (so a symlinked or differently cased path is
    /// the same folder) or by git common dir (so a linked worktree is its repository's project). The deepest match
    /// wins, which is the inner project of two nested ones. `None` when no project holds the folder. Nothing is run:
    /// the common dir is read from `.git` with its back-link checked (`identity::git_common_dir`).
    pub fn find_by_path(&self, path: &Path) -> Result<Option<Project>> {
        let Ok(canon) = fs::canonicalize(path) else { return Ok(None) };
        let listing = self.list(true)?;
        if listing.truncated {
            // A partial inventory could miss the right project and match an outer one: refuse instead.
            return Err(StoreError::Full);
        }
        for dir in canon.ancestors() {
            let Ok(meta) = fs::metadata(dir) else { continue };
            let common = identity::git_common_dir(dir).map(|c| c.to_string_lossy().into_owned());
            let hit = listing.projects.iter().find(|p| {
                (p.root_key.dev, p.root_key.ino) == (meta.dev(), meta.ino()) || (common.is_some() && p.root_key.git_common_dir == common)
            });
            if let Some(p) = hit {
                return Ok(Some(p.clone()));
            }
        }
        Ok(None)
    }

    fn read_all(&self) -> Result<Listing> {
        let (max_projects, max_entries) = self.limits;
        let mut out = Listing::default();
        let names: Vec<String> = self.root()?.names()?.into_iter().filter(|n| !n.starts_with('.')).collect();
        out.truncated = names.len() > max_entries;
        for name in names.into_iter().take(max_entries) {
            if !contract::valid_project_id(&name) {
                out.skipped += 1;
                continue;
            }
            match self.read_one(&name) {
                Ok(_) if out.projects.len() >= max_projects => {
                    out.truncated = true;
                    break;
                }
                Ok(p) => out.projects.push(p),
                Err(_) => out.skipped += 1,
            }
        }
        out.projects.sort_by(|a, b| (&a.created, &a.id).cmp(&(&b.created, &b.id)));
        Ok(out)
    }

    /// The project's folder, pinned (never through a symlink). `NotFound` when there is no such folder.
    fn pin(&self, id: &str) -> Result<Pinned> {
        if !contract::valid_project_id(id) {
            return Err(StoreError::NotFound);
        }
        match self.root()?.child(id, false) {
            Ok(d) => Ok(d),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(StoreError::NotFound),
            Err(e) => Err(e.into()),
        }
    }

    fn read_one(&self, id: &str) -> Result<Project> {
        self.read_pinned(&self.pin(id)?, id)
    }

    fn read_pinned(&self, dir: &Pinned, id: &str) -> Result<Project> {
        let bytes = match dir.read("project.json", contract::MAX_RECORD_BYTES as u64) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(StoreError::NotFound),
            Err(e) => return Err(e.into()),
        };
        let p = contract::parse_project(&bytes)?;
        if p.id != id {
            return Err(StoreError::Invalid(ContractError::Invalid("project id does not match its folder")));
        }
        Ok(p)
    }

    // ------------------------------------------------------------------------------------------------ writes

    fn write(&self, p: &Project) -> Result<()> {
        p.validate()?;
        let dir = self.root()?.child(&p.id, true)?;
        let mut bytes = serde_json::to_vec_pretty(p).map_err(|e| StoreError::Io(io::Error::other(e)))?;
        bytes.push(b'\n');
        dir.write_atomic("project.json", &bytes)?;
        Ok(())
    }

    /// The project that owns `folder` (the same folder through a symlink, or a worktree of its repository), or `None`:
    /// a folder no project owns, one that is gone or refused. Read only: nothing is created, restored or written.
    pub fn find(&self, folder: &Path) -> Result<Option<Project>> {
        let Ok(r) = identity::resolve(folder, &self.guard) else { return Ok(None) };
        let _g = self.lock();
        let all = self.read_all()?.projects;
        let same = |p: &&Project| (p.root_key.dev, p.root_key.ino) == (r.key.dev, r.key.ino) || (r.key.git_common_dir.is_some() && p.root_key.git_common_dir == r.key.git_common_dir);
        Ok(all.iter().find(same).cloned())
    }

    /// Add the folder `picked` as a project. The same folder again (through a symlink, in other letter case, or as a
    /// worktree of an existing project's repository) finds the existing project. A folder inside another project, or
    /// holding one, is refused unless `confirm_nested`.
    pub fn add(&self, picked: &Path, confirm_nested: bool) -> Result<Added> {
        let r = identity::resolve(picked, &self.guard)?;
        let _g = self.lock();
        // Matching an existing project and the nesting check need the WHOLE inventory; a partial scan could create a
        // second project for a folder it did not list, or miss an overlap. Refuse instead.
        let listing = self.read_all()?;
        if listing.truncated {
            return Err(StoreError::Full);
        }
        let all = listing.projects;
        let same = |p: &&Project| (p.root_key.dev, p.root_key.ino) == (r.key.dev, r.key.ino) || (r.key.git_common_dir.is_some() && p.root_key.git_common_dir == r.key.git_common_dir);
        if let Some(found) = all.iter().find(same) {
            if found.archived.is_some() {
                let mut p = found.clone();
                p.archived = None;
                p.rev += 1;
                self.write(&p)?;
                return Ok(Added::Restored(p));
            }
            return Ok(Added::Existing(found.clone()));
        }
        if all.len() >= self.limits.0 {
            return Err(StoreError::Full);
        }
        if !confirm_nested {
            let root = r.root.as_path();
            if let Some(o) = all.iter().filter(|p| p.archived.is_none()).find(|p| root.starts_with(&p.root) || Path::new(&p.root).starts_with(root)) {
                return Err(StoreError::Nested { with_id: o.id.clone(), with_name: o.name.clone() });
            }
        }
        let name = r.root.file_name().map_or_else(|| "project".to_string(), |n| n.to_string_lossy().into_owned());
        let p = Project::new(contract::fresh_id(contract::PROJECT_PREFIX)?, name, r.root.to_string_lossy().into_owned(), r.key, self.stamp());
        self.write(&p)?;
        Ok(Added::Created(p))
    }

    pub fn rename(&self, id: &str, name: &str, expect_rev: Option<u64>) -> Result<Project> {
        let _g = self.lock();
        let mut p = self.read_one(id)?;
        check_rev(&p, expect_rev)?;
        let name = name.trim();
        if name == p.name {
            return Ok(p);
        }
        p.name = name.to_string();
        p.rev += 1;
        self.write(&p)?; // validates the name
        Ok(p)
    }

    /// Hide the project. Nothing it refers to is touched. Archiving an archived project changes nothing.
    pub fn archive(&self, id: &str, expect_rev: Option<u64>) -> Result<Project> {
        let _g = self.lock();
        let mut p = self.read_one(id)?;
        check_rev(&p, expect_rev)?;
        if p.archived.is_some() {
            return Ok(p);
        }
        p.archived = Some(self.stamp());
        p.rev += 1;
        self.write(&p)?;
        Ok(p)
    }

    // ----------------------------------------------------------------------------------------------- sessions

    /// Append one session record to its project's `sessions.jsonl` (0600).
    pub fn append_session(&self, s: &Session) -> Result<()> {
        s.validate()?;
        let _g = self.lock();
        let dir = self.pin(&s.project_id)?;
        self.read_pinned(&dir, &s.project_id)?;
        let mut line = serde_json::to_vec(s).map_err(|e| StoreError::Io(io::Error::other(e)))?;
        if line.len() >= contract::MAX_RECORD_BYTES {
            return Err(StoreError::Invalid(ContractError::TooLarge));
        }
        line.push(b'\n');
        dir.append("sessions.jsonl", &line)?;
        Ok(())
    }

    /// The project's sessions, the newest line per id, in first-seen order. Only the tail of the file is read; a damaged
    /// line is skipped but counted, and a line of another major version is flagged (`unsupported`).
    pub fn sessions(&self, project_id: &str) -> Result<SessionLog> {
        let _g = self.lock();
        let dir = self.pin(project_id)?;
        self.read_pinned(&dir, project_id)?;
        let (buf, truncated) = match dir.read_tail("sessions.jsonl", SESSIONS_READ_CAP) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(SessionLog::default()),
            Err(e) => return Err(e.into()),
        };
        let mut lines: Vec<&[u8]> = buf.split(|b| *b == b'\n').filter(|l| !l.is_empty()).collect();
        if truncated && !lines.is_empty() {
            lines.remove(0); // the cut start: a partial line, not a damaged one
        }
        let mut log = contract::latest_sessions_of(Some(project_id), &lines);
        log.truncated = truncated;
        Ok(log)
    }
}

fn check_rev(p: &Project, expect: Option<u64>) -> Result<()> {
    match expect {
        Some(r) if r != p.rev => Err(StoreError::Conflict { current_rev: p.rev }),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
