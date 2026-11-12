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
use crate::bus::Bus;
use crate::sessions::Supervisor;
use crate::vajra::gitw::{self, GitwError, Gitw, GlobalBaseline, SessionQuiesce};

/// Every agent launch refused because the startup baseline failed.
pub const AGENTS_OFF: &str = "agents_off";
/// The preflight row's action while git is missing: Apple's installer, opened only when the human clicks it.
pub const INSTALL_DEV_TOOLS: &str = "install_dev_tools";
const NO_GIT: &str = "Agents are off: git isn't installed. Vajra needs Apple's developer tools for it.";

/// Why agents are off, and the one action that fixes it, if there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Off {
    pub why: String,
    /// What to do, when there is something: a restart helps only a failed take, never a tampered copy.
    pub fix: &'static str,
    pub action: Option<&'static str>,
}

const RESTART: &str = "Restart Vajra. If this keeps happening, check that git works in Terminal.";

/// Where git can come from on macOS: the selected developer dir, then the Command Line Tools, then Xcode. `/usr/bin/git`
/// is only a shim into one of them; without them, running it opens Apple's install dialog.
pub fn dev_roots() -> Vec<PathBuf> {
    let selected = std::fs::read_link("/var/db/xcode_select_link").ok();
    selected.into_iter().chain(["/Library/Developer/CommandLineTools", "/Applications/Xcode.app/Contents/Developer"].map(PathBuf::from)).collect()
}

/// A path check only: nothing is run, so a Mac without the tools sees no dialog at launch.
pub fn git_installed(roots: &[PathBuf]) -> bool {
    roots.iter().any(|r| r.join("usr/bin/git").is_file())
}

pub struct WorkerHost {
    git: PathBuf,
    home: PathBuf,
    data: PathBuf,
    work: PathBuf,
    /// Taken once at open. `Err`: why agents are off and what to do.
    baseline: Result<GlobalBaseline, Off>,
    rt: OnceLock<WorkerRuntime>,
    startup_swept: crate::procsweep::Swept,
}

impl WorkerHost {
    /// The roots check and the baseline, nothing else: no thread starts here. With no git under `dev_roots`, git is never
    /// run and agents stay off with the install action.
    /// `sweep`: this app holds the instance locks (6d2d), so the work root's leftover workers are an earlier life's of
    /// its own, never a running app's. Without them nothing is swept.
    pub fn open(git: &Path, data: &Path, home: &Path, xdg: Option<&Path>, dev_roots: &[PathBuf], sweep: bool) -> WorkerHost {
        let work = gitw::work_root(home);
        let baseline = if git_installed(dev_roots) {
            gitw::check_roots(data, &work).and_then(|()| GlobalBaseline::take_or_load(git, data, home, xdg)).map_err(|e| Off { why: why(&e), fix: fix(&e), action: None })
        } else {
            Err(Off { why: NO_GIT.into(), fix: "", action: Some(INSTALL_DEV_TOOLS) })
        };
        // Before any agent can start: an earlier app life's workers that a crash left running are killed, each proved by
        // its ticket TMPDIR under this work root and a start before this app (6c-3, disclosed in the lifecycle report).
        let startup_swept = if sweep && work.is_dir() { crate::procsweep::sweep_earlier_lives(&work) } else { Default::default() };
        WorkerHost { git: git.to_path_buf(), home: home.to_path_buf(), data: data.to_path_buf(), work, baseline, rt: OnceLock::new(), startup_swept }
    }

    /// `Err`: the line to show where a launch was refused.
    pub fn agents_allowed(&self) -> Result<(), String> {
        self.baseline.as_ref().map(|_| ()).map_err(|o| o.why.clone())
    }

