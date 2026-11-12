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

use crate::control::fsx;
use crate::owned::{Ended, Kind, Ledger};

pub const CORE_FILE: &str = "core.json";
pub const PLAN_FILE: &str = "plan.json";
pub const ENDED_FILE: &str = "ended.json";
/// The most of any record read back.
const RECORD_CAP: u64 = 256 * 1024;
/// The most runs listed.
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
    /// Leads and workers of an earlier life killed here.
    pub killed: usize,
    /// Signalled, but some of their session still runs (kept in the ledger, tried again at the next start).
    pub survived: usize,
    /// Records whose pid another process holds now: never signalled.
    pub reused: usize,
    /// Worker tickets written `app_crashed`.
    pub tickets: usize,
    /// Core runs written ended `lead_lost`/`app_restarted`.
    pub runs: Vec<String>,
}

/// `<data>/runs/<run>`, only when it is a real directory (not a link) and `run` is one plain path component.
fn run_dir(data: &Path, run: &str) -> Option<PathBuf> {
    let one = !run.is_empty() && run != "." && run != ".." && !run.contains('/') && !run.contains('\0');
    let d = data.join("runs").join(run);
    (one && std::fs::symlink_metadata(&d).is_ok_and(|m| m.is_dir())).then_some(d)
}

fn read(dir: &Path, name: &str) -> std::io::Result<Value> {
    let b = fsx::read_nofollow_capped(&dir.join(name), RECORD_CAP)?;
    serde_json::from_slice(&b).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn write(data: &Path, run: &str, name: &str, v: &Value) -> std::io::Result<()> {
    let runs = data.join("runs");
    fsx::private_dir(&runs)?;
    let one = !run.is_empty() && run != "." && run != ".." && !run.contains('/');
    if !one {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "run id"));
    }
    let d = runs.join(run);
    fsx::private_dir(&d)?;
    fsx::write_atomic(&d, name, &serde_json::to_vec(v).map_err(std::io::Error::other)?).map(|_| ())
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
    if run_dir(data, run).is_some_and(|d| std::fs::symlink_metadata(d.join(ENDED_FILE)).is_ok()) {
        return Ok(false);
    }
    write(data, run, ENDED_FILE, &json!({"v": 1, "status": status, "reason": reason})).map(|()| true)
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
        }
        if ended == Ended::Reused {
            continue;
        }
        // Only a worker killed here was still running: one that had exited is ended by the replay (`app_restarted`).
        if let (Kind::Worker, Ended::Killed, Some(run), Some(t)) = (o.kind, ended, o.run.as_deref(), o.ticket.as_deref()) {
            if run_dir(data, run).is_some() && super::workers::record_crashed(data, run, t) {
                out.tickets += 1;
            }
        }
    }
    // Every core run with a start record and no end did not survive: this runs before any run of this life exists, and a
    // run that ended in an earlier life recorded its end. Its lead was killed above, or had died with the app.
    for run in list_runs(data) {
        let Some(d) = run_dir(data, &run) else { continue };
        if d.join(CORE_FILE).is_file() && std::fs::symlink_metadata(d.join(ENDED_FILE)).is_err() && record_ended(data, &run, "lead_lost", "app_restarted").unwrap_or(false) {
            out.runs.push(run);
        }
    }
    out.runs.sort();
    out.runs.dedup();
    out
}

fn list_runs(data: &Path) -> Vec<String> {
    let runs = data.join("runs");
    if !std::fs::symlink_metadata(&runs).is_ok_and(|m| m.is_dir()) {
        return Vec::new();
    }
    let mut v: Vec<String> = std::fs::read_dir(&runs).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
    v.sort();
    v.reverse(); // newest first: run ids sort by time
    v.truncate(MAX_LISTED);
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
        let Some(d) = run_dir(data, &run) else { continue };
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

fn resume_refusal(d: &Path, run: &str, core: &Option<Value>, env: &ResumeEnv) -> Option<&'static str> {
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
