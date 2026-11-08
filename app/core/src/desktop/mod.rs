//! The app's command layer: one `App` owns the bus, the PTY supervisor, the
//! control service and the read-only Vajra bridge, and answers each UI call.
//! `main.rs` only maps Tauri commands onto these methods.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::bus::{self, Bus, Config, Event, Subscriber};
use crate::control::service::{Actor, ControlError, Service, WorkerInfo};
use crate::adapter::{Engine, adapter_for};
use crate::control::ledger::Ledger;
use crate::ids;
use crate::lock;
use crate::crew::record::{JournalEntry, Meta, parse_journal, parse_meta};
use crate::crew::redact::Redactor;
use crate::crew::tailer::{Tailer, Transcript};
use crate::crew::tree::{self, Node, Signals, Status, Worker};
use crate::logging::Logger;
use crate::paths;
use crate::sessions::argv::{self, InvalidInput, RunInput};
use crate::sessions::{SpawnSpec, Supervisor};
use crate::vajra::allowlist::Allowlist;
use crate::vajra::bridge::{Bridge, BridgeError, Output, Request, parse_kb_hits};
use crate::vajra::preflight;
use crate::vajra::state::Reader;

/// Every command answers `Ok(value)` or `Err({error, detail?})`.
pub type Res = Result<Value, Value>;
/// Emits a `changed` payload to the UI (see `App::set_change_sink`).
pub type ChangeSink = Box<dyn Fn(Value) + Send + Sync>;

/// Bus events coalesce for this long before one `changed` is emitted.
const COALESCE: Duration = Duration::from_millis(150);
/// Upper bound for one `tail` / `read_spill` page.
const MAX_PAGE: usize = 64 * 1024;
/// Most rows and text one `stream_window` call returns: a view shows tens of rows, so this is a ceiling, not a default.
const WINDOW_MAX_LINES: usize = 500;
const WINDOW_MAX_BYTES: usize = 1024 * 1024;
/// Largest worker meta file and workflow journal read whole (both are Claude Code files; over-limit ones are skipped).
const MAX_META_BYTES: u64 = 64 * 1024;
const MAX_JOURNAL_BYTES: u64 = 8 * 1024 * 1024;
/// Evidence checks run this many at a time, each for at most `EVIDENCE_TIMEOUT`.
const EVIDENCE_WORKERS: usize = 4;
const EVIDENCE_TIMEOUT: Duration = Duration::from_secs(4);
/// The whole of one `evidence_list` call (every batch together) may take this long. A campaign can hold 500 tasks, so
/// without it a slow check per task would occupy the request for many minutes; checks not started by then read `checking`
/// and the next call (the UI re-asks while any row is `checking`) continues from the cached verdicts.
const EVIDENCE_BUDGET: Duration = Duration::from_secs(5);
/// Verdicts kept for unchanged evidence (cleared when it grows past this).
const EVIDENCE_CACHE_MAX: usize = 512;
// A check must be able to get its full timeout inside one call's budget, or "checking" could never turn into a verdict.
const _: () = assert!(EVIDENCE_TIMEOUT.as_secs() <= EVIDENCE_BUDGET.as_secs());
/// A check that timed out or could not start is remembered as unverified this long, so a hung check is not re-run (and the
/// checks queued behind it starved) on every call.
/// The evidence file read for a criterion's detail, the output tail kept per failed command, and how many commands are listed.
const PROOF_FILE_MAX: u64 = 64 * 1024;
const PROOF_LOG_TAIL: u64 = 4 * 1024;
const PROOF_CMDS_MAX: usize = 50;
const PROOF_TEXT_MAX: usize = 2000;
/// How long the HEAD and cleanliness of a repo are reused between `evidence_list` calls (kept apart from the verdict cache,
/// so an edit turns a pass stale on the next call). Git gets at most this much of one call's budget.
const WORKTREE_STATE_TTL: Duration = Duration::from_secs(5);
/// What the run itself writes into its repo, so it is not an edit to the verified tree: `vajra-verify-work.sh` excludes the
/// walkthrough dir (evidence, `cmd-N.log`, `diff.patch`, `untracked.txt`) from its own tree-stability check, and crew
/// worktrees live under `.claude/worktrees/`. Keep in step with `EXCL` in `scripts/vajra-verify-work.sh`.
const RUN_OWN_OUTPUT: [&str; 2] = ["data/walkthroughs", ".claude/worktrees"];
/// A repo whose state could not be read (git failed or was slow) is not asked again for this long.
const WORKTREE_STATE_UNKNOWN_TTL: Duration = Duration::from_secs(30);
const WORKTREE_STATE_TIMEOUT: Duration = Duration::from_secs(2);
const EVIDENCE_FAILED_TTL: Duration = Duration::from_secs(60);
/// Evidence files larger than this are always re-checked (their bytes are the cache key).
const EVIDENCE_KEY_MAX: u64 = 16 * 1024;
/// A verdict for evidence checked in a repo depends on the repo's state, which the key cannot see: it is reused this long.
const EVIDENCE_LIVE_TTL: Duration = Duration::from_secs(5);
const PTY_ROWS: u16 = 40;
const PTY_COLS: u16 = 120;

fn fail(code: &str, detail: impl std::fmt::Display) -> Value {
    json!({"error": code, "detail": detail.to_string()})
}

fn ctl<T>(r: Result<T, ControlError>) -> Result<T, Value> {
    r.map_err(|e| e.to_json())
}

fn invalid_input(e: InvalidInput) -> Value {
    invalid(e.field, e.reason)
}

/// A run session started (or resumed) by this app.
#[derive(Clone, Debug)]
struct Run {
    session_id: String,
    run_id: String,
    campaign: String,
    cwd: PathBuf,
    /// Every nonce this run was launched or resumed with: its transcripts and
    /// a replayed resume may show any of them, after its session ended too.
    nonces: Redactor,
}

/// Untracked file names `diff` lists.
const UNTRACKED_MAX: usize = 200;
/// Most integrity verifies running at once, across every `campaign_integrity` call.
const VERIFY_SLOTS: usize = 4;
/// Longest a verify waits for a free slot (the total budget for a call); past it the call reads "unknown" and starts nothing.
const VERIFY_QUEUE_BUDGET: Duration = Duration::from_secs(15);
/// How long a real verdict is reused. The badge is a trust signal, so the window is short; it bounds the verify
/// processes started by a burst of `changed` events to one per campaign per window (and `refresh` skips it).
const VERDICT_TTL: Duration = Duration::from_secs(5);

/// One in-flight verify, shared by every caller that asks for the same campaign while it runs.
type Flight = Arc<(Mutex<Option<&'static str>>, Condvar)>;
enum Verdict {
    Fresh(Duration, &'static str),
    Flying(Flight),
}

/// The integrity cache. Every check (a listing or explicit verify, or a report read) takes a ticket from
/// `App::tickets` when it starts, before it reads anything, and `last` keeps the newest ticket that settled per
/// campaign. A check that started before the newest settled one can add a failure but never replace a verdict with
/// "ok" or clear it: an older success must not hide a newer failure, and a failure is never lost to ordering.
#[derive(Default)]
struct Verdicts {
    by_slug: HashMap<String, Verdict>,
    last: HashMap<String, u64>,
    /// A forced list refresh asked for a verify that starts after this ticket. Until one settles, nothing older may
    /// answer for the campaign: not a kept verdict, not a verify that was already running when the refresh was asked.
    floor: HashMap<String, u64>,
}

/// Ends a flight even if the verify panics, so waiters are never left hanging.
struct FlightGuard<'a> {
    app: &'a App,
    slug: &'a str,
    flight: Flight,
    done: bool,
}
impl Drop for FlightGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            // Forget only this flight; a newer verdict that replaced it stays.
            let mut m = lock(&self.app.verdicts);
            if matches!(m.by_slug.get(self.slug), Some(Verdict::Flying(f)) if Arc::ptr_eq(f, &self.flight)) {
                m.by_slug.remove(self.slug);
            }
            drop(m);
            *lock(&self.flight.0) = Some("unknown");
            self.flight.1.notify_all();
        }
    }
}
/// A taken verify slot; freed when dropped, even if the verify panics.
struct VerifySlot<'a>(&'a (Mutex<usize>, Condvar));
impl Drop for VerifySlot<'_> {
    fn drop(&mut self) {
        *lock(&self.0 .0) += 1;
        self.0 .1.notify_one();
    }
}
const LIST_VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

