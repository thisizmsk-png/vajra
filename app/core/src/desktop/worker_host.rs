//! ADR 16.6 at startup, and where the worker runtime lives in the app.
//!
//! The global git baseline is taken (or loaded) in `App::open`. `main` binds the control socket and opens the window
//! only after `open` returns, so no Director-General, run or worker can launch before it. A baseline that can't be taken
//! fails closed: the app still opens, every agent launch is refused with one plain line, and preflight shows the row.
//!
//! The runtime itself is built on first use: an idle app holds no thread, timer, watcher or git process for it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};

use super::workers::{CleanUp, Config, Hooks, State, WorkerRuntime};
use crate::adapter::profiles;
use crate::control::service::{ControlError, CoreWorkers};
use crate::providers::launch;
use crate::bus::Bus;
use crate::sessions::Supervisor;
use crate::vajra::gitw::{self, GitwError, Gitw, GlobalBaseline, SessionQuiesce};

/// Every agent launch refused because the startup baseline failed.
pub const AGENTS_OFF: &str = "agents_off";

pub struct WorkerHost {
    git: PathBuf,
    home: PathBuf,
    data: PathBuf,
    work: PathBuf,
    /// Taken once at open. `Err`: the line that says why agents are off and what to do.
    baseline: Result<GlobalBaseline, String>,
    rt: OnceLock<WorkerRuntime>,
}

impl WorkerHost {
    /// The roots check and the baseline, nothing else: no thread starts here.
    pub fn open(git: &Path, data: &Path, home: &Path, xdg: Option<&Path>) -> WorkerHost {
        let work = gitw::work_root(home);
        let baseline = gitw::check_roots(data, &work).and_then(|()| GlobalBaseline::take_or_load(git, data, home, xdg)).map_err(|e| why(&e));
        WorkerHost { git: git.to_path_buf(), home: home.to_path_buf(), data: data.to_path_buf(), work, baseline, rt: OnceLock::new() }
    }

    /// `Err`: the line to show where a launch was refused.
    pub fn agents_allowed(&self) -> Result<(), String> {
        self.baseline.as_ref().map(|_| ()).map_err(Clone::clone)
    }

    /// The launch refusal as an IPC error.
    pub fn refuse(&self) -> Result<(), Value> {
        self.agents_allowed().map_err(|why| json!({"error": AGENTS_OFF, "detail": why}))
    }

    /// The preflight row while agents are off; `None` when the baseline is held.
    pub fn preflight_row(&self) -> Option<Value> {
        let why = self.agents_allowed().err()?;
        Some(json!({"name": "Git settings for agents", "ok": false, "detail": why, "fix": "Restart Vajra. If this keeps happening, check that git works in Terminal."}))
    }

    /// The runtime, built the first time it is asked for (6d dispatches through it). Refused while agents are off.
    pub fn runtime(&self, sup: &Arc<Supervisor>, bus: &Arc<Bus>, on_repo: impl Fn(&Path) -> Vec<String> + Send + Sync + 'static, cfg: impl FnOnce() -> Config) -> Result<&WorkerRuntime, Value> {
        let g = self.baseline.as_ref().map_err(|why| json!({"error": AGENTS_OFF, "detail": why}))?;
        Ok(self.rt.get_or_init(|| {
            let quiesce = Arc::new(SessionQuiesce::new(sup.clone(), on_repo));
            let gw = Arc::new(Gitw::new(self.git.clone(), self.home.clone(), self.data.clone(), self.work.clone(), g, quiesce));
            WorkerRuntime::new(cfg(), gw, sup.clone(), bus.clone(), Hooks::default())
        }))
    }

    /// Built yet? (Tests: an idle app never builds it.)
    pub fn runtime_built(&self) -> bool {
        self.rt.get().is_some()
    }

    /// Settings: the disk under `<work>`, every copy any app life made. A walk only: no git, and no runtime is built.
    pub fn disk_use(&self) -> Value {
        let copies = copies_under(&self.work);
        json!({"copies": copies, "bytes": super::workers::allocated(&self.work)})
    }

    /// Settings' Clean up. Only this app life's tickets are known, so only their copies can be checked; with no runtime
    /// built there is nothing it may remove.
    pub fn clean_up(&self) -> CleanUp {
        self.rt.get().map(WorkerRuntime::clean_up).unwrap_or(CleanUp { removed: Vec::new() })
    }
}

/// `core:<ticket>` controls (16.7), for the control service: Message and Redirect per the ticket's harness profile,
/// Stop through `cancel` (a group kill for both harnesses today).
impl CoreWorkers for WorkerHost {
    fn live(&self, ticket: &str) -> Result<(String, u32), ControlError> {
        let v = self.rt.get().and_then(|rt| rt.view(ticket)).ok_or(ControlError::WorkerNotFound)?;
        if v.state == State::Ended {
            return Err(ControlError::WorkerFinished);
        }
        Ok((v.run, v.round))
    }

    fn message_off(&self, ticket: &str) -> &'static str {
        let engine = self.rt.get().and_then(|rt| rt.view(ticket)).and_then(|v| launch::harness_of(&v.account));
        engine.and_then(|e| profiles::controls(e).message).unwrap_or(profiles::NO_MESSAGE)
    }

    fn stop(&self, ticket: &str) {
        if let Some(rt) = self.rt.get() {
            rt.cancel(ticket);
        }
    }
}

/// Worktrees sit at `<work>/<project>/<run>/<task>-r<n>`, next to that run's `tmp`.
fn copies_under(work: &Path) -> usize {
    let dirs = |p: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(p).map(|rd| rd.flatten().filter(|d| d.file_type().is_ok_and(|t| t.is_dir())).map(|d| d.path()).collect()).unwrap_or_default()
    };
    dirs(work).iter().flat_map(|p| dirs(p)).flat_map(|r| dirs(&r)).filter(|t| t.file_name().is_some_and(|n| n != "tmp")).count()
}

/// The plain line for a baseline that couldn't be taken.
fn why(e: &GitwError) -> String {
    match e {
        GitwError::Tampered { .. } => "Agents are off: the saved copy of your git settings was changed outside Vajra, so it can't be trusted. Re-taking it comes in a later Vajra update.".into(),
        GitwError::WorktreeRootInvalid => "Agents are off: the folder for worker copies overlaps the app's data folder.".into(),
        e => format!("Agents are off: Vajra couldn't record your git settings at startup ({}).", e.code()),
    }
}
