//! "Ask the Lead" at the app level (ADR-13): the three commands `ask_lead`, `cancel_ask` and `lead_status`.
//!
//! One ask is one headless `claude -p` turn (see `control::lead_run`). This file decides whether it may run (the run
//! is one this app launched, no ask is in flight, the Vajra guard is in place and proven), records it in the ledger
//! *before* it spawns, and streams the reply into the bus stream `lead-<run_id>`.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};

use super::orchestrator::{self, Orchestrator};
use super::{App, Res, Run, project_slug, runindex};
use crate::paths;
use crate::bus::Bus;
use crate::control::lead::{self, LeadError, frame};
use crate::control::lead_run::{self, Done, Handle, InitInfo, Limits, Sink, Tag};
use crate::control::lead_settings;
use crate::control::service::{AskRecord, AskState};
use crate::control::{fsx, nonce, sha256};
use crate::crew::record::{self, LineKind};
use crate::crew::redact::Redactor;
use crate::ids;
use crate::lock;
use crate::projects::contract::{SessionKind, SessionStatus};
use crate::sessions::argv::{LeadArgv, build_lead_argv};
use crate::sessions::user_env;
use crate::vajra::bridge::Request;
use crate::vajra::preflight::find_on_path;

/// What the human sees when the doctor reports the OS sandbox (with its denyWrite on the app dir) as not active. One
/// line: why, and what to do.
pub(super) const SANDBOX_NEEDED: &str = "Asking the Director-General needs the Claude Code sandbox. Run /sandbox in Claude Code, then retry.";
/// `read_only` detail for a run loaded from the index whose directory the app has no record of.
const NO_TRUSTED_CWD: &str = "the app has no trusted record of this run's directory (it was not asked about before the restart)";
/// File name of the run's directory inside `<data>/control/lead/<run_id>/`.
const CWD_FILE: &str = "cwd";
/// The run's project (its git common dir, `caps::project_key`) as the app found it at launch, before any worker ran.
const KEY_FILE: &str = "project_key";
/// Where the run's project sits: the folder holding the nearest `.git` entry (`git_location`), as found at launch. The pin
/// is keyed by this LOCATION, never by what `.git` later contains.
const LOC_FILE: &str = "project_loc";
/// The project's main checkout as resolved at launch, or `-` when it could not be (`main_checkout`). Guarded like the others.
const MAIN_FILE: &str = "project_main";
/// Most other-project folders the Lead's deny rules hold; more refuses the ask (#80 review: never drop one).
pub(super) const MAX_OTHER_ROOTS: usize = 256;
/// The adapter that ran the run (`claude` or `codex`). Guarded like the cwd; `runs.jsonl` only ever hints.
const PROVIDER_FILE: &str = "provider";
const SESSION_FILE: &str = "session";
const CAMPAIGN_FILE: &str = "campaign";
/// Present for a run that started no new workflow (the approval-only resume).
const APPROVAL_ONLY_FILE: &str = "approval_only";
/// Unix seconds when the app launched the run. The run id is only a uniqueness label (it can name a future minute).
const STARTED_FILE: &str = "started";
/// `read_only` detail for a run this app did not launch.
const NOT_OURS: &str = "run not launched by this app";
/// How long a passing `vajra-doctor.sh` answer is reused.
const DOCTOR_TTL: Duration = Duration::from_secs(60);
/// Most of a Lead transcript read back after a restart.
const REBUILD_BYTES: u64 = 4 * 1024 * 1024;
/// Where the guard script lives under `$HOME`.
const GUARD_SCRIPT: &str = ".claude/skills/vajra/hooks/pre-tool-use.sh";

/// Test seam: runs right after the settings file is written, before it is verified.
#[cfg(test)]
pub(super) type AfterWrite = Box<dyn Fn(&Path) + Send>;
/// Test only: runs in `ask_lead` after the orchestrator was first looked up and before the ask lock is taken.
#[cfg(test)]
pub(super) type BeforeAskLock = Box<dyn Fn() + Send>;

pub(super) struct Active {
    ask_id: String,
    handle: Arc<Handle>,
}

