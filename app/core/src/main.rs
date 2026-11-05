//! Vajra Desktop entry point: a thin Tauri v2 shell over the C1-C6 core.
//!
//! `--mcp` runs the stdio MCP server (the Lead chat session's side of the
//! control socket) and never starts Tauri. Otherwise the app opens its data
//! dir, the bus, the PTY supervisor and the control service, serves the
//! owner-only socket, and registers one Tauri command per UI call. The UI
//! learns about new output through a single `changed` event and pulls the
//! data with `tail` / `read_spill`.
//!
//! ADR-9: closing the window hides it; runs keep going in this process.
//! The menu bar item and the Cmd-Q prompt are M4.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tauri::{Emitter, Manager, RunEvent, State, WindowEvent};

use vajra_desktop::bus::{self, Bus, Config, Event, Subscriber};
use vajra_desktop::control::service::{self, Actor, ControlError, Service, WorkerInfo};
use vajra_desktop::control::{self, ledger::Ledger, mcp, nonce, socket};
use vajra_desktop::crew::record::{parse_journal, parse_meta};
use vajra_desktop::crew::tree::{self, Node, Signals, Status, Worker};
use vajra_desktop::logging::{self, Logger};
use vajra_desktop::paths;
use vajra_desktop::sessions::argv::{RunInput, build_run_argv_at, run_id_at};
use vajra_desktop::sessions::{SpawnSpec, Supervisor};
use vajra_desktop::vajra::allowlist::Allowlist;
use vajra_desktop::vajra::bridge::{Bridge, Request, parse_kb_hits};
use vajra_desktop::vajra::preflight;
use vajra_desktop::vajra::state::Reader;

/// Every command answers `Ok(value)` or `Err({error, detail?})`.
type Res = Result<Value, Value>;

/// The one event the UI listens to.
const CHANGED: &str = "changed";
/// Bus events coalesce for this long before one `changed` is emitted.
const COALESCE: Duration = Duration::from_millis(150);
/// Upper bound for one `tail` / `read_spill` page.
const MAX_PAGE: usize = 1024 * 1024;
const PTY_ROWS: u16 = 40;
const PTY_COLS: u16 = 120;

fn fail(code: &str, detail: impl std::fmt::Display) -> Value {
    json!({"error": code, "detail": detail.to_string()})
}

