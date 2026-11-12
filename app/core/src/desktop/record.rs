//! The signed run record (ADR-16 16.5a, PR 6h): a core run's campaign and task graph, written through the engine
//! scripts only. The core never opens the HMAC key and never writes under `~/.claude/vajra`; this module uses no
//! `std::fs` (a test reads this file to hold that).
//!
//! - **Plan** (round 1, synchronous): `create`, then `add` for each task, then `validate`. A failure ends the run
//!   before dispatch with the script's first error line. An existing campaign is reused only when this run is a resume
//!   the core made (`resumed`) and `show --json` matches exactly (slug, goal, the features in order, nothing passed,
//!   status active, signature valid).
//! - **Events** (claim, outcome reports, round progress, the stuck checkpoint) run in order on the record's own thread,
//!   never under the protocol lock. Each is retried once after a clean non-zero exit, then becomes a card note. A write
//!   that timed out is never retried: `verify` runs instead, and a broken signature is on the card.
//! - **Locks.** A write that finds the campaign locked puts a lock card on the run: the lock's path (built from HOME and
//!   the stored slug, never from script output), whether any `vajra-campaign.sh` runs now, and Repair (`unlock
//!   --if-stale`). The core never removes a lock.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::vajra::bridge::{Bridge, BridgeError, CloseStatus, Feature, Output, Request, TaskStatus};
use crate::vajra::bridge_record::{self as bridge_text, GOAL_MAX, PROGRESS_MAX, SUMMARY_MAX, TITLE_MAX};

/// Campaign and task writes wait up to 10 s for the script's lock, then do their work.
pub const RECORD_TIMEOUT: Duration = Duration::from_secs(60);

/// What the human does with a lock nobody may remove on their behalf (16.5a).
pub const LOCK_TEXT: &str = "Make sure no Vajra run or terminal command is using this campaign, then delete this folder.";

/// The record's campaign: fixed at Start (`seats.json`), never recomputed.
#[derive(Clone, Debug)]
pub struct RecordConfig {
    pub run: String,
    pub slug: String,
    /// `evidence_trust` from `seats.json`.
    pub trust: String,
    /// The HOME the scripts see: only for the lock card's path.
    pub home: PathBuf,
    /// This run continues one the core started: an existing campaign may be reused.
    pub resumed: bool,
}

