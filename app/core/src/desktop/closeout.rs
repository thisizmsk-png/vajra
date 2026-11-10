//! Closing finished workers so idle sessions stop holding memory (owner request 2026-11-09).
//!
//! A worker is *finished* when its task ended: done, failed or stopped. It is closed only when that loses nothing:
//! its worktree is clean (`git status` empty) and nothing is unpushed (no commit ahead of its upstream; no upstream at
//! all counts as unpushed) and no terminal in its worktree is running anything. Closing releases what the worker
//! holds: the idle user terminals open in its worktree are hung up and its transcript is no longer tailed. The worker is marked closed in a small persisted set and stays in
//! the crew view and history. Nothing on disk (worktree, branch, transcript) is deleted.
//!
//! Never closed: the Director-General (it is not a subagent, so it is never a candidate), a worker that is mid-turn
//! (working), one waiting on the human (blocked, a question, a queued message) or one whose git state cannot be read.
//! A worker with unsaved work or a busy terminal is kept and reported with exact counts; only the human's "Close anyway"
//! closes it, and only if the counts they confirmed (files, commits, busy terminals) still hold.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{App, Res, fail, lock};
use crate::crew::tree::{Status, Worker};
use crate::vajra::bridge::Request;

/// How often the automatic pass looks (it never runs two at once).
const AUTO_EVERY: Duration = Duration::from_secs(10);
/// Each git read gets at most this long; a slow read keeps the worker open (nothing is guessed).
const GIT_TIMEOUT: Duration = Duration::from_secs(2);
/// The read-only status call reuses a worktree's answer this long, so a busy UI does not run git for every finished worker on every refresh.
const STATUS_TTL: Duration = Duration::from_secs(5);

/// What would be lost by closing. `no_upstream` counts as unpushed work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Unsaved {
    pub changed_files: u32,
    pub unpushed_commits: u32,
    pub no_upstream: bool,
    /// Terminals in the worktree still running something (a dev server, an editor): closing would end them.
    pub busy_terminals: u32,
}

impl Unsaved {
    fn git_clean(&self) -> bool {
        self.changed_files == 0 && self.unpushed_commits == 0 && !self.no_upstream
    }
    fn is_clean(&self) -> bool {
        self.git_clean() && self.busy_terminals == 0
    }
    /// The counts "Close anyway" asks the human to confirm: changed files, unpushed commits, busy terminals.
    fn confirm_key(&self) -> (u32, u32, u32) {
        (self.changed_files, self.unpushed_commits, self.busy_terminals)
    }
    fn put(&self, v: &mut Value) {
        v["changed_files"] = json!(self.changed_files);
        v["unpushed_commits"] = json!(self.unpushed_commits);
        v["no_upstream"] = json!(self.no_upstream);
        v["busy_terminals"] = json!(self.busy_terminals);
    }
}

/// Why a worker is kept open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Skip {
    /// Mid-turn, or its state is not known to be finished.
    Working,
    /// Blocked, asking a question, or a message to it is queued: the human is the next move.
    WaitingOnYou,
    /// Not in a crew worktree, so there is nothing to check or release.
    NoWorktree,
    /// Git did not answer in time or failed; losing work cannot be ruled out.
    Unreadable,
    Unsaved(Unsaved),
}

impl Skip {
    fn code(&self) -> &'static str {
        match self {
            Skip::Working => "working",
            Skip::WaitingOnYou => "waiting_on_you",
            Skip::NoWorktree => "no_worktree",
            Skip::Unreadable => "git_unreadable",
            Skip::Unsaved(u) if u.git_clean() => "terminal_busy",
            Skip::Unsaved(_) => "unsaved_work",
        }
    }

    /// Worth telling the human about in the bulk result (a working worker is not finished, so it is not "skipped").
    fn notable(&self) -> bool {
        !matches!(self, Skip::Working)
    }

    fn to_json(self, worker: &str, label: &str) -> Value {
        let mut v = json!({"worker": worker, "label": label, "reason": self.code()});
        if let Skip::Unsaved(u) = self {
            u.put(&mut v);
        }
        v
    }
}