    /// While git is missing: the action that opens Apple's installer.
    pub fn install_action(&self) -> Option<&'static str> {
        self.baseline.as_ref().err().and_then(|o| o.action)
    }

    /// The launch refusal as an IPC error.
    pub fn refuse(&self) -> Result<(), Value> {
        self.agents_allowed().map_err(|why| json!({"error": AGENTS_OFF, "detail": why}))
    }

    /// The preflight row while agents are off; `None` when the baseline is held.
    pub fn preflight_row(&self) -> Option<Value> {
        let off = self.baseline.as_ref().err()?;
        Some(match off.action {
            Some(a) => json!({"name": "Git settings for agents", "ok": false, "detail": off.why, "fix": "", "action": a, "action_label": "Install developer tools", "action_note": "When the install finishes, restart Vajra."}),
            None => json!({"name": "Git settings for agents", "ok": false, "detail": off.why, "fix": off.fix}),
        })
    }

    /// The runtime, built the first time it is asked for (6d dispatches through it). Refused while agents are off.
    pub fn runtime(
        &self,
        sup: &Arc<Supervisor>,
        bus: &Arc<Bus>,
        on_repo: impl Fn(&Path) -> Vec<String> + Send + Sync + 'static,
        hooks: impl FnOnce() -> Hooks,
        cfg: impl FnOnce() -> Config,
    ) -> Result<&WorkerRuntime, Value> {
        let g = self.baseline.as_ref().map_err(|o| json!({"error": AGENTS_OFF, "detail": o.why}))?;
        Ok(self.rt.get_or_init(|| {
            // The runs `on_repo` names, plus every core worker by its ticket's pinned common dir (#151 codex P1).
            let sessions = Arc::new(crate::vajra::gitw::RepoSessions::default());
            let reg = sessions.clone();
            let on_repo = move |common: &Path| {
                let mut ids = on_repo(common);
                ids.extend(reg.on(common));
                ids
            };
            let quiesce = Arc::new(SessionQuiesce::new(sup.clone(), on_repo));
            let gw = Arc::new(Gitw::new(self.git.clone(), self.home.clone(), self.data.clone(), self.work.clone(), g, quiesce));
            WorkerRuntime::with_sessions(cfg(), gw, sup.clone(), bus.clone(), hooks(), sessions)
        }))
    }

    /// App quit: the runtime's tickets end (see `WorkerRuntime::quit`). Nothing to do if it was never built.
    pub fn quit(&self, wait: std::time::Duration) -> Vec<String> {
        self.rt.get().map(|rt| rt.quit(wait)).unwrap_or_default()
    }

    /// What the startup sweep killed (an earlier app life's worker processes, 6c-3), for the lifecycle report.
    pub fn startup_swept(&self) -> crate::procsweep::Swept {
        self.startup_swept
    }

    /// The runtime's descendant sweep totals (`WorkerRuntime::swept`); zero if it was never built.
    pub fn runtime_swept(&self) -> (u64, u64) {
        self.rt.get().map(WorkerRuntime::swept).unwrap_or_default()
    }

    /// Built yet? (Tests: an idle app never builds it.)
    pub fn runtime_built(&self) -> bool {
        self.rt.get().is_some()
    }

    /// The crew view's `core:<ticket>` workers for `run`, from the runtime if it was built (never builds it).
    pub fn core_workers(&self, run: &str) -> Vec<crate::crew::tree::CoreWorker> {
        self.rt.get().map(|rt| rt.list(run).iter().map(super::workers::TicketView::core_worker).collect()).unwrap_or_default()
    }

    /// Settings: the disk under `<work>`, every copy any app life made. A walk only: no git, and no runtime is built.
    pub fn disk_use(&self) -> Value {
        let copies = copies_under(&self.work);
        let (bytes, partial) = super::workers::allocated(&self.work);
        json!({"copies": copies, "bytes": bytes, "partial": partial})
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

    fn message_to(&self, ticket: &str) -> Result<String, &'static str> {
        let v = self.rt.get().and_then(|rt| rt.view(ticket)).ok_or(profiles::NO_MESSAGE)?;
        // The same answer the crew node shows (#151).
        if let Some(why) = v.message_off() {
            return Err(why);
        }
        v.harness_session.ok_or(profiles::NOT_STARTED)
    }

    fn stop(&self, ticket: &str) -> bool {
        self.rt.get().is_some_and(|rt| rt.cancel(ticket))
    }
}

/// Worktrees sit at `<work>/<project>/<run>/<task>-r<n>`, next to that run's `tmp`.
fn copies_under(work: &Path) -> usize {
    let dirs = |p: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(p).map(|rd| rd.flatten().filter(|d| d.file_type().is_ok_and(|t| t.is_dir())).map(|d| d.path()).collect()).unwrap_or_default()
    };
    dirs(work).iter().flat_map(|p| dirs(p)).flat_map(|r| dirs(&r)).filter(|t| t.file_name().is_some_and(|n| n != "tmp")).count()
}

/// What the row suggests: nothing for a tampered copy or overlapping folders (a restart finds them again).
fn fix(e: &GitwError) -> &'static str {
    match e {
        GitwError::Tampered { .. } | GitwError::WorktreeRootInvalid => "",
        _ => RESTART,
    }
}

/// The plain line for a baseline that couldn't be taken.
fn why(e: &GitwError) -> String {
    match e {
        GitwError::Tampered { .. } => "Agents are off: the saved copy of your git settings was changed outside Vajra, so it can't be trusted. Re-taking it comes in a later Vajra update.".into(),
        GitwError::WorktreeRootInvalid => "Agents are off: the folder for worker copies overlaps the app's data folder.".into(),
        e => format!("Agents are off: Vajra couldn't record your git settings at startup ({}).", e.code()),
    }
}
