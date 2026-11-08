//! Bringing campaigns that existed before projects into projects (ADR-15 section 1.5, D2): the app SUGGESTS groups, the
//! owner confirms once. Nothing is moved, rewritten or deleted: confirming creates project records and session records
//! that point at the old campaigns, and drops `<data>/projects/.migrated-v1`.
//!
//! Inputs are untrusted. `runs.jsonl` is worker-writable, so a launch line only proposes a folder: it must be an
//! absolute, existing directory, is canonicalised, and goes through the same refusals as a folder the owner picks. A
//! forged line can at worst put a campaign in a suggested group the owner can decline. The Lead session id comes from
//! the guarded `<data>/control/lead/<run_id>/lead_session`, never from `runs.jsonl`. Campaigns with no launch line
//! (started from a terminal) are listed read-only and never adopted. A campaign that cannot be placed stays in
//! "Imported (unsorted)"; it is never assigned silently.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::contract::{self, RootKey, Session, SessionKind, SessionStatus};
use super::identity::{self, Guard};
use super::store::{Store, StoreError};
use crate::control::{fsx, time};
use crate::ids;

/// One launch line from `runs.jsonl`, already parsed. Oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub campaign: String,
    pub run_id: String,
    pub session_id: String,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
    pub root: String,
    pub name: String,
    pub campaigns: Vec<String>,
    #[serde(skip)]
    pub key: Option<RootKey>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Plan {
    /// Suggested projects, by name.
    pub groups: Vec<Group>,
    /// Campaigns this app launched whose folder is gone, unusable or refused: shown as "Imported (unsorted)".
    pub unsorted: Vec<String>,
    /// Campaigns with no launch line (started elsewhere): listed, read-only, not adopted.
    pub terminal: Vec<String>,
}

pub fn marker(data: &Path) -> PathBuf {
    data.join("projects").join(".migrated-v1")
}

pub fn is_done(data: &Path) -> bool {
    fs::symlink_metadata(marker(data)).is_ok()
}

/// Group the campaigns by the live git root of the folder their newest launch used.
pub fn plan(launches: &[Launch], campaigns: &[String], guard: &Guard) -> Plan {
    let mut newest: BTreeMap<&str, &Launch> = BTreeMap::new();
    for l in launches {
        newest.insert(&l.campaign, l); // oldest first, so the last wins
    }
    let mut out = Plan::default();
    let mut by_root: BTreeMap<(u64, u64), Group> = BTreeMap::new();
    let mut seen = HashSet::new();
    for c in campaigns {
        if !seen.insert(c.as_str()) {
            continue;
        }
        let Some(l) = newest.get(c.as_str()) else {
            out.terminal.push(c.clone());
            continue;
        };
        let placed = (c.as_str(), &l.cwd);
        match place(placed.1, guard) {
            Some(r) if contract_label(c) => {
                let g = by_root.entry((r.key.dev, r.key.ino)).or_insert_with(|| Group {
                    name: r.root.file_name().map_or_else(|| "project".into(), |n| n.to_string_lossy().into_owned()),
                    root: r.root.to_string_lossy().into_owned(),
                    campaigns: Vec::new(),
                    key: Some(r.key.clone()),
                });
                g.campaigns.push(c.clone());
            }
            _ => out.unsorted.push(c.clone()),
        }
    }
    out.groups = by_root.into_values().collect();
    out.groups.sort_by(|a, b| (&a.name, &a.root).cmp(&(&b.name, &b.root)));
    out
}

fn place(cwd: &Path, guard: &Guard) -> Option<identity::Resolved> {
    if !cwd.is_absolute() {
        return None;
    }
    identity::resolve(cwd, guard).ok()
}

