//! After a crash (6d2d, vajra-43's design note items b and e2). At start-up, before any agent of this life exists, the
//! ownership ledger's leads and workers that are still provably ours are killed (`owned::Ledger::end_agents`). A killed
//! worker's ticket is written ended `failed`/`abandoned`/`app_crashed`, so the worker runtime's replay shows it. A run
//! whose lead an earlier life recorded ends `lead_lost`/`app_restarted`. Nothing is ever re-run by itself.
//!
//! Each core run keeps three small records next to its `tickets.jsonl` under `<data>/runs/<run>/`, all written
//! atomically into the run's private dir and read back without following links: `core.json` (what it was started on),
//! `plan.json` (the accepted plan, for the resume charter) and `ended.json` (how it ended). `core_runs` lists every
//! ended core run with whether it can be resumed and, if not, why.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::owned::{Ended, Kind, Ledger};
use crate::projects::pinned::Pinned;

pub const CORE_FILE: &str = "core.json";
pub const PLAN_FILE: &str = "plan.json";
pub const ENDED_FILE: &str = "ended.json";
/// The most of any record read back.
const RECORD_CAP: u64 = 256 * 1024;
/// The most ended core runs `core_runs` lists (the reconcile is not capped by it).
const MAX_LISTED: usize = 200;

/// Why an ended run cannot be resumed. The reason codes are API; the card's words are in `docs/design/changed-strings.md`.
pub const CWD_MISSING: &str = "cwd_missing";
pub const PROVIDER_NOT_INSTALLED: &str = "provider_not_installed";
pub const LEAD_UNVERIFIED: &str = "lead_unverified";
pub const SEATS_UNREADABLE: &str = "seats_unreadable";
pub const PLAN_UNREADABLE: &str = "plan_unreadable";

/// What the start-up reconcile did, for the lifecycle report.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Reconciled {
    /// Another instance holds the data folder: nothing was reconciled, and this app starts no agent.
    pub other_instance: bool,
    /// Records given up on after `owned::MAX_TRIES` starts that could not end them.
    pub given_up: usize,
    /// Leads and workers of an earlier life killed here.
    pub killed: usize,
    /// Signalled, but some of their session still runs (kept in the ledger, tried again at the next start).
    pub survived: usize,
    /// Records whose pid another process holds now: never signalled.
    pub reused: usize,
    /// Survivors whose record could not be written back to the ledger: the next start cannot try them again.
    pub unrecorded: usize,
    /// Worker tickets written `app_crashed`.
    pub tickets: usize,
    /// Core runs written ended `lead_lost`/`app_restarted`.
    pub runs: Vec<String>,
}

/// `<data>/runs/<run>`, held open: every step is opened relative to the one before it (O_DIRECTORY | O_NOFOLLOW), as
/// `workers::open_record` opens `tickets.jsonl`, so a folder swapped for a link after the check cannot redirect a read or
/// write. `run` must be one plain path component. With `create`, the missing folders are made at 0700.
fn run_dir(data: &Path, run: &str, create: bool) -> std::io::Result<Pinned> {
    let one = !run.is_empty() && run != "." && run != ".." && !run.contains('/') && !run.contains('\0');
    if !one {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "run id"));
    }
    Pinned::open(data, false)?.child("runs", create)?.child(run, create)
}

