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
/// Where the run's project sits: the folder holding the nearest `.git` entry (`git_location`), as found at launch. The pin
/// is keyed by this LOCATION, never by what `.git` later contains.
const LOC_FILE: &str = "project_loc";
/// The project's main checkout as resolved at launch (the pinned store project's recorded root), or `-` when it could not
/// be. Guarded like the others, and denied to other projects' Directors-General even if the project later leaves the store.
const MAIN_FILE: &str = "project_main";
/// Most other-project run folders the Director-General's deny rules hold; more refuses the ask (#80 review: never
/// drop one).
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

/// The Director-General cannot be asked: why, in words.
fn status_off(why: String) -> Res {
    Ok(json!({"available": false, "read_only_reason": why, "busy": false, "name": orchestrator::NAME}))
}

/// `status_off` for a lookup that failed. A store refusal says which, so the window can offer "Add this folder as a
/// project" (decision Q1).
fn store_refusal(e: &LeadError) -> Res {
    let mut v = status_off(e.detail().unwrap_or_else(|| e.code().to_string()))?;
    if let LeadError::NotAProject(_) | LeadError::ProjectArchived | LeadError::ProjectMissing = e {
        let j = e.to_json();
        for k in ["error", "action", "path"] {
            if let Some(x) = j.get(k) {
                v[k] = x.clone();
            }
        }
    }
    Ok(v)
}

impl App {
    // ---------------------------------------------------------------- commands

    /// The Lead chat's state for a run: can it be asked, and is a question running now?
    pub fn lead_status(&self, run_id: &str) -> Res {
        if !ids::valid_run_id(run_id) {
            return Err(super::invalid("run_id", "must be YYYYMMDD-HHMM"));
        }
        let t = match self.lead_target(run_id) {
            Ok(t) => t,
            Err(e) => return status_off(e.detail().unwrap_or_else(|| NOT_OURS.to_string())),
        };
        let orch = match self.orchestrator_for_run(&t.run_id, &t.cwd) {
            Ok(o) => o,
            Err(e) => return store_refusal(&e),
        };
        // After a restart the bus is empty; bring the history back from the Director-General's own transcript.
        self.rebuild_lead_stream(&orch, &self.project_redactor(&t, &orch));
        self.orchestrator_status(&orch)
    }

    /// `lead_status` by project (FG-02c, decision Q2): the project's Director-General, with no run needed. A read only:
    /// nothing is created and no stand-in conversation is carried over (the first ask does that), so a project never
    /// asked reads `not_started`. The stream is not rebuilt from the transcript here (that needs a run's nonces to
    /// redact); the run form does it.
    pub fn lead_status_for_project(&self, project_id: &str) -> Res {
        if !crate::projects::contract::valid_project_id(project_id) {
            return Err(super::invalid("project_id", "must be a project id"));
        }
        let project = match self.active_project(project_id) {
            Ok(p) => p,
            Err(e) => return store_refusal(&e),
        };
        match self.recorded_orchestrator(project_id) {
            Ok(Some(orch)) => self.orchestrator_status(&orch),
            Ok(None) => {
                // Shown as conversation 1 in the project's folder, which is what its first ask starts.
                let orch = Orchestrator { id: project.id.clone(), root: PathBuf::from(&project.root), dir: self.orchestrator_dir(&project.id), generation: 1, session: None };
                let mut v = self.orchestrator_status(&orch)?;
                if v["state"] == "idle" {
                    v["state"] = json!("not_started");
                    v["reason"] = json!("not asked yet");
                }
                Ok(v)
            }
            Err(e) => status_off(e.detail().unwrap_or_else(|| e.code().to_string())),
        }
    }