enum Outcome {
    /// The check ran: green or not, with why it was not (`None` when green).
    Verdict(bool, Option<Value>),
    TimedOut,
    /// The check could not start; the code is a `reason` code.
    Failed(&'static str),
}

/// `(HEAD, dirty)` of a repo.
type RepoState = (String, bool);
/// Per repo: when `RepoState` was read and what it was (`None`: the read failed).
type WorktreeCache = HashMap<PathBuf, (std::time::Instant, Option<RepoState>)>;
type EvKey = (PathBuf, Vec<u8>, Option<String>, Option<String>, Option<PathBuf>);

pub struct App {
    data: PathBuf,
    home: PathBuf,
    claude: PathBuf,
    pub bus: Arc<Bus>,
    pub sup: Supervisor,
    pub control: Arc<Mutex<Service>>,
    bridge: Bridge,
    /// Longest one campaign's integrity check may run while listing; a check that overruns reads "unknown".
    verify_timeout: Duration,
    /// Free verify slots (see `VERIFY_SLOTS`) and the longest a call waits for one.
    verify_slots: (Mutex<usize>, Condvar),
    verify_queue_budget: Duration,
    /// slug -> a real verdict with the time it was taken, or the verify now running for it. "unknown" is never kept.
    verdicts: Mutex<Verdicts>,
    /// Integrity-check tickets, taken when a check starts (see `Verdicts`).
    tickets: std::sync::atomic::AtomicU64,
    verdict_ttl: Duration,
    /// Monotonic time since start; tests replace it.
    clock: Arc<dyn Fn() -> Duration + Send + Sync>,
    /// `None` until `~/.claude` exists.
    reader: Mutex<Option<Reader>>,
    /// By bus stream id.
    runs: Mutex<HashMap<String, Run>>,
    /// Worker transcripts by agent id, tailed into stream `w-<agent_id>`.
    transcripts: Mutex<HashMap<String, Transcript>>,
    /// Held from the one-active-run check until the run is recorded, so two
    /// Start clicks cannot both pass the check.
    launching: Mutex<()>,
    /// Run ids this process handed out. A resume replaces its run's entry, so `runs` alone would forget one.
    issued: Mutex<std::collections::HashSet<String>>,
    /// ADR-13: the Lead chat (asks, the guard, the claude binary).
    lead: lead::LeadState,
    /// Plan id -> the `plan_sha256` the last `inbox` call showed for it. `approve_plan` needs it to match.
    shown: Mutex<HashMap<String, String>>,
    /// Plan ids being decided right now: one decision per plan at a time.
    deciding: Mutex<std::collections::HashSet<String>>,
    pub log: Option<Arc<Logger>>,
    /// How long one evidence check may run before its criterion reads `unverified`.
    evidence_timeout: Duration,
    /// Total time one `evidence_list` call may spend running checks.
    evidence_budget: Duration,
    /// Verdicts of finished checks, keyed on the evidence file's exact bytes plus head, mode and repo, so a rewritten file
    /// (whatever it does to size or mtime) is always re-checked. Archived bundles depend on nothing else and are kept; a
    /// verdict from a live repo expires after `EVIDENCE_LIVE_TTL`.
    evidence_cache: Mutex<HashMap<EvKey, (bool, Option<std::time::Instant>)>>,
    /// Why each non-green verdict in `evidence_cache` was not green (same key, same lifetime), so a cached answer says why too.
    evidence_why: Mutex<HashMap<EvKey, Value>>,
    /// `(HEAD, dirty)` of a run repo, from `git rev-parse HEAD` and `git status`; only successful reads are kept.
    worktree_state: Mutex<WorktreeCache>,
    /// Campaigns with an evidence sweep running: a second call (a change event) answers from the cache instead of starting another.
    sweeping: Mutex<std::collections::HashSet<String>>,
    /// ADR-15: the project records under `<data>/projects`.
    projects: crate::projects::store::Store,
    /// Action receipts and per-project revisions (see `state`).
    acts: Mutex<state::Hub>,
    /// Where receipt-driven `changed` events go (see `state::set_change_sink`).
    sink: Mutex<Option<ChangeSink>>,
}

impl App {
    /// Open everything under `data` (the app data dir) for the user whose
    /// home is `home`. Nothing under `home` is written.
    pub fn open(data: &Path, home: &Path) -> std::io::Result<Self> {
        paths::ensure_private_dir(data)?;
        let logs = paths::logs_dir(data);
        let log = Logger::open(&logs).ok().map(Arc::new);
        let control_log = Logger::open(&logs.join("control")).ok();
        let claude = home.join(".claude");
        let bus = Arc::new(Bus::open(&data.join("spill"), Config::default())?);
        let svc = Service::open(data, claude.join("vajra/audit.log"), control_log)?;
        let projects = crate::projects::store::Store::open(data, home)?;
        let acts = Mutex::new(state::Hub::open(data)?);
        Ok(Self {
            projects,
            acts,
            sink: Mutex::new(None),
            data: data.to_path_buf(),
            home: home.to_path_buf(),
            lead: lead::LeadState::new(home),
            shown: Mutex::new(HashMap::new()),
            deciding: Mutex::new(Default::default()),
            bridge: Bridge::new(claude.join("skills/vajra/scripts")).with_home(home.to_path_buf()),
            verify_timeout: LIST_VERIFY_TIMEOUT,
            verify_slots: (Mutex::new(VERIFY_SLOTS), Condvar::new()),
            verify_queue_budget: VERIFY_QUEUE_BUDGET,
            verdicts: Mutex::new(Verdicts::default()),
            tickets: std::sync::atomic::AtomicU64::new(0),
            verdict_ttl: VERDICT_TTL,
            clock: {
                let t0 = std::time::Instant::now();
                Arc::new(move || t0.elapsed())
            },
            reader: Mutex::new(None),
            claude,
            sup: Supervisor::new(bus.clone(), log.clone()),
            bus,
            control: Arc::new(Mutex::new(svc)),
            runs: Mutex::new(HashMap::new()),
            evidence_timeout: EVIDENCE_TIMEOUT,
            evidence_budget: EVIDENCE_BUDGET,
            evidence_cache: Mutex::new(HashMap::new()),
            evidence_why: Mutex::new(HashMap::new()),
            worktree_state: Mutex::new(HashMap::new()),
            sweeping: Mutex::new(std::collections::HashSet::new()),
            transcripts: Mutex::new(HashMap::new()),
            launching: Mutex::new(()),
            issued: Mutex::new(Default::default()),
            log,
        })
    }

