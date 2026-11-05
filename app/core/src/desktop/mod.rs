//! The app's command layer: one `App` owns the bus, the PTY supervisor, the
//! control service and the read-only Vajra bridge, and answers each UI call.
//! `main.rs` only maps Tauri commands onto these methods.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::bus::{self, Bus, Config, Event, Subscriber};
use crate::control::service::{Actor, ControlError, Service, WorkerInfo};
use crate::adapter::{Engine, adapter_for};
use crate::control::ledger::Ledger;
use crate::ids::{self, valid_run_id};
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

pub struct App {
    data: PathBuf,
    claude: PathBuf,
    pub bus: Arc<Bus>,
    pub sup: Supervisor,
    pub control: Arc<Mutex<Service>>,
    bridge: Bridge,
    /// `None` until `~/.claude` exists.
    reader: Mutex<Option<Reader>>,
    /// By bus stream id.
    runs: Mutex<HashMap<String, Run>>,
    /// Worker transcripts by agent id, tailed into stream `w-<agent_id>`.
    transcripts: Mutex<HashMap<String, Transcript>>,
    /// Held from the one-active-run check until the run is recorded, so two
    /// Start clicks cannot both pass the check.
    launching: Mutex<()>,
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
            bridge: Bridge::new(claude.join("skills/vajra/scripts")),
            reader: Mutex::new(None),
            claude,
            sup: Supervisor::new(bus.clone(), log.clone()),
            bus,
            control: Arc::new(Mutex::new(svc)),
            runs: Mutex::new(HashMap::new()),
            transcripts: Mutex::new(HashMap::new()),
            launching: Mutex::new(()),
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

    fn run_by_id(&self, run_id: &str) -> Option<(String, Run)> {
        lock(&self.runs).iter().find(|(_, r)| r.run_id == run_id).map(|(k, r)| (k.clone(), r.clone()))
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
        self.with_reader(|r| {
            let slugs = r.campaigns().map_err(|e| fail("io_error", e))?;
            let list: Vec<Value> = slugs
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
                .collect();
            Ok(json!({"campaigns": list}))
        })
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
        if let Err(std::sync::TryLockError::WouldBlock) = self.launching.try_lock() {
            return;
        }
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
        Ok(json!({"stream": stream, "session_id": session_id, "run_id": run_id, "campaign": campaign}))
    }

    pub fn start_run(&self, input: RunInput, cwd: &str) -> Res {
        let cwd = repo_dir(cwd)?;
        argv::validate(&input).map_err(invalid_input)?;
        let now = ids::unix_now();
        let session_id = ids::uuid_v4().map_err(|e| fail("io_error", e))?;
        let run_id = ids::run_id_at(now);
        let campaign = input.campaign.clone().unwrap_or_else(|| format!("deliver-{run_id}"));
        let r = self.launch(&session_id, &run_id, &campaign, &cwd, |n| {
            argv::build_run_argv_at(&input, n, now, &session_id).map_err(invalid_input)
        }, false)?;
        self.note("run_started", &[]);
        Ok(r)
    }

    /// ADR-9 resume: `claude --resume <session_id>` continuing the campaign
    /// through `/vajra-deliver` with a rotated steering nonce (design 5).
    pub fn resume_run(&self, session_id: &str, run_id: &str, campaign: &str, cwd: &str) -> Res {
        let cwd = repo_dir(cwd)?;
        if !ids::valid_session_id(session_id) {
            return Err(invalid("session_id", "must be a canonical uuid"));
        }
        if !valid_run_id(run_id) {
            return Err(invalid("run_id", "must be YYYYMMDD-HHMM"));
        }
        if !bus::valid_stream_id(campaign) {
            return Err(invalid("campaign", "not a campaign slug"));
        }
        let claude = adapter_for(Engine::Claude);
        let r = self.launch(session_id, run_id, campaign, &cwd, |n| {
            claude.resume_argv(session_id, campaign, n).map_err(invalid_input)
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
        let run = self.run_by_id(run_id);
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
                    let t = self.bus.tail(&worker_stream(&w.agent_id), 0, INSPECT_BYTES);
                    let cwd = tr.get(&w.agent_id).and_then(|t| t.cwd()).map(str::to_string);
                    (w.agent_id.clone(), (cwd, t.lines.iter().map(|l| l.text.to_string()).collect()))
                })
                .collect()
        } else {
            HashMap::new()
        };
        // Only the live run's workers are steering targets; viewing a past run
        // never retargets the control service.
        if running {
            lock(&self.control).set_workers(
            workers
                .iter()
                .map(|w| {
                    let (cwd, lines) = ctx.get(&w.agent_id).cloned().unwrap_or_default();
                    WorkerInfo {
                    worker: w.clone(),
                    task_id: None,
                    title: Some(w.label.clone()),
                    round: None,
                    live: !matches!(w.status, Status::Done | Status::Failed),
                    last_activity: None,
                    cwd,
                    lines,
                }})
                .collect(),
            );
        }
        let run_status = if running { Status::InProgress } else { Status::Unknown };
        let lead = run.as_ref().map(|_| run_status);
        let node = self.with_reader(|r| {
            Ok(tree::snapshot(r, slug, run_id, lead, run_status, &workers, &Signals::default()))
        })?;
        Ok(node_json(&node))
    }

    fn discover_workers(&self, run: &Run) -> Result<Vec<Worker>, Value> {
        let allow = self.allow()?;
        let dir = self
            .claude
            .join("projects")
            .join(project_slug(&run.cwd))
            .join(&run.session_id)
            .join("subagents");
        let read = |p: &Path| -> Option<String> {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut allow.open_allowlisted(p).ok()?, &mut s).ok()?;
            Some(s)
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
            let journal = read(&d.join("journal.jsonl")).map(|s| parse_journal(&s)).unwrap_or_default();
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            let mut metas: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            metas.sort();
            for p in metas {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let Some(id) = name.strip_prefix("agent-").and_then(|n| n.strip_suffix(".meta.json")) else { continue };
                let Some(meta) = read(&p).and_then(|s| parse_meta(&s)) else { continue };
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
        let evidence = match self.bridge.run(&Request::CheckEvidence { slug: slug.into(), task_id: task_id.into() }) {
            Ok(o) => json!({"code": o.code, "result": serde_json::from_str::<Value>(o.stdout.trim()).ok()}),
            Err(e) => json!({"code": null, "error": e.to_string()}),
        };
        Ok(json!({"task": task, "stale": snap.stale, "evidence": evidence}))
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

#[cfg(test)]
mod tests;