/// Finished, and not waiting on the human. Pure so the table is tested without a repo.
pub(super) fn judge(status: Status, stopped: bool, message_pending: bool) -> Result<(), Skip> {
    // A queued message first: a finished worker the human just wrote to is still the human's move, and closing it would
    // stop the tailing that shows the reply.
    if message_pending || matches!(status, Status::Blocked | Status::Question) {
        return Err(Skip::WaitingOnYou);
    }
    if matches!(status, Status::Done | Status::Failed) || stopped {
        return Ok(());
    }
    Err(Skip::Working)
}

/// The effective setting for a run: `project` is the lookup of the run's project (`Err` if it could not be done),
/// `read` the preference read for a scope (`None` = could not be read). The global value is used only when no project
/// holds the folder; a project whose own read fails stays off even if the global value is on.
fn auto_setting(project: Result<Option<String>, ()>, read: impl Fn(Option<&str>) -> Option<bool>) -> bool {
    match project {
        Err(()) => false,
        Ok(Some(id)) => read(Some(&id)).unwrap_or(false),
        Ok(None) => read(None).unwrap_or(false),
    }
}

/// The persisted set of closed workers (id -> unix seconds), plus the automatic pass's pacing.
pub(super) struct Store {
    path: PathBuf,
    closed: Mutex<BTreeMap<String, u64>>,
    busy: AtomicBool,
    last: Mutex<Option<Instant>>,
    /// worktree -> (when read, answer) for `closeout_status` only; closing always reads fresh.
    seen: Mutex<HashMap<PathBuf, (Instant, Option<Unsaved>)>>,
}

impl Store {
    pub(super) fn open(data: &Path) -> Self {
        let path = data.join("closed_workers.json");
        // An unreadable file reads as "nothing closed": the worst outcome is a worker looking open again.
        let closed = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { path, closed: Mutex::new(closed), busy: AtomicBool::new(false), last: Mutex::new(None), seen: Mutex::new(HashMap::new()) }
    }

    pub(super) fn is_closed(&self, id: &str) -> bool {
        lock(&self.closed).contains_key(id)
    }

    fn mark(&self, id: &str) -> std::io::Result<()> {
        let mut set = lock(&self.closed);
        if set.contains_key(id) {
            return Ok(());
        }
        set.insert(id.to_string(), crate::ids::unix_now());
        let tmp = self.path.with_extension("json.tmp");
        let written = serde_json::to_vec(&*set).map_err(std::io::Error::other).and_then(|b| std::fs::write(&tmp, b)).and_then(|()| std::fs::rename(&tmp, &self.path));
        if written.is_err() {
            set.remove(id);
        }
        written
    }
}

/// A worker found in a known run, with what the control service says about it.
struct Cand {
    worker: Worker,
    /// The effective `auto_close_finished` of the run's project (its override, else the global value).
    auto: bool,
    stopped: bool,
    pending: bool,
}

/// What closing does to a worktree's user terminals (the app's `Terminals` in production, a fake in tests).
pub trait WorktreeTerminals {
    /// How many terminals opened in the worktree are running something (a shell that cannot be asked counts).
    fn busy(&self, wt: &Path) -> u32;
    /// Hang up every terminal opened in the worktree. An error keeps the worker open.
    fn release(&self, wt: &Path) -> Result<(), String>;
}

impl App {
    /// Whether a worker was closed (the crew and the pump skip tailing it).
    pub(super) fn worker_closed(&self, id: &str) -> bool {
        self.closeout.is_closed(id)
    }