impl RecordConfig {
    /// The slug must be the one the evidence trust names (`seat_start::evidence_of`): a lead or a worker can't pick it.
    pub fn check(&self) -> Result<(), String> {
        let want = if self.trust == "outside_guard" { format!("deliver-{}-og", self.run) } else { format!("deliver-{}", self.run) };
        if self.slug != want {
            return Err(format!("the run's campaign {} is not the one its evidence trust names", self.slug));
        }
        bridge_text::slug(&self.slug).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// One task of round 1's plan, as the task graph keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRecord {
    /// `<run>.<id>`.
    pub id: String,
    pub title: String,
    pub agent: String,
    pub deps: Vec<String>,
    pub files: Vec<String>,
    pub criteria: Vec<String>,
    pub branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRecord {
    pub goal: String,
    pub features: Vec<Feature>,
    pub tasks: Vec<TaskRecord>,
}

/// A best-effort write (the failure table's second row).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// `claim <id> <agent> --stacked`, then `report <id> in_progress`.
    Claim { task: String, agent: String, worktree: Option<PathBuf> },
    Report { task: String, status: TaskStatus, summary: String },
    Progress(String),
    Checkpoint(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LockCard {
    pub path: String,
    /// Any `vajra-campaign.sh` process is running now (the process table, read once when the lock was found).
    pub script_running: bool,
    pub text: &'static str,
    /// Repair (`unlock --if-stale`) is offered; false once the engine answered that it doesn't have it.
    pub repair: bool,
}

/// What the run card shows of the record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RecordCard {
    pub campaign: String,
    pub notes: Vec<String>,
    pub lock: Option<LockCard>,
    /// `verify` failed after a write was killed: the record can't be trusted, and the run can't be verified.
    pub signature_broken: bool,
}

/// The record a run writes. `CoreRecord` is the real one; the protocol takes any.
pub trait RunRecord: Send + Sync {
    /// Round 1: the campaign and the task graph. `Err`: the script's first error line.
    fn plan(&self, p: &PlanRecord) -> Result<(), String>;
    fn send(&self, e: Event);
    fn card(&self) -> RecordCard;
    /// The lock card's Repair.
    fn repair(&self) -> RecordCard;
    /// Wait until every event sent so far has run.
    fn flush(&self);
}

enum Msg {
    Event(Event),
    Flush(mpsc::Sender<()>),
}

struct Writer {
    cfg: RecordConfig,
    bridge: Bridge,
    card: Mutex<RecordCard>,
}

/// The record through the bridge, with one writer thread for the events.
pub struct CoreRecord {
    w: Arc<Writer>,
    tx: Mutex<mpsc::Sender<Msg>>,
}

/// The first line of what a failed call said: stderr, else stdout, else its exit.
fn first_line(o: &Output) -> String {
    let line = o.stderr.lines().chain(o.stdout.lines()).map(str::trim).find(|l| !l.is_empty()).map(str::to_string);
    let line = line.unwrap_or_else(|| if o.timed_out { "timed out".into() } else { format!("exited {:?}", o.code) });
    bridge_text::one_line(&line, 300)
}

/// A call's outcome: `Ok`, or why not, and whether it may be retried.
enum Fail {
    /// A clean non-zero exit.
    Exit(String),
    /// Killed at its timeout: never retried.
    Timeout(String),
    /// Refused before spawning, or the spawn failed.
    Never(String),
}

impl Fail {
    fn why(&self) -> &str {
        match self {
            Fail::Exit(w) | Fail::Timeout(w) | Fail::Never(w) => w,
        }
    }
}

impl Writer {
    fn call(&self, req: &Request) -> Result<Output, Fail> {
        match self.bridge.run(req) {
            Ok(o) if o.exit() == Some(0) => Ok(o),
            Ok(o) => {
                let why = first_line(&o);
                if o.stderr.contains("campaign is locked (") {
                    self.found_lock();
                } else if o.stderr.contains("tasks.json is locked") {
                    self.note("the task graph is locked by another writer; it clears itself after 2 minutes".into());
                }
                Err(if o.timed_out { Fail::Timeout(why) } else { Fail::Exit(why) })
            }
            Err(e @ BridgeError::Invalid(_)) => Err(Fail::Never(e.to_string())),
            Err(e) => Err(Fail::Never(e.to_string())),
        }
    }

    /// A best-effort write: one retry after a clean non-zero exit; after a timeout, `verify` instead.
    fn write(&self, req: &Request) -> Result<(), String> {
        let first = match self.call(req) {
            Ok(_) => return Ok(()),
            Err(f) => f,
        };
        match first {
            Fail::Exit(_) => self.call(req).map(|_| ()).map_err(|f| f.why().to_string()),
            Fail::Timeout(why) => {
                self.verify();
                Err(why)
            }
            Fail::Never(why) => Err(why),
        }
    }

    fn verify(&self) {
        let ok = matches!(self.bridge.run(&Request::CampaignVerify { slug: self.cfg.slug.clone() }), Ok(o) if o.exit() == Some(0));
        if !ok {
            crate::lock(&self.card).signature_broken = true;
        }
    }

    fn note(&self, n: String) {
        let mut c = crate::lock(&self.card);
        if !c.notes.contains(&n) && c.notes.len() < 32 {
            c.notes.push(n);
        }
    }

    fn found_lock(&self) {
        let running = matches!(self.bridge.run(&Request::ProcessList), Ok(o) if o.stdout.lines().any(|l| l.contains("vajra-campaign.sh")));
        let path = self.cfg.home.join(".claude/vajra/campaigns").join(&self.cfg.slug).join(".lock");
        let mut c = crate::lock(&self.card);
        let repair = c.lock.as_ref().is_none_or(|l| l.repair);
        c.lock = Some(LockCard { path: path.to_string_lossy().into_owned(), script_running: running, text: LOCK_TEXT, repair });
    }

    fn requests(&self, e: &Event) -> Vec<(Request, &'static str)> {
        let slug = self.cfg.slug.clone();
        match e {
            Event::Claim { task, agent, worktree } => vec![
                (Request::TasksClaim { slug: slug.clone(), id: task.clone(), worker: agent.clone(), worktree: worktree.clone() }, "claim"),
                (Request::TasksReport { slug, id: task.clone(), status: TaskStatus::InProgress, summary: None }, "report"),
            ],
            Event::Report { task, status, summary } => {
                vec![(Request::TasksReport { slug, id: task.clone(), status: *status, summary: Some(bridge_text::one_line(summary, SUMMARY_MAX)) }, "report")]
            }
            Event::Progress(t) => vec![(Request::CampaignProgress { slug, text: bridge_text::one_line(t, PROGRESS_MAX) }, "progress")],
            Event::Checkpoint(l) => vec![(Request::CampaignCheckpoint { slug, label: bridge_text::one_line(l, 200) }, "checkpoint")],
        }
    }

    fn event(&self, e: &Event) {
        for (req, what) in self.requests(e) {
            // The progress text is checked as strictly as the record's own variants.
            if let Request::CampaignProgress { text, .. } = &req
                && bridge_text::text(text, PROGRESS_MAX, true).is_err()
            {
                self.note(format!("the record skipped a progress line it couldn't send ({what})"));
                continue;
            }
            if let Err(why) = self.write(&req) {
                self.note(format!("the run record's {what} failed: {why}"));
            }
        }
    }

    fn plan(&self, p: &PlanRecord) -> Result<(), String> {
        let goal = bridge_text::one_line(&p.goal, GOAL_MAX);
        let create = Request::CampaignCreate { slug: self.cfg.slug.clone(), goal: goal.clone(), features: p.features.clone() };
        match self.call(&create) {
            Ok(_) => {}
            Err(Fail::Exit(why)) if why.contains("campaign already exists") => self.reuse(&goal, &p.features)?,
            Err(f) => return Err(f.why().to_string()),
        }
        for t in &p.tasks {
            let add = Request::TasksAdd {
                slug: self.cfg.slug.clone(),
                id: t.id.clone(),
                title: bridge_text::one_line(&t.title, TITLE_MAX),
                owner: t.agent.clone(),
                deps: t.deps.clone(),
                files: t.files.clone(),
                acceptance: t.criteria.clone(),
                branch: t.branch.clone(),
            };
            // `add` of the same definition answers "unchanged", so a retry is safe.
            self.write(&add)?;
        }
        self.write(&Request::TasksValidate { slug: self.cfg.slug.clone() })
    }

    /// An existing campaign, reused only for a resume the core made, and only when it is exactly this run's.
    fn reuse(&self, goal: &str, features: &[Feature]) -> Result<(), String> {
        let slug = &self.cfg.slug;
        if !self.cfg.resumed {
            return Err(format!("campaign {slug} already exists, and this run didn't create it"));
        }
        let o = self.call(&Request::CampaignShow { slug: slug.clone() }).map_err(|f| f.why().to_string())?;
        let v: Value = serde_json::from_str(&o.stdout).map_err(|_| format!("campaign {slug} could not be read back"))?;
        let c = &v["campaign"];
        let same_features = v["features"].as_array().is_some_and(|fs| {
            fs.len() == features.len()
                && fs.iter().zip(features).all(|(x, f)| x["id"] == f.id.as_str() && x["description"] == f.desc.as_str() && x["passes"] == false)
        });
        let ok = v["signature_valid"] == true && c["slug"] == slug.as_str() && c["status"] == "active" && c["goal"] == goal && same_features;
        if ok { Ok(()) } else { Err(format!("campaign {slug} already exists and isn't this run's (goal, criteria, status or signature differ)")) }
    }
}

impl CoreRecord {
    /// `bridge` runs with the record's cwd and `RECORD_TIMEOUT` (the caller sets both).
    pub fn new(cfg: RecordConfig, bridge: Bridge) -> Result<Self, String> {
        cfg.check()?;
        if bridge.record_cwd.is_none() {
            return Err("the run record has no working directory".into());
        }
        let card = RecordCard { campaign: cfg.slug.clone(), ..RecordCard::default() };
        let w = Arc::new(Writer { cfg, bridge, card: Mutex::new(card) });
        let (tx, rx) = mpsc::channel::<Msg>();
        let worker = Arc::clone(&w);
        std::thread::Builder::new()
            .name("vajra-record".into())
            .spawn(move || {
                for m in rx {
                    match m {
                        Msg::Event(e) => worker.event(&e),
                        Msg::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })
            .map_err(|e| format!("the run record could not start: {e}"))?;
        Ok(CoreRecord { w, tx: Mutex::new(tx) })
    }
}

impl RunRecord for CoreRecord {
    fn plan(&self, p: &PlanRecord) -> Result<(), String> {
        self.w.plan(p)
    }

    fn send(&self, e: Event) {
        if crate::lock(&self.tx).send(Msg::Event(e)).is_err() {
            self.w.note("the run record stopped; later steps were not recorded".into());
        }
    }

    fn card(&self) -> RecordCard {
        crate::lock(&self.w.card).clone()
    }

    fn repair(&self) -> RecordCard {
        let out = self.w.bridge.run(&Request::CampaignUnlockStale { slug: self.w.cfg.slug.clone() });
        let mut c = crate::lock(&self.w.card);
        match out {
            Ok(o) if o.exit() == Some(0) => c.lock = None,
            Ok(o) => {
                let why = first_line(&o);
                if let Some(l) = c.lock.as_mut() {
                    // Exit 2 is the script's usage error: an engine from before `unlock --if-stale`.
                    l.repair &= o.code != Some(2);
                }
                drop(c);
                self.w.note(format!("Repair didn't remove the lock: {why}"));
                c = crate::lock(&self.w.card);
            }
            Err(e) => {
                drop(c);
                self.w.note(format!("Repair didn't run: {e}"));
                c = crate::lock(&self.w.card);
            }
        }
        c.clone()
    }

    fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        if crate::lock(&self.tx).send(Msg::Flush(tx)).is_ok() {
            let _ = rx.recv();
        }
    }
}

/// Close an orphan: a campaign named in a core run's `seats.json` that is still active after its run ended.
pub fn close_abandoned(bridge: &Bridge, slug: &str) -> Result<(), String> {
    match bridge.run(&Request::CampaignClose { slug: slug.to_string(), status: CloseStatus::Abandoned }) {
        Ok(o) if o.exit() == Some(0) => Ok(()),
        Ok(o) => Err(first_line(&o)),
        Err(e) => Err(e.to_string()),
    }
}

/// `show <slug> --json`'s status, if the script answers.
pub fn campaign_status(bridge: &Bridge, slug: &str) -> Option<String> {
    let o = bridge.run(&Request::CampaignShow { slug: slug.to_string() }).ok().filter(|o| o.exit() == Some(0))?;
    let v: Value = serde_json::from_str(&o.stdout).ok()?;
    v["campaign"]["status"].as_str().map(str::to_string)
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