fn contract_label(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.') && !s.starts_with('.')
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Created {
    pub id: String,
    pub name: String,
    pub outcome: &'static str,
    pub sessions: usize,
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct Outcome {
    pub projects: Vec<Created>,
    /// Accepted roots that overlap an existing project and were not added (add them by hand to confirm the overlap).
    pub nested: Vec<String>,
    /// Accepted roots that were not in the suggestion at all.
    pub unknown: Vec<String>,
}

#[derive(Debug)]
pub enum MigrateError {
    AlreadyDone,
    Store(StoreError),
    Io(std::io::Error),
}

impl From<StoreError> for MigrateError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<std::io::Error> for MigrateError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Create the projects for the `accepted` suggested roots (matched against a fresh plan, never against caller-supplied
/// data), give each its sessions, and drop the marker. The owner confirms once: whatever was not accepted stays
/// unsorted, and the marker stops the suggestion from coming back.
pub fn confirm(store: &Store, data: &Path, guard: &Guard, launches: &[Launch], campaigns: &[String], accepted: &[String]) -> Result<Outcome, MigrateError> {
    // One confirm at a time, from the done-check to the marker: a double click or a second window waits, then finds the
    // marker and is refused, instead of writing every session twice.
    let _one = store.migration_lock();
    if is_done(data) {
        return Err(MigrateError::AlreadyDone);
    }
    let plan = plan(launches, campaigns, guard);
    let mut out = Outcome::default();
    for want in accepted {
        let canon = fs::canonicalize(want).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| want.clone());
        let Some(g) = plan.groups.iter().find(|g| g.root == canon) else {
            out.unknown.push(want.clone());
            continue;
        };
        let added = match store.add(Path::new(&g.root), false) {
            Ok(a) => a,
            Err(StoreError::Nested { .. }) => {
                out.nested.push(g.root.clone());
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let project = added.project().clone();
        let mut have: HashSet<(String, &'static str)> = store
            .sessions(&project.id)?
            .sessions
            .iter()
            .filter_map(|s| Some((s.run_id.clone()?, kind_name(&s.kind))))
            .collect();
        let mut written = 0;
        for l in launches.iter().filter(|l| g.campaigns.contains(&l.campaign)) {
            let started = run_started(&l.run_id);
            let Some(started) = started else { continue };
            let mut parent = None;
            if let Some(lead) = lead_session(data, &l.run_id) {
                if have.insert((l.run_id.clone(), "orchestrator")) {
                    let id = contract::fresh_id(contract::SESSION_PREFIX).map_err(StoreError::from)?;
                    store.append_session(&session(&project.id, &id, SessionKind::Orchestrator, None, Some(lead), l, &started))?;
                    parent = Some(id);
                    written += 1;
                }
            }
            if have.insert((l.run_id.clone(), "run")) {
                let id = contract::fresh_id(contract::SESSION_PREFIX).map_err(StoreError::from)?;
                store.append_session(&session(&project.id, &id, SessionKind::Run, parent, Some(l.session_id.clone()), l, &started))?;
                written += 1;
            }
        }
        out.projects.push(Created { id: project.id, name: project.name, outcome: added.outcome(), sessions: written });
    }
    fsx::write_atomic(&data.join("projects"), ".migrated-v1", b"1\n")?;
    Ok(out)
}

fn kind_name(k: &SessionKind) -> &'static str {
    match k {
        SessionKind::Orchestrator => "orchestrator",
        SessionKind::Run => "run",
        _ => "other",
    }
}

fn session(project: &str, id: &str, kind: SessionKind, parent: Option<String>, native: Option<String>, l: &Launch, started: &str) -> Session {
    Session {
        schema: contract::SESSION_SCHEMA.into(),
        id: id.into(),
        project_id: project.into(),
        kind,
        parent,
        provider: "claude".into(),
        native_session_id: native,
        run_id: Some(l.run_id.clone()),
        campaign: Some(l.campaign.clone()),
        task_id: None,
        attempt: None,
        generation: 1,
        // Whether the old run finished is not known from the index; the app does not claim it did.
        status: SessionStatus::Stopped,
        started: started.into(),
        ended: None,
    }
}

/// `YYYYMMDD-HHMM` (UTC) as an ISO timestamp.
fn run_started(run_id: &str) -> Option<String> {
    if !ids::valid_run_id(run_id) {
        return None;
    }
    let t = format!("{}-{}-{}T{}:{}:00Z", &run_id[0..4], &run_id[4..6], &run_id[6..8], &run_id[9..11], &run_id[11..13]);
    time::parse_iso(&t).map(|_| t)
}

/// The run's Lead session from the guarded control dir (a canonical uuid, or nothing).
fn lead_session(data: &Path, run_id: &str) -> Option<String> {
    let p = data.join("control").join("lead").join(run_id).join("lead_session");
    let text = String::from_utf8(fsx::read_nofollow_capped(&p, 128).ok()?).ok()?;
    let id = text.trim();
    ids::valid_session_id(id).then(|| id.to_string())
}

#[cfg(test)]
#[path = "migrate_tests.rs"]
mod tests;