    pub fn note(&self, event: &'static str, fields: &[(&'static str, u64)]) {
        if let Some(l) = &self.log {
            let _ = l.log(event, fields);
        }
    }

    fn allow(&self) -> Result<Allowlist, Value> {
        Allowlist::new(&self.claude, &self.data).map_err(|e| fail("claude_dir_missing", e))
    }

    fn with_reader<T>(&self, f: impl FnOnce(&mut Reader) -> Result<T, Value>) -> Result<T, Value> {
        let mut g = lock(&self.reader);
        if g.is_none() {
            *g = Some(Reader::new(self.allow()?));
        }
        f(g.as_mut().expect("reader set above"))
    }

    /// The run a UI view names. A minute-level run id alone is not an identity, so the campaign must match too: two
    /// runs can never be confused for one another here.
    fn run_by_id(&self, slug: &str, run_id: &str) -> Option<(String, Run)> {
        lock(&self.runs).iter().find(|(_, r)| r.run_id == run_id && r.campaign == slug).map(|(k, r)| (k.clone(), r.clone()))
    }

    /// The first minute at or after `now` that no known run and no campaign on disk uses. The workflow takes a
    /// `YYYYMMDD-HHMM` run id (it names branches and the default campaign), so uniqueness comes from the choice of minute.
    /// If the next 120 minutes are all taken (a forged index can fill them) nothing is issued: there is no fallback to
    /// a used id, because reusing one would hand a new run another run's guarded record.
    fn fresh_run_id(&self, now: u64) -> Result<(u64, String), Value> {
        let on_disk: Vec<String> = self.with_reader(|r| Ok(r.campaigns().unwrap_or_default())).unwrap_or_default();
        // Run ids of earlier app lives: after a restart in the same minute, an older campaign's id is still taken.
        let indexed = runindex::run_ids(&self.data.join("runs.jsonl"));
        // The guarded run record (`control/lead/<id>/`) counts too: the index is worker-writable and can be emptied, and
        // reusing an id would keep that record's Lead session and approval-only marker for a different run.
        let recorded = |id: &str| self.data.join("control").join("lead").join(id).symlink_metadata().is_ok();
        // `deliver-{id}` is the workflow's default campaign name (`CAMPAIGN = RESUME || deliver-${RUN}` in workflows/vajra-deliver.js).
        let used = |id: &str| lock(&self.runs).values().any(|r| r.run_id == id) || lock(&self.issued).contains(id) || indexed.contains(id) || on_disk.iter().any(|c| c == &format!("deliver-{id}")) || recorded(id);
        let pick = (0..120u64)
            .map(|k| now + 60 * k)
            .map(|t| (t, ids::run_id_at(t)))
            .find(|(_, id)| !used(id))
            .ok_or_else(|| fail("run_id_exhausted", "no unused run id in the next 120 minutes; check runs.jsonl"))?;
        lock(&self.issued).insert(pick.1.clone());
        Ok(pick)
    }

    // ---------------------------------------------------------------- start

    pub fn preflight(&self) -> Res {
        let allow = self.allow()?;
        let rows = preflight::run(&allow, &self.bridge, &preflight::path_dirs());
        let ok = preflight::start_enabled(&rows);
        let rows: Vec<Value> =
            rows.iter().map(|r| json!({"name": r.name, "ok": r.ok, "detail": r.detail, "fix": r.fix})).collect();
        Ok(json!({"rows": rows, "start_enabled": ok}))
    }

    pub fn list_campaigns(&self) -> Res {
        self.list_campaigns_opt(false)
    }

    /// `refresh` re-verifies every campaign now, whatever the last verdict's age.
    pub fn list_campaigns_opt(&self, refresh: bool) -> Res {
        // Summaries are read under the reader lock; the verify subprocesses run after it is released, so a slow
        // or hung verify cannot block any other reader call.
        let mut list: Vec<Value> = self.with_reader(|r| {
            let slugs = r.campaigns().map_err(|e| fail("io_error", e))?;
            Ok(slugs
                .iter()
                .map(|s| match r.campaign(s) {
                    Ok(c) => json!({
                        "slug": s, "goal": c.goal, "status": c.status,
                        "created": c.created, "updated": c.updated,
                        "passing": c.passing, "total": c.total,
                        "last_integration": c.last_integration.map(|i| json!({"branch": i.branch, "sha": i.sha, "from": i.from})),
                        "checkpoints": c.checkpoints,
                    }),
                    // A campaign dir the reader cannot summarise is still listed.
                    Err(e) => json!({"slug": s, "error": e.to_string()}),
                })
                .collect())
        })?;
        // The listing never verifies: a campaign whose last verdict is still fresh shows it, every other one reads
        // "checking" and the UI asks `campaign_integrity` for it (so one slow verify cannot hold the list up).
        for c in list.iter_mut() {
            let slug = c["slug"].as_str().unwrap_or_default().to_string();
            if refresh {
                // The follow-up `campaign_integrity` calls carry no refresh flag, so the forced refresh has to reach them
                // through the cache: drop the kept verdict (a verify already running stays, and is waited for).
                self.forget_fresh(&slug);
            }
            c["integrity"] = json!(if refresh { "checking" } else { self.cached_integrity(&slug).unwrap_or("checking") });
        }
        Ok(json!({"campaigns": list}))
    }

    /// The control service steers one session, so one run is active at a time.
    fn no_active_run(&self) -> Result<(), Value> {
        self.reap_locked();
        if self.active_run_count() > 0 {
            return Err(json!({"error": "run_active", "detail": "finish or stop the active run first"}));
        }
        Ok(())
    }

    /// Runs whose PTY is still open.
    pub fn active_run_count(&self) -> usize {
        lock(&self.runs).keys().filter(|s| self.sup.is_running(s).unwrap_or(false)).count()
    }

    /// End the control session once its run's PTY exited, so the Lead cannot
    /// queue to its workers (their messages expire). The run stays in `runs`:
    /// its crew view still reads its workers.
    pub fn reap_runs(&self) {
        // A launch in flight owns the session it just took; it reaps itself.
        // The guard stays bound until `reap_locked` returns, so a launch cannot install a session in between.
        let _guard = match self.launching.try_lock() {
            Ok(g) => g,
            Err(std::sync::TryLockError::WouldBlock) => return,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        };
        self.reap_locked();
    }

    fn reap_locked(&self) {
        let ended: Vec<String> = lock(&self.runs)
            .iter()
            .filter(|(s, _)| !self.sup.is_running(s).unwrap_or(false))
            .map(|(_, r)| r.session_id.clone())
            .collect();
        let mut svc = lock(&self.control);
        for id in ended {
            if svc.has_session(&id) {
                svc.end_session(&id);
            }
        }
    }

    /// Take the control session, build the argv with its nonce, spawn. A
    /// failed build or spawn ends the session again, so nothing is left live.
    pub fn launch(
        &self,
        session_id: &str,
        run_id: &str,
        campaign: &str,
        cwd: &Path,
        argv: impl FnOnce(&str) -> Result<Vec<String>, Value>,
        rotate: bool,
    ) -> Res {
        self.launch_kind(session_id, run_id, campaign, cwd, argv, rotate, true)
    }

    /// `workflow` is false only for the approval-only resume (ADR-14), which starts no new `vajra-deliver`: its run id
    /// is not a workflow identity, so plan ids keep naming the run of the workflow that produced the plan.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn launch_kind(
        &self,
        session_id: &str,
        run_id: &str,
        campaign: &str,
        cwd: &Path,
        argv: impl FnOnce(&str) -> Result<Vec<String>, Value>,
        rotate: bool,
        workflow: bool,
    ) -> Res {
        let _one = lock(&self.launching);
        self.no_active_run()?;
        let nonce = {
            let mut svc = lock(&self.control);
            ctl(svc.start_session(session_id, run_id))?;
            let n = if rotate { ctl(svc.resume_session(session_id)) } else { Ok(svc.nonce(session_id).unwrap_or_default()) };
            match n {
                Ok(n) => n.to_string(),
                Err(e) => {
                    svc.end_session(session_id);
                    return Err(e);
                }
            }
        };
        let stream = format!("run-{session_id}");
        let mut nonces = lock(&self.runs).get(&stream).map(|r| r.nonces.clone()).unwrap_or_default();
        nonces.add(Some(&nonce));
        let spawned = argv(&nonce).and_then(|argv| {
            let spec = SpawnSpec { argv, cwd: cwd.to_path_buf(), rows: PTY_ROWS, cols: PTY_COLS, redact: nonces.clone() };
            self.sup.spawn(&stream, spec).map_err(|e| fail("spawn_failed", e))
        });
        if let Err(e) = spawned {
            lock(&self.control).end_session(session_id);
            return Err(e);
        }
        lock(&self.runs).insert(
            stream.clone(),
            Run { session_id: session_id.into(), run_id: run_id.into(), campaign: campaign.into(), cwd: cwd.to_path_buf(), nonces },
        );
        // Remember which session ran this campaign, so it can be resumed into the same session after a restart.
        let entry = runindex::Entry { campaign: campaign.into(), session_id: session_id.into(), run_id: run_id.into(), cwd: cwd.to_path_buf(), lead_session_id: None };
        if runindex::append(&self.data.join("runs.jsonl"), &entry).is_err() {
            self.note("run_index_failed", &[]);
        }
        // The guarded record of the run's directory (the index above is worker-writable): without it the Lead is
        // refused for this run after a restart, but the launch itself goes on.
        if self.record_run(run_id, cwd, session_id, campaign, workflow, ids::unix_now() as i64).is_err() {
            self.note("lead_cwd_record_failed", &[]);
        }
        Ok(json!({"stream": stream, "session_id": session_id, "run_id": run_id, "campaign": campaign}))
    }

    /// The session and directory to resume each campaign this app started: `claude --resume` needs the prior session
    /// id, and campaign state does not hold it. A campaign started elsewhere (a terminal) is simply absent, and so is
    /// one whose directory no longer exists.
    pub fn resume_targets(&self) -> Res {
        let targets: serde_json::Map<String, Value> = runindex::latest(&self.data.join("runs.jsonl"))
            .into_iter()
            .filter(|(_, e)| e.cwd.is_dir())
            .map(|(k, e)| (k, json!({"session_id": e.session_id, "run_id": e.run_id, "cwd": e.cwd.to_string_lossy()})))
            .collect();
        Ok(json!({"targets": targets}))
    }

    /// The workflow takes only a commit sha, so a branch or ref is resolved
    /// through the read-only git bridge first.
    fn resolve_base(&self, cwd: &Path, base: &str) -> Result<String, Value> {
        if argv::is_sha(base) {
            return Ok(base.to_string());
        }
        let args = vec!["rev-parse".into(), "--verify".into(), "--quiet".into(), format!("{base}^{{commit}}")];
        let out = self.bridge.run(&Request::Git { cwd: cwd.to_path_buf(), args }).map_err(|e| fail("io_error", e))?;
        let sha = out.stdout.trim();
        if out.exit() == Some(0) && argv::is_full_sha(sha) {
            Ok(sha.to_string())
        } else {
            Err(invalid("base", "is not a branch or commit in this repository"))
        }
    }

    pub fn start_run(&self, mut input: RunInput, cwd: &str) -> Res {
        let cwd = repo_dir(cwd)?;
        argv::validate(&input).map_err(invalid_input)?;
        input.base = self.resolve_base(&cwd, &input.base)?;
        let (now, run_id) = self.fresh_run_id(ids::unix_now())?;
        let session_id = ids::uuid_v4().map_err(|e| fail("io_error", e))?;
        let campaign = input.campaign.clone().unwrap_or_else(|| format!("deliver-{run_id}"));
        let r = self.launch(&session_id, &run_id, &campaign, &cwd, |n| {
            argv::build_run_argv_at(&input, n, now, &session_id).map_err(invalid_input)
        }, false)?;
        self.note("run_started", &[]);
        Ok(r)
    }

    /// ADR-9 resume: `claude --resume <session_id>` continuing the campaign
    /// through `/vajra-deliver` with a rotated steering nonce (design 5).
    pub fn resume_run(&self, session_id: &str, campaign: &str, cwd: &str) -> Res {
        let cwd = repo_dir(cwd)?;
        if !ids::valid_session_id(session_id) {
            return Err(invalid("session_id", "must be a canonical uuid"));
        }
        if !bus::valid_stream_id(campaign) {
            return Err(invalid("campaign", "not a campaign slug"));
        }
        // ADR-9 item 5: every resume is a new run with a fresh UTC id, never the id of an earlier resume.
        let (_, run_id) = self.fresh_run_id(ids::unix_now())?;
        let run_id = run_id.as_str();
        let claude = adapter_for(Engine::Claude);
        let r = self.launch(session_id, run_id, campaign, &cwd, |n| {
            claude.resume_argv(session_id, run_id, campaign, n).map_err(invalid_input)
        }, true)?;
        self.note("run_resumed", &[]);
        Ok(r)
    }

    // ---------------------------------------------------------------- crew

    /// The crew tree for a run. Workers are read from the run session's
    /// `subagents` dir (meta files and workflow journals) when the app knows
    /// the session; the control service is told about them so steering can
    /// address them.
    pub fn crew(&self, slug: &str, run_id: &str) -> Res {
        let run = self.run_by_id(slug, run_id);
        let running = run.as_ref().is_some_and(|(s, _)| self.sup.is_running(s).unwrap_or(false));
        let workers = match &run {
            Some((_, r)) => self.discover_workers(r)?,
            None => Vec::new(),
        };
        let ctx: HashMap<String, (Option<String>, Vec<String>)> = if running {
            let tr = lock(&self.transcripts);
            workers
                .iter()
                .map(|w| {
                    // The newest retained output, not the oldest: an inspector needs the current failure or question.
                    let recent = self.bus.recent(&worker_stream(&w.agent_id), INSPECT_BYTES);
                    let cwd = tr.get(&w.agent_id).and_then(|t| t.cwd()).map(str::to_string);
                    (w.agent_id.clone(), (cwd, recent.iter().map(|l| l.text.to_string()).collect()))
                })
                .collect()
        } else {
            HashMap::new()
        };
        // Only the live run's workers are steering targets; viewing a past run
        // never retargets the control service.
        if let (true, Some((_, r))) = (running, &run) {
            self.publish_workers(&r.session_id, run_id, &workers, &ctx);
        }
        let run_status = if running { Status::InProgress } else { Status::Unknown };
        let lead = run.as_ref().map(|_| run_status);
        // Overlays come from the live control session only; a past run shows none.
        let signals = match (&run, running) {
            (Some((_, r)), true) => lock(&self.control).signals(&r.session_id),
            _ => Signals::default(),
        };
        let node = self.with_reader(|r| Ok(tree::snapshot(r, slug, run_id, lead, run_status, &workers, &signals)))?;
        Ok(node_json(&node))
    }

    fn publish_workers(&self, session_id: &str, run_id: &str, workers: &[Worker], ctx: &HashMap<String, (Option<String>, Vec<String>)>) {
        let infos = workers
            .iter()
            .map(|w| {
                let (cwd, lines) = ctx.get(&w.agent_id).cloned().unwrap_or_default();
                let (task_id, round) = task_meta(run_id, &w.label);
                WorkerInfo {
                    worker: w.clone(),
                    task_id,
                    title: Some(w.label.clone()),
                    round,
                    live: !matches!(w.status, Status::Done | Status::Failed),
                    last_activity: None,
                    cwd,
                    lines,
                }
            })
            .collect();
        lock(&self.control).set_workers(session_id, infos);
        self.workers_published();
    }

    /// Keep the control service's workers and the transcript tails current for every live run, without any UI view
    /// asking: steering and MCP inspect must work while the crew view is closed. Called by the core's own poll loop.
    pub fn refresh_workers(&self) {
        let live: Vec<Run> = lock(&self.runs).iter().filter(|(s, _)| self.sup.is_running(s).unwrap_or(false)).map(|(_, r)| r.clone()).collect();
        for run in live {
            self.refresh_run_workers(&run);
        }
    }

    fn refresh_run_workers(&self, run: &Run) {
        let Ok(workers) = self.discover_workers(run) else { return };
        let ctx: HashMap<String, (Option<String>, Vec<String>)> = {
            let tr = lock(&self.transcripts);
            workers
                .iter()
                .map(|w| {
                    let recent = self.bus.recent(&worker_stream(&w.agent_id), INSPECT_BYTES);
                    let cwd = tr.get(&w.agent_id).and_then(|t| t.cwd()).map(str::to_string);
                    (w.agent_id.clone(), (cwd, recent.iter().map(|l| l.text.to_string()).collect()))
                })
                .collect()
        };
        self.publish_workers(&run.session_id, &run.run_id, &workers, &ctx);
    }

    fn discover_workers(&self, run: &Run) -> Result<Vec<Worker>, Value> {
        let allow = self.allow()?;
        let dir = self
            .claude
            .join("projects")
            .join(project_slug(&run.cwd))
            .join(&run.session_id)
            .join("subagents");
        // These files are Claude Code's, outside our control, and journals grow during a run: an over-limit file is
        // skipped (and counted), never loaded; the read itself is also bounded in case it grows after the size check.
        let read = |p: &Path, cap: u64| -> Option<String> {
            use std::io::Read;
            let f = allow.open_allowlisted(p).ok()?;
            if f.metadata().ok()?.len() > cap {
                self.note("worker_file_too_large", &[("limit", cap)]);
                return None;
            }
            let mut s = String::new();
            f.take(cap + 1).read_to_string(&mut s).ok()?;
            (s.len() as u64 <= cap).then_some(s)
        };
        let mut dirs = vec![dir.clone()];
        if let Ok(rd) = std::fs::read_dir(dir.join("workflows")) {
            dirs.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
        let mut red = run.nonces.clone();
        {
            red.add(lock(&self.control).nonce(&run.session_id));
        }
        let mut out = Vec::new();
        for d in dirs {
            let journal = read(&d.join("journal.jsonl"), MAX_JOURNAL_BYTES).map(|s| parse_journal(&s)).unwrap_or_default();
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            let mut metas: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            metas.sort();
            for p in metas {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let Some(id) = name.strip_prefix("agent-").and_then(|n| n.strip_suffix(".meta.json")) else { continue };
                let Some(meta) = read(&p, MAX_META_BYTES).and_then(|s| parse_meta(&s)) else { continue };
                let j = journal.iter().rev().find(|j| j.agent_id == id);
                out.push(self.pump_worker(&allow, &red, &d.join(format!("agent-{id}.jsonl")), id, &meta, j));
            }
        }
        Ok(out)
    }

    /// Tail one worker transcript into its stream (new bytes only) and build its node.
    fn pump_worker(&self, allow: &Allowlist, red: &Redactor, path: &Path, id: &str, meta: &Meta, j: Option<&JournalEntry>) -> Worker {
        let mut map = lock(&self.transcripts);
        let tr = map
            .entry(id.to_string())
            .or_insert_with(|| Transcript::new(Tailer::new(allow.clone(), path.to_path_buf()), Redactor::default()));
        tr.absorb(red);
        let stream = worker_stream(id);
        if bus::valid_stream_id(&stream)
            && let Err(e) = tr.pump(&self.bus, &stream)
        {
            self.note("transcript_read_failed", &[("kind", e.kind() as u64)]);
        }
        tree::worker_from(id, meta, j, Some(tr))
    }

    pub fn tail(&self, stream: &str, after_seq: u64, max_bytes: usize) -> Res {
        if !bus::valid_stream_id(stream) {
            return Err(invalid("stream", "not a stream id"));
        }
        let t = self.bus.tail(stream, after_seq, max_bytes.clamp(1, MAX_PAGE));
        Ok(json!({
            "lines": lines_json(&t.lines), "gap": t.gap, "head_seq": t.head_seq,
            "spill_start": t.spill_start, "spill_end": t.spill_end,
        }))
    }

    pub fn read_spill(&self, stream: &str, offset: u64, max_bytes: usize) -> Res {
        if !bus::valid_stream_id(stream) {
            return Err(invalid("stream", "not a stream id"));
        }
        let p = self.bus.read_spill(stream, offset, max_bytes.clamp(1, MAX_PAGE)).map_err(|e| fail("io_error", e))?;
        Ok(json!({"lines": lines_json(&p.lines), "next_offset": p.next_offset, "gap": p.gap, "end": p.end}))
    }

    /// The `max_lines` (at most 1000) lines just before `before_seq`, for "Load earlier": adjacent to the reader's window,
    /// whatever the size of the spill.
    pub fn read_spill_before(&self, stream: &str, before_seq: u64, max_lines: usize) -> Res {
        if !bus::valid_stream_id(stream) {
            return Err(invalid("stream", "not a stream id"));
        }
        let p = self.bus.read_spill_before(stream, before_seq, max_lines.clamp(1, 1000)).map_err(|e| fail("io_error", e))?;
        Ok(json!({"lines": lines_json(&p.lines), "at_start": p.at_start, "gap": p.gap}))
    }

    /// The rows a stream view is showing and nothing else (#45): up to `max_lines` (at most 500) starting at seq `from`,
    /// or the newest ones when `from` is null, within `max_bytes` (at most 1 MiB) of text. `head_seq` and `first_seq` size
    /// the scrollbar; `at_start`, `at_end` and `gap` tell the view what lies beyond the window and what was lost.
    pub fn stream_window(&self, stream: &str, from: Option<u64>, max_lines: usize, max_bytes: usize) -> Res {
        if !bus::valid_stream_id(stream) {
            return Err(invalid("stream", "not a stream id"));
        }
        let w = self.bus.window(stream, from, max_lines.clamp(1, WINDOW_MAX_LINES), max_bytes.clamp(1, WINDOW_MAX_BYTES)).map_err(|e| fail("io_error", e))?;
        Ok(json!({
            "lines": lines_json(&w.lines), "head_seq": w.head_seq, "first_seq": w.first_seq,
            "at_start": w.at_start, "at_end": w.at_end, "gap": w.gap,
        }))
    }

    pub fn task(&self, slug: &str, task_id: &str) -> Res {
        let snap = self.with_reader(|r| Ok(r.tasks(slug)))?;
        let found = snap
            .value
            .as_ref()
            .and_then(|v| v.get("tasks"))
            .and_then(Value::as_array)
            .and_then(|ts| ts.iter().find(|t| t.get("id").and_then(Value::as_str) == Some(task_id)))
            .cloned();
        let Some(task) = found else { return Err(fail("task_not_found", task_id)) };
        let evidence = match self.evidence_request(slug, &task) {
            None => Value::Null,
            Some(req) => match self.bridge.run(&req) {
                // A cut or killed run has no result and no code: its exit code and partial text mean nothing.
                Ok(o) => json!({"code": o.exit(), "result": o.stdout_whole().and_then(|s| serde_json::from_str::<Value>(s.trim()).ok()), "truncated": o.truncated}),
                Err(e) => json!({"code": null, "error": e.to_string()}),
            },
        };
        Ok(json!({"task": task, "stale": snap.stale, "evidence": evidence}))
    }

    /// S4c: the Run view's phase timeline and task graph in one read-only call (AC-4.1.x, AC-4.2.x).
    pub fn run_overview(&self, slug: &str) -> Res {
        if !bus::valid_stream_id(slug) {
            return Err(invalid("slug", "not a campaign slug"));
        }
        let (snap, progress) = self.with_reader(|r| Ok((r.tasks(slug), r.progress(slug).unwrap_or_default())))?;
        let tasks: Option<&[Value]> = snap.value.as_ref().and_then(|v| v.get("tasks")).and_then(Value::as_array).map(Vec::as_slice);
        // `tasks.json` holds every run's tasks: the graph and the Crew link are for the current run only.
        let (run_id, scoped) = runview::scope(&progress, tasks);
        let tl = runview::timeline(&progress, tasks);
        Ok(json!({
            "slug": slug,
            "run_id": run_id,
            "stale": snap.stale,
            "timeline": runview::timeline_json(&tl),
            "graph": runview::graph(&run_id, scoped.as_deref().unwrap_or(&[])),
        }))
    }

    /// S4a: every acceptance criterion of a campaign with its proof status, in one call.
    /// `pass` = the task's signed evidence checks green; `unverified` = evidence exists but its check is red, unreadable,
    /// timed out or cannot be located (never counted as pass); `checking` = its check has not run yet (over the call's time
    /// budget, or another sweep is running); `fail` = the task failed; `not_run` = no evidence yet
    /// (a closed task without evidence is also `not_run`, never pass). Checks run at most `EVIDENCE_WORKERS` at a time and the whole call is bounded by `EVIDENCE_BUDGET`.
    /// The recorded commands, exit codes and output come from `evidence_detail`, per task; the list carries only the count.
    pub fn evidence_list(&self, slug: &str) -> Res {
        if !bus::valid_stream_id(slug) {
            return Err(invalid("slug", "not a campaign slug"));
        }
        // The budget covers the whole call, the repo-state reads included.
        let deadline = std::time::Instant::now() + self.evidence_budget;
        let snap = self.with_reader(|r| Ok(r.tasks(slug)))?;
        let Some(tasks) = snap.value.as_ref().and_then(|v| v.get("tasks")).and_then(Value::as_array) else {
            return Err(fail("not_found", "no tasks for this campaign"));
        };
        // The key is the evidence file's exact bytes (small files only) with head, mode and repo: nothing a writer can leave
        // unchanged while changing what the check sees. A file too large or unreadable is never cached.
        let key = |req: &Request| -> Option<EvKey> {
            let Request::CheckEvidence { path, head, archived, repo } = req else { return None };
            let f = std::fs::File::open(path).ok()?;
            if f.metadata().ok()?.len() > EVIDENCE_KEY_MAX {
                return None;
            }
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut std::io::Read::take(f, EVIDENCE_KEY_MAX + 1), &mut bytes).ok()?;
            (bytes.len() as u64 <= EVIDENCE_KEY_MAX).then(|| (path.clone(), bytes, head.clone(), archived.clone(), repo.clone()))
        };
        let mut verdict: HashMap<usize, bool> = HashMap::new();
        let mut why: HashMap<usize, Value> = HashMap::new();
        let mut repo_of: HashMap<usize, PathBuf> = HashMap::new();
        let mut jobs: Vec<(usize, Request, Option<EvKey>)> = Vec::new();
        {
            let cache = self.evidence_cache.lock().unwrap_or_else(|e| e.into_inner());
            let cached_why = self.evidence_why.lock().unwrap_or_else(|e| e.into_inner());
            for (i, t) in tasks.iter().enumerate() {
                let Some(req) = self.evidence_request(slug, t) else {
                    // Evidence that cannot be checked here (outside the campaign, `..` path, no repo) is unverified, not pending.
                    if t.get("evidence").is_some_and(|e| !e.is_null()) {
                        verdict.insert(i, false);
                        why.insert(i, reason(self.evidence_skip_code(t), None, None));
                    }
                    continue;
                };
                if let Request::CheckEvidence { repo: Some(r), .. } = &req {
                    repo_of.insert(i, r.clone());
                }
                let k = key(&req);
                let hit = k.as_ref().and_then(|k| cache.get(k)).filter(|(_, until)| until.is_none_or(|u| u > std::time::Instant::now()));
                match hit {
                    Some((v, _)) => {
                        verdict.insert(i, *v);
                        if !*v {
                            if let Some(w) = k.as_ref().and_then(|k| cached_why.get(k)) {
                                why.insert(i, w.clone());
                            }
                        }
                    }
                    None => jobs.push((i, req, k)),
                }
            }
        }
        // Only one sweep per campaign at a time: while one runs, another call answers from the cache and leaves the rest `checking`.
        let claimed = self.sweeping.lock().unwrap_or_else(|e| e.into_inner()).insert(slug.to_string());
        struct Release<'a>(&'a Mutex<std::collections::HashSet<String>>, String, bool);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                if self.2 { self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.1); }
            }
        }
        let _release = Release(&self.sweeping, slug.to_string(), claimed);
        for batch in jobs.chunks(EVIDENCE_WORKERS) {
            if !claimed {
                break;
            }
            // Out of budget: the checks not yet started read `checking` (never pass).
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            let mut bridge = self.bridge.clone();
            bridge.timeout = self.evidence_timeout.min(left);
            let (home, installed) = (self.home.as_path(), bridge.scripts_dir.join("vajra_tasks.py").is_file());
            // A check cut short by the remaining budget (not by its own full timeout) did not get a fair run: `checking`, not `unverified`.
            let cut_by_budget = left < self.evidence_timeout;
            let got: Vec<(usize, Outcome)> = std::thread::scope(|sc| {
                let hs: Vec<_> = batch
                    .iter()
                    .map(|(i, req, _)| {
                        let b = &bridge;
                        sc.spawn(move || {
                            let out = if !installed {
                                Outcome::Failed("spawn_failed")
                            } else {
                                match b.run(req) {
                                    Err(e) => Outcome::Failed(bridge_failure(&e)),
                                    Ok(o) if o.timed_out => Outcome::TimedOut,
                                    Ok(o) => evidence_outcome(&o, home),
                                }
                            };
                            (*i, out)
                        })
                    })
                    .collect();
                hs.into_iter().filter_map(|h| h.join().ok()).collect()
            });
            let mut cache = self.evidence_cache.lock().unwrap_or_else(|e| e.into_inner());
            let mut cached_why = self.evidence_why.lock().unwrap_or_else(|e| e.into_inner());
            if cache.len() > EVIDENCE_CACHE_MAX {
                cache.clear();
                cached_why.clear();
            }
            for (i, out) in got {
                // Cut short by the budget (not by its own full timeout): no verdict yet, so `checking`, and it is not remembered.
                if matches!(out, Outcome::TimedOut) && cut_by_budget {
                    continue;
                }
                let (green, remember, w) = match out {
                    Outcome::Verdict(g, w) => (g, true, w),
                    Outcome::TimedOut => (false, false, Some(reason("timeout", None, None))),
                    Outcome::Failed(code) => (false, false, Some(reason(code, None, None))),
                };
                verdict.insert(i, green);
                if let Some(w) = w.clone() {
                    why.insert(i, w);
                }
                // Remember it only if the file still holds the bytes the key was made from: the script opened it after the
                // key was read, so a swapped file would otherwise store this verdict under the wrong content.
                let Some(k) = jobs.iter().find(|j| j.0 == i).and_then(|j| j.2.clone()) else { continue };
                let req = &jobs.iter().find(|j| j.0 == i).expect("job").1;
                if key(req).as_ref() != Some(&k) {
                    continue;
                }
                let until = if remember {
                    k.3.is_none().then(|| std::time::Instant::now() + EVIDENCE_LIVE_TTL)
                } else {
                    Some(std::time::Instant::now() + EVIDENCE_FAILED_TTL)
                };
                match w {
                    Some(w) if !green => { cached_why.insert(k.clone(), w); }
                    _ => { cached_why.remove(&k); }
                }
                cache.insert(k, (green, until));
            }
        }
        // The checks ran first (the first batch gets its full timeout). Now, within what is left of the budget, the state of each
        // run repo that a passing check depends on: read once per call, reused for 5 s, and a repo that could not be read is
        // remembered as unknown for 30 s so a slow one is not paid for on every call.
        let mut repos: HashMap<PathBuf, Option<(String, bool)>> = HashMap::new();
        for (i, r) in &repo_of {
            if verdict.get(i) == Some(&true) && !repos.contains_key(r) {
                repos.insert(r.clone(), self.worktree_state(r, deadline));
            }
        }
        let mut pending = 0usize;
        let mut rows = Vec::new();
        for (i, t) in tasks.iter().enumerate() {
            let id = t.get("id").and_then(Value::as_str).unwrap_or("");
            let has_evidence = t.get("evidence").is_some_and(|e| !e.is_null());
            let task_status = t.get("status").and_then(Value::as_str).unwrap_or("");
            // A task marked failed is `fail` whatever its evidence says; only then does evidence decide pass or unverified.
            let status = if task_status == "failed" {
                "fail"
            } else if has_evidence {
                match verdict.get(&i) {
                    Some(true) => {
                        // Green evidence is current only while the repo is still at the evidence's head and clean (an edit, even
                        // uncommitted, or a moved branch makes it stale). If git could not be read the check's verdict stands.
                        let want = t.get("evidence").and_then(|e| e.get("head")).and_then(Value::as_str);
                        let stale = repo_of.get(&i).and_then(|r| repos.get(r)).and_then(Option::as_ref)
                            .is_some_and(|(head, dirty)| *dirty || want.is_some_and(|w| w != head));
                        if stale { "stale" } else { "pass" }
                    }
                    Some(false) => "unverified",
                    None => { pending += 1; "checking" }
                }
            } else {
                "not_run"
            };
            let ev = t.get("evidence").filter(|e| !e.is_null());
            let report = t.get("report").and_then(|r| r.get("summary")).and_then(Value::as_str);
            let criteria: Vec<Value> = t.get("acceptance").and_then(Value::as_array).cloned().unwrap_or_default();
            for c in criteria {
                rows.push(json!({
                    "task": id,
                    "title": t.get("title"),
                    "owner": t.get("owner_agent"),
                    "criterion": c,
                    "status": status,
                    "why": if status == "unverified" { why.get(&i).cloned() } else { None },
                    "head": ev.and_then(|e| e.get("head")),
                    "path": ev.and_then(|e| e.get("path")),
                    "commands": ev.and_then(|e| e.get("commands")),
                    "report": report,
                }));
            }
        }
        Ok(json!({"slug": slug, "stale": snap.stale, "pending": pending, "rows": rows}))
    }

    /// The proof behind one task's evidence, for the Run detail: each recorded command with its exit code and, for a failed
    /// one, the tail of its output, plus the verifier's notes. Read from the evidence file (at most 64 KiB; a larger one is
    /// reported `truncated` with no commands) and from `cmd-N.log` files next to it (plain sibling file names only, regular
    /// files only, the last 4 KiB). All text is cleaned (escapes stripped) and redacted with the nonces of this campaign's
    /// runs before it leaves the core, and is shown as plain text.
    pub fn evidence_detail(&self, slug: &str, task_id: &str) -> Res {
        if !bus::valid_stream_id(slug) {
            return Err(invalid("slug", "not a campaign slug"));
        }
        let snap = self.with_reader(|r| Ok(r.tasks(slug)))?;
        let task = snap
            .value
            .as_ref()
            .and_then(|v| v.get("tasks"))
            .and_then(Value::as_array)
            .and_then(|ts| ts.iter().find(|t| t.get("id").and_then(Value::as_str) == Some(task_id)))
            .cloned();
        let Some(task) = task else { return Err(fail("task_not_found", task_id)) };
        let mut red = Redactor::default();
        for r in lock(&self.runs).values().filter(|r| r.campaign == slug) {
            red.absorb(&r.nonces);
        }
        let text = |s: &str, max: usize| -> String {
            let cut: String = s.chars().take(max).collect();
            cut.split('\n').map(|l| red.clean(l)).collect::<Vec<_>>().join("\n")
        };
        let notes = task.get("report").and_then(|r| r.get("summary")).and_then(Value::as_str).map(|n| text(n, PROOF_TEXT_MAX));
        let mut out = json!({"task": task_id, "commands": [], "notes": notes, "truncated": false});
        // The same path rules as the check (no `..`, an archived bundle or the run repo); anything else shows no commands.
        let Some(Request::CheckEvidence { path, .. }) = self.evidence_request(slug, &task) else { return Ok(out) };
        let regular = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file());
        if !regular(&path) {
            return Ok(out);
        }
        let read_tail = |p: &Path, max: u64| -> Option<String> {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(p).ok()?;
            let len = f.metadata().ok()?.len();
            f.seek(SeekFrom::Start(len.saturating_sub(max))).ok()?;
            let mut b = Vec::new();
            f.take(max).read_to_end(&mut b).ok()?;
            Some(String::from_utf8_lossy(&b).into_owned())
        };
        if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > PROOF_FILE_MAX {
            out["truncated"] = json!(true);
            return Ok(out);
        }
        let Some(doc) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return Ok(out) };
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let cmds: Vec<Value> = doc
            .get("commands")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .take(PROOF_CMDS_MAX)
                    .map(|c| {
                        let exit = c.get("exit").and_then(Value::as_i64);
                        let passed = c.get("passed").and_then(Value::as_bool).unwrap_or(exit == Some(0));
                        let log = c.get("log").and_then(Value::as_str).filter(|n| {
                            !n.is_empty() && n.len() <= 64 && !n.contains(['/', '\\']) && *n != "." && *n != ".."
                        });
                        let tail = if passed { None } else { log.map(|n| dir.join(n)).filter(|p| regular(p)).and_then(|p| read_tail(&p, PROOF_LOG_TAIL)).map(|t| text(&t, PROOF_LOG_TAIL as usize)) };
                        json!({
                            "cmd": c.get("cmd").and_then(Value::as_str).map(|s| text(s, PROOF_TEXT_MAX)),
                            "exit_code": exit,
                            "passed": passed,
                            "output_tail": tail,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        out["commands"] = json!(cmds);
        Ok(out)
    }

    /// An archived bundle (under the campaign's evidence dir) is checked
    /// without a repo; live evidence is checked in the repo of the run that
    /// made it. `None` when the task has no evidence path or no repo is known.
    /// `(HEAD, dirty)` of `repo`, where dirty means any tracked edit or untracked file (`git status --porcelain -unormal`;
    /// an untracked directory is one line, so a large untracked tree stays cheap). `None` when git fails or times out:
    /// the caller then shows the check's own verdict, never `stale`, and nothing is remembered.
    fn worktree_state(&self, repo: &Path, deadline: std::time::Instant) -> Option<(String, bool)> {
        if let Some((at, st)) = lock(&self.worktree_state).get(repo) {
            let ttl = if st.is_some() { WORKTREE_STATE_TTL } else { WORKTREE_STATE_UNKNOWN_TTL };
            if at.elapsed() < ttl {
                return st.clone();
            }
        }
        let budget = deadline.saturating_duration_since(std::time::Instant::now());
        if budget.is_zero() {
            return None; // no time to ask: nothing learned, nothing remembered
        }
        // Both git calls count against the call's deadline: each gets at most 2 s and never more than what is left.
        let git = |args: &[&str]| {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return None;
            }
            let mut bridge = self.bridge.clone();
            bridge.timeout = WORKTREE_STATE_TIMEOUT.min(left);
            bridge
                .run(&Request::Git { cwd: repo.to_path_buf(), args: args.iter().map(|a| a.to_string()).collect() })
                .ok()
                .filter(|o| o.exit() == Some(0))
        };
        let read = || {
            let head = git(&["rev-parse", "HEAD"])?.stdout.trim().to_string();
            let mut status = vec!["status".to_string(), "--porcelain".into(), "-unormal".into(), "--".into(), ".".into()];
            status.extend(RUN_OWN_OUTPUT.iter().map(|d| format!(":(exclude){d}")));
            let status: Vec<&str> = status.iter().map(String::as_str).collect();
            let dirty = !git(&status)?.stdout.trim().is_empty();
            Some((head, dirty))
        };
        let st = read();
        // Remember a failure only if git had its full time to answer: running out of the call's budget says nothing about the repo.
        if st.is_some() || budget >= WORKTREE_STATE_TIMEOUT {
            lock(&self.worktree_state).insert(repo.to_path_buf(), (std::time::Instant::now(), st.clone()));
        }
        st
    }

    /// Why `evidence_request` gave nothing for a task that has evidence.
    fn evidence_skip_code(&self, task: &Value) -> &'static str {
        let Some(path) = task.get("evidence").and_then(|e| e.get("path")).and_then(Value::as_str) else { return "io_error" };
        if Path::new(path).components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            "outside_campaign"
        } else if self.allow().is_err() {
            "io_error"
        } else {
            "no_run_registered"
        }
    }

    fn evidence_request(&self, slug: &str, task: &Value) -> Option<Request> {
        let ev = task.get("evidence")?;
        let path = PathBuf::from(ev.get("path")?.as_str()?);
        let head = ev.get("head").and_then(Value::as_str).map(String::from);
        let archive_dir = self.allow().ok()?.campaigns_dir().join(slug).join("evidence");
        // `starts_with` is lexical: a path that climbs out with `..` is never treated as archived.
        let climbs = path.components().any(|c| matches!(c, std::path::Component::ParentDir));
        if climbs {
            return None;
        }
        if path.starts_with(&archive_dir) {
            return Some(Request::CheckEvidence { path, head, archived: Some(slug.into()), repo: None });
        }
        let repo = lock(&self.runs).values().find(|r| r.campaign == slug)?.cwd.clone();
        Some(Request::CheckEvidence { path, head, archived: None, repo: Some(repo) })
    }

    /// The Worktree and Diff tabs' header for a worker: its worktree path, branch and the base to diff against. Read-only.
    /// Limited to crew worktrees: the worker's transcript cwd, resolved through links, must be a directory strictly under
    /// `<repo>/.claude/worktrees/` of a run this app started. Anything else (the repo itself, a link out) is refused,
    /// so the tab can never be pointed at an arbitrary directory. `base` is the merge-base of the worktree's HEAD and
    /// the run repo's HEAD.
    pub fn worktree_info(&self, worker: &str) -> Res {
        if !bus::valid_stream_id(&worker_stream(worker)) {
            return Err(invalid("worker", "not a worker id"));
        }
        let cwd = lock(&self.transcripts).get(worker).map(|t| t.cwd().map(str::to_string));
        let Some(cwd) = cwd else { return Err(fail("not_found", "no such worker")) };
        let refuse = || fail("not_a_crew_worktree", "the worker has no crew worktree");
        let wt = cwd.map(PathBuf::from).and_then(|p| p.canonicalize().ok()).ok_or_else(refuse)?;
        let repo = lock(&self.runs)
            .values()
            .map(|r| r.cwd.clone())
            .find(|r| {
                r.canonicalize().is_ok_and(|r| {
                    wt.strip_prefix(r.join(".claude/worktrees")).is_ok_and(|rest| rest.components().next().is_some())
                })
            })
            .ok_or_else(refuse)?;
        if !wt.is_dir() {
            return Err(refuse());
        }
        let git = |dir: &Path, args: &[&str]| -> Option<String> {
            let o = self.bridge.run(&Request::Git { cwd: dir.to_path_buf(), args: args.iter().map(|a| a.to_string()).collect() }).ok()?;
            (o.exit() == Some(0)).then(|| o.stdout.trim().to_string())
        };
        let branch = git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let base = git(&repo, &["rev-parse", "HEAD"]).and_then(|h| git(&wt, &["merge-base", "HEAD", &h]));
        Ok(json!({"worker": worker, "worktree": wt.to_string_lossy(), "branch": branch, "base": base}))
    }

    pub fn diff(&self, cwd_str: &str, base: &str) -> Res {
        let cwd = repo_dir(cwd_str)?;
        if base.is_empty() || base.starts_with('-') || base.len() > 255 {
            return Err(invalid("base", "must be a ref or commit"));
        }
        let o = self
            .bridge
            .run(&Request::Git { cwd, args: vec!["diff".into(), base.into(), "--".into()] })
            .map_err(|e| fail("git_failed", e))?;
        // `git diff <base>` leaves out files git does not track yet, so a worker's brand-new files are listed apart
        // (names only, ignore rules respected), instead of the tab calling an added-files-only change "no changes".
        let mut untracked: Vec<String> = Vec::new();
        let mut untracked_truncated = false;
        // The diff itself is not interpreted: the client gets `code`, `truncated` and `timed_out` and shows a cut diff with its notice.
        if o.code == Some(0) && !o.timed_out {
            if let Ok(u) = self.bridge.run(&Request::Git {
                cwd: repo_dir(cwd_str)?,
                args: ["ls-files", "--others", "--exclude-standard", "-z"].map(String::from).to_vec(),
            }) {
                if u.code == Some(0) && !u.timed_out {
                    // A list cut at the cap is a prefix; its last name may be half a path, so it is dropped.
                    let mut all: Vec<&str> = u.stdout.split('\0').filter(|p| !p.is_empty()).collect();
                    if u.truncated {
                        all.pop();
                    }
                    untracked_truncated = u.truncated || all.len() > UNTRACKED_MAX;
                    untracked = all.into_iter().take(UNTRACKED_MAX).map(String::from).collect();
                }
            }
        }
        Ok(json!({"code": o.code, "diff": o.stdout, "truncated": o.truncated, "timed_out": o.timed_out,
                  "untracked": untracked, "untracked_truncated": untracked_truncated}))
    }

    // ---------------------------------------------------------------- control

    pub fn control_message(&self, worker: &str, text: &str) -> Res {
        ctl(lock(&self.control).message_worker(Actor::Human, worker, text))
    }

    pub fn control_redirect(&self, worker: &str, text: &str) -> Res {
        ctl(lock(&self.control).redirect_worker(Actor::Human, worker, text))
    }

    pub fn control_stop(&self, worker: &str, reason: Option<&str>) -> Res {
        ctl(lock(&self.control).stop_worker(Actor::Human, worker, reason))
    }

    /// `control_stop` that refuses unless the worker is still on attempt `expect`, checked under the control lock.
    pub(super) fn control_stop_expecting(&self, worker: &str, reason: Option<&str>, expect: Option<u64>) -> Res {
        ctl(lock(&self.control).stop_worker_expecting(Actor::Human, worker, reason, expect))
    }

    pub fn confirm_stop(&self, action_id: &str) -> Res {
        ctl(lock(&self.control).confirm_stop(Actor::Human, action_id))
    }

    pub fn cancel_stop(&self, action_id: &str) -> Res {
        ctl(lock(&self.control).cancel_stop(Actor::Human, action_id))
    }

    pub fn resume_worker(&self, worker: &str) -> Res {
        ctl(lock(&self.control).resume_worker(Actor::Human, worker))
    }

    pub fn ledger(&self) -> Res {
        let path = self.data.join("control/ledger.jsonl");
        let recs = Ledger::read_all(&path).map_err(|e| fail("io_error", e))?;
        Ok(json!({"records": recs}))
    }

    pub fn kb_search(&self, query: &str, k: u32, repo: &str) -> Res {
        let repo = repo_dir(repo)?;
        let o = self
            .bridge
            .run(&Request::KbSearch { query: query.into(), k: k.clamp(1, 50), repo })
            .map_err(|e| fail("kb_failed", e))?;
        // A cut buffer is not parsed: partial JSON could name the wrong hits.
        let hits: Vec<Value> =
            parse_kb_hits(o.stdout_whole().unwrap_or_default()).into_iter().map(|h| json!({"id": h.id, "kind": h.kind, "title": h.title})).collect();
        Ok(json!({"hits": hits, "code": o.exit(), "truncated": o.truncated}))
    }

    /// A real verdict still inside `verdict_ttl`, or `None` (a verify running, none taken, expired, or "unknown").
    /// Forget a kept verdict so the next check of `slug` verifies again. A verify in flight is left alone.
    fn forget_fresh(&self, slug: &str) {
        let floor = self.ticket();
        let mut m = lock(&self.verdicts);
        if matches!(m.by_slug.get(slug), Some(Verdict::Fresh(..))) {
            m.by_slug.remove(slug);
        }
        m.floor.insert(slug.to_string(), floor);
    }

    fn cached_integrity(&self, slug: &str) -> Option<&'static str> {
        let m = lock(&self.verdicts);
        // A forced refresh is pending: whatever an older verify left behind is not shown (see `Verdicts::floor`).
        if m.floor.contains_key(slug) {
            return None;
        }
        match m.by_slug.get(slug) {
            Some(&Verdict::Fresh(at, v)) if (self.clock)().saturating_sub(at) < self.verdict_ttl => Some(v),
            _ => None,
        }
    }

    /// #28: one campaign's badge, `{slug, integrity}` with integrity `ok`, `failed` or `unknown`. The UI asks once per
    /// listed campaign. At most `VERIFY_SLOTS` verifies run at once across all calls, one per campaign (a second call
    /// for the same campaign waits for the first); a call that cannot get a slot within the budget, a verify that
    /// fails to run and one that overruns its timeout all read `unknown`, which is never kept as a verdict.
    pub fn campaign_integrity(&self, slug: &str, refresh: bool) -> Res {
        if !bus::valid_stream_id(slug) {
            return Err(invalid("slug", "not a campaign slug"));
        }
        Ok(json!({"slug": slug, "integrity": self.integrity_of(slug, refresh)}))
    }

    /// "ok", "failed" (signature invalid, exit 3) or "unknown" (the check could not run, or overran
    /// `verify_timeout`). A real verdict is reused for `verdict_ttl`; callers that arrive while a verify for the same
    /// campaign is running wait for it instead of starting another. `refresh` skips the reuse and, if a verify is
    /// already running, waits for it and then runs its own (that one may have started before the change).
    fn integrity_of(&self, slug: &str, refresh: bool) -> &'static str {
        self.integrity_run(slug, refresh).0
    }

    /// `integrity_of` plus, when this call ran the verify itself, its result (callers that only waited get `None`).
    /// An explicit `verify_campaign` goes through here with `refresh`, so it shares the campaign's flight: it waits for
    /// a running check and then runs its own, and an older check can never overwrite the verdict it settled.
    fn integrity_run(&self, slug: &str, refresh: bool) -> (&'static str, Option<Result<crate::vajra::bridge::Output, crate::vajra::bridge::BridgeError>>) {
        if !bus::valid_stream_id(slug) {
            return ("unknown", None);
        }
        let flight: Flight = loop {
            let mut m = lock(&self.verdicts);
            // A pending forced list refresh makes this call a refresh too (the follow-up calls carry no flag).
            let refresh = refresh || m.floor.contains_key(slug);
            let waiting = match m.by_slug.get(slug) {
                Some(&Verdict::Fresh(at, v)) if !refresh && (self.clock)().saturating_sub(at) < self.verdict_ttl => return (v, None),
                Some(Verdict::Flying(f)) => Some(f.clone()),
                _ => None,
            };
            if let Some(f) = waiting {
                drop(m);
                let mut got = lock(&f.0);
                while got.is_none() {
                    got = f.1.wait(got).unwrap_or_else(|e| e.into_inner());
                }
                if !refresh && !lock(&self.verdicts).floor.contains_key(slug) {
                    return (got.unwrap_or("unknown"), None);
                }
                continue;
            }
            let f: Flight = Arc::new((Mutex::new(None), Condvar::new()));
            m.by_slug.insert(slug.to_string(), Verdict::Flying(f.clone()));
            break f;
        };
        let mut guard = FlightGuard { app: self, slug, flight, done: false };
        // Only the call that owns the flight takes a slot; a budget overrun drops the guard, which ends the flight as
        // "unknown" and keeps nothing.
        let Some(_slot) = self.take_verify_slot() else { return ("unknown", None) };
        let ticket = self.ticket();
        let mut bridge = self.bridge.clone();
        bridge.timeout = self.verify_timeout;
        let res = bridge.run(&Request::CampaignVerify { slug: slug.into() });
        let verdict = match res.as_ref().ok().and_then(|o| o.exit()) {
            Some(0) => "ok",
            Some(3) => "failed",
            _ => "unknown",
        };
        self.settle(slug, verdict, ticket);
        *lock(&guard.flight.0) = Some(verdict);
        guard.flight.1.notify_all();
        guard.done = true;
        (verdict, Some(res))
    }

    fn take_verify_slot(&self) -> Option<VerifySlot<'_>> {
        let deadline = std::time::Instant::now() + self.verify_queue_budget;
        let mut free = lock(&self.verify_slots.0);
        while *free == 0 {
            let left = deadline.checked_duration_since(std::time::Instant::now())?;
            free = self.verify_slots.1.wait_timeout(free, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        *free -= 1;
        Some(VerifySlot(&self.verify_slots))
    }

    /// A fresh ticket for an integrity check that is about to read the campaign (strictly increasing).
    fn ticket(&self) -> u64 {
        self.tickets.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    /// Remember the verdict of the check that took `ticket` (or forget the slug when it could not run).
    /// A check that started before the newest one already settled may only add "failed": its "ok" or "unknown"
    /// is dropped, so a stale success never replaces a newer failure. A "failed" always lands (fail closed): at
    /// worst it shows for one TTL after a repair, and an explicit verify or `refresh` clears it.
    fn settle(&self, slug: &str, verdict: &'static str, ticket: u64) {
        let mut m = lock(&self.verdicts);
        if m.floor.get(slug).is_some_and(|f| ticket > *f) {
            m.floor.remove(slug); // this check read the campaign after the refresh was asked for
        }
        let last = m.last.get(slug).copied().unwrap_or(0);
        if ticket < last && verdict != "failed" {
            return;
        }
        m.last.insert(slug.to_string(), last.max(ticket));
        if verdict == "unknown" {
            m.by_slug.remove(slug);
        } else {
            m.by_slug.insert(slug.to_string(), Verdict::Fresh((self.clock)(), verdict));
        }
    }

    /// AC-1.2.1 / AC-1.2.2: `vajra-campaign.sh verify <slug>` before a resume. Exit 0 is `ok`; exit 3 (signature
    /// invalid) is `ok: false` with the restore command for the newest checkpoint listed; any other result is an
    /// error, never a pass. Read-only: it never touches the HMAC key itself, the script does the check.
    pub fn verify_campaign(&self, slug: &str) -> Res {
        if !bus::valid_stream_id(slug) {
            return Err(invalid("slug", "not a campaign slug"));
        }
        let o = match self.integrity_run(slug, true).1 {
            Some(Ok(o)) => o,
            Some(Err(crate::vajra::bridge::BridgeError::Invalid(_))) => return Err(invalid("slug", "not a campaign slug")),
            Some(Err(e)) => return Err(fail("verify_failed", e)),
            None => return Err(fail("verify_failed", "no result")),
        };
        match o.exit() {
            Some(0) => Ok(json!({"slug": slug, "ok": true})),
            Some(3) => {
                let newest = self
                    .with_reader(|r| r.checkpoints(slug).map_err(|e| fail("io_error", e)))
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|n| n.strip_suffix(".json")?.parse::<u32>().ok())
                    .max();
                let restore = newest.map(|n| format!("vajra-campaign.sh resume {slug} --from-checkpoint {n}"));
                Ok(json!({"slug": slug, "ok": false, "restore": restore}))
            }
            _ if o.truncated => Err(fail("verify_failed", "output over the cap")),
            code => Err(fail("verify_failed", format!("exit {code:?}: {}", o.stderr.trim()))),
        }
    }

    /// AC-6.1.2 / AC-6.1.3: the stored delivery report as text, from held bytes. Every input is read once
    /// (`vajra::held`); the signature, `featuresDefSha256` and `reportSha256` are all checked against those bytes,
    /// and the text returned is exactly the held report. The script only answers "is this sig valid for these
    /// hashes" (`verify-digest`), so it stays the only HMAC key holder. Result `{slug, integrity, text?}` where
    /// integrity is `ok`, `no_report`, `integrity_failed`, `not_text` or `unknown` (too large / could not run).
    pub fn read_report(&self, slug: &str) -> Res {
        self.read_report_with(slug, &|_| {}, &|| {})
    }

    /// `seam` is passed to the held reader and is also called with `"check:report"` right before the report check;
    /// `between` runs after the read and before the digest check (tests). A symlinked `campaigns` dir is refused by
    /// the held reader (`O_NOFOLLOW`), so every report read there settles `failed`: that fails closed on purpose.
    pub(crate) fn read_report_with(&self, slug: &str, seam: &dyn Fn(&str), between: &dyn Fn()) -> Res {
        use crate::control::sha256::hex_digest;
        use crate::vajra::held::{self, HeldError};
        if !bus::valid_stream_id(slug) {
            return Err(invalid("slug", "not a campaign slug"));
        }
        let verdict = |integrity: &str, text: Option<String>| {
            let mut v = json!({"slug": slug, "integrity": integrity});
            if let Some(t) = text {
                v["text"] = json!(t);
            }
            Ok(v)
        };
        // `integrity: "unknown"` plus a `reason` (a fixed code, see `reason`): which of the six ways the check could not run.
        let unknown = |why: Value| -> Res { Ok(json!({"slug": slug, "integrity": "unknown", "reason": why})) };
        let home = self.home.as_path();
        // Taken before anything is read, so this check is ordered against every other one (see `Verdicts`).
        let ticket = self.ticket();
        let allow = self.allow().map_err(|e| fail("io_error", e))?;
        let (dir, key) = (allow.campaigns_dir().to_path_buf(), allow.key_path());
        let h = match held::read_campaign(&dir, slug, &key, seam) {
            Ok(h) => h,
            Err(HeldError::NotFound) => return Err(fail("not_found", "no such campaign")),
            Err(HeldError::Failed(_)) | Err(HeldError::TooLarge("state.sig")) => {
                self.settle(slug, "failed", ticket);
                return verdict("integrity_failed", None);
            }
            Err(HeldError::TooLarge(name)) => return unknown(reason("oversize", Some(name.to_string()), None)),
            // The OS error text carries paths, so only the code is sent.
            Err(HeldError::Io(_)) => return unknown(reason("io_error", None, None)),
        };
        between();
        let bad = |this: &Self| {
            this.settle(slug, "failed", ticket);
            verdict("integrity_failed", None)
        };
        if !held::json_without_duplicate_keys(&h.campaign) || !held::json_without_duplicate_keys(&h.features) {
            return bad(self);
        }
        let (Ok(campaign), Ok(features)) =
            (serde_json::from_slice::<Value>(&h.campaign), serde_json::from_slice::<Value>(&h.features))
        else {
            return bad(self);
        };
        if !features.get("features").is_some_and(Value::is_array) {
            return bad(self);
        }
        let sig: Vec<u8> = h.sig.iter().copied().filter(|b| !matches!(b, b' ' | b'\r' | b'\n')).collect();
        if sig.len() != 64 || !sig.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return bad(self);
        }
        let sig = String::from_utf8_lossy(&sig).into_owned();
        let input = format!(
            "campaign {}\nfeatures {}\nprogress {}\nsig {}\n",
            hex_digest(&h.campaign),
            hex_digest(&h.features),
            hex_digest(&h.progress),
            sig
        );
        let mut bridge = self.bridge.clone();
        bridge.timeout = self.verify_timeout;
        // The campaign script dies on a missing jq ("jq is required", exit 1) before it dispatches verify-digest, so a
        // missing jq has to be found here, not left to look like a bad exit of the digest step.
        if bridge.find_jq().is_none() {
            return unknown(reason("jq_missing", None, None));
        }
        let o = match bridge.run(&Request::VerifyDigest { input: input.into_bytes() }) {
            Ok(o) => o,
            Err(e) => return unknown(reason(bridge_failure(&e), None, None)),
        };
        // A run cut at the cap or killed by the timeout says nothing reliable about its exit code: a cut exit 3 must
        // not record a tamper. `exit()` is None for both, which falls to the reason below.
        match o.exit() {
            Some(0) => {}
            Some(3) => return bad(self),
            _ => return unknown(output_failure(&o, home)),
        }
        // featuresDefSha256: the exact canonicaliser the script uses, fed the held bytes. Output cut at the bridge's
        // cap would hash short and read as a tamper where plain `verify` says ok, so a truncated run is "unknown".
        let def = match bridge.run(&Request::FeaturesDef { features: h.features.clone() }) {
            Ok(o) if o.exit() == Some(0) => o.stdout.replace('\n', ""),
            Ok(o) if o.truncated && !o.timed_out && o.code == Some(0) => return unknown(reason("oversize", Some("features output".into()), None)),
            Ok(o) => return unknown(output_failure(&o, home)),
            Err(e) => return unknown(reason(bridge_failure(&e), None, None)),
        };
        if campaign.get("featuresDefSha256").and_then(Value::as_str) != Some(hex_digest(def.as_bytes()).as_str()) {
            return bad(self);
        }
        // "ok" is published once, only after every check that applies has passed (plain `verify` also fails a
        // recorded report that is missing or edited), so a concurrent listing can never see a partial success.
        seam("check:report");
        let want = campaign.get("reportSha256").and_then(Value::as_str).unwrap_or("");
        if want.is_empty() {
            self.settle(slug, "ok", ticket);
            return verdict("no_report", None);
        }
        let Some(report) = h.report else { return bad(self) };
        if hex_digest(&report) != want {
            return bad(self);
        }
        self.settle(slug, "ok", ticket);
        match String::from_utf8(report) {
            Ok(t) => verdict("ok", Some(t)),
            Err(_) => verdict("not_text", None),
        }
    }

    pub fn take_over(&self, stream: &str, on: bool) -> Res {
        self.sup.set_take_over(stream, on).map_err(|e| fail("session_error", e))?;
        Ok(json!({"stream": stream, "take_over": on}))
    }

    /// The repository roots a terminal may open in: the run directories of the runs this app started or resumed in this
    /// process (the in-memory map, never `runs.jsonl`, which a worker can write). Each root's own `.claude/worktrees/*`
    /// folders are allowed too (see `terminal::policy::place`). ADR-15 replaces this with the project records.
    pub fn terminal_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = lock(&self.runs).values().map(|r| r.cwd.clone()).collect();
        roots.sort();
        roots.dedup();
        roots
    }

    pub fn active_runs(&self) -> Res {
        self.reap_runs();
        let runs = lock(&self.runs).clone();
        let mut active: Vec<Value> = runs
            .iter()
            .filter(|(s, _)| self.sup.is_running(s).unwrap_or(false))
            .map(|(s, r)| json!({"stream": s, "session_id": r.session_id, "run_id": r.run_id, "campaign": r.campaign}))
            .collect();
        active.sort_by(|a, b| a["stream"].as_str().cmp(&b["stream"].as_str()));
        Ok(json!({"count": active.len(), "runs": active}))
    }
}

