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

pub use super::record_ledger::{Ledger, ReadBack};
use crate::vajra::bridge::{Bridge, BridgeError, CloseStatus, Feature, Output, Request, TaskStatus};
use crate::vajra::bridge_record::{self as bridge_text, GOAL_MAX, PROGRESS_MAX, SUMMARY_MAX, TITLE_MAX};

/// One campaign or task write's timeout: the script waits up to 10 s for its lock, then does its work.
pub const RECORD_TIMEOUT: Duration = Duration::from_secs(60);

/// The whole plan-time record (create, every add, validate), which runs under the protocol lock: past this, the
/// remaining calls aren't started and the run ends before dispatch.
pub const PLAN_DEADLINE: Duration = Duration::from_secs(180);

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
    /// The record may be called tamper-evident: false for an `outside_guard` run, whose agents could reach the key.
    pub tamper_evident: bool,
}

/// Where the core keeps its ledger, and how it reads the campaign back (`record_app.rs`: the app's data dir and the
/// held reader). Here so this module stays without `std::fs`.
pub trait LedgerStore: Send + Sync {
    /// The ledger saved for this campaign, if any.
    fn load(&self) -> Result<Option<Ledger>, String>;
    fn save(&self, l: &Ledger) -> Result<(), String>;
    /// The campaign and task graph as they are now, signature and features definition checked.
    fn read_back(&self, slug: &str) -> Result<ReadBack, String>;
    /// The sha256 of the evidence file the core is about to hand a task close (the bytes the script will hash).
    fn evidence_sha256(&self, path: &std::path::Path) -> Result<String, String>;
}

