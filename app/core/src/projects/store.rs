//! The project store (ADR-15 D1: JSON files). `<data>/projects/<id>/project.json` is one record, rewritten atomically at
//! 0600; `<data>/projects/<id>/sessions.jsonl` is append-only (the newest line per session id wins). Nothing here
//! moves, rewrites or deletes campaigns, the run index, the ledger or the control dirs: a project record only refers
//! to them. Every read goes through the contract's validation, and a record whose id does not match its folder name is
//! skipped, so one file cannot impersonate another project.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use super::contract::{self, ContractError, Project, Session};
use super::identity::{self, Guard, IdentityError};
use crate::control::{fsx, time};

/// Only the tail of a sessions file is read.
const SESSIONS_READ_CAP: u64 = 1024 * 1024;
/// No more projects than this are listed (a store with more is not one a person made).
const MAX_PROJECTS: usize = 500;

#[derive(Debug)]
pub enum StoreError {
    NotFound,
    /// The record changed since the caller read it (`expect_rev` did not match).
    Conflict { current_rev: u64 },
    /// The folder overlaps another project's folder; pass `confirm_nested` to add it anyway.
    Nested { with_id: String, with_name: String },
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
}

type Clock = Box<dyn Fn() -> SystemTime + Send + Sync>;

pub struct Store {
    dir: PathBuf,
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

    pub fn with_clock(data: &Path, home: &Path, now: Clock) -> io::Result<Self> {
        let dir = data.join("projects");
        fsx::private_dir(&dir)?;
        // The guard compares against canonical paths; a data dir that is a symlink is compared by its target.
        let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        Ok(Self { dir, guard: Guard { home: canon(home), data: canon(data) }, lock: Mutex::new(()), now })
    }

    /// The folders that can never be a project.
    pub fn guard(&self) -> &Guard {
        &self.guard
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn stamp(&self) -> String {
        time::iso_secs((self.now)())
    }

    fn project_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
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
        let mut out = Listing::default();
        let names = fsx::DirHandle::open(&self.dir)?.names()?;
        for name in names.into_iter().filter(|n| !n.starts_with('.')) {
            if out.projects.len() + out.skipped >= MAX_PROJECTS {
                break;
            }
            if !contract::valid_project_id(&name) {
                out.skipped += 1;
                continue;
            }
            match self.read_one(&name) {
                Ok(p) => out.projects.push(p),
                Err(_) => out.skipped += 1,
            }
        }
        out.projects.sort_by(|a, b| (&a.created, &a.id).cmp(&(&b.created, &b.id)));
        Ok(out)
    }

    fn read_one(&self, id: &str) -> Result<Project> {
        if !contract::valid_project_id(id) {
            return Err(StoreError::NotFound);
        }
        let bytes = match fsx::read_nofollow_capped(&self.project_dir(id).join("project.json"), contract::MAX_RECORD_BYTES as u64) {
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
        let dir = self.project_dir(&p.id);
        fsx::private_dir(&dir)?;
        let mut bytes = serde_json::to_vec_pretty(p).map_err(|e| StoreError::Io(io::Error::other(e)))?;
        bytes.push(b'\n');
        fsx::write_atomic(&dir, "project.json", &bytes)?;
        Ok(())
    }

    /// Add the folder `picked` as a project. The same folder again (through a symlink, in other letter case, or as a
    /// worktree of an existing project's repository) finds the existing project. A folder inside another project, or
    /// holding one, is refused unless `confirm_nested`.
    pub fn add(&self, picked: &Path, confirm_nested: bool) -> Result<Added> {
        let r = identity::resolve(picked, &self.guard)?;
        let _g = self.lock();
        let all = self.read_all()?.projects;
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
        use std::io::Write;
        s.validate()?;
        let _g = self.lock();
        self.read_one(&s.project_id)?;
        let mut line = serde_json::to_vec(s).map_err(|e| StoreError::Io(io::Error::other(e)))?;
        if line.len() >= contract::MAX_RECORD_BYTES {
            return Err(StoreError::Invalid(ContractError::TooLarge));
        }
        line.push(b'\n');
        let mut f = fsx::open_append(&self.project_dir(&s.project_id).join("sessions.jsonl"))?;
        f.write_all(&line)?;
        Ok(())
    }

    /// The project's sessions, the newest line per id, in first-seen order. Only the tail of the file is read; a damaged
    /// line is skipped.
    pub fn sessions(&self, project_id: &str) -> Result<Vec<Session>> {
        let _g = self.lock();
        self.read_one(project_id)?;
        let path = self.project_dir(project_id).join("sessions.jsonl");
        let mut f = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let len = f.metadata()?.len();
        let mut buf = Vec::new();
        f.seek(SeekFrom::Start(len.saturating_sub(SESSIONS_READ_CAP)))?;
        f.take(SESSIONS_READ_CAP).read_to_end(&mut buf)?;
        let lines: Vec<&[u8]> = buf.split(|b| *b == b'\n').filter(|l| !l.is_empty()).collect();
        Ok(contract::latest_sessions(&lines))
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