// -------------------------------------------------------------------- helpers

/// Recent transcript bytes kept for the Lead's `inspect_worker`.
const INSPECT_BYTES: usize = 16 * 1024;

fn worker_stream(agent_id: &str) -> String {
    format!("w-{agent_id}")
}

/// Longest reason text sent to the UI.
const REASON_DETAIL_MAX: usize = 200;

/// A verification step that could not give a verdict. `code` is one of a fixed set chosen here (`spawn_failed`, `timeout`,
/// `bad_exit`, `jq_missing`, `oversize`, `io_error`, and for evidence also `refused`, `no_run_registered`,
/// `outside_campaign`); `detail` is bounded, single-line text (see `reason_detail`) and is untrusted: show it as text only.
fn reason(code: &str, detail: Option<String>, exit_code: Option<i32>) -> Value {
    let mut v = json!({"code": code});
    if let Some(d) = detail.filter(|d| !d.is_empty()) {
        v["detail"] = json!(d);
    }
    if let Some(c) = exit_code {
        v["exit_code"] = json!(c);
    }
    v
}

fn bridge_failure(e: &BridgeError) -> &'static str {
    match e {
        BridgeError::Spawn(_) => "spawn_failed",
        BridgeError::Invalid(m) if m == "jq not found" => "jq_missing",
        BridgeError::Invalid(_) => "io_error",
    }
}