    /// The run's project's effective `auto_close_finished`: its own override if it has one, else the global value. The
    /// global value is used only when no project holds the folder. If the project cannot be looked up, or its own
    /// preference cannot be read, the answer is "off": a doubt never closes a worker on its own.
    fn auto_close_on(&self, cwd: &Path) -> bool {
        let project = self.projects.find_by_path(cwd).map(|p| p.map(|p| p.id)).map_err(|_| ());
        auto_setting(project, |p| self.preferences_read(p).ok().and_then(|v| v["effective"]["auto_close_finished"].as_bool()))
    }

    fn closeout_candidates(&self) -> Vec<Cand> {
        let runs: Vec<super::Run> = lock(&self.runs).values().cloned().collect();
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for run in runs {
            let Ok(workers) = self.discover_workers(&run) else { continue };
            let auto = self.auto_close_on(&run.cwd);
            let sig = lock(&self.control).signals(&run.session_id);
            for w in workers {
                if self.closeout.is_closed(&w.agent_id) || !seen.insert(w.agent_id.clone()) {
                    continue;
                }
                let (stopped, pending) = (sig.stopped.contains(&w.agent_id), sig.message_pending.contains(&w.agent_id));
                out.push(Cand { worker: w, auto, stopped, pending });
            }
        }
        out
    }