fn ctl(r: Result<Value, ControlError>) -> Res {
    r.map_err(|e: ControlError| e.to_json())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A run session started (or resumed) by this app.
#[derive(Clone, Debug)]
struct Run {
    session_id: String,
    run_id: String,
    campaign: String,
    cwd: PathBuf,
}

struct App {
    data: PathBuf,
    claude: PathBuf,
    bus: Arc<Bus>,
    sup: Supervisor,
    control: Arc<Mutex<Service>>,
    bridge: Bridge,
    /// `None` until `~/.claude` exists.
    reader: Mutex<Option<Reader>>,
    /// By bus stream id.
    runs: Mutex<HashMap<String, Run>>,
    log: Option<Arc<Logger>>,
}

impl App {
    /// Open everything under `data` (the app data dir) for the user whose
    /// home is `home`. Nothing under `home` is written.
    fn open(data: &Path, home: &Path) -> std::io::Result<Self> {
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
            log,
        })
    }

    fn note(&self, event: &'static str, fields: &[(&'static str, u64)]) {
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

    fn preflight(&self) -> Res {
        let allow = self.allow()?;
        let rows = preflight::run(&allow, &self.bridge, &preflight::path_dirs());
        let ok = preflight::start_enabled(&rows);
        let rows: Vec<Value> =
            rows.iter().map(|r| json!({"name": r.name, "ok": r.ok, "detail": r.detail, "fix": r.fix})).collect();
        Ok(json!({"rows": rows, "start_enabled": ok}))
    }

    fn list_campaigns(&self) -> Res {
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

    fn start_run(&self, input: RunInput, cwd: &str) -> Res {
        let cwd = repo_dir(cwd)?;
        let now = unix_now();
        let session_id = new_uuid().map_err(|e| fail("io_error", e))?;
        let run_id = run_id_at(now);
        // Validate the form before a control session is created for it.
        build_run_argv_at(&input, &"0".repeat(32), now, &session_id).map_err(|e| invalid(e.field, e.reason))?;
        let nonce = {
            let mut svc = lock(&self.control);
            ctl(svc.start_session(&session_id, &run_id).map(|_| Value::Null))?;
            svc.nonce().unwrap_or_default().to_string()
        };
        let argv = build_run_argv_at(&input, &nonce, now, &session_id).map_err(|e| invalid(e.field, e.reason))?;
        let campaign = input.campaign.clone().unwrap_or_else(|| format!("deliver-{run_id}"));
        let stream = format!("run-{session_id}");
        self.sup
            .spawn(&stream, SpawnSpec { argv, cwd: cwd.clone(), rows: PTY_ROWS, cols: PTY_COLS })
            .map_err(|e| fail("spawn_failed", e))?;
        lock(&self.runs).insert(stream.clone(), Run { session_id: session_id.clone(), run_id: run_id.clone(), campaign: campaign.clone(), cwd });
        self.note("run_started", &[]);
        Ok(json!({"stream": stream, "session_id": session_id, "run_id": run_id, "campaign": campaign}))
    }

    /// ADR-9 resume: `claude --resume <session_id>` then `/vajra continue`
    /// for the campaign. The steering nonce rotates (design 5).
    fn resume_run(&self, session_id: &str, run_id: &str, campaign: &str, cwd: &str) -> Res {
        let cwd = repo_dir(cwd)?;
        if !control::queue::valid_session_id(session_id) {
            return Err(invalid("session_id", "not a session id"));
        }
        if !valid_run_id(run_id) {
            return Err(invalid("run_id", "must be YYYYMMDD-HHMM"));
        }
        if !bus::valid_stream_id(campaign) {
            return Err(invalid("campaign", "not a campaign slug"));
        }
        {
            let mut svc = lock(&self.control);
            ctl(svc.start_session(session_id, run_id).map(|_| Value::Null))?;
            ctl(svc.resume_session().map(|_| Value::Null))?;
        }
        let argv = resume_argv(session_id, campaign);
        let stream = format!("run-{session_id}");
        self.sup
            .spawn(&stream, SpawnSpec { argv, cwd: cwd.clone(), rows: PTY_ROWS, cols: PTY_COLS })
            .map_err(|e| fail("spawn_failed", e))?;
        lock(&self.runs).insert(
            stream.clone(),
            Run { session_id: session_id.into(), run_id: run_id.into(), campaign: campaign.into(), cwd },
        );
        self.note("run_resumed", &[]);
        Ok(json!({"stream": stream, "session_id": session_id, "run_id": run_id, "campaign": campaign}))
    }

    // ---------------------------------------------------------------- crew

    /// The crew tree for a run. Workers are read from the run session's
    /// `subagents` dir (meta files and workflow journals) when the app knows
    /// the session; the control service is told about them so steering can
    /// address them.
    fn crew(&self, slug: &str, run_id: &str) -> Res {
        let run = self.run_by_id(run_id);
        let running = run.as_ref().is_some_and(|(s, _)| self.sup.is_running(s).unwrap_or(false));
        let workers = match &run {
            Some((_, r)) => self.discover_workers(r)?,
            None => Vec::new(),
        };
        lock(&self.control).set_workers(
            workers
                .iter()
                .map(|w| WorkerInfo {
                    worker: w.clone(),
                    task_id: None,
                    title: Some(w.label.clone()),
                    round: None,
                    live: running && !matches!(w.status, Status::Done | Status::Failed),
                    last_activity: None,
                    cwd: None,
                    lines: Vec::new(),
                })
                .collect(),
        );
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
                out.push(tree::worker_from(id, &meta, j, None));
            }
        }
        Ok(out)
    }

    fn tail(&self, stream: &str, after_seq: u64, max_bytes: usize) -> Res {
        if !bus::valid_stream_id(stream) {
            return Err(invalid("stream", "not a stream id"));
        }
        let t = self.bus.tail(stream, after_seq, max_bytes.clamp(1, MAX_PAGE));
        Ok(json!({
            "lines": lines_json(&t.lines), "gap": t.gap, "head_seq": t.head_seq,
            "spill_start": t.spill_start, "spill_end": t.spill_end,
        }))
    }

    fn read_spill(&self, stream: &str, offset: u64, max_bytes: usize) -> Res {
        if !bus::valid_stream_id(stream) {
            return Err(invalid("stream", "not a stream id"));
        }
        let p = self.bus.read_spill(stream, offset, max_bytes.clamp(1, MAX_PAGE)).map_err(|e| fail("io_error", e))?;
        Ok(json!({"lines": lines_json(&p.lines), "next_offset": p.next_offset, "gap": p.gap, "end": p.end}))
    }

    fn task(&self, slug: &str, task_id: &str) -> Res {
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

    fn diff(&self, cwd: &str, base: &str) -> Res {
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

    fn control_message(&self, worker: &str, text: &str) -> Res {
        ctl(lock(&self.control).message_worker(Actor::Human, worker, text))
    }

    fn control_redirect(&self, worker: &str, text: &str) -> Res {
        ctl(lock(&self.control).redirect_worker(Actor::Human, worker, text))
    }

    fn control_stop(&self, worker: &str, reason: Option<&str>) -> Res {
        ctl(lock(&self.control).stop_worker(Actor::Human, worker, reason))
    }

    fn confirm_stop(&self, action_id: &str) -> Res {
        ctl(lock(&self.control).confirm_stop(Actor::Human, action_id))
    }

    fn resume_worker(&self, worker: &str) -> Res {
        ctl(lock(&self.control).resume_worker(Actor::Human, worker))
    }

    fn ledger(&self) -> Res {
        let path = self.data.join("control/ledger.jsonl");
        let recs = Ledger::read_all(&path).map_err(|e| fail("io_error", e))?;
        Ok(json!({"records": recs}))
    }

    fn kb_search(&self, query: &str, k: u32, repo: &str) -> Res {
        let repo = repo_dir(repo)?;
        let o = self
            .bridge
            .run(&Request::KbSearch { query: query.into(), k: k.clamp(1, 50), repo })
            .map_err(|e| fail("kb_failed", e))?;
        let hits: Vec<Value> =
            parse_kb_hits(&o.stdout).into_iter().map(|h| json!({"id": h.id, "kind": h.kind, "title": h.title})).collect();
        Ok(json!({"hits": hits, "code": o.code}))
    }

    fn take_over(&self, stream: &str, on: bool) -> Res {
        self.sup.set_take_over(stream, on).map_err(|e| fail("session_error", e))?;
        Ok(json!({"stream": stream, "take_over": on}))
    }

    fn active_runs(&self) -> Res {
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

fn invalid(field: &str, reason: &str) -> Value {
    json!({"error": "invalid_args", "detail": format!("{field}: {reason}")})
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// `YYYYMMDD-HHMM`.
fn valid_run_id(r: &str) -> bool {
    let b = r.as_bytes();
    b.len() == 13 && b[8] == b'-' && b.iter().enumerate().all(|(i, c)| i == 8 || c.is_ascii_digit())
}

/// An existing absolute directory.
fn repo_dir(s: &str) -> Result<PathBuf, Value> {
    let p = PathBuf::from(s);
    if !p.is_absolute() || !p.is_dir() {
        return Err(invalid("cwd", "must be an existing absolute directory"));
    }
    Ok(p)
}

/// A v4 UUID from 16 random bytes.
fn new_uuid() -> std::io::Result<String> {
    Ok(uuid_from_hex(&nonce::random_hex(16)?))
}

fn uuid_from_hex(h: &str) -> String {
    let mut c: Vec<char> = h.chars().collect();
    c[12] = '4';
    let v = c[16].to_digit(16).unwrap_or(0);
    c[16] = char::from_digit((v & 0x3) | 0x8, 16).unwrap_or('8');
    let s: String = c.into_iter().collect();
    format!("{}-{}-{}-{}-{}", &s[0..8], &s[8..12], &s[12..16], &s[16..20], &s[20..32])
}

/// Claude Code's project dir name for a cwd: every non-alphanumeric is `-`.
fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn resume_argv(session_id: &str, campaign: &str) -> Vec<String> {
    vec!["claude".into(), "--resume".into(), session_id.into(), format!("/vajra continue --campaign {campaign}")]
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
fn collect_changes(sub: &Subscriber, wait: Duration) -> Option<Value> {
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

fn is_mcp(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a == "--mcp")
}

// -------------------------------------------------------------------- commands

#[tauri::command(async)]
fn preflight(app: State<'_, App>) -> Res {
    app.preflight()
}

#[tauri::command(async)]
fn list_campaigns(app: State<'_, App>) -> Res {
    app.list_campaigns()
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
fn start_run(
    app: State<'_, App>,
    ask: String,
    max_rounds: u32,
    base: String,
    max_in_flight: u32,
    campaign: Option<String>,
    cwd: String,
) -> Res {
    app.start_run(RunInput { ask, max_rounds, base, max_in_flight, campaign }, &cwd)
}

#[tauri::command(async)]
fn resume_run(app: State<'_, App>, session_id: String, run_id: String, campaign: String, cwd: String) -> Res {
    app.resume_run(&session_id, &run_id, &campaign, &cwd)
}

#[tauri::command(async)]
fn crew(app: State<'_, App>, slug: String, run_id: String) -> Res {
    app.crew(&slug, &run_id)
}

#[tauri::command(async)]
fn tail(app: State<'_, App>, stream: String, after_seq: u64, max_bytes: usize) -> Res {
    app.tail(&stream, after_seq, max_bytes)
}

#[tauri::command(async)]
fn read_spill(app: State<'_, App>, stream: String, offset: u64, max_bytes: usize) -> Res {
    app.read_spill(&stream, offset, max_bytes)
}

#[tauri::command(async)]
fn task(app: State<'_, App>, slug: String, task_id: String) -> Res {
    app.task(&slug, &task_id)
}

#[tauri::command(async)]
fn diff(app: State<'_, App>, cwd: String, base: String) -> Res {
    app.diff(&cwd, &base)
}

#[tauri::command(async)]
fn control_message(app: State<'_, App>, worker: String, text: String) -> Res {
    app.control_message(&worker, &text)
}

#[tauri::command(async)]
fn control_redirect(app: State<'_, App>, worker: String, text: String) -> Res {
    app.control_redirect(&worker, &text)
}

#[tauri::command(async)]
fn control_stop(app: State<'_, App>, worker: String, reason: Option<String>) -> Res {
    app.control_stop(&worker, reason.as_deref())
}

#[tauri::command(async)]
fn confirm_stop(app: State<'_, App>, action_id: String) -> Res {
    app.confirm_stop(&action_id)
}

#[tauri::command(async)]
fn resume_worker(app: State<'_, App>, worker: String) -> Res {
    app.resume_worker(&worker)
}

#[tauri::command(async)]
fn ledger(app: State<'_, App>) -> Res {
    app.ledger()
}

#[tauri::command(async)]
fn kb_search(app: State<'_, App>, query: String, k: u32, repo: String) -> Res {
    app.kb_search(&query, k, &repo)
}

#[tauri::command(async)]
fn take_over(app: State<'_, App>, stream: String, on: bool) -> Res {
    app.take_over(&stream, on)
}

#[tauri::command(async)]
fn active_runs(app: State<'_, App>) -> Res {
    app.active_runs()
}

// -------------------------------------------------------------------- main

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data = match paths::data_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("vajra-desktop: data dir: {e}");
            std::process::exit(1);
        }
    };
    if is_mcp(&args) {
        // Before Tauri: stdout belongs to the MCP protocol.
        let code = match mcp::run_stdio(&control::socket_path(&data)) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("vajra-desktop --mcp: {e}");
                1
            }
        };
        std::process::exit(code);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let app = match App::open(&data, &home) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("vajra-desktop: startup: {e}");
            std::process::exit(1);
        }
    };
    if let Some(l) = &app.log {
        let _ = l.log_cold_start(logging::process_start_time());
    }
    let _watcher = service::spawn_watcher(app.control.clone(), Duration::from_secs(1));
    match socket::bind(&control::socket_path(&data)) {
        Ok(listener) => {
            let svc = app.control.clone();
            // SAFETY: getuid has no preconditions and cannot fail.
            let uid = unsafe { libc::getuid() };
            std::thread::spawn(move || socket::serve(listener, svc, uid));
        }
        Err(e) => {
            eprintln!("vajra-desktop: control socket: {e}");
            app.note("control_socket_failed", &[]);
        }
    }
    let sub = app.bus.subscribe();

    let built = tauri::Builder::default()
        .manage(app)
        .setup(move |a| {
            let handle = a.handle().clone();
            std::thread::spawn(move || {
                loop {
                    if let Some(payload) = collect_changes(&sub, Duration::from_secs(1)) {
                        let _ = handle.emit(CHANGED, payload);
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // ADR-9: closing the window keeps runs going.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            preflight,
            list_campaigns,
            start_run,
            resume_run,
            crew,
            tail,
            read_spill,
            task,
            diff,
            control_message,
            control_redirect,
            control_stop,
            confirm_stop,
            resume_worker,
            ledger,
            kb_search,
            take_over,
            active_runs,
        ])
        .build(tauri::generate_context!());
    let tauri_app = match built {
        Ok(a) => a,
        Err(e) => {
            eprintln!("vajra-desktop: tauri: {e}");
            std::process::exit(1);
        }
    };
    tauri_app.run(|handle, event| match event {
        // Dock icon click after the window was hidden.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            if let Some(w) = handle.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        RunEvent::Exit => handle.state::<App>().sup.shutdown(),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    /// A temp dir holding `home/` and `data/`; removed on drop.
    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Self {
            let d = std::env::temp_dir()
                .join(format!("vajra-main-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            let _ = fs::remove_dir_all(&d);
            fs::create_dir_all(d.join("home/.claude/vajra/campaigns")).unwrap();
            Self(d)
        }
        fn home(&self) -> PathBuf {
            self.0.join("home")
        }
        fn app(&self) -> App {
            App::open(&self.0.join("data"), &self.home()).unwrap()
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn code(v: &Value) -> &str {
        v["error"].as_str().unwrap_or("")
    }

    #[test]
    fn mcp_flag_is_found_only_after_argv0() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_mcp(&a(&["vajra-desktop", "--mcp"])));
        assert!(is_mcp(&a(&["vajra-desktop", "-x", "--mcp"])));
        assert!(!is_mcp(&a(&["--mcp"])));
        assert!(!is_mcp(&a(&["vajra-desktop", "--mcpx"])));
    }

    #[test]
    fn uuid_is_v4_shaped() {
        let u = new_uuid().unwrap();
        let parts: Vec<&str> = u.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
        assert!(parts[2].starts_with('4'));
        assert!(matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'));
        assert!(control::queue::valid_session_id(&u));
        assert_eq!(uuid_from_hex("ffffffffffffffffffffffffffffffff"), "ffffffff-ffff-4fff-bfff-ffffffffffff");
    }

    #[test]
    fn project_slug_matches_claude_code() {
        assert_eq!(project_slug(Path::new("/Users/a/my.repo")), "-Users-a-my-repo");
    }

    #[test]
    fn resume_argv_is_an_array_with_continue() {
        let v = resume_argv("abc-12345678", "deliver-20261105-1810");
        assert_eq!(v[..3], ["claude", "--resume", "abc-12345678"]);
        assert_eq!(v[3], "/vajra continue --campaign deliver-20261105-1810");
    }

    #[test]
    fn changes_coalesce_into_one_payload() {
        let t = Tmp::new();
        let app = t.app();
        let sub = app.bus.subscribe();
        assert!(collect_changes(&sub, Duration::from_millis(20)).is_none());
        app.bus.push("s1", "a").unwrap();
        app.bus.push("s2", "b").unwrap();
        app.bus.push("s1", "c").unwrap();
        let p = collect_changes(&sub, Duration::from_secs(1)).unwrap();
        assert_eq!(p["streams"], json!(["s1", "s2"]));
        assert_eq!(p["snapshot_needed"], json!(false));
        assert!(sub.is_empty());
    }

    #[test]
    fn tail_and_read_spill_return_json_pages() {
        let t = Tmp::new();
        let app = t.app();
        app.bus.push("s", "one").unwrap();
        app.bus.push("s", "two").unwrap();
        let v = app.tail("s", 0, 1 << 20).unwrap();
        assert_eq!(v["head_seq"], json!(2));
        assert_eq!(v["lines"][1]["text"], json!("two"));
        assert_eq!(code(&app.tail("../x", 0, 10).unwrap_err()), "invalid_args");
        let p = app.read_spill("s", 0, 1024).unwrap();
        assert!(p["lines"].as_array().unwrap().is_empty());
        assert_eq!(code(&app.read_spill("a/b", 0, 10).unwrap_err()), "invalid_args");
    }

    #[test]
    fn start_run_rejects_bad_input_before_creating_a_session() {
        let t = Tmp::new();
        let app = t.app();
        let input = |ask: &str| RunInput {
            ask: ask.into(),
            max_rounds: 2,
            base: "main".into(),
            max_in_flight: 4,
            campaign: None,
        };
        let cwd = t.0.to_string_lossy().into_owned();
        assert_eq!(code(&app.start_run(input("   "), &cwd).unwrap_err()), "invalid_args");
        assert_eq!(code(&app.start_run(input("do it"), "relative/dir").unwrap_err()), "invalid_args");
        assert!(!t.0.join("data/control/sessions").exists() || fs::read_dir(t.0.join("data/control/sessions")).unwrap().next().is_none());
        assert_eq!(app.active_runs().unwrap(), json!({"count": 0, "runs": []}));
    }

    #[test]
    fn resume_run_validates_ids() {
        let t = Tmp::new();
        let app = t.app();
        let cwd = t.0.to_string_lossy().into_owned();
        let e = app.resume_run("bad id;rm", "20261105-1810", "deliver-x", &cwd).unwrap_err();
        assert_eq!(code(&e), "invalid_args");
        let e = app.resume_run("abc-12345678", "2026", "deliver-x", &cwd).unwrap_err();
        assert_eq!(code(&e), "invalid_args");
        let e = app.resume_run("abc-12345678", "20261105-1810", "../x", &cwd).unwrap_err();
        assert_eq!(code(&e), "invalid_args");
    }

    #[test]
    fn control_commands_act_as_the_human_and_fail_closed_without_a_session() {
        let t = Tmp::new();
        let app = t.app();
        assert_eq!(code(&app.control_message("w1", "hi").unwrap_err()), "control_disabled");
        assert_eq!(code(&app.control_stop("w1", None).unwrap_err()), "control_disabled");
        assert_eq!(code(&app.confirm_stop("a000").unwrap_err()), "action_not_found");
        let l = app.ledger().unwrap();
        assert!(l["records"].is_array());
    }

    #[test]
    fn take_over_on_an_unknown_stream_errors() {
        let t = Tmp::new();
        let app = t.app();
        assert_eq!(code(&app.take_over("run-none", true).unwrap_err()), "session_error");
    }

    #[test]
    fn campaigns_and_crew_read_through_the_allowlist() {
        let t = Tmp::new();
        let camp = t.home().join(".claude/vajra/campaigns/deliver-20261105-1810");
        fs::create_dir_all(&camp).unwrap();
        fs::write(
            camp.join("tasks.json"),
            serde_json::to_vec(&json!({"campaign": "deliver-20261105-1810", "schema": "vajra.tasks/v1",
                "tasks": [{"id": "t1", "owner_agent": "bhima", "status": "in_progress"}]}))
            .unwrap(),
        )
        .unwrap();
        let app = t.app();
        let c = app.list_campaigns().unwrap();
        assert!(c["campaigns"].as_array().unwrap().iter().any(|x| x["slug"] == "deliver-20261105-1810"));
        let tree = app.crew("deliver-20261105-1810", "20261105-1810").unwrap();
        assert!(tree["kind"].is_string());
        let s = tree.to_string();
        assert!(s.contains("in_progress"), "{s}");
        assert_eq!(code(&app.task("deliver-20261105-1810", "t9").unwrap_err()), "task_not_found");
    }

    #[test]
    fn workers_are_discovered_from_the_session_subagents_dir() {
        let t = Tmp::new();
        let repo = t.0.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let sid = "11111111-2222-4333-8444-555555555555";
        let sub = t.home().join(".claude/projects").join(project_slug(&repo)).join(sid).join("subagents");
        fs::create_dir_all(sub.join("workflows/wf_1")).unwrap();
        fs::write(sub.join("agent-a1.meta.json"), r#"{"agentType":"bhima","description":"top"}"#).unwrap();
        fs::write(sub.join("workflows/wf_1/agent-a2.meta.json"), r#"{"agentType":"vajra:arjuna","description":"d"}"#).unwrap();
        fs::write(
            sub.join("workflows/wf_1/journal.jsonl"),
            "{\"type\":\"started\",\"agentId\":\"a2\",\"label\":\"task.t1.r1\",\"phase\":\"Execute\"}\n{\"type\":\"result\",\"agentId\":\"a2\"}\n",
        )
        .unwrap();
        let app = t.app();
        let run = Run { session_id: sid.into(), run_id: "20261105-1810".into(), campaign: "c".into(), cwd: repo };
        let ws = app.discover_workers(&run).unwrap();
        assert_eq!(ws.len(), 2);
        let a2 = ws.iter().find(|w| w.agent_id == "a2").unwrap();
        assert_eq!(a2.label, "task.t1.r1");
        assert_eq!(a2.status, Status::Done);
        assert_eq!(ws.iter().find(|w| w.agent_id == "a1").unwrap().status, Status::Unknown);
    }

    #[test]
    fn missing_claude_dir_is_an_error_not_a_panic() {
        let t = Tmp::new();
        fs::remove_dir_all(t.home().join(".claude")).unwrap();
        let app = t.app();
        assert_eq!(code(&app.list_campaigns().unwrap_err()), "claude_dir_missing");
        assert_eq!(code(&app.preflight().unwrap_err()), "claude_dir_missing");
    }

    fn rss_kb() -> u64 {
        let out = std::process::Command::new("/bin/ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    /// Perf probe, run by hand: `cargo test --release --bin vajra-desktop
    /// synthetic_streams_memory -- --ignored --nocapture`. 10 streams x
    /// 200 lines/s through the bus with one subscriber draining it the way
    /// the `changed` emitter does. Fails if RSS growth passes the
    /// NFR-PERF-2 budget (15 MB per stream).
    #[test]
    #[ignore]
    fn synthetic_streams_memory() {
        let t = Tmp::new();
        let app = t.app();
        let sub = app.bus.subscribe();
        let before = rss_kb();
        let p = bus::synthetic::Producer::start(app.bus.clone(), 10, 200, 120);
        let end = std::time::Instant::now() + Duration::from_secs(30);
        let mut payloads = 0;
        while std::time::Instant::now() < end {
            if collect_changes(&sub, Duration::from_millis(200)).is_some() {
                payloads += 1;
            }
        }
        let after = rss_kb();
        let pushed = p.stop();
        let st = app.bus.stats();
        println!(
            "synthetic: pushed={pushed} payloads={payloads} rss_before_kb={before} rss_after_kb={after} delta_kb={} ring_bytes={} spill_bytes={}",
            after.saturating_sub(before),
            st.ring_bytes,
            st.spill_bytes
        );
        assert!(after.saturating_sub(before) <= 10 * 15 * 1024);
    }

    #[test]
    fn node_json_is_recursive() {
        let leaf = Node {
            id: "w".into(),
            kind: tree::NodeKind::Worker,
            title: "Bhima".into(),
            engine_id: Some("bhima".into()),
            status: Status::Done,
            overlays: vec!["stopped"],
            children: vec![],
        };
        let root = Node { id: "r".into(), kind: tree::NodeKind::Run, title: "Run".into(), engine_id: None, status: Status::InProgress, overlays: vec![], children: vec![leaf] };
        let v = node_json(&root);
        assert_eq!(v["kind"], "run");
        assert_eq!(v["children"][0]["kind"], "worker");
        assert_eq!(v["children"][0]["overlays"], json!(["stopped"]));
    }
}