/// First non-empty line of `raw`, control characters dropped, the home directory shown as `~`, at most
/// `REASON_DETAIL_MAX` characters. The reason text of a script can name paths; the home is the part that identifies the user.
fn reason_detail(raw: &str, home: &Path) -> String {
    let mut line = raw.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string();
    for h in [Some(home.to_path_buf()), home.canonicalize().ok()].into_iter().flatten() {
        let h = h.to_string_lossy().into_owned();
        if h.len() > 1 {
            line = line.replace(&h, "~");
        }
    }
    line.chars().filter(|c| !c.is_control()).take(REASON_DETAIL_MAX).collect()
}

/// A script that ran but did not give an answer: `timeout`, or `bad_exit` with its exit code and the first line of its
/// stderr (or the `error` of its JSON stdout).
fn output_failure(o: &Output, home: &Path) -> Value {
    if o.timed_out {
        return reason("timeout", None, None);
    }
    // Output cut at the bridge's cap: the exit code and the partial text of a cut run say nothing reliable.
    if o.truncated {
        return reason("oversize", Some("script output over the cap".into()), None);
    }
    let from_stdout = || serde_json::from_str::<Value>(o.stdout.trim()).ok().and_then(|v| v.get("error").and_then(Value::as_str).map(String::from));
    // The script's own missing-dependency message is a missing jq, not an unexplained exit.
    if o.stderr.contains("jq is required") {
        return reason("jq_missing", None, None);
    }
    let text = if o.stderr.trim().is_empty() { from_stdout().unwrap_or_default() } else { o.stderr.clone() };
    reason("bad_exit", Some(reason_detail(&text, home)), o.code)
}

