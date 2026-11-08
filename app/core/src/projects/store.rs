//! The project store (ADR-15 D1: JSON files). `<data>/projects/<id>/project.json` is one record, rewritten atomically at
//! 0600; `<data>/projects/<id>/sessions.jsonl` is append-only (the newest line per session id wins). Nothing here
//! moves, rewrites or deletes campaigns, the run index, the ledger or the control dirs: a project record only refers
//! to them. Every read goes through the contract's validation, and a record whose id does not match its folder name is
//! skipped, so one file cannot impersonate another project.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

use super::contract::{self, ContractError, Project, Session, SessionLog};
use super::identity::{self, Guard, IdentityError};
use super::pinned::Pinned;
use crate::control::{fsx, time};

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
    /// afterwards (a rename plus a symlink) cannot redirect anything.
    root: Pinned,
    limits: (usize, usize),
    guard: Guard,
    /// Serialises every read-modify-write of a record within the process.
    lock: Mutex<()>,
    now: Clock,
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
        let dir = data.join("projects");
        fsx::private_dir(&dir)?;
        let root = Pinned::open(&dir, false)?;
        // The guard compares against canonical paths; a data dir that is a symlink is compared by its target.
        let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        Ok(Self { root, limits: (MAX_PROJECTS, MAX_ENTRIES), guard: Guard { home: canon(home), data: canon(data) }, lock: Mutex::new(()), now })
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

    fn read_all(&self) -> Result<Listing> {
        let (max_projects, max_entries) = self.limits;
        let mut out = Listing::default();
        let names: Vec<String> = self.root.names()?.into_iter().filter(|n| !n.starts_with('.')).collect();
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
        match self.root.child(id, false) {
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
        let dir = self.root.child(&p.id, true)?;
        let mut bytes = serde_json::to_vec_pretty(p).map_err(|e| StoreError::Io(io::Error::other(e)))?;
        bytes.push(b'\n');
        dir.write_atomic("project.json", &bytes)?;
        Ok(())
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