    /// The ADR-15 2.1 state of one Director-General.
    fn orchestrator_status(&self, orch: &Orchestrator) -> Res {
        let off = status_off;
        let busy = lock(&self.lead.active).get(&orch.id).map(|a| a.ask_id.clone());
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
            "project_id": orch.id, "project_name": self.projects.get(&orch.id).ok().map(|p| p.name), "generation": orch.generation, "stream": orch.stream(),
        });
        if let Some(id) = busy {
            out["ask_id"] = json!(id);
        }
        Ok(out)
    }

    /// The project whose Director-General is running ask `ask_id`, if one is.
    pub(super) fn ask_owner(&self, ask_id: &str) -> Option<String> {
        lock(&self.lead.active).iter().find(|(_, a)| a.ask_id == ask_id).map(|(p, _)| p.clone())
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
        self.ask_lead_expecting(run_id, text, None).map_err(|e| e.to_json())
    }

    /// `ask_lead` aimed at conversation `expect` (a receipt's `expect_generation`): checked again under the ask lock, so
    /// a "New conversation" that lands first turns the ask into `stale_generation`, never into a turn of the new one.
    pub(super) fn ask_lead_expecting(&self, run_id: &str, text: &str, expect: Option<u32>) -> Result<Value, LeadError> {
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
        let fresh = self.active_project(&orch.id).and_then(|_| self.orchestrator(&orch.id, &orch.root));
        let fresh = fresh.and_then(|o| match expect {
            Some(g) if g != o.generation => Err(LeadError::StaleGeneration(o.generation)),
            _ => Ok(o),
        });
        let orch = match fresh {
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
        let project = self.projects.find_by_path(cwd).ok().flatten().map(|p| p.id);
        self.record_run_as(run_id, cwd, project.as_deref(), session_id, campaign, workflow, started)
    }

    /// `record_run` with a store project chosen by the caller; the location is the folder's own now (tests only).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_run_as(&self, run_id: &str, cwd: &Path, project: Option<&str>, session_id: &str, campaign: &str, workflow: bool, started: i64) -> std::io::Result<()> {
        self.record_run_pinned(run_id, cwd, &git_location(cwd), project, session_id, campaign, workflow, started)
    }

    /// `record_run` with the exact (location, store project) pair the launch checked (`launch_project`), written before
    /// anything is spawned: neither is read from the repo again, so a `.git` the launched process creates or redirects
    /// afterwards cannot change what was pinned (#80 review).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_run_pinned(&self, run_id: &str, cwd: &Path, loc: &Path, project: Option<&str>, session_id: &str, campaign: &str, workflow: bool, started: i64) -> std::io::Result<()> {
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
        // The project is pinned now, as the launch found it before spawning (ADR-15 2.3). A folder that is no project
        // gets no pin (the launch refuses one; a record made otherwise is refused when asked about).
        if let Some(p) = project {
            fsx::write_atomic(&dir, orchestrator::PROJECT_FILE, p.as_bytes())?;
        }
        fsx::write_atomic(&dir, LOC_FILE, loc.to_string_lossy().as_bytes())?;
        // The main checkout, resolved once at launch (#80 review): the pinned store project's root; a folder that is no
        // project is its own checkout. A pin the store cannot read now is recorded as unresolved (`-`).
        let main: Option<PathBuf> = match project {
            Some(p) => self.projects.get(p).ok().map(|p| PathBuf::from(p.root)),
            None => Some(loc.to_path_buf()),
        };
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

    /// The folders of the app's other projects, for the Director-General's deny rules (ADR-15 D8, FG-02c): the recorded
    /// root of every other project in the store, archived ones too (archiving hides a project, it does not make its
    /// folder readable to others), and every distinct folder a run of another project was launched in (a worktree
    /// outside its project's root, #80 review), newest first, at most `MAX_RUN_FOLDERS`. Never `repo`'s own root or
    /// a run of its own project, and never a folder that holds `repo` (denying an ancestor would deny the
    /// Director-General its own project). A store that cannot be listed in full refuses the ask: a partial list would
    /// leave some project readable.
    ///
    /// Which project a run belongs to is the store id pinned in its guarded record at launch, never the repo's
    /// worker-writable `.git` re-read now (#80 review): `own` is the asking Director-General's project, and a run with
    /// no pin counts as another project's. More than `MAX_OTHER_ROOTS` run folders refuses the ask rather than leave
    /// one undenied.
    pub(super) fn other_project_roots(&self, own_project: &str, repo: &Path) -> Result<Vec<PathBuf>, LeadError> {
        let listing = self.projects.list(true).map_err(|_| LeadError::GuardMissing("the project list could not be read for the deny rules".into()))?;
        if listing.truncated {
            return Err(LeadError::GuardMissing("the project list is too large to build the deny rules".into()));
        }
        let own = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
        let mut out: Vec<PathBuf> = Vec::new();
        for p in listing.projects {
            let root = PathBuf::from(&p.root);
            let real = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
            if real == own || own.starts_with(&real) || out.contains(&root) {
                continue;
            }
            out.push(root);
        }
        let mut added = 0;
        for run in self.recorded_runs() {
            let Some(cwd) = self.read_run_cwd(&run) else { continue };
            if self.pinned_project(&run).as_deref() == Some(own_project) {
                continue;
            }
            // The run folder and the checkout root the launch pinned (`project_loc`, #80 review): a submodule's or a
            // separate-git-dir repo's root is not the run folder, and may not be in the store.
            // And the main checkout resolved at launch (`project_main`): `-` means it could not be resolved, so it cannot be
            // protected and the ask is refused rather than left open (#80 review).
            let pinned_main = match fsx::read_nofollow(&self.lead_sid_dir(&run).join(MAIN_FILE)).ok().and_then(|b| String::from_utf8(b).ok()) {
                Some(m) if m.trim() == "-" => {
                    return Err(LeadError::GuardMissing("another project's main checkout could not be resolved, so the Director-General cannot deny it; start that project from its main folder".into()));
                }
                Some(m) => Some(PathBuf::from(m.trim())).filter(|p| p.is_absolute()),
                None => None,
            };
            for dir in std::iter::once(cwd).chain(self.read_run_loc(&run)).chain(pinned_main) {
                let real = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
                // Inside its own root is its own project (an inner project's root is denied above, from the store).
                if own.starts_with(&real) || real.starts_with(&own) || out.contains(&dir) {
                    continue;
                }
                if added >= MAX_OTHER_ROOTS {
                    return Err(LeadError::GuardMissing(format!("more than {MAX_OTHER_ROOTS} other run folders to deny; the Director-General cannot be started with all of them")));
                }
                out.push(dir);
                added += 1;
            }
        }
        Ok(out)
    }

    /// The folders of project `own_project` (the asking run's) that need the secret-file rules: the project's main checkout
    /// (`root`) plus the folder of every recorded run pinned to it (a worktree outside the main checkout,
    /// a subfolder), from the guarded records and never the worker-writable index or `.git`. A folder inside another
    /// already listed is covered by its `**` rules and left out. More than `MAX_OTHER_ROOTS` refuses the ask.
    pub(super) fn own_project_folders(&self, own_project: &str, root: &Path) -> Result<Vec<PathBuf>, LeadError> {
        let mut out: Vec<PathBuf> = vec![root.to_path_buf()];
        for run in self.recorded_runs() {
            let Some(cwd) = self.read_run_cwd(&run) else { continue };
            // Dropped only when BOTH its spelling and its real path sit under `d`: a folder spelled under `d` (a symlink) can really
            // be outside it, and one spelled differently (`/var` vs `/private/var`) needs its own supplied-form rules.
            let real = std::fs::canonicalize(&cwd).unwrap_or_else(|_| cwd.clone());
            let covered = |d: &PathBuf| cwd.starts_with(d) && real.starts_with(std::fs::canonicalize(d).unwrap_or_else(|_| d.clone()));
            if self.pinned_project(&run).as_deref() != Some(own_project) || out.iter().any(covered) {
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

    /// The project a launch in `cwd` runs under, and the location it was found at: the exact pair a launch records before it
    /// spawns anything (`record_run_pinned`), read from the guarded run records before any Git metadata is read (#80
    /// review, on the store id).
    ///
    /// A project sits at the LOCATION of the nearest `.git` entry above `cwd` (`git_location`; the folder itself when there
    /// is none). The first guarded launch at a location pins it to a store project and records both. Every later launch
    /// at the same location reuses that pin, whatever `.git` now contains: the root, a subfolder and a sibling resolve
    /// alike, and a worker that rewrites `.git` cannot move any of them to another project. A nested repository has its
    /// own `.git` at another location, so it is its own project, but a NEW location inside a pinned Git project's tree is
    /// admitted only as a nested repo or a submodule (`project_of_new_location`).
    ///
    /// Records of one location that disagree on its project refuse rather than guess. The old record formats are migrated
    /// once from their guarded location: a record with no `project_loc` gets its location written back, and one with no
    /// store pin (or a pre-store `k_` stand-in pin) gets its store project (`migrate_old_pin`). Anything else malformed
    /// refuses. The pinned project must still be active.
    pub(super) fn launch_project(&self, cwd: &Path) -> Result<(PathBuf, crate::projects::contract::Project), Value> {
        let want = git_location(cwd);
        let mut ids: Vec<String> = Vec::new();
        for r in self.recorded_runs() {
            let Some(c) = self.read_run_cwd(&r) else { continue };
            let dir = self.lead_sid_dir(&r);
            let old = std::fs::symlink_metadata(dir.join(LOC_FILE)).is_err();
            let loc = if old { git_location(&c) } else { self.read_run_loc(&r).unwrap_or_default() };
            if loc != want {
                continue;
            }
            let pid = match self.pinned_project(&r) {
                Some(p) if crate::projects::contract::valid_project_id(&p) => p,
                _ => match self.migrate_old_pin(&r, &loc)? {
                    Some(p) => p,
                    // A well-formed old record whose folder is no project now: it names no project to keep.
                    None => continue,
                },
            };
            if old {
                // Best effort: a record that cannot be written back is derived again next time.
                let _ = fsx::write_atomic(&dir, LOC_FILE, loc.to_string_lossy().as_bytes());
            }
            if !ids.contains(&pid) {
                ids.push(pid);
            }
        }
        let project = match ids.len() {
            0 => self.project_of_new_location(&want, cwd)?,
            1 => self.active_project(&ids[0]).map_err(|e| e.to_json())?,
            _ => return Err(super::fail("project_membership_inconsistent", "earlier runs of this folder were recorded under different projects")),
        };
        Ok((want, project))
    }

    /// The store project for a NEW project location `want`, from ONE look at its `.git` and never a second walk.
    ///
    /// Only a pinned GIT project's tree constrains what nests under it (a plain folder's pin, like `$HOME`'s, leaves every
    /// location under it unconstrained). Outside every such tree it is the store project of the folder (`project_of_folder`).
    /// Inside one, the location may become its own project only if its `.git` is (a) a real directory (not a symlink) with
    /// no `commondir` file: a self-contained nested repository, identified by that directory; or (b) a `gitdir:` file whose
    /// canonical target lies INSIDE an enclosing Git project's common dir (as the store recorded it) with no `commondir`
    /// file on the resolution path (the target and every directory up to that common dir): a submodule, identified by
    /// exactly the target checked. Its project is then the store project rooted at `want` or recorded with that identity as
    /// its common dir; none is `not_a_project`, never the enclosing project. Anything else (a pointer into another project,
    /// a symlinked `.git`, a redirecting `commondir`, an unresolvable target, a `.git` removed from the pinned tree) refuses
    /// with `project_membership_inconsistent` and is never pinned. An enclosing pin whose project the store cannot read
    /// counts as a Git project with no known common dir (fail closed).
    fn project_of_new_location(&self, want: &Path, cwd: &Path) -> Result<crate::projects::contract::Project, Value> {
        let mut enclosing: Vec<Option<PathBuf>> = Vec::new();
        for r in self.recorded_runs() {
            let (Some(loc), Some(pid)) = (self.read_run_loc(&r), self.pinned_project(&r)) else { continue };
            if loc == want || !want.starts_with(&loc) {
                continue;
            }
            let common = match self.projects.get(&pid) {
                // The store recorded the common dir canonical: one that now resolves anywhere else (replaced by a symlink,
                // or under a redirected parent) has been redirected and admits nothing (#80 review). It still constrains.
                Ok(p) => match p.root_key.git_common_dir {
                    Some(c) => exact_self(Path::new(&c)),
                    None => continue, // a plain project's pin
                },
                Err(_) => None,
            };
            if !enclosing.contains(&common) {
                enclosing.push(common);
            }
        }
        if enclosing.is_empty() {
            return self.project_of_folder(cwd).map_err(|e| e.to_json());
        }
        let refuse = |why: &str| super::fail("project_membership_inconsistent", why);
        let git = want.join(".git");
        let md = std::fs::symlink_metadata(&git).map_err(|_| refuse("this folder's .git cannot be read"))?;
        let identity = if md.is_dir() {
            if std::fs::symlink_metadata(git.join("commondir")).is_ok() {
                return Err(refuse("this folder's .git points at another project's common dir"));
            }
            // This very directory; a swap to a link between the check and here no longer names `want/.git`.
            let key = std::fs::canonicalize(&git).map_err(|_| refuse("this folder's .git cannot be read"))?;
            if key.parent() != Some(want) || key.file_name() != Some(std::ffi::OsStr::new(".git")) {
                return Err(refuse("this folder's .git changed while it was checked"));
            }
            key
        } else if md.is_file() {
            let body = fsx::read_nofollow(&git).ok().and_then(|b| String::from_utf8(b).ok()).ok_or_else(|| refuse("this folder's .git cannot be read"))?;
            let target = body.lines().next().and_then(|l| l.strip_prefix("gitdir:")).map(str::trim).filter(|t| !t.is_empty());
            let real = target.and_then(|t| std::fs::canonicalize(want.join(t)).ok());
            match real.and_then(|real| enclosing.iter().flatten().find_map(|common| nested_identity(&real, common))) {
                Some(identity) => identity,
                None => return Err(refuse("this folder's .git is neither a nested repository nor a submodule of the enclosing project")),
            }
        } else {
            return Err(refuse("this folder's .git is neither a nested repository nor a submodule of the enclosing project"));
        };
        self.project_with_identity(want, &identity, cwd)
    }

    /// The store project rooted at `want` (by device and inode) or recorded with `identity` as its common dir, compared by
    /// canonical path: the project an admitted nested location names, found without reading its `.git` again.
    fn project_with_identity(&self, want: &Path, identity: &Path, cwd: &Path) -> Result<crate::projects::contract::Project, Value> {
        use std::os::unix::fs::MetadataExt;
        let listing = self.projects.list(true).map_err(|e| super::fail("io", e))?;
        if listing.truncated {
            return Err(super::fail("io", "the project store could not be read whole"));
        }
        let at = std::fs::metadata(want).ok().map(|m| (m.dev(), m.ino()));
        let hit = listing.projects.into_iter().find(|p| {
            at == Some((p.root_key.dev, p.root_key.ino)) || p.root_key.git_common_dir.as_ref().and_then(|c| exact_self(Path::new(c))).as_deref() == Some(identity)
        });
        match hit {
            Some(p) if p.archived.is_some() => Err(LeadError::ProjectArchived.to_json()),
            Some(p) => Ok(p),
            None => Err(LeadError::NotAProject(cwd.to_string_lossy().into_owned()).to_json()),
        }
    }

    /// The project location pinned when the run was launched.
    fn read_run_loc(&self, run_id: &str) -> Option<PathBuf> {
        let body = fsx::read_nofollow(&self.lead_sid_dir(run_id).join(LOC_FILE)).ok()?;
        let p = PathBuf::from(String::from_utf8(body).ok()?.trim());
        p.is_absolute().then_some(p)
    }

    /// The one-time migration of the old record formats to a store pin, for `launch_project`. A well-formed record (guarded
    /// cwd, session and campaign) with no pin, or with a pre-store `k_` stand-in pin that its guarded cwd still gives, gets
    /// the store project of its guarded cwd now, written back under the guard: `Ok(Some(id))`. `Ok(None)` when that folder
    /// is no project (nothing is written). Anything else (a pin file that is not a valid id or stand-in, a stand-in its
    /// folder no longer gives, a record missing its session or campaign, a write that fails) refuses.
    fn migrate_old_pin(&self, run_id: &str, cwd: &Path) -> Result<Option<String>, Value> {
        let refuse = |why: &str| Err(super::fail("project_membership_inconsistent", why));
        let dir = self.lead_sid_dir(run_id);
        let file = dir.join(orchestrator::PROJECT_FILE);
        if std::fs::symlink_metadata(&file).is_ok() {
            match self.pinned_project(run_id) {
                Some(k) if ids::valid_standin_ref(&k) && orchestrator::project_ref(cwd).0 == k => {}
                Some(k) if ids::valid_standin_ref(&k) => return refuse("an earlier run of this folder was recorded under another project"),
                _ => return refuse("an earlier run of this folder has an unreadable project record"),
            }
        }
        if self.read_run_record(run_id).is_none() {
            return refuse("an earlier run of this folder has no recorded project");
        }
        let Some(p) = self.projects.find_by_path(cwd).map_err(|_| super::fail("project_membership_inconsistent", "the project list could not be read"))? else {
            return Ok(None);
        };
        if fsx::write_atomic(&dir, orchestrator::PROJECT_FILE, p.id.as_bytes()).is_err() {
            return refuse("an earlier run's project could not be recorded");
        }
        Ok(self.pinned_project(run_id).filter(|q| *q == p.id))
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
        let others = self.other_project_roots(&orch.id, &orch.root)?;
        // The project's own folders (main checkout plus every recorded run or worktree folder) all get the secret-file
        // rules; the self-test and the recheck below use this same set.
        let repos = self.own_project_folders(&orch.id, &orch.root)?;
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
            let (project, generation) = (orch.id.clone(), orch.generation);
            move |state: AskState, text: Option<&str>, reason: Option<&str>| {
                lock(&svc)
                    .record_ask(&AskRecord { ask_id: &ask_id, run_id: &run_id, session_id: &session_id, lead_session_id: &lead_sid, state, text, reason, project: Some((&project, generation)) })
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
            "project_id": orch.id, "project_name": self.projects.get(&orch.id).ok().map(|p| p.name), "generation": orch.generation, "name": orchestrator::NAME,
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
/// `p` if it still canonicalizes to exactly itself (a path the store recorded canonical), else `None`: a common dir now
/// reached through a symlink or a redirected parent is no longer the project it was recorded as (#80 review).
fn exact_self(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok().filter(|c| c == p)
}

/// The identity of a nested `gitdir:` target `real` (canonical) inside an enclosing pinned Git project's common dir
/// `common`, or `None`: no `commondir` above the target on the way up to `common`, and then either (a) none in the
/// target itself, a submodule keyed by exactly the target; or (b) one that resolves to `common`, a linked worktree of the
/// enclosing project keyed by its common dir. Any other `commondir` refuses (#80 review).
fn nested_identity(real: &Path, common: &Path) -> Option<PathBuf> {
    if !real.starts_with(common) || !real.ancestors().skip(1).take_while(|d| d.starts_with(common)).all(|d| std::fs::symlink_metadata(d.join("commondir")).is_err()) {
        return None;
    }
    let cd = real.join("commondir");
    match fsx::read_nofollow(&cd) {
        Err(_) if std::fs::symlink_metadata(&cd).is_err() => Some(real.to_path_buf()),
        Ok(c) => {
            let resolved = String::from_utf8(c).ok().and_then(|c| std::fs::canonicalize(real.join(c.trim())).ok());
            (resolved.as_deref() == Some(common)).then(|| common.to_path_buf())
        }
        Err(_) => None,
    }
}

pub(super) fn git_location(cwd: &Path) -> PathBuf {
    let real = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    real.ancestors().find(|a| std::fs::symlink_metadata(a.join(".git")).is_ok()).map_or_else(|| real.clone(), Path::to_path_buf)
}