    /// Changed files (tracked edits and untracked files, one per file) and unpushed commits of a worktree, or `None`
    /// if git could not say.
    fn inspect_unsaved(&self, wt: &Path) -> Option<Unsaved> {
        let git = |args: &[&str]| {
            let mut bridge = self.bridge.clone();
            bridge.timeout = GIT_TIMEOUT;
            bridge.run(&Request::Git { cwd: wt.to_path_buf(), args: args.iter().map(|a| a.to_string()).collect() }).ok().filter(|o| o.exit().is_some())
        };
        let count = |o: &crate::vajra::bridge::Output| o.stdout.lines().filter(|l| !l.trim().is_empty()).count() as u32;
        let status = git(&["status", "--porcelain", "-uall"]).filter(|o| o.exit() == Some(0))?;
        let has_upstream = git(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])?.exit() == Some(0);
        let log = if has_upstream { git(&["log", "--format=%H", "@{u}..HEAD"]) } else { git(&["log", "--format=%H", "HEAD", "--not", "--remotes"]) }
            .filter(|o| o.exit() == Some(0))?;
        Some(Unsaved { changed_files: count(&status), unpushed_commits: count(&log), no_upstream: !has_upstream, busy_terminals: 0 })
    }

    fn inspect_cached(&self, wt: &Path) -> Option<Unsaved> {
        if let Some((at, u)) = lock(&self.closeout.seen).get(wt) {
            if at.elapsed() < STATUS_TTL {
                return *u;
            }
        }
        let u = self.inspect_unsaved(wt);
        lock(&self.closeout.seen).insert(wt.to_path_buf(), (Instant::now(), u));
        u
    }

    /// The worktree to release if closing `c` loses nothing, else why not.
    fn closeout_assess(&self, c: &Cand, cached: bool, terms: &dyn WorktreeTerminals) -> Result<PathBuf, Skip> {
        judge(c.worker.status, c.stopped, c.pending)?;
        let (wt, _) = self.crew_worktree(&c.worker.agent_id).map_err(|_| Skip::NoWorktree)?;
        let mut u = if cached { self.inspect_cached(&wt) } else { self.inspect_unsaved(&wt) }.ok_or(Skip::Unreadable)?;
        u.busy_terminals = terms.busy(&wt);
        if u.is_clean() { Ok(wt) } else { Err(Skip::Unsaved(u)) }
    }

    fn closeout_release(&self, id: &str, wt: &Path, terms: &dyn WorktreeTerminals) -> Res {
        terms.release(wt).map_err(|e| fail("release_failed", e))?;
        self.closeout.mark(id).map_err(|e| fail("io_error", e))?;
        self.note("worker_closed", &[]);
        Ok(json!({"ok": true, "worker": id, "closed": true}))
    }

    /// Close one worker. Without `anyway` only a worker that loses nothing is closed. With `anyway` (the counts the
    /// human confirmed: changed files, unpushed commits) a worker with unsaved work is closed too, but only if those
    /// are still the counts; otherwise nothing is closed and the fresh counts come back to be confirmed again.
    /// A working, waiting or unreadable worker is never closed, forced or not.
    pub(super) fn close_worker(&self, id: &str, anyway: Option<(u32, u32, u32)>, terms: &dyn WorktreeTerminals) -> Res {
        if self.closeout.is_closed(id) {
            return Ok(json!({"ok": true, "worker": id, "closed": true, "already": true}));
        }
        let cand = self.closeout_candidates().into_iter().find(|c| c.worker.agent_id == id).ok_or_else(|| fail("not_found", "no such open worker"))?;
        match self.closeout_assess(&cand, false, terms) {
            Ok(wt) => self.closeout_release(id, &wt, terms),
            Err(Skip::Unsaved(u)) => {
                let confirmed = anyway == Some(u.confirm_key());
                if !confirmed {
                    let mut e = fail(if anyway.is_some() { "unsaved_work_changed" } else { Skip::Unsaved(u).code() }, "this worker holds work that closing would lose");
                    u.put(&mut e);
                    return Err(e);
                }
                let (wt, _) = self.crew_worktree(id)?;
                self.closeout_release(id, &wt, terms)
            }
            Err(skip) => Err(fail(skip.code(), "this worker cannot be closed now")),
        }
    }

    /// Close every worker that loses nothing; say what was kept and why.
    /// `auto_only` is the automatic pass: it leaves out workers whose project has the setting off. The button ignores it.
    pub(super) fn close_finished(&self, terms: &dyn WorktreeTerminals, auto_only: bool) -> Res {
        let (mut closed, mut skipped) = (Vec::new(), Vec::new());
        for c in self.closeout_candidates().into_iter().filter(|c| c.auto || !auto_only) {
            let (id, label) = (c.worker.agent_id.as_str(), c.worker.label.as_str());
            match self.closeout_assess(&c, false, terms) {
                Ok(wt) => match self.closeout_release(id, &wt, terms) {
                    Ok(_) => closed.push(json!({"worker": id, "label": label})),
                    Err(e) => skipped.push(json!({"worker": id, "label": label, "reason": e["error"]})),
                },
                Err(s) if s.notable() => skipped.push(s.to_json(id, label)),
                Err(_) => {}
            }
        }
        Ok(json!({"ok": true, "closed": closed, "skipped": skipped}))
    }

    /// Per worker of a run: `closed`, `closable`, `working`, `waiting_on_you`, `unsaved_work` (with counts), `no_worktree`
    /// or `git_unreadable`. Read-only; the crew view and the sidebar ask for it.
    pub fn closeout_status(&self, slug: &str, run_id: &str, terms: &dyn WorktreeTerminals) -> Res {
        let Some((_, run)) = self.run_by_id(slug, run_id) else { return Ok(json!({"workers": []})) };
        let workers = self.discover_workers(&run)?;
        let sig = lock(&self.control).signals(&run.session_id);
        let rows: Vec<Value> = workers
            .into_iter()
            .map(|w| {
                if self.closeout.is_closed(&w.agent_id) {
                    return json!({"worker": w.agent_id, "state": "closed"});
                }
                let c = Cand { auto: true, stopped: sig.stopped.contains(&w.agent_id), pending: sig.message_pending.contains(&w.agent_id), worker: w };
                let id = c.worker.agent_id.clone();
                match self.closeout_assess(&c, true, terms) {
                    Ok(_) => json!({"worker": id, "state": "closable"}),
                    Err(s) => {
                        let mut v = s.to_json(&id, &c.worker.label);
                        v["state"] = v["reason"].take();
                        v
                    }
                }
            })
            .collect();
        Ok(json!({"workers": rows}))
    }

    /// One automatic pass if it is due and the setting is on. Claims the slot (so passes never overlap); the caller
    /// runs `closeout_auto` and it frees the slot.
    pub fn closeout_due(&self) -> bool {
        {
            let mut last = lock(&self.closeout.last);
            if last.is_some_and(|t| t.elapsed() < AUTO_EVERY) {
                return false;
            }
            *last = Some(Instant::now());
        }
        !self.closeout.busy.swap(true, Ordering::SeqCst)
    }

    /// Returns how many workers it closed (the caller tells the UI when it is not zero).
    pub fn closeout_auto(&self, terms: &dyn WorktreeTerminals) -> usize {
        /// Frees the slot even if the pass panics, so one bad pass cannot stop automatic closing for the session.
        struct Free<'a>(&'a AtomicBool);
        impl Drop for Free<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _free = Free(&self.closeout.busy);
        // The setting is read per run, for that run's project, inside `close_finished`.
        self.close_finished(terms, true).ok().and_then(|r| r["closed"].as_array().map(Vec::len)).unwrap_or(0)
    }

    pub fn act_close_worker(&self, env: &super::Envelope, worker: &str, anyway: Option<(u32, u32, u32)>, terms: &dyn WorktreeTerminals) -> Value {
        self.act(env, crate::control::receipt::Caller::Ui, true, "close_worker", worker, None::<u64>, || self.close_worker(worker, anyway, terms))
    }

    pub fn act_close_finished(&self, env: &super::Envelope, terms: &dyn WorktreeTerminals) -> Value {
        self.act(env, crate::control::receipt::Caller::Ui, true, "close_finished", &env.project_id, None::<u64>, || self.close_finished(terms, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finished_workers_are_judged_closable_and_the_rest_are_kept_for_the_right_reason() {
        assert_eq!(judge(Status::Done, false, false), Ok(()));
        assert_eq!(judge(Status::Failed, false, false), Ok(()));
        assert_eq!(judge(Status::InProgress, true, false), Ok(()), "a stopped worker is finished");
        assert_eq!(judge(Status::InProgress, false, false), Err(Skip::Working));
        assert_eq!(judge(Status::Unknown, false, false), Err(Skip::Working), "unknown is never assumed finished");
        assert_eq!(judge(Status::Question, false, false), Err(Skip::WaitingOnYou));
        assert_eq!(judge(Status::Blocked, false, false), Err(Skip::WaitingOnYou));
        assert_eq!(judge(Status::InProgress, false, true), Err(Skip::WaitingOnYou));
        assert_eq!(judge(Status::Done, false, true), Err(Skip::WaitingOnYou), "a queued message keeps a finished worker open");
        assert_eq!(judge(Status::Failed, true, true), Err(Skip::WaitingOnYou));
        assert_eq!(judge(Status::Done, true, true), Err(Skip::WaitingOnYou));
    }

    #[test]
    fn a_project_whose_own_setting_cannot_be_read_stays_off_even_when_global_is_on() {
        let read = |p: Option<&str>| if p.is_some() { None } else { Some(true) };
        assert!(!auto_setting(Ok(Some("p_1".into())), read), "the project read failed: off, not global");
        assert!(auto_setting(Ok(None), read), "no project holds the folder: the global value");
        assert!(!auto_setting(Err(()), read), "the project lookup failed: off");
        assert!(!auto_setting(Ok(None), |_| None), "the global read failed: off");
        assert!(auto_setting(Ok(Some("p_1".into())), |_| Some(true)));
    }

    #[test]
    fn no_upstream_alone_is_unsaved_work() {
        let u = Unsaved { changed_files: 0, unpushed_commits: 0, no_upstream: true, busy_terminals: 0 };
        assert!(!u.is_clean());
        assert!(!Unsaved { changed_files: 0, unpushed_commits: 0, no_upstream: false, busy_terminals: 1 }.is_clean(), "a busy terminal alone keeps it open");
        assert!(Unsaved { changed_files: 0, unpushed_commits: 0, no_upstream: false, busy_terminals: 0 }.is_clean());
    }
}