fn read(dir: &Pinned, name: &str) -> std::io::Result<Value> {
    let b = dir.read(name, RECORD_CAP)?;
    serde_json::from_slice(&b).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Is `name` in `dir` at all (a file, link or anything else)? Asked of the held folder, never through a path.
fn present(dir: &Pinned, name: &str) -> bool {
    dir.exists(name)
}

fn write(data: &Path, run: &str, name: &str, v: &Value) -> std::io::Result<()> {
    let d = run_dir(data, run, true)?;
    d.write_atomic(name, &serde_json::to_vec(v).map_err(std::io::Error::other)?)
}

/// The run was started: its project, folder and lead account, for the resume check.
pub fn record_started(data: &Path, run: &str, project: &str, cwd: &Path, lead_account: &str) -> std::io::Result<()> {
    write(data, run, CORE_FILE, &json!({"v": 1, "project": project, "cwd": cwd, "lead_account": lead_account}))
}

/// The accepted plan, as `RunProtocol::plan_record` gives it.
pub fn record_plan(data: &Path, run: &str, plan: &Value) -> std::io::Result<()> {
    write(data, run, PLAN_FILE, plan)
}

/// How the run ended. The first end recorded stays: a later one (a crash reconcile after a clean end) changes nothing.
pub fn record_ended(data: &Path, run: &str, status: &str, reason: &str) -> std::io::Result<bool> {
    let d = run_dir(data, run, true)?;
    if present(&d, ENDED_FILE) {
        return Ok(false);
    }
    d.write_atomic(ENDED_FILE, &serde_json::to_vec(&json!({"v": 1, "status": status, "reason": reason})).map_err(std::io::Error::other)?).map(|()| true)
}

/// At start-up, before any agent of this life starts: end what an earlier life's crash left running.
pub fn reconcile(data: &Path, ledger: &Ledger) -> Reconciled {
    let mut out = Reconciled::default();
    for (o, ended) in ledger.end_agents() {
        match ended {
            Ended::Killed => out.killed += 1,
            Ended::Survived => out.survived += 1,
            Ended::Reused => out.reused += 1,
            Ended::Gone => {}
            Ended::GivenUp => out.given_up += 1,
            Ended::Unrecorded => {
                out.survived += 1;
                out.unrecorded += 1;
            }
        }
        if ended == Ended::Reused {
            continue;
        }
        // Only a worker killed here was still running: one that had exited is ended by the replay (`app_restarted`).
        // `record_crashed` opens the run's folder through the same held-descriptor chain.
        if let (Kind::Worker, Ended::Killed, Some(run), Some(t)) = (o.kind, ended, o.run.as_deref(), o.ticket.as_deref()) {
            if super::workers::record_crashed(data, run, t) {
                out.tickets += 1;
            }
        }
    }
    // Every core run with a start record and no end did not survive: this runs before any run of this life exists, and a
    // run that ended in an earlier life recorded its end. Its lead was killed above, or had died with the app.
    for run in list_runs(data) {
        let Ok(d) = run_dir(data, &run, false) else { continue };
        if present(&d, CORE_FILE) && !present(&d, ENDED_FILE) && record_ended(data, &run, "lead_lost", "app_restarted").unwrap_or(false) {
            out.runs.push(run);
        }
    }
    out.runs.sort();
    out.runs.dedup();
    out
}

/// Every run folder's name, newest first (run ids sort by time). Not capped: the reconcile must see every unfinished
/// core run, however many folders come after it; `core_runs` caps what it lists after filtering.
fn list_runs(data: &Path) -> Vec<String> {
    let Ok(runs) = Pinned::open(data, false).and_then(|d| d.child("runs", false)) else { return Vec::new() };
    let mut v = runs.names().unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

/// What the resume check needs from outside this module.
pub struct ResumeEnv<'a> {
    /// The run's guarded seats: `Ok(None)` when none were recorded.
    pub seats: &'a dyn Fn(&str) -> std::io::Result<Option<Value>>,
    /// Is the harness for this lead account installed?
    pub installed: &'a dyn Fn(&str) -> bool,
    /// Is a lead on this account's harness allowed (its profile verified)?
    pub lead_ok: &'a dyn Fn(&str) -> bool,
}

/// Every ended core run, newest first: `{run, project, status, reason, resumable, why}`. `why` is the first reason it
/// cannot be resumed, or null.
pub fn core_runs(data: &Path, env: &ResumeEnv) -> Vec<Value> {
    let mut out = Vec::new();
    for run in list_runs(data) {
        if out.len() >= MAX_LISTED {
            break;
        }
        let Ok(d) = run_dir(data, &run, false) else { continue };
        let Ok(ended) = read(&d, ENDED_FILE) else { continue };
        let core = read(&d, CORE_FILE).ok();
        let text = |v: &Option<Value>, k: &str| v.as_ref().and_then(|v| v[k].as_str()).map(String::from);
        let why = resume_refusal(&d, &run, &core, env);
        out.push(json!({
            "run": run,
            "project": text(&core, "project"),
            "status": ended["status"].as_str(),
            "reason": ended["reason"].as_str(),
            "resumable": why.is_none(),
            "why": why,
        }));
    }
    out
}

fn resume_refusal(d: &Pinned, run: &str, core: &Option<Value>, env: &ResumeEnv) -> Option<&'static str> {
    let Some(core) = core else { return Some(SEATS_UNREADABLE) };
    let cwd = core["cwd"].as_str().map(PathBuf::from).filter(|p| p.is_absolute());
    if !cwd.is_some_and(|c| c.is_dir()) {
        return Some(CWD_MISSING);
    }
    let Some(account) = core["lead_account"].as_str() else { return Some(SEATS_UNREADABLE) };
    if !(env.installed)(account) {
        return Some(PROVIDER_NOT_INSTALLED);
    }
    if !(env.lead_ok)(account) {
        return Some(LEAD_UNVERIFIED);
    }
    if !matches!((env.seats)(run), Ok(Some(_))) {
        return Some(SEATS_UNREADABLE);
    }
    if read(d, PLAN_FILE).is_err() {
        return Some(PLAN_UNREADABLE);
    }
    None
}

#[cfg(test)]
#[path = "recover_tests.rs"]
mod tests;
