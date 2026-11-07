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
use crate::vajra::bridge::{Bridge, Request, parse_kb_hits};
use crate::vajra::preflight;
use crate::vajra::state::Reader;

/// Every command answers `Ok(value)` or `Err({error, detail?})`.
pub type Res = Result<Value, Value>;

/// Bus events coalesce for this long before one `changed` is emitted.
const COALESCE: Duration = Duration::from_millis(150);
/// Upper bound for one `tail` / `read_spill` page.
const MAX_PAGE: usize = 64 * 1024;
/// Largest worker meta file and workflow journal read whole (both are Claude Code files; over-limit ones are skipped).
const MAX_META_BYTES: u64 = 64 * 1024;
const MAX_JOURNAL_BYTES: u64 = 8 * 1024 * 1024;
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

const LIST_VERIFY_WORKERS: usize = 4;
/// How long a real verdict is reused. The badge is a trust signal, so the window is short; it bounds the verify
/// processes started by a burst of `changed` events to one per campaign per window (and `refresh` skips it).
const VERDICT_TTL: Duration = Duration::from_secs(5);

/// One in-flight verify, shared by every caller that asks for the same campaign while it runs.
type Flight = Arc<(Mutex<Option<&'static str>>, Condvar)>;
enum Verdict {
    Fresh(Duration, &'static str),
    Flying(Flight),
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
            lock(&self.app.verdicts).remove(self.slug);
            *lock(&self.flight.0) = Some("unknown");
            self.flight.1.notify_all();
        }
    }
}
const LIST_VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

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
    /// slug -> a real verdict with the time it was taken, or the verify now running for it. "unknown" is never kept.
    verdicts: Mutex<HashMap<String, Verdict>>,
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
        Ok(Self {
            data: data.to_path_buf(),
            home: home.to_path_buf(),
            lead: lead::LeadState::new(home),
            shown: Mutex::new(HashMap::new()),
            deciding: Mutex::new(Default::default()),
            bridge: Bridge::new(claude.join("skills/vajra/scripts")).with_home(home.to_path_buf()),
            verify_timeout: LIST_VERIFY_TIMEOUT,
            verdicts: Mutex::new(HashMap::new()),
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
    fn fresh_run_id(&self, now: u64) -> (u64, String) {
        let on_disk: Vec<String> = self.with_reader(|r| Ok(r.campaigns().unwrap_or_default())).unwrap_or_default();
        // Run ids of earlier app lives: after a restart in the same minute, an older campaign's id is still taken.
        let indexed = runindex::run_ids(&self.data.join("runs.jsonl"));
        let used = |id: &str| lock(&self.runs).values().any(|r| r.run_id == id) || lock(&self.issued).contains(id) || indexed.contains(id) || on_disk.iter().any(|c| c == &format!("deliver-{id}"));
        let pick = (0..120u64).map(|k| now + 60 * k).map(|t| (t, ids::run_id_at(t))).find(|(_, id)| !used(id)).unwrap_or((now, ids::run_id_at(now)));
        lock(&self.issued).insert(pick.1.clone());
        pick
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
        // Every listing verifies every campaign afresh (the verdict depends on the campaign files, the links they
        // follow and the signing key, so nothing is remembered between listings), on a few worker threads.
        let todo: Vec<(usize, String)> = list
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c["slug"].as_str().unwrap_or_default().to_string()))
            .collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let verdicts: Mutex<Vec<(usize, &'static str)>> = Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for _ in 0..todo.len().min(LIST_VERIFY_WORKERS) {
                s.spawn(|| loop {
                    let n = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((i, slug)) = todo.get(n) else { break };
                    let v = self.integrity_of(slug, refresh);
                    lock(&verdicts).push((*i, v));
                });
            }
        });
        for (i, v) in lock(&verdicts).iter() {
            list[*i]["integrity"] = json!(*v);
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
        if svc.session_id().is_some_and(|id| ended.iter().any(|e| e == id)) {
            svc.end_session();
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
        let _one = lock(&self.launching);
        self.no_active_run()?;
        let nonce = {
            let mut svc = lock(&self.control);
            ctl(svc.start_session(session_id, run_id))?;
            let n = if rotate { ctl(svc.resume_session()) } else { Ok(svc.nonce().unwrap_or_default()) };
            match n {
                Ok(n) => n.to_string(),
                Err(e) => {
                    svc.end_session();
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
            lock(&self.control).end_session();
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
        if self.record_run_cwd(run_id, cwd).is_err() {
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
        if out.code == Some(0) && sha.len() == 40 && argv::is_sha(sha) {
            Ok(sha.to_string())
        } else {
            Err(invalid("base", "is not a branch or commit in this repository"))
        }
    }

    pub fn start_run(&self, mut input: RunInput, cwd: &str) -> Res {
        let cwd = repo_dir(cwd)?;
        argv::validate(&input).map_err(invalid_input)?;
        input.base = self.resolve_base(&cwd, &input.base)?;
        let (now, run_id) = self.fresh_run_id(ids::unix_now());
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
        let (_, run_id) = self.fresh_run_id(ids::unix_now());
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
        if running {
            self.publish_workers(run_id, &workers, &ctx);
        }
        let run_status = if running { Status::InProgress } else { Status::Unknown };
        let lead = run.as_ref().map(|_| run_status);
        // Overlays come from the live control session only; a past run shows none.
        let signals = if running { lock(&self.control).signals() } else { Signals::default() };
        let node = self.with_reader(|r| Ok(tree::snapshot(r, slug, run_id, lead, run_status, &workers, &signals)))?;
        Ok(node_json(&node))
    }

    fn publish_workers(&self, run_id: &str, workers: &[Worker], ctx: &HashMap<String, (Option<String>, Vec<String>)>) {
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
        lock(&self.control).set_workers(infos);
    }

    /// Keep the control service's workers and the transcript tails current for the live run, without any UI view
    /// asking: steering and MCP inspect must work while the crew view is closed. Called by the core's own poll loop.
    pub fn refresh_workers(&self) {
        let live = lock(&self.runs).iter().find(|(s, _)| self.sup.is_running(s).unwrap_or(false)).map(|(_, r)| r.clone());
        let Some(run) = live else { return };
        let Ok(workers) = self.discover_workers(&run) else { return };
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
        self.publish_workers(&run.run_id, &workers, &ctx);
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
            let svc = lock(&self.control);
            if svc.session_id() == Some(run.session_id.as_str()) {
                red.add(svc.nonce());
            }
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
                Ok(o) => json!({"code": o.code, "result": serde_json::from_str::<Value>(o.stdout.trim()).ok()}),
                Err(e) => json!({"code": null, "error": e.to_string()}),
            },
        };
        Ok(json!({"task": task, "stale": snap.stale, "evidence": evidence}))
    }

    /// An archived bundle (under the campaign's evidence dir) is checked
    /// without a repo; live evidence is checked in the repo of the run that
    /// made it. `None` when the task has no evidence path or no repo is known.
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

    pub fn diff(&self, cwd: &str, base: &str) -> Res {
        let cwd = repo_dir(cwd)?;
        if base.is_empty() || base.starts_with('-') || base.len() > 255 {
            return Err(invalid("base", "must be a ref or commit"));
        }
        let o = self
            .bridge
            .run(&Request::Git { cwd, args: vec!["diff".into(), base.into(), "--".into()] })
            .map_err(|e| fail("git_failed", e))?;
        Ok(json!({"code": o.code, "diff": o.stdout, "truncated": o.truncated, "timed_out": o.timed_out}))
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
        let hits: Vec<Value> =
            parse_kb_hits(&o.stdout).into_iter().map(|h| json!({"id": h.id, "kind": h.kind, "title": h.title})).collect();
        Ok(json!({"hits": hits, "code": o.code}))
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
            let waiting = match m.get(slug) {
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
                if !refresh {
                    return (got.unwrap_or("unknown"), None);
                }
                continue;
            }
            let f: Flight = Arc::new((Mutex::new(None), Condvar::new()));
            m.insert(slug.to_string(), Verdict::Flying(f.clone()));
            break f;
        };
        let mut guard = FlightGuard { app: self, slug, flight, done: false };
        let mut bridge = self.bridge.clone();
        bridge.timeout = self.verify_timeout;
        let res = bridge.run(&Request::CampaignVerify { slug: slug.into() });
        let verdict = match res.as_ref().ok().and_then(|o| o.code) {
            Some(0) => "ok",
            Some(3) => "failed",
            _ => "unknown",
        };
        self.settle(slug, verdict);
        *lock(&guard.flight.0) = Some(verdict);
        guard.flight.1.notify_all();
        guard.done = true;
        (verdict, Some(res))
    }

    #[allow(dead_code)] // read_report (#22b) settles through it
    fn settle_code(&self, slug: &str, code: Option<i32>) {
        let v = match code {
            Some(0) => "ok",
            Some(3) => "failed",
            _ => "unknown",
        };
        let m = lock(&self.verdicts);
        if !matches!(m.get(slug), Some(Verdict::Flying(_))) {
            drop(m);
            self.settle(slug, v);
        }
    }

    /// Remember a real verdict for the slug (or forget it when the check could not run).
    fn settle(&self, slug: &str, verdict: &'static str) {
        let mut m = lock(&self.verdicts);
        if verdict == "unknown" {
            m.remove(slug);
        } else {
            m.insert(slug.to_string(), Verdict::Fresh((self.clock)(), verdict));
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
        match o.code {
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
            code => Err(fail("verify_failed", format!("exit {code:?}: {}", o.stderr.trim()))),
        }
    }

    pub fn take_over(&self, stream: &str, on: bool) -> Res {
        self.sup.set_take_over(stream, on).map_err(|e| fail("session_error", e))?;
        Ok(json!({"stream": stream, "take_over": on}))
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
mod wfresult;
mod decide;
mod lead;
mod runindex;

#[cfg(test)]
mod tests;

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