/// What `check-evidence` said: green, refused (its `reasons[0]`), or did not answer.
fn evidence_outcome(o: &Output, home: &Path) -> Outcome {
    // A cut response cannot give a verdict, even if what was kept happens to parse.
    if o.truncated {
        return Outcome::Verdict(false, Some(output_failure(o, home)));
    }
    let parsed = serde_json::from_str::<Value>(o.stdout.trim()).ok();
    match (o.code, parsed) {
        (Some(0), Some(v)) if v.get("green").and_then(Value::as_bool) == Some(true) => Outcome::Verdict(true, None),
        (Some(0), Some(v)) => {
            let first = v.get("reasons").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).unwrap_or("");
            Outcome::Verdict(false, Some(reason("refused", Some(reason_detail(first, home)), None)))
        }
        _ => Outcome::Verdict(false, Some(output_failure(o, home))),
    }
}

fn invalid(field: &str, reason: &str) -> Value {
    json!({"error": "invalid_args", "detail": format!("{field}: {reason}")})
}



/// An existing absolute directory.
fn repo_dir(s: &str) -> Result<PathBuf, Value> {
    let p = PathBuf::from(s);
    if !p.is_absolute() || !p.is_dir() {
        return Err(invalid("cwd", "must be an existing absolute directory"));
    }
    Ok(p)
}