/// A campaign write through the ledger: marked pending (and saved) before it runs, so a crash mid-write leaves it
/// unknown; applied after exit 0; unknown for good after a timeout. A ledger that can't be saved stops the write.
fn ledgered(store: &dyn LedgerStore, ledger: &Mutex<Ledger>, req: &Request, run: impl FnOnce() -> Result<Output, Fail>) -> Result<Output, Fail> {
    if !req.is_campaign_write() {
        return run();
    }
    let what = format!("{req:?}").split([' ', '{', '(']).next().unwrap_or("write").to_string();
    // A close's evidence is hashed before the script reads it: bytes swapped in between hash differently, so the
    // record then fails its check rather than counting someone else's evidence as the core's.
    let evidence = match req {
        Request::TasksClose { evidence, .. } => {
            Some(store.evidence_sha256(evidence).map_err(|e| Fail::Never(format!("the task's evidence can't be read: {e}")))?)
        }
        _ => None,
    };
    {
        let mut l = crate::lock(ledger);
        l.pending = Some(what.clone());
        store.save(&l).map_err(|e| Fail::Never(format!("the run record's ledger could not be saved: {e}")))?;
    }
    let r = run();
    let mut l = crate::lock(ledger);
    l.pending = None;
    match &r {
        Ok(o) => l.applied(req, &o.stdout, evidence.as_deref()),
        Err(Fail::Timeout(_)) => l.uncertain = Some(what),
        Err(_) => {}
    }
    // A failed save leaves the saved ledger pending: the record then reads as not verified, never as verified.
    let _ = store.save(&l);
    r
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
    ledger: Mutex<Ledger>,
    store: Arc<dyn LedgerStore>,
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
        ledgered(self.store.as_ref(), &self.ledger, req, || self.run(req))
    }

    fn run(&self, req: &Request) -> Result<Output, Fail> {
        match self.bridge.run(req) {
            Ok(o) if o.exit() == Some(0) => Ok(o),
            Ok(o) => {
                let why = first_line(&o);
                if o.stderr.contains("campaign is locked (") {
                    self.found_lock();
                } else if o.stderr.contains("tasks.json is locked") {
                    self.note("the task graph is locked by another writer; it clears itself after 2 minutes".into());
                }
                // Exit 3 with "signature INVALID" is the campaign script's `require_valid`: the record is broken now.
                if o.code == Some(3) && o.stderr.contains("signature INVALID") {
                    crate::lock(&self.card).signature_broken = true;
                    return Err(Fail::Never(why));
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
        let started = std::time::Instant::now();
        let in_time = || if started.elapsed() < PLAN_DEADLINE { Ok(()) } else { Err(format!("the run record took longer than {} s", PLAN_DEADLINE.as_secs())) };
        let goal = bridge_text::one_line(&p.goal, GOAL_MAX);
        let create = Request::CampaignCreate { slug: self.cfg.slug.clone(), goal: goal.clone(), features: p.features.clone() };
        match self.call(&create) {
            Ok(_) => {}
            Err(Fail::Exit(why)) if why.contains("campaign already exists") => self.reuse(&goal, &p.features)?,
            Err(f) => return Err(f.why().to_string()),
        }
        for t in &p.tasks {
            in_time()?;
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
        in_time()?;
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
        if !ok {
            return Err(format!("campaign {slug} already exists and isn't this run's (goal, criteria, status or signature differ)"));
        }
        // And everything in it is what the core wrote: a write by anyone else since makes it someone else's.
        self.verify_record().map_err(|why| format!("campaign {slug} already exists and isn't only the core's: {why}"))
    }

    /// The campaign and task graph read back are exactly what the core's ledger says it wrote.
    fn verify_record(&self) -> Result<(), String> {
        let rb = self.store.read_back(&self.cfg.slug)?;
        crate::lock(&self.ledger).check(&rb)
    }
}

impl CoreRecord {
    /// `bridge` runs with the record's cwd and `RECORD_TIMEOUT` (the caller sets both). `store` keeps this campaign's
    /// ledger: a resume continues the saved one; a fresh run must find none.
    pub fn new(cfg: RecordConfig, bridge: Bridge, store: Arc<dyn LedgerStore>) -> Result<Self, String> {
        cfg.check()?;
        if bridge.record_cwd.is_none() {
            return Err("the run record has no working directory".into());
        }
        let ledger = match (store.load()?, cfg.resumed) {
            (Some(l), true) if l.slug == cfg.slug => l,
            (Some(_), true) => return Err("the saved run record ledger is for another campaign".into()),
            (Some(_), false) => return Err(format!("campaign {} already has a run record ledger, and this run didn't write it", cfg.slug)),
            (None, _) => Ledger::new(&cfg.slug),
        };
        let card = RecordCard { campaign: cfg.slug.clone(), tamper_evident: cfg.trust != "outside_guard", ..RecordCard::default() };
        let w = Arc::new(Writer { cfg, bridge, card: Mutex::new(card), ledger: Mutex::new(ledger), store });
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

    /// The record is the core's own: the campaign and task graph read back match every write the core made, and none
    /// it didn't. Call after `flush`. `Err`: why not; the run can't be verified on this record.
    pub fn verify_record(&self) -> Result<(), String> {
        self.w.verify_record()
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
/// The close goes through the campaign's ledger when it has one, so the record stays the core's.
pub fn close_abandoned(bridge: &Bridge, slug: &str, store: &dyn LedgerStore) -> Result<(), String> {
    let req = Request::CampaignClose { slug: slug.to_string(), status: CloseStatus::Abandoned };
    let run = || match bridge.run(&req) {
        Ok(o) if o.exit() == Some(0) => Ok(o),
        Ok(o) if o.timed_out => Err(Fail::Timeout(first_line(&o))),
        Ok(o) => Err(Fail::Exit(first_line(&o))),
        Err(e) => Err(Fail::Never(e.to_string())),
    };
    let r = match store.load()? {
        Some(l) if l.slug == slug => ledgered(store, &Mutex::new(l), &req, run),
        Some(_) => return Err("the saved run record ledger is for another campaign".into()),
        None => run(),
    };
    r.map(|_| ()).map_err(|f| f.why().to_string())
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