/// Everything the Lead needs that is not the run itself.
pub(super) struct LeadState {
    /// The absolute claude binary, resolved once at startup. Never a PATH lookup at ask time.
    bin: Mutex<Option<PathBuf>>,
    /// The environment the Lead (and the self-test) runs with: `HOME`, `PATH`, `LANG`, `TERM`, `USER`, `SHELL`.
    env: Mutex<Vec<(&'static str, OsString)>>,
    /// The running ask per project id. Taking the entry is the "one ask in flight per Director-General" lock, and the
    /// map's size is the app-wide turn count (`orchestrator::MAX_TURNS`).
    pub(super) active: Arc<Mutex<HashMap<String, Active>>>,
    /// (settings sha256, claude --version) pairs that passed the startup self-test.
    proven: Mutex<HashSet<(String, String)>>,
    /// Set when the app quits: no new ask starts.
    closing: AtomicBool,
    doctor: Mutex<Option<(Instant, bool)>>,
    limits: Mutex<Limits>,
    /// The app's own executable, for the `--mcp` server entry in `mcp.json`.
    exe: PathBuf,
    #[cfg(test)]
    pub(super) after_write: Mutex<Option<AfterWrite>>,
    #[cfg(test)]
    pub(super) before_ask_lock: Mutex<Option<BeforeAskLock>>,
}

impl LeadState {
    pub(super) fn new(home: &Path) -> Self {
        let path_dirs = crate::vajra::preflight::path_dirs();
        Self {
            bin: Mutex::new(resolve_claude(home, &path_dirs)),
            env: Mutex::new(user_env()),
            active: Arc::new(Mutex::new(HashMap::new())),
            proven: Mutex::new(HashSet::new()),
            closing: AtomicBool::new(false),
            doctor: Mutex::new(None),
            limits: Mutex::new(Limits::default()),
            exe: std::env::current_exe().unwrap_or_default(),
            #[cfg(test)]
            after_write: Mutex::new(None),
            #[cfg(test)]
            before_ask_lock: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub(super) fn set_bin(&self, p: Option<PathBuf>) {
        *lock(&self.bin) = p;
    }

    #[cfg(test)]
    pub(super) fn set_env(&self, e: Vec<(&'static str, OsString)>) {
        *lock(&self.env) = e;
    }

    #[cfg(test)]
    pub(super) fn set_limits(&self, l: Limits) {
        *lock(&self.limits) = l;
    }
}

/// The claude binary as an absolute path: `$PATH` first, then the places the installers put it. Resolved once at
/// startup; the AO shell's PATH had no `claude`, so the answer must not depend on the PATH at ask time.
pub fn resolve_claude(home: &Path, path_dirs: &[PathBuf]) -> Option<PathBuf> {
    if let Some(p) = find_on_path("claude", path_dirs) {
        return p.is_absolute().then_some(p);
    }
    let mut dirs: Vec<PathBuf> = vec![
        home.join(".local/bin"),
        home.join(".claude/local"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    // nvm: newest node version first (numeric, so v22.18.0 beats v9.11.2).
    dirs.extend(crate::userpath::nvm_bins(home));
    find_on_path("claude", &dirs)
}

fn io(e: impl std::fmt::Display) -> LeadError {
    LeadError::Io(e.to_string())
}

fn sha_hex(bytes: &[u8]) -> String {
    sha256::hex_digest(bytes)
}

/// What a run needs to be asked about.
pub(super) struct Target {
    /// Loaded from `runs.jsonl` (after a restart) rather than from a run this app life launched.
    pub(super) from_disk: bool,
    pub(super) session_id: String,
    pub(super) run_id: String,
    pub(super) cwd: PathBuf,
    pub(super) nonces: Redactor,
}

/// Bus lines of one ask: `<tag>\t<cleaned text>`.
struct StreamSink {
    bus: Arc<Bus>,
    stream: String,
    red: Redactor,
    on_init: Box<dyn FnMut(&InitInfo) + Send>,
}

impl StreamSink {
    fn push(&self, tag: &str, text: &str) {
        let _ = self.bus.push(&self.stream, &format!("{tag}\t{}", self.red.clean(text)));
    }
}

impl Sink for StreamSink {
    fn init(&mut self, info: &InitInfo) {
        (self.on_init)(info);
    }
    fn event(&mut self, tag: Tag, text: &str) {
        self.push(tag.as_str(), text);
    }
}

/// The self-test's sink: it keeps every raw line so the canary can be looked for in all of stdout.
struct Probe {
    raw: String,
}

impl Sink for Probe {
    fn raw(&mut self, line: &str) {
        self.raw.push_str(line);
        self.raw.push('\n');
    }
    fn event(&mut self, _: Tag, _: &str) {}
}

impl App {
    // ---------------------------------------------------------------- commands

    /// The Lead chat's state for a run: can it be asked, and is a question running now?
    pub fn lead_status(&self, run_id: &str) -> Res {
        if !ids::valid_run_id(run_id) {
            return Err(super::invalid("run_id", "must be YYYYMMDD-HHMM"));
        }
        let off = |why: String| Ok(json!({"available": false, "read_only_reason": why, "busy": false, "name": orchestrator::NAME}));
        let t = match self.lead_target(run_id) {
            Ok(t) => t,
            Err(e) => return off(e.detail().unwrap_or_else(|| NOT_OURS.to_string())),
        };
        let orch = match self.orchestrator_for_run(&t.run_id, &t.cwd) {
            Ok(o) => o,
            Err(e) => return off(e.detail().unwrap_or_else(|| e.code().to_string())),
        };
        let busy = lock(&self.lead.active).get(&orch.id).map(|a| a.ask_id.clone());
        // After a restart the bus is empty; bring the history back from the Director-General's own transcript.
        self.rebuild_lead_stream(&orch, &self.project_redactor(&t, &orch));
        if lock(&self.lead.bin).is_none() {
            return off("claude not found".into());
        }
        // ADR-15 2.1 states, each with its reason in words.
        let running = self.running_in_project(&orch.id);
        let (state, reason) = match (&busy, running) {
            (Some(_), _) => ("thinking", "answering a question"),
            (None, true) => ("running", "a run of this project is going"),
            (None, false) => ("idle", "waiting for a question"),
        };
        let mut out = json!({
            "available": true, "busy": busy.is_some(), "name": orchestrator::NAME, "state": state, "reason": reason,
            "project_id": orch.id, "generation": orch.generation, "stream": orch.stream(),
        });
        if let Some(id) = busy {
            out["ask_id"] = json!(id);
        }
        Ok(out)
    }

    /// Whether a run of project `pid` is going now: one pinned to it whose process is still alive. `runs` keeps finished runs
    /// for their crew views, so presence there is not liveness (the supervisor's state is).
    pub(super) fn running_in_project(&self, pid: &str) -> bool {
        let live: Vec<String> = lock(&self.runs).iter().filter(|(s, _)| self.sup.is_running(s).unwrap_or(false)).map(|(_, r)| r.run_id.clone()).collect();
        live.iter().any(|run| self.pinned_project(run).as_deref() == Some(pid))
    }

    /// Cancel the question that is running: the Lead's whole process group is killed.
    pub fn cancel_ask(&self, ask_id: &str) -> Res {
        if !lead::valid_ulid(ask_id) {
            return Err(super::invalid("ask_id", "must be a ULID"));
        }
        let handle = lock(&self.lead.active).values().find(|a| a.ask_id == ask_id).map(|a| a.handle.clone());
        match handle {
            Some(h) => {
                h.cancel();
                Ok(json!({"ask_id": ask_id, "state": "failed", "reason": "cancelled"}))
            }
            None => Err(json!({"error": "ask_not_found"})),
        }
    }

    /// App quit: refuse new asks, kill every Lead (and self-test) process group, and wait for the driver threads to
    /// reap them. Without this a Lead would outlive the app with no watchdog, and a restarted app could `--resume`
    /// the same Lead session while the orphan still writes it.
    pub fn shutdown_lead(&self) {
        {
            // Under the same lock the ask takes, so no ask slips in after this point.
            let _active = lock(&self.lead.active);
            self.lead.closing.store(true, Ordering::SeqCst);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let handles: Vec<Arc<Handle>> = lock(&self.lead.active).values().map(|a| a.handle.clone()).collect();
            if handles.is_empty() || Instant::now() >= deadline {
                break;
            }
            for h in &handles {
                h.cancel();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Ask the Lead one question about a run. Returns once the Lead is spawned; the reply streams into
    /// `lead-<run_id>`. The first ask after a start, or after the generated settings or the CLI version change,
    /// first runs the startup self-test (one or two tiny turns, a few seconds), so the call can take that long.
    /// The ask can be cancelled meanwhile: `cancel_ask` kills the self-test process too, because both share the
    /// ask's handle, and the call then returns `cancelled`.
    pub fn ask_lead(&self, run_id: &str, text: &str) -> Res {
        self.ask_lead_inner(run_id, text).map_err(|e| e.to_json())
    }

    fn ask_lead_inner(&self, run_id: &str, text: &str) -> Result<Value, LeadError> {
        if !ids::valid_run_id(run_id) {
            return Err(LeadError::Invalid("run_id: must be YYYYMMDD-HHMM".into()));
        }
        self.data_dir_guarded()?;
        let target = self.lead_target(run_id)?;
        let orch = self.orchestrator_for_run(&target.run_id, &target.cwd)?;
        let bin = lock(&self.lead.bin).clone().ok_or(LeadError::ClaudeNotFound)?;
        let ask_id = lead::new_ulid(SystemTime::now())?;
        // The nonce of every session of this run, plus every live one, is redacted from the text.
        let red = self.project_redactor(&target, &orch);
        let framed = frame(&ask_id, run_id, text, &red)?;
        #[cfg(test)]
        if let Some(f) = lock(&self.lead.before_ask_lock).as_ref() {
            f();
        }

        // One ask in flight per Director-General: two `--resume` writers would fork its transcript. And at most
        // `MAX_TURNS` turns across the app.
        let handle = Handle::new();
        {
            let mut active = lock(&self.lead.active);
            if self.lead.closing.load(Ordering::SeqCst) {
                return Err(LeadError::ShuttingDown);
            }
            if active.contains_key(&orch.id) {
                return Err(LeadError::Busy);
            }
            if active.len() >= orchestrator::MAX_TURNS {
                return Err(LeadError::TurnsFull);
            }
            active.insert(orch.id.clone(), Active { ask_id: ask_id.clone(), handle: handle.clone() });
        }
        // #82 review: the generation and session are read again now that this ask holds the project's entry.
        // `new_conversation` bumps them only while no entry is held, so this copy is the current conversation; the one
        // read before the lock may predate a "New conversation" and must never be resumed or re-published.
        // The project is checked again here, not only when the request came in: one archived meanwhile gets nothing.
        let orch = match self.active_project(&orch.id).and_then(|_| self.orchestrator(&orch.id, &orch.root)) {
            Ok(o) => o,
            Err(e) => {
                lock(&self.lead.active).remove(&orch.id);
                return Err(e);
            }
        };
        let started = self.start_ask(&target, &orch, &bin, &ask_id, &framed, &handle, red);
        if started.is_err() {
            lock(&self.lead.active).remove(&orch.id);
        }
        started
    }

    // ---------------------------------------------------------------- the run

    /// The run, if this app launched or resumed it: in memory, or in `runs.jsonl`. A finished run still counts.
    /// A run id is only minute-based, so it must name exactly one (campaign, session): if the live runs and the whole
    /// index hold two different ones for it, nothing is selected and the ask is refused, never guessed.
    pub(super) fn lead_target(&self, run_id: &str) -> Result<Target, LeadError> {
        let live: Vec<Run> = lock(&self.runs).values().filter(|r| r.run_id == run_id).cloned().collect();
        let indexed: Vec<runindex::Entry> = runindex::entries(&self.data.join("runs.jsonl")).into_iter().filter(|e| e.run_id == run_id).collect();
        let mut ids: HashSet<(String, String)> = live.iter().map(|r| (r.campaign.clone(), r.session_id.clone())).collect();
        ids.extend(indexed.iter().map(|e| (e.campaign.clone(), e.session_id.clone())));
        if ids.len() > 1 {
            return Err(LeadError::Invalid("run_id: more than one run has this id; it cannot be told apart".into()));
        }
        if let Some(mut r) = live.into_iter().next() {
            self.add_disk_nonce(&mut r.nonces, &r.session_id);
            return Ok(Target { from_disk: false, session_id: r.session_id, run_id: r.run_id, cwd: r.cwd, nonces: r.nonces });
        }
        // A run loaded from disk (after a restart): its control nonce is still on disk and must stay out of the
        // ask text, the ledger, the bus and the rebuilt history.
        let e = indexed.into_iter().next_back().ok_or(LeadError::ReadOnly(NOT_OURS))?;
        // The index is worker-writable, so its `cwd` is only a claim: the app's own record under the guarded control
        // dir (written when the run was first asked about) must exist and agree, or the run is not served.
        match self.read_run_cwd(run_id) {
            Some(c) if c == e.cwd => {}
            _ => return Err(LeadError::ReadOnly(NO_TRUSTED_CWD)),
        }
        let mut nonces = Redactor::default();
        self.add_disk_nonce(&mut nonces, &e.session_id);
        Ok(Target { from_disk: true, session_id: e.session_id, run_id: e.run_id, cwd: e.cwd, nonces })
    }

    /// `<data>/control/lead/<run_id>/`: the app's guarded record of a run it launched (directory, session, campaign and
    /// project). It sits under the directory the worker guard protects; `runs.jsonl` (worker-writable) is not trusted.
    fn lead_sid_dir(&self, run_id: &str) -> PathBuf {
        self.data.join("control").join("lead").join(run_id)
    }

    /// Record the run's directory under the guarded control dir. Called from the launch/resume path, where the app
    /// itself has just chosen the directory, so a finished run that was never asked about can still be asked after a
    /// restart. A failure only means the Lead is refused for that run later (fail closed); the launch is unaffected.
    ///
    /// It also records the run's session id and campaign (ADR-14): `runs.jsonl` is worker-writable, so a plan approval
    /// or reject trusts only an index line whose directory, session and campaign all equal this record.
    #[cfg(test)]
    pub(super) fn record_run(&self, run_id: &str, cwd: &Path, session_id: &str, campaign: &str, workflow: bool, started: i64) -> std::io::Result<()> {
        self.record_run_as(run_id, cwd, &super::caps::project_key(cwd), session_id, campaign, workflow, started)
    }

    /// `record_run` with a project `key` chosen by the caller; the location is the folder's own now (tests only).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_run_as(&self, run_id: &str, cwd: &Path, key: &Path, session_id: &str, campaign: &str, workflow: bool, started: i64) -> std::io::Result<()> {
        self.record_run_pinned(run_id, cwd, &git_location(cwd), key, session_id, campaign, workflow, started)
    }

    /// `record_run` with the exact (project location, key) pair the launch checked (`launch_identity`), written before
    /// anything is spawned: neither is read from the repo again, so a `.git` the launched process creates or redirects
    /// afterwards cannot change what was pinned (#80 review).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_run_pinned(&self, run_id: &str, cwd: &Path, loc: &Path, key: &Path, session_id: &str, campaign: &str, workflow: bool, started: i64) -> std::io::Result<()> {
        let control = self.data.join("control");
        fsx::private_dir(&control)?;
        fsx::private_dir(&control.join("lead"))?;
        let dir = self.lead_sid_dir(run_id);
        fsx::private_dir(&dir)?;
        fsx::write_atomic(&dir, SESSION_FILE, session_id.as_bytes())?;
        fsx::write_atomic(&dir, CAMPAIGN_FILE, campaign.as_bytes())?;
        fsx::write_atomic(&dir, STARTED_FILE, started.to_string().as_bytes())?;
        if !workflow {
            fsx::write_atomic(&dir, APPROVAL_ONLY_FILE, b"1")?;
        }
        // The project is pinned now, before anything is spawned, while the app itself has just chosen the directory
        // (ADR-15 2.3). A folder that is no project gets no pin (the launch refuses one; a record made otherwise is refused
        // when asked about). The key and location are the exact pair the launch checked (#80 review).
        if let Ok(Some(p)) = self.projects.find_by_path(cwd) {
            fsx::write_atomic(&dir, orchestrator::PROJECT_FILE, p.id.as_bytes())?;
        }
        fsx::write_atomic(&dir, KEY_FILE, key.to_string_lossy().as_bytes())?;
        fsx::write_atomic(&dir, LOC_FILE, loc.to_string_lossy().as_bytes())?;
        let main = main_checkout(loc, key);
        fsx::write_atomic(&dir, MAIN_FILE, main.as_ref().map_or("-".into(), |m| m.to_string_lossy()).as_bytes())?;
        // The cwd goes last: a run with a cwd record has its session and campaign records too.
        fsx::write_atomic(&dir, CWD_FILE, cwd.to_string_lossy().as_bytes()).map(|_| ())
    }

    /// Record, before the run starts, which adapter it runs on. A run launched by this build always has this record, so a
    /// resume trusts it and never the worker-writable index. A failure here refuses the launch.
    pub(super) fn record_run_provider(&self, run_id: &str, engine: crate::adapter::Engine) -> std::io::Result<()> {
        let control = self.data.join("control");
        fsx::private_dir(&control)?;
        fsx::private_dir(&control.join("lead"))?;
        let dir = self.lead_sid_dir(run_id);
        fsx::private_dir(&dir)?;
        fsx::write_atomic(&dir, PROVIDER_FILE, engine.id().as_bytes()).map(|_| ())
    }

    /// The guarded provider of a run: `Some` for a run this build launched, `None` for one from before providers
    /// existed (all of which ran on claude) or whose record is unreadable.
    pub(super) fn read_run_provider(&self, run_id: &str) -> Option<crate::adapter::Engine> {
        let body = fsx::read_nofollow(&self.lead_sid_dir(run_id).join(PROVIDER_FILE)).ok()?;
        crate::adapter::Engine::from_id(String::from_utf8(body).ok()?.trim())
    }

    /// The guarded (cwd, session id, campaign) of a run, or `None` if any part is missing (a run from before this
    /// record existed is not trusted for plan decisions).
    pub(super) fn read_run_record(&self, run_id: &str) -> Option<(PathBuf, String, String, bool)> {
        let text = |f: &str| fsx::read_nofollow(&self.lead_sid_dir(run_id).join(f)).ok().and_then(|b| String::from_utf8(b).ok());
        let workflow = !self.lead_sid_dir(run_id).join(APPROVAL_ONLY_FILE).exists();
        Some((self.read_run_cwd(run_id)?, text(SESSION_FILE)?.trim().to_string(), text(CAMPAIGN_FILE)?.trim().to_string(), workflow))
    }

    /// When the app launched the run (unix seconds), or `None` if it was not recorded.
    pub(super) fn read_run_started(&self, run_id: &str) -> Option<i64> {
        let body = fsx::read_nofollow(&self.lead_sid_dir(run_id).join(STARTED_FILE)).ok()?;
        String::from_utf8(body).ok()?.trim().parse::<i64>().ok().filter(|t| *t > 0)
    }

    /// The folders of the app's other projects, for the Lead's deny rules (ADR-15 D8): the cwd of every run this app
    /// recorded under its guarded control dir (never the worker-writable run index), newest first and at most
    /// `MAX_OTHER_ROOTS`, minus every run of `repo`'s own project and every folder that holds `repo` (denying an
    /// ancestor would deny the Lead its own project). Every distinct run folder, plus each project's main working tree.
    ///
    /// Which project a run belongs to is the key pinned when the app launched it (`KEY_FILE`), for the asking run and
    /// for every other one: the repo's `.git` and `commondir` are worker-writable, so they are never re-read here
    /// (#80 review). A run with no pinned key counts as another project (its folder is denied); an asking run with
    /// none is refused. More than `MAX_OTHER_ROOTS` folders refuses the ask rather than leave one undenied.
    pub(super) fn other_project_roots(&self, own_run: &str, repo: &Path) -> Result<Vec<PathBuf>, LeadError> {
        let own = self.read_run_key(own_run).ok_or_else(|| LeadError::GuardMissing("this run's project was not recorded when it was launched; start a new run".into()))?;
        let own_real = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
        let mut out: Vec<PathBuf> = Vec::new();
        for run in self.recorded_runs() {
            let Some(cwd) = self.read_run_cwd(&run) else { continue };
            let key = self.read_run_key(&run);
            if key.as_ref() == Some(&own) {
                continue;
            }
            // Every distinct run folder (a subfolder, an outside worktree) and the project's main working tree (the
            // folder that holds its `.git` dir, from the pinned key), so a project whose runs were all elsewhere still
            // has its main folder denied.
            let main = key.filter(|k| k.file_name() == Some(std::ffi::OsStr::new(".git"))).and_then(|k| k.parent().map(Path::to_path_buf));
            // The run folder, the project's main tree (from a `.git` key), and the checkout root the launch pinned (`project_loc`):
            // a submodule's or separate-git-dir repo's root is neither the cwd nor inferable from its key.
            let pinned_loc = self.read_run_loc(&run);
            // The main checkout resolved at launch (a submodule or separate-git-dir repo run from an outside worktree has a key that
            // names no checkout). `-` means it could not be resolved: that checkout cannot be protected, so the ask is refused.
            let pinned_main = match fsx::read_nofollow(&self.lead_sid_dir(&run).join(MAIN_FILE)).ok().and_then(|b| String::from_utf8(b).ok()) {
                Some(m) if m.trim() == "-" => {
                    return Err(LeadError::GuardMissing("another project's main checkout could not be resolved, so the Lead cannot deny it; start that project from its main folder".into()));
                }
                Some(m) => Some(PathBuf::from(m.trim())).filter(|p| p.is_absolute()),
                None => None,
            };
            for dir in std::iter::once(cwd).chain(main).chain(pinned_loc).chain(pinned_main) {
                let real = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
                if own_real.starts_with(&real) || out.contains(&dir) {
                    continue;
                }
                if out.len() >= MAX_OTHER_ROOTS {
                    return Err(LeadError::GuardMissing(format!("more than {MAX_OTHER_ROOTS} other project folders to deny; the Lead cannot be started with all of them")));
                }
                out.push(dir);
            }
        }
        Ok(out)
    }

    /// The folders of the asking run's OWN project that need the secret-file rules: the project's main checkout
    /// (`root`) plus the folder of every recorded run of the same pinned project (a worktree outside the main checkout,
    /// a subfolder), from the guarded records and never the worker-writable index or `.git`. A folder inside another
    /// already listed is covered by its `**` rules and left out. More than `MAX_OTHER_ROOTS` refuses the ask.
    pub(super) fn own_project_folders(&self, own_run: &str, root: &Path) -> Result<Vec<PathBuf>, LeadError> {
        let own = self.read_run_key(own_run).ok_or_else(|| LeadError::GuardMissing("this run's project was not recorded when it was launched; start a new run".into()))?;
        let mut out: Vec<PathBuf> = vec![root.to_path_buf()];
        for run in self.recorded_runs() {
            let Some(cwd) = self.read_run_cwd(&run) else { continue };
            // Dropped only when BOTH its spelling and its real path sit under `d`: a folder spelled under `d` (a symlink) can really
            // be outside it, and one spelled differently (`/var` vs `/private/var`) needs its own supplied-form rules.
            let real = std::fs::canonicalize(&cwd).unwrap_or_else(|_| cwd.clone());
            let covered = |d: &PathBuf| cwd.starts_with(d) && real.starts_with(std::fs::canonicalize(d).unwrap_or_else(|_| d.clone()));
            if self.read_run_key(&run).as_ref() != Some(&own) || out.iter().any(covered) {
                continue;
            }
            if out.len() >= MAX_OTHER_ROOTS {
                return Err(LeadError::GuardMissing(format!("more than {MAX_OTHER_ROOTS} folders of this project to guard; the Lead cannot be started with all of them")));
            }
            out.push(cwd);
        }
        Ok(out)
    }

    /// Every run with a record under the guarded control dir, newest first.
    fn recorded_runs(&self) -> Vec<String> {
        let mut runs: Vec<String> = std::fs::read_dir(self.data.join("control").join("lead"))
            .map(|r| r.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| ids::valid_run_id(n)).collect())
            .unwrap_or_default();
        runs.sort_unstable_by(|a, b| b.cmp(a));
        runs
    }

    /// The project key pinned when the run was launched.
    pub(super) fn read_run_key(&self, run_id: &str) -> Option<PathBuf> {
        let body = fsx::read_nofollow(&self.lead_sid_dir(run_id).join(KEY_FILE)).ok()?;
        let p = PathBuf::from(String::from_utf8(body).ok()?.trim());
        p.is_absolute().then_some(p)
    }

    /// The key to pin for a launch in `cwd`, read before anything is spawned.
    ///
    /// A project is keyed by the LOCATION of the nearest `.git` entry above `cwd` (`git_location`; the folder itself when
    /// there is none). The first guarded launch at a location pins it to a project identity (`project_key`, from the common
    /// dir at that moment) and records both under the guard. Every later launch whose location is the same reuses that pin,
    /// whatever `.git` now contains: the root, a subfolder and a sibling all resolve alike, and a worker that rewrites
    /// `.git` cannot move any of them to another project. A nested repository has its own `.git` at another location, so it
    /// is its own project. A linked worktree is its own location, mapped to its main project by its first launch's pin.
    ///
    /// Records of one location that disagree on its project refuse the launch rather than guess. A record written before
    /// locations existed (no `project_loc`) has its location derived once from its guarded cwd and written back; one with
    /// neither key nor location is the base format, migrated by `migrate_old_key`. Anything else malformed refuses.
    #[cfg(test)]
    pub(super) fn launch_key(&self, cwd: &Path) -> Result<PathBuf, Value> {
        self.launch_identity(cwd).map(|(_, key)| key)
    }

    /// `launch_key` with the project location it was found at: the pair a launch records, as one look.
    pub(super) fn launch_identity(&self, cwd: &Path) -> Result<(PathBuf, PathBuf), Value> {
        let want = git_location(cwd);
        let mut keys: Vec<PathBuf> = Vec::new();
        for r in self.recorded_runs() {
            let Some(c) = self.read_run_cwd(&r) else { continue };
            let dir = self.lead_sid_dir(&r);
            let old = std::fs::symlink_metadata(dir.join(LOC_FILE)).is_err();
            let loc = if old { git_location(&c) } else { self.read_run_loc(&r).unwrap_or_default() };
            if loc != want {
                continue;
            }
            let key = match (self.read_run_key(&r), old) {
                (Some(k), _) => k,
                (None, true) => self.migrate_old_key(&r, &loc).ok_or_else(|| super::fail("project_membership_inconsistent", "an earlier run of this folder has no recorded project"))?,
                (None, false) => return Err(super::fail("project_membership_inconsistent", "an earlier run of this folder has no recorded project")),
            };
            if old {
                // Best effort: a record that cannot be written back is derived again next time.
                let _ = fsx::write_atomic(&dir, LOC_FILE, loc.to_string_lossy().as_bytes());
            }
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        match keys.len() {
            0 => Ok((want.clone(), self.identify_new_location(&want)?)),
            1 => Ok((want, keys.remove(0))),
            _ => Err(super::fail("project_membership_inconsistent", "earlier runs of this folder were recorded under different projects")),
        }
    }

    /// The key for a NEW project location `want`, derived from ONE look at its `.git` and never re-derived.
    ///
    /// Outside every pinned project's tree the key is whatever that `.git` names (`caps::key_of_entry`). Inside an
    /// already-pinned tree the location may become its own project only if its `.git` is (a) a real directory (not a
    /// symlink) with no `commondir` file: a self-contained nested repository, keyed by that directory; or (b) a `gitdir:`
    /// file whose canonical target lies INSIDE an enclosing pinned project's common dir AND has no `commondir` file on the
    /// resolution path (the target and every directory up to that common dir): a submodule, keyed by exactly the canonical
    /// target that was checked. Anything else (a pointer into another project, a symlinked `.git`, a redirecting
    /// `commondir`, an unresolvable target) is refused and never pinned, so a worker cannot plant a `.git` that pulls a
    /// subfolder into another project.
    fn identify_new_location(&self, want: &Path) -> Result<PathBuf, Value> {
        let mut enclosing: Vec<PathBuf> = Vec::new();
        for r in self.recorded_runs() {
            let (Some(loc), Some(key)) = (self.read_run_loc(&r), self.read_run_key(&r)) else { continue };
            // Only a Git project (its key is a common dir, not the folder itself) constrains what may nest under it; a plain
            // folder's pin leaves a plain subfolder its own plain location.
            if loc != want && want.starts_with(&loc) && key != loc && !enclosing.contains(&key) {
                enclosing.push(key);
            }
        }
        if enclosing.is_empty() {
            return Ok(super::caps::key_of_entry(want, want).unwrap_or_else(|| want.to_path_buf()));
        }
        let refuse = |why: &str| super::fail("project_membership_inconsistent", why);
        let git = want.join(".git");
        let md = std::fs::symlink_metadata(&git).map_err(|_| refuse("this folder's .git cannot be read"))?;
        if md.is_dir() {
            if std::fs::symlink_metadata(git.join("commondir")).is_ok() {
                return Err(refuse("this folder's .git points at another project's common dir"));
            }
            // The key is this very directory; a swap to a link between the check and here no longer names `want/.git`.
            let key = std::fs::canonicalize(&git).map_err(|_| refuse("this folder's .git cannot be read"))?;
            return if key.parent() == Some(want) && key.file_name() == Some(std::ffi::OsStr::new(".git")) { Ok(key) } else { Err(refuse("this folder's .git changed while it was checked")) };
        }
        if md.is_file() {
            let body = fsx::read_nofollow(&git).ok().and_then(|b| String::from_utf8(b).ok()).ok_or_else(|| refuse("this folder's .git cannot be read"))?;
            let target = body.lines().next().and_then(|l| l.strip_prefix("gitdir:")).map(str::trim).filter(|t| !t.is_empty());
            if let Some(real) = target.and_then(|t| std::fs::canonicalize(want.join(t)).ok()) {
                // A pinned common dir counts only while it is still exactly itself: the key was stored canonical, so one that now
                // resolves anywhere else (replaced by a symlink, or under a redirected parent) has been redirected and admits nothing.
                for common in enclosing.iter().filter(|k| std::fs::canonicalize(k).is_ok_and(|c| &c == *k)) {
                    // No `commondir` above the target on the way up to the common dir.
                    if !real.starts_with(common) || !real.ancestors().skip(1).take_while(|d| d.starts_with(common)).all(|d| std::fs::symlink_metadata(d.join("commondir")).is_err()) {
                        continue;
                    }
                    match fsx::read_nofollow(&real.join("commondir")) {
                        // A submodule's git dir: its own common dir, keyed by exactly the target that was checked.
                        Err(_) if std::fs::symlink_metadata(real.join("commondir")).is_err() => return Ok(real),
                        // A linked worktree of the enclosing project: its `commondir` must resolve to this pinned common dir.
                        Ok(c) => {
                            let resolved = String::from_utf8(c).ok().and_then(|c| std::fs::canonicalize(real.join(c.trim())).ok());
                            if resolved.as_ref() == Some(common) {
                                return Ok(common.clone());
                            }
                        }
                        Err(_) => {}
                    }
                }
            }
        }
        Err(refuse("this folder's .git is neither a nested repository nor a submodule of the enclosing project"))
    }

    /// The project location pinned when the run was launched.
    fn read_run_loc(&self, run_id: &str) -> Option<PathBuf> {
        let body = fsx::read_nofollow(&self.lead_sid_dir(run_id).join(LOC_FILE)).ok()?;
        let p = PathBuf::from(String::from_utf8(body).ok()?.trim());
        p.is_absolute().then_some(p)
    }

    /// The one-time migration of the base record format: a run with a guarded cwd, session and campaign but neither a key nor
    /// a location was written before they existed. Derive its key from its guarded cwd now and write it back under the guard.
    /// Anything else (a key file that exists but is unusable, a missing session or campaign) is not migrated.
    fn migrate_old_key(&self, run_id: &str, loc: &Path) -> Option<PathBuf> {
        let dir = self.lead_sid_dir(run_id);
        if std::fs::symlink_metadata(dir.join(KEY_FILE)).is_ok() || self.read_run_record(run_id).is_none() {
            return None;
        }
        // From the same location the caller matched on, never a second walk of the repo.
        let key = super::caps::key_of_entry(loc, loc).unwrap_or_else(|| loc.to_path_buf());
        fsx::write_atomic(&dir, KEY_FILE, key.to_string_lossy().as_bytes()).ok()?;
        self.read_run_key(run_id)
    }

    pub(super) fn read_run_cwd(&self, run_id: &str) -> Option<PathBuf> {
        let body = fsx::read_nofollow(&self.lead_sid_dir(run_id).join(CWD_FILE)).ok()?;
        let p = PathBuf::from(String::from_utf8(body).ok()?.trim());
        p.is_absolute().then_some(p)
    }

    /// Every nonce that could appear in the project's shared Director-General conversation: the persisted nonce of the
    /// session of EVERY recorded run of this project (read from the guarded records), the in-memory nonce history of
    /// each live run of it, and every live control session's. The conversation is shared by all of the project's runs,
    /// so history replayed after a restart for run B must not show run A's nonce.
    fn project_redactor(&self, t: &Target, orch: &Orchestrator) -> Redactor {
        let mut red = t.nonces.clone();
        for run in self.recorded_runs() {
            if !self.run_in_project(&run, &orch.id) {
                continue;
            }
            let sid = fsx::read_nofollow(&self.lead_sid_dir(&run).join(SESSION_FILE)).ok().and_then(|b| String::from_utf8(b).ok());
            if let Some(sid) = sid {
                self.add_disk_nonce(&mut red, sid.trim());
            }
        }
        for r in lock(&self.runs).values() {
            if self.run_in_project(&r.run_id, &orch.id) {
                red.absorb(&r.nonces);
                self.add_disk_nonce(&mut red, &r.session_id);
            }
        }
        red.absorb(&lock(&self.control).redactor());
        red
    }

    /// Whether a recorded run is one of project `pid`'s, for redaction: its pinned store id decides; a run with no store
    /// id pinned yet (recorded before its folder was a project, or a stand-in pin from before the store) counts when its
    /// guarded folder is that project's, as `run_project` would resolve it. Widening only ever redacts more.
    fn run_in_project(&self, run_id: &str, pid: &str) -> bool {
        match self.pinned_project(run_id) {
            Some(p) if crate::projects::contract::valid_project_id(&p) => p == pid,
            _ => self.read_run_cwd(run_id).and_then(|cwd| self.projects.find_by_path(&cwd).ok().flatten()).is_some_and(|p| p.id == pid),
        }
    }

    /// Add the session's persisted control nonce (read under the hook's rules), if there is a valid one.
    fn add_disk_nonce(&self, red: &mut Redactor, session_id: &str) {
        if !ids::valid_session_id(session_id) {
            return;
        }
        if let Ok(n) = nonce::read(&self.data.join("control").join(session_id)) {
            red.add(Some(&n));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_ask(
        &self,
        t: &Target,
        orch: &Orchestrator,
        bin: &Path,
        ask_id: &str,
        framed: &lead::FramedAsk,
        handle: &Arc<Handle>,
        red: Redactor,
    ) -> Result<Value, LeadError> {
        if !orch.root.is_dir() {
            return Err(LeadError::Invalid("cwd: the project's folder is gone".into()));
        }
        let env = lock(&self.lead.env).clone();
        self.guard_preconditions(&env)?;
        // Same environment for the version probe, the self-test and the ask, with the CLI's own directory on PATH.
        let env = cli_env(env, bin);
        // A run this app life launched is authoritative in memory: remember its directory under the guarded dir, so
        // that after a restart the (worker-writable) index cannot supply a different one.
        if !t.from_disk {
            fsx::private_dir(&self.data.join("control").join("lead")).map_err(io)?;
            let rec = self.lead_sid_dir(&t.run_id);
            fsx::private_dir(&rec).map_err(io)?;
            fsx::write_atomic(&rec, CWD_FILE, t.cwd.to_string_lossy().as_bytes()).map_err(io)?;
            // And its project, as a launch pins it: later asks follow this record, not a new look at the folder.
            if self.pinned_project(&t.run_id).is_none() {
                fsx::write_atomic(&rec, orchestrator::PROJECT_FILE, orch.id.as_bytes()).map_err(io)?;
            }
        }
        // Per-project files (one ask per Director-General at a time), under `control/`, which the worker guard keeps
        // workers from writing (on the default data dir). Two projects' asks can overlap; each has its own files.
        let dir = &orch.dir;
        // One snapshot of the other projects' folders (ADR-15 D8) for this whole ask: the self-test, the files and the
        // recheck below all use the same rules. A project that first appears during the ask is in the next one.
        let others = self.other_project_roots(&t.run_id, &orch.root)?;
        // The project's own folders (main checkout plus every recorded run or worktree folder) all get the secret-file
        // rules; the self-test and the recheck below use this same set.
        let repos = self.own_project_folders(&t.run_id, &orch.root)?;
        let settings = self.write_lead_files(dir, &repos, &orch.id, orch.generation, &others)?;
        self.ensure_selftest(bin, &env, dir, &repos, &others, &settings, &t.run_id, handle)?;
        // The self-test can take minutes; a worker with the same uid could have rewritten the files meanwhile. Write
        // them again and check both byte for byte, immediately before the real launch.
        if self.write_lead_files(dir, &repos, &orch.id, orch.generation, &others)? != settings {
            return Err(LeadError::GuardMissing("the Director-General's settings changed during the self-test".into()));
        }
        // Only this generation's MCP calls are served from now on.
        lock(&self.control).set_generation(&orch.id, orch.generation).map_err(|e| LeadError::Io(e.code().to_string()))?;

        let (lead_sid, resume) = match &orch.session {
            Some(s) => (s.clone(), true),
            None => (ids::uuid_v4()?, false),
        };
        let argv = build_lead_argv(&LeadArgv {
            claude: bin,
            lead_session_id: &lead_sid,
            resume,
            settings: &dir.join("settings.json"),
            mcp_config: &dir.join("mcp.json"),
        })
        .map_err(|e| LeadError::Invalid(e.to_string()))?;

        let stream = orch.stream();
        // The history of earlier asks is back on the stream before this one is added to it.
        self.rebuild_lead_stream(orch, &red);
        let svc = self.control.clone();
        let ledger = {
            let (svc, run_id, session_id, lead_sid, ask_id) = (svc.clone(), t.run_id.clone(), t.session_id.clone(), lead_sid.clone(), ask_id.to_string());
            move |state: AskState, text: Option<&str>, reason: Option<&str>| {
                lock(&svc)
                    .record_ask(&AskRecord { ask_id: &ask_id, run_id: &run_id, session_id: &session_id, lead_session_id: &lead_sid, state, text, reason })
                    .map_err(|e| LeadError::Io(e.code().to_string()))
            }
        };
        // The last check before the process: the self-test can take minutes, and the project may be archived meanwhile.
        self.active_project(&orch.id)?;
        // Record first, spawn second: if the ledger cannot take the line, the Lead is never started.
        ledger(AskState::Queued, Some(framed.text()), None)?;
        let child = match lead_run::spawn(&argv, &orch.root, &env, framed, handle) {
            Ok(c) => c,
            Err(e) => {
                let _ = ledger(AskState::Failed, None, Some(e.code()));
                return Err(e);
            }
        };
        let _ = self.bus.push(&stream, &format!("you\t{}", red.clean(framed.text())));
        // The window's record of the conversation (FG-01 sessions.jsonl); `--resume` never reads it.
        let record = self.note_orchestrator(orch, Some(&lead_sid), SessionStatus::Running);
        let (projects, rec_project, rec_gen) = (self.projects.clone(), orch.id.clone(), orch.generation);

        let limits = *lock(&self.lead.limits);
        let project = orch.id.clone();
        let bus = self.bus.clone();
        let active = self.lead.active.clone();
        let log = self.log.clone();
        let handle = handle.clone();
        let first = !resume;
        let sid_dir = orch.dir.clone();
        let sid_text = lead_sid.clone();
        let sid_text_end = lead_sid.clone();
        let ledger_init = ledger.clone();
        std::thread::spawn(move || {
            let on_init: Box<dyn FnMut(&InitInfo) + Send> = Box::new(move |_| {
                // The turn passed the tool and mode check: the ask is delivered. A new conversation's session is
                // remembered only now, so a turn that never started leaves no id behind to `--resume`.
                let _ = ledger_init(AskState::Delivered, None, None);
                if first && fsx::write_atomic(&sid_dir, orchestrator::SESSION_FILE, sid_text.as_bytes()).is_err() {
                    if let Some(l) = &log {
                        let _ = l.log("lead_index_failed", &[]);
                    }
                }
            });
            let mut sink = StreamSink { bus: bus.clone(), stream: stream.clone(), red, on_init };
            let end = lead_run::drive(child, &handle, limits, &mut sink);
            match &end {
                Ok(Done { is_error: false, .. }) => {
                    let _ = ledger(AskState::Done, None, None);
                }
                Ok(_) => {
                    let _ = ledger(AskState::Failed, None, Some("lead_error"));
                }
                Err(e) => {
                    let _ = ledger(AskState::Failed, None, Some(e.code()));
                    sink.push("error", e.code());
                }
            }
            // The turn is over: the orchestrator is idle again (or its turn failed), still on this conversation.
            if let Some(id) = record {
                let status = if matches!(&end, Ok(Done { is_error: false, .. })) { SessionStatus::Stopped } else { SessionStatus::Failed };
                let line = orchestrator::session_line(id, &rec_project, SessionKind::Orchestrator, None, Some(&sid_text_end), None, rec_gen, status);
                let _ = projects.append_session(&line);
            }
            lock(&active).remove(&project);
        });
        Ok(json!({
            "ask_id": ask_id, "state": "queued", "stream": orch.stream(), "lead_session_id": lead_sid,
            "project_id": orch.id, "generation": orch.generation, "name": orchestrator::NAME,
        }))
    }

    // ---------------------------------------------------------------- the guard (ADR-13 2a)

    /// The Lead's generated files and the runs' guarded records live under `<data>/control/`, which the installed
    /// worker guard protects only at its default place. Under any other data dir a worker could rewrite them, so the
    /// Lead is not started (checked first, before any record there is believed).
    fn data_dir_guarded(&self) -> Result<(), LeadError> {
        let guarded = paths::resolve_data_dir(None, &self.home);
        let same = |a: &Path, b: &Path| matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y);
        if !same(&self.data, &guarded) {
            return Err(LeadError::GuardMissing("Director-General unavailable: data dir is not guard-protected (VAJRA_DESKTOP_DATA override)".into()));
        }
        Ok(())
    }

    /// Nothing is spawned unless every one of these holds: the guard script is a regular file, the doctor passes,
    /// and the Lead's environment keeps `HOME` and a `PATH` that has `bash` and `jq` (without them the guard would
    /// fail closed and block every Lead tool).
    fn guard_preconditions(&self, env: &[(&'static str, OsString)]) -> Result<(), LeadError> {
        let miss = |why: &str| LeadError::GuardMissing(why.to_string());
        self.data_dir_guarded()?;
        let script = self.home.join(GUARD_SCRIPT);
        if !std::fs::metadata(&script).is_ok_and(|m| m.is_file()) {
            return Err(miss("pre-tool-use.sh is missing"));
        }
        self.doctor_ok().map_err(miss)?;
        let get = |k: &str| env.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
        if get("HOME").is_none_or(|h| h.is_empty()) {
            return Err(miss("HOME is not set for the Director-General"));
        }
        let dirs: Vec<PathBuf> = get("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        if find_on_path("bash", &dirs).is_none() || find_on_path("jq", &dirs).is_none() {
            return Err(miss("bash and jq must be on the Director-General's PATH"));
        }
        Ok(())
    }

    /// `vajra-doctor.sh` must pass, and it must report the OS sandbox with its denyWrite on the app dir as active.
    /// Without the sandbox the worker guard is the only thing keeping a worker out of the files the Lead trusts
    /// (its settings, MCP config and session mapping), so the Lead is unavailable rather than half-protected.
    fn doctor_ok(&self) -> Result<(), &'static str> {
        let mut cache = lock(&self.lead.doctor);
        if let Some((at, true)) = *cache {
            if at.elapsed() < DOCTOR_TTL {
                return Ok(());
            }
        }
        let out = self.bridge.run(&Request::Doctor).ok().filter(|o| o.exit() == Some(0));
        let verdict = match out {
            None => Err("vajra doctor does not pass"),
            Some(o) if o.stdout.lines().any(|l| l.starts_with("PASS") && l.contains("denyWrite active")) => Ok(()),
            Some(_) => Err(SANDBOX_NEEDED),
        };
        *cache = Some((Instant::now(), verdict.is_ok()));
        verdict
    }

    /// Write `<orchestrator dir>/{settings,mcp}.json` and check the settings file on disk is the generated one, byte for
    /// byte. Returns the settings' sha256. Rewritten on every ask, so an edit between asks never matters; the check
    /// closes the gap between writing and spawning.
    fn write_lead_files(&self, dir: &Path, repos: &[PathBuf], project: &str, generation: u32, others: &[PathBuf]) -> Result<String, LeadError> {
        if !self.data.is_absolute() {
            return Err(LeadError::GuardMissing("the data dir must be an absolute path".into()));
        }
        fsx::private_dir(dir).map_err(io)?;
        let want = lead_settings::settings_for(&self.home, repos, &self.data, others)?;
        let mcp = json!({"mcpServers": {"vajra-desktop": {
            "command": self.lead.exe.to_string_lossy(),
            "args": ["--mcp"],
            "env": {
                "VAJRA_DESKTOP_DATA": self.data.to_string_lossy(),
                crate::control::mcp::PROJECT_ENV: project,
                crate::control::mcp::GEN_ENV: generation.to_string(),
            },
        }}});
        let mcp = mcp.to_string();
        let mcp_path = fsx::write_atomic(dir, "mcp.json", mcp.as_bytes()).map_err(io)?;
        let path = fsx::write_atomic(dir, "settings.json", want.as_bytes()).map_err(io)?;
        #[cfg(test)]
        if let Some(f) = lock(&self.lead.after_write).as_ref() {
            f(&path);
        }
        verify_settings_file(&path, &want)?;
        // The MCP config names a command the CLI runs before `init` is checked: it is held to the same standard.
        verify_settings_file(&mcp_path, &mcp)?;
        Ok(sha_hex(want.as_bytes()))
    }

    /// The startup self-test (fail closed). When the generated settings or the CLI version changes, one canary turn
    /// runs before the first real ask: a random file under the data dir, covered by a generated rule, with the hooks
    /// removed. The turn must be denied and the content must be absent from stdout, or the Lead is not started.
    #[allow(clippy::too_many_arguments)]
    fn ensure_selftest(&self, bin: &Path, env: &[(&'static str, OsString)], dir: &Path, repos: &[PathBuf], others: &[PathBuf], settings_sha: &str, run_id: &str, h: &Arc<Handle>) -> Result<(), LeadError> {
        let version = claude_version(bin, env)?;
        let key = (settings_sha.to_string(), version);
        if lock(&self.lead.proven).contains(&key) {
            return Ok(());
        }
        let fail = |why: &str| LeadError::GuardMissing(format!("startup self-test: {why}"));
        let canary_dir = dir.join("canary").join(nonce::random_hex(8)?);
        fsx::private_dir(&dir.join("canary")).map_err(io)?;
        fsx::private_dir(&canary_dir).map_err(io)?;
        let secret = format!("CANARY-{}", nonce::random_hex(16)?);
        let file = fsx::write_atomic(&canary_dir, "c.txt", secret.as_bytes()).map_err(io)?;
        let result = (|| {
            // The deny layer alone: the same generated rules, no hooks.
            let bare = lead_settings::settings_no_hooks_for(&self.home, repos, &self.data, others)?;
            let bare_path = fsx::write_atomic(dir, "selftest-settings.json", bare.as_bytes()).map_err(io)?;
            verify_settings_file(&bare_path, &bare)?;
            // Two tries: the model may answer without touching the file, which proves nothing.
            for _ in 0..2 {
                // A fresh session id per attempt: the CLI refuses an id whose transcript already exists, and the
                // first attempt leaves one even when the model never touched the file.
                let sid = ids::uuid_v4()?;
                let argv = build_lead_argv(&LeadArgv { claude: bin, lead_session_id: &sid, resume: false, settings: &bare_path, mcp_config: &dir.join("mcp.json") })
                    .map_err(|e| LeadError::Invalid(e.to_string()))?;
                let ask = lead::new_ulid(SystemTime::now())?;
                let framed = frame(&ask, run_id, &format!("Read the file {} and show its content.", file.display()), &Redactor::default())?;
                let child = lead_run::spawn(&argv, dir, env, &framed, h)?;
                let mut probe = Probe { raw: String::new() };
                // The self-test is bounded like an ask, but never longer than two minutes.
                let lim = *lock(&self.lead.limits);
                let done = lead_run::drive(child, h, Limits { total: lim.total.min(Duration::from_secs(120)), ..lim }, &mut probe)?;
                if probe.raw.contains(&secret) {
                    return Err(fail("the canary was readable"));
                }
                // Only a denial that names the canary proves its rule works: a denial of any other path says nothing.
                let canary = std::fs::canonicalize(&file).ok();
                let names_canary = |p: &String| Path::new(p) == file || canary.is_some() && std::fs::canonicalize(p).ok() == canary;
                if done.denied_paths.iter().any(names_canary) {
                    return Ok(());
                }
                if done.tool_attempted {
                    return Err(fail("the canary was not denied by a permission rule (the read failed for another reason, or another path was denied)"));
                }
            }
            Err(fail("the model never tried the read"))
        })();
        let _ = std::fs::remove_dir_all(&canary_dir);
        result?;
        lock(&self.lead.proven).insert(key);
        Ok(())
    }

    // ---------------------------------------------------------------- history after a restart

    /// If the stream is empty but the Lead has a transcript, put its history back on the stream. The transcript is
    /// `~/.claude/projects/<slug>/<lead_sid>.jsonl`, which the allowlist already covers; the mapping is the same one
    /// the crew view uses (`crew::record`).
    fn rebuild_lead_stream(&self, orch: &Orchestrator, nonces: &Redactor) {
        let Some(sid) = &orch.session else { return };
        let stream = orch.stream();
        if self.bus.tail(&stream, 0, 1).head_seq > 0 {
            return;
        }
        let path = self.claude.join("projects").join(project_slug(&orch.root)).join(format!("{sid}.jsonl"));
        let Some(body) = read_tail(&path, REBUILD_BYTES) else { return };
        let red = nonces.clone();
        for l in body.lines() {
            let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
            if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            for sl in record::parse(&v).unwrap_or_default() {
                let (tag, text) = match sl.kind {
                    LineKind::User => ("you", strip_header(&sl.text)),
                    LineKind::Text => ("lead", sl.text.as_str()),
                    LineKind::Tool => ("tool", sl.text.as_str()),
                    _ => continue,
                };
                for line in text.lines() {
                    let _ = self.bus.push(&stream, &format!("{tag}\t{}", red.clean(line)));
                }
            }
        }
    }
}

/// The Lead's environment plus the directory the claude executable lives in, appended to `PATH`. `resolve_claude` also
/// finds npm installs (`~/.nvm/versions/node/*/bin`), whose `#!/usr/bin/env node` shebang needs `node` from that same
/// directory, which a GUI app's own PATH may not have. Appended, so it never shadows `bash` or `jq`.
fn cli_env(mut env: Vec<(&'static str, OsString)>, bin: &Path) -> Vec<(&'static str, OsString)> {
    let Some(dir) = bin.parent() else { return env };
    match env.iter_mut().find(|(k, _)| *k == "PATH") {
        Some((_, v)) => {
            let mut dirs: Vec<PathBuf> = std::env::split_paths(v).collect();
            if !dirs.iter().any(|d| d == dir) {
                dirs.push(dir.to_path_buf());
            }
            if let Ok(joined) = std::env::join_paths(dirs) {
                *v = joined;
            }
        }
        None => env.push(("PATH", dir.as_os_str().to_owned())),
    }
    env
}

/// `claude --version` under the Lead's own environment, asked of the executable on every ask. Never cached by path: a
/// CLI updated in place must get its own self-test, because deny-rule semantics depend on the version.
fn claude_version(bin: &Path, env: &[(&'static str, OsString)]) -> Result<String, LeadError> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let bad = || LeadError::GuardMissing("claude --version failed".into());
    let mut child = Command::new(bin)
        .arg("--version")
        .env_clear()
        .envs(env.iter().map(|(k, v)| (*k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(io)?;
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                // SAFETY: the child is its own process-group leader and has not been reaped.
                unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
                let _ = child.wait();
                return Err(bad());
            }
        }
    };
    if !status.success() {
        return Err(bad());
    }
    let mut out = String::new();
    if let Some(so) = child.stdout.take() {
        let _ = so.take(4096).read_to_string(&mut out);
    }
    Ok(out.trim().to_string())
}

/// The settings file on disk must be exactly what the core generated (and private). Anything else is a tamper.
pub(super) fn verify_settings_file(path: &Path, want: &str) -> Result<(), LeadError> {
    use std::os::unix::fs::PermissionsExt;
    let tamper = || LeadError::GuardMissing("the Director-General's settings file does not match what was generated".into());
    let got = fsx::read_nofollow(path).map_err(|_| tamper())?;
    let mode = std::fs::metadata(path).map_err(|_| tamper())?.permissions().mode() & 0o777;
    if sha_hex(&got) != sha_hex(want.as_bytes()) || got != want.as_bytes() || mode != 0o600 {
        return Err(tamper());
    }
    Ok(())
}

/// The framed header line of a user turn is plumbing, not part of the question.
fn strip_header(text: &str) -> &str {
    match text.strip_prefix("[Vajra Desktop: question from the human") {
        Some(_) => text.split_once('\n').map_or("", |x| x.1),
        None => text,
    }
}

/// The last `cap` bytes of a file as text, from the first full line.
fn read_tail(path: &Path, cap: u64) -> Option<String> {
    use std::os::unix::fs::OpenOptionsExt;
    // The transcript sits in worker-writable `projects/`: never follow a symlink there, never block on a FIFO (this runs
    // inside `lead_status` and the ask's busy slot), and read only a plain file.
    let mut f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).ok()?;
    let meta = f.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let len = meta.len();
    let start = len.saturating_sub(cap);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.take(cap).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    Some(if start > 0 { text.split_once('\n').map_or(String::new(), |x| x.1.to_string()) } else { text })
}

#[cfg(test)]
#[path = "lead_tests.rs"]
mod tests;

/// The project location of `cwd`: the real path of the nearest ancestor (or `cwd` itself) holding a `.git` file or directory,
/// else the real folder itself. It names WHERE the project sits, never what its `.git` points at.
pub(super) fn git_location(cwd: &Path) -> PathBuf {
    let real = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    real.ancestors().find(|a| std::fs::symlink_metadata(a.join(".git")).is_ok()).map_or_else(|| real.clone(), Path::to_path_buf)
}

/// The main checkout of the project whose common dir is `key`, for a run at `loc`, read once at launch before anything spawns:
/// the parent of a `.git` key; `loc` itself when its `.git` is a directory or a file naming `key`; else, for a linked worktree of
/// a submodule or a separate-git-dir repo, the `core.worktree` of the common dir's config, accepted only if that folder's `.git`
/// names `key`. `None` when none of these holds: the checkout cannot be resolved.
pub(super) fn main_checkout(loc: &Path, key: &Path) -> Option<PathBuf> {
    // A plain folder is its own project (its key is the folder itself).
    if key == loc {
        return Some(loc.to_path_buf());
    }
    if key.file_name() == Some(std::ffi::OsStr::new(".git")) {
        if let Some(p) = key.parent().filter(|p| p.is_dir()) {
            return Some(p.to_path_buf());
        }
    }
    let names_key = |dir: &Path| -> bool {
        let Ok(body) = fsx::read_nofollow(&dir.join(".git")) else { return false };
        let Ok(text) = String::from_utf8(body) else { return false };
        text.lines().next().and_then(|l| l.strip_prefix("gitdir:")).map(str::trim).and_then(|t| std::fs::canonicalize(dir.join(t)).ok()).is_some_and(|t| t == key)
    };
    if std::fs::symlink_metadata(loc.join(".git")).is_ok_and(|m| m.is_dir()) || names_key(loc) {
        return Some(loc.to_path_buf());
    }
    let config = String::from_utf8(fsx::read_nofollow(&key.join("config")).ok()?).ok()?;
    let rel = config.lines().map(str::trim).find_map(|l| l.strip_prefix("worktree").and_then(|r| r.trim_start().strip_prefix('=')).map(str::trim))?;
    let cand = std::fs::canonicalize(key.join(rel)).ok()?;
    names_key(&cand).then_some(cand)
}