/// Claude Code's project dir name for a cwd: every non-alphanumeric is `-`.
fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn lines_json(lines: &[bus::Line]) -> Vec<Value> {
    lines.iter().map(|l| json!({"seq": l.seq, "text": &*l.text})).collect()
}

fn node_json(n: &Node) -> Value {
    json!({
        "id": n.id,
        "kind": format!("{:?}", n.kind).to_lowercase(),
        "title": n.title,
        "engine_id": n.engine_id,
        // The name the UI shows for the role (AC-2.1.3); the engine id is only a secondary field.
        "role": n.engine_id.as_deref().map(crate::crew::names::ui_name),
        "status": n.status.as_str(),
        "overlays": n.overlays,
        "children": n.children.iter().map(node_json).collect::<Vec<_>>(),
    })
}

/// Wait up to `wait` for bus events, coalesce for `COALESCE`, and return the
/// `changed` payload, or `None` when nothing happened.
pub fn collect_changes(sub: &Subscriber, wait: Duration) -> Option<Value> {
    let first = sub.recv_timeout(wait)?;
    std::thread::sleep(COALESCE);
    let mut streams = BTreeSet::new();
    let mut snapshot_needed = false;
    let mut add = |e: Event| match e {
        Event::Line { stream, .. } => {
            streams.insert(stream.to_string());
        }
        Event::SnapshotNeeded => snapshot_needed = true,
    };
    add(first);
    while let Some(e) = sub.try_recv() {
        add(e);
    }
    Some(json!({"streams": streams, "snapshot_needed": snapshot_needed}))
}

mod inbox;
mod projects;
mod wfresult;
mod decide;
mod lead;
mod runindex;
mod runview;
mod state;
pub use state::Envelope;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod state_tests;

/// The namespaced task id (`<run_id>.<task>`) and round of a workflow label such as `task.t1.r2`; `(None, None)` for
/// a label that names no task (`verify.r1`, `kb.search`). The workflow namespaces ids as `${RUN}.${id}`.
fn task_meta(run_id: &str, label: &str) -> (Option<String>, Option<u32>) {
    let mut parts = label.split('.');
    if parts.next() != Some("task") {
        return (None, None);
    }
    let Some(id) = parts.next().filter(|t| !t.is_empty()) else { return (None, None) };
    let round = parts.rev().find_map(|p| p.strip_prefix('r').and_then(|n| n.parse().ok()));
    (Some(format!("{run_id}.{id}")), round)
}
