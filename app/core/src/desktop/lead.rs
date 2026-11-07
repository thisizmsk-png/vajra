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
use crate::sessions::argv::{LeadArgv, build_lead_argv};
use crate::sessions::scrubbed_env;
use crate::vajra::bridge::Request;
use crate::vajra::preflight::find_on_path;

/// `read_only` detail for a run this app did not launch.
const NOT_OURS: &str = "run not launched by this app";
/// How long a passing `vajra-doctor.sh` answer is reused.
const DOCTOR_TTL: Duration = Duration::from_secs(60);
/// Most of a Lead transcript read back after a restart.
const REBUILD_BYTES: u64 = 4 * 1024 * 1024;
/// Where the guard script lives under `$HOME`.
/// File name of the Lead session id inside `<data>/control/lead/<run_id>/`.
const LEAD_SID_FILE: &str = "lead_session";
const GUARD_SCRIPT: &str = ".claude/skills/vajra/hooks/pre-tool-use.sh";

/// Test seam: runs right after the settings file is written, before it is verified.
#[cfg(test)]
pub(super) type AfterWrite = Box<dyn Fn(&Path) + Send>;

struct Active {
    ask_id: String,
    handle: Arc<Handle>,
}

/// Everything the Lead needs that is not the run itself.
pub(super) struct LeadState {
    /// The absolute claude binary, resolved once at startup. Never a PATH lookup at ask time.
    bin: Mutex<Option<PathBuf>>,
    /// The environment the Lead (and the self-test) runs with: `HOME`, `PATH`, `LANG`, `TERM`, `USER`, `SHELL`.
    env: Mutex<Vec<(&'static str, OsString)>>,
    /// The running ask per run id. Taking the entry is the "one ask in flight" lock.
    active: Arc<Mutex<HashMap<String, Active>>>,
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
}

impl LeadState {
    pub(super) fn new(home: &Path) -> Self {
        let path_dirs = crate::vajra::preflight::path_dirs();
        Self {
            bin: Mutex::new(resolve_claude(home, &path_dirs)),
            env: Mutex::new(scrubbed_env(|k| std::env::var_os(k))),
            active: Arc::new(Mutex::new(HashMap::new())),
            proven: Mutex::new(HashSet::new()),
            closing: AtomicBool::new(false),
            doctor: Mutex::new(None),
            limits: Mutex::new(Limits::default()),
            exe: std::env::current_exe().unwrap_or_default(),
            #[cfg(test)]
            after_write: Mutex::new(None),
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
    // nvm: newest node version last.
    let mut nvm: Vec<PathBuf> = std::fs::read_dir(home.join(".nvm/versions/node"))
        .map(|r| r.filter_map(|e| e.ok()).map(|e| e.path().join("bin")).collect())
        .unwrap_or_default();
    nvm.sort();
    dirs.extend(nvm.into_iter().rev());
    find_on_path("claude", &dirs)
}

fn io(e: impl std::fmt::Display) -> LeadError {
    LeadError::Io(e.to_string())
}

fn sha_hex(bytes: &[u8]) -> String {
    sha256::hex_digest(bytes)
}

/// What a run needs to be asked about.
struct Target {
    session_id: String,
    run_id: String,
    cwd: PathBuf,
    lead_session_id: Option<String>,
    nonces: Redactor,
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
        let busy = lock(&self.lead.active).get(run_id).map(|a| a.ask_id.clone());
        let t = match self.lead_target(run_id) {
            Ok(t) => t,
            Err(LeadError::Invalid(why)) => return Ok(json!({"available": false, "read_only_reason": why, "busy": false})),
            Err(_) => return Ok(json!({"available": false, "read_only_reason": NOT_OURS, "busy": false})),
        };
        // After a restart the bus is empty; bring the history back from the Lead's own transcript.
        self.rebuild_lead_stream(&t);
        if lock(&self.lead.bin).is_none() {
            return Ok(json!({"available": false, "read_only_reason": "claude not found", "busy": false}));
        }
        let mut out = json!({"available": true, "busy": busy.is_some()});
        if let Some(id) = busy {
            out["ask_id"] = json!(id);
        }
        Ok(out)
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
        let target = self.lead_target(run_id)?;
        let bin = lock(&self.lead.bin).clone().ok_or(LeadError::ClaudeNotFound)?;
        let ask_id = lead::new_ulid(SystemTime::now())?;
        // The nonce of every session of this run, plus the live one, is redacted from the text.
        let mut red = target.nonces.clone();
        red.absorb(&lock(&self.control).redactor());
        let framed = frame(&ask_id, run_id, text, &red)?;

        // One ask in flight per run: two `--resume` writers would fork the Lead's transcript.
        let handle = Handle::new();
        {
            let mut active = lock(&self.lead.active);
            if self.lead.closing.load(Ordering::SeqCst) {
                return Err(LeadError::ShuttingDown);
            }
            if active.contains_key(run_id) {
                return Err(LeadError::Busy);
            }
            active.insert(run_id.to_string(), Active { ask_id: ask_id.clone(), handle: handle.clone() });
        }
        let started = self.start_ask(&target, &bin, &ask_id, &framed, &handle, red);
        if started.is_err() {
            lock(&self.lead.active).remove(run_id);
        }
        started
    }

    // ---------------------------------------------------------------- the run

    /// The run, if this app launched or resumed it: in memory, or in `runs.jsonl`. A finished run still counts.
    /// A run id is only minute-based, so it must name exactly one (campaign, session): if the live runs and the whole
    /// index hold two different ones for it, nothing is selected and the ask is refused, never guessed.
    fn lead_target(&self, run_id: &str) -> Result<Target, LeadError> {
        let live: Vec<Run> = lock(&self.runs).values().filter(|r| r.run_id == run_id).cloned().collect();
        let indexed: Vec<runindex::Entry> = runindex::entries(&self.data.join("runs.jsonl")).into_iter().filter(|e| e.run_id == run_id).collect();
        let mut ids: HashSet<(String, String)> = live.iter().map(|r| (r.campaign.clone(), r.session_id.clone())).collect();
        ids.extend(indexed.iter().map(|e| (e.campaign.clone(), e.session_id.clone())));
        if ids.len() > 1 {
            return Err(LeadError::Invalid("run_id: more than one run has this id; it cannot be told apart".into()));
        }
        // The Lead session id is read from the guarded control dir, never from `runs.jsonl`: a worker can write that.
        let lead_sid = self.read_lead_sid(run_id);
        if let Some(mut r) = live.into_iter().next() {
            self.add_disk_nonce(&mut r.nonces, &r.session_id);
            return Ok(Target { lead_session_id: lead_sid, session_id: r.session_id, run_id: r.run_id, cwd: r.cwd, nonces: r.nonces });
        }
        // A run loaded from disk (after a restart): its control nonce is still on disk and must stay out of the
        // ask text, the ledger, the bus and the rebuilt history.
        let e = indexed.into_iter().next_back().ok_or(LeadError::ReadOnly(NOT_OURS))?;
        let mut nonces = Redactor::default();
        self.add_disk_nonce(&mut nonces, &e.session_id);
        Ok(Target { session_id: e.session_id, run_id: e.run_id, cwd: e.cwd, lead_session_id: lead_sid, nonces })
    }

    /// `<data>/control/lead/<run_id>/lead_session`: the authoritative run to Lead-session mapping. It sits under the
    /// directory the worker guard protects; `runs.jsonl` (worker-writable) is not trusted for it.
    fn lead_sid_dir(&self, run_id: &str) -> PathBuf {
        self.data.join("control").join("lead").join(run_id)
    }

    fn read_lead_sid(&self, run_id: &str) -> Option<String> {
        let body = fsx::read_nofollow(&self.lead_sid_dir(run_id).join(LEAD_SID_FILE)).ok()?;
        let sid = String::from_utf8(body).ok()?.trim().to_string();
        ids::valid_session_id(&sid).then_some(sid)
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

    fn start_ask(
        &self,
        t: &Target,
        bin: &Path,
        ask_id: &str,
        framed: &lead::FramedAsk,
        handle: &Arc<Handle>,
        red: Redactor,
    ) -> Result<Value, LeadError> {
        if !t.cwd.is_dir() {
            return Err(LeadError::Invalid("cwd: the run's directory is gone".into()));
        }
        let env = lock(&self.lead.env).clone();
        self.guard_preconditions(&env)?;
        // Same environment for the version probe, the self-test and the ask, with the CLI's own directory on PATH.
        let env = cli_env(env, bin);
        // Per-run files: the settings hold the run repo's deny rules, and asks for two different runs can overlap
        // (the busy lock is per run). A shared file would let run B rewrite it before run A's CLI reads it.
        // Under `control/`, which the worker guard already keeps workers from writing (on the default data dir).
        let base = self.data.join("control").join("lead");
        fsx::private_dir(&base).map_err(io)?;
        let dir = base.join(&t.run_id);
        let settings = self.write_lead_files(&dir, &t.cwd, &t.run_id)?;
        self.ensure_selftest(bin, &env, &dir, &t.cwd, &settings, &t.run_id, handle)?;
        // The self-test can take minutes; a worker with the same uid could have rewritten the files meanwhile. Write
        // them again and check both byte for byte, immediately before the real launch.
        if self.write_lead_files(&dir, &t.cwd, &t.run_id)? != settings {
            return Err(LeadError::GuardMissing("the Lead settings changed during the self-test".into()));
        }

        let (lead_sid, resume) = match &t.lead_session_id {
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

        let stream = format!("lead-{}", t.run_id);
        // The history of earlier asks is back on the stream before this one is added to it.
        self.rebuild_lead_stream(t);
        let svc = self.control.clone();
        let ledger = {
            let (svc, run_id, session_id, lead_sid, ask_id) = (svc.clone(), t.run_id.clone(), t.session_id.clone(), lead_sid.clone(), ask_id.to_string());
            move |state: AskState, text: Option<&str>, reason: Option<&str>| {
                lock(&svc)
                    .record_ask(&AskRecord { ask_id: &ask_id, run_id: &run_id, session_id: &session_id, lead_session_id: &lead_sid, state, text, reason })
                    .map_err(|e| LeadError::Io(e.code().to_string()))
            }
        };
        // Record first, spawn second: if the ledger cannot take the line, the Lead is never started.
        ledger(AskState::Queued, Some(framed.text()), None)?;
        let child = match lead_run::spawn(&argv, &t.cwd, &env, framed, handle) {
            Ok(c) => c,
            Err(e) => {
                let _ = ledger(AskState::Failed, None, Some(e.code()));
                return Err(e);
            }
        };
        let _ = self.bus.push(&stream, &format!("you\t{}", red.clean(framed.text())));

        let limits = *lock(&self.lead.limits);
        let run_id = t.run_id.clone();
        let bus = self.bus.clone();
        let active = self.lead.active.clone();
        let log = self.log.clone();
        let handle = handle.clone();
        let first = !resume;
        let sid_dir = self.lead_sid_dir(&t.run_id);
        let sid_text = lead_sid.clone();
        let ledger_init = ledger.clone();
        std::thread::spawn(move || {
            let on_init: Box<dyn FnMut(&InitInfo) + Send> = Box::new(move |_| {
                // The Lead passed the tool and mode check: the ask is delivered. A new Lead session is remembered
                // only now, so a Lead that never started leaves no id behind to `--resume`.
                let _ = ledger_init(AskState::Delivered, None, None);
                if first && fsx::write_atomic(&sid_dir, LEAD_SID_FILE, sid_text.as_bytes()).is_err() {
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
            lock(&active).remove(&run_id);
        });
        Ok(json!({"ask_id": ask_id, "state": "queued", "stream": format!("lead-{}", t.run_id), "lead_session_id": lead_sid}))
    }

    // ---------------------------------------------------------------- the guard (ADR-13 2a)

    /// Nothing is spawned unless every one of these holds: the guard script is a regular file, the doctor passes,
    /// and the Lead's environment keeps `HOME` and a `PATH` that has `bash` and `jq` (without them the guard would
    /// fail closed and block every Lead tool).
    fn guard_preconditions(&self, env: &[(&'static str, OsString)]) -> Result<(), LeadError> {
        let miss = |why: &str| LeadError::GuardMissing(why.to_string());
        // The Lead's generated files live under `<data>/control/`, which the installed worker guard protects only at
        // its default place. Under any other data dir a worker could rewrite them, so the Lead is not started.
        let guarded = paths::resolve_data_dir(None, &self.home);
        let same = |a: &Path, b: &Path| matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y);
        if !same(&self.data, &guarded) {
            return Err(miss("Lead unavailable: data dir is not guard-protected (VAJRA_DESKTOP_DATA override)"));
        }
        let script = self.home.join(GUARD_SCRIPT);
        if !std::fs::metadata(&script).is_ok_and(|m| m.is_file()) {
            return Err(miss("pre-tool-use.sh is missing"));
        }
        self.doctor_ok().map_err(miss)?;
        let get = |k: &str| env.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
        if get("HOME").is_none_or(|h| h.is_empty()) {
            return Err(miss("HOME is not set for the Lead"));
        }
        let dirs: Vec<PathBuf> = get("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        if find_on_path("bash", &dirs).is_none() || find_on_path("jq", &dirs).is_none() {
            return Err(miss("bash and jq must be on the Lead's PATH"));
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
        let out = self.bridge.run(&Request::Doctor).ok().filter(|o| o.code == Some(0) && !o.timed_out);
        let verdict = match out {
            None => Err("vajra doctor does not pass"),
            Some(o) if o.stdout.lines().any(|l| l.starts_with("PASS") && l.contains("denyWrite active")) => Ok(()),
            Some(_) => Err("Lead unavailable: the OS sandbox with denyWrite on the app dir is not active (enable it with /sandbox)"),
        };
        *cache = Some((Instant::now(), verdict.is_ok()));
        verdict
    }

    /// Write `<data>/lead/<run_id>/{settings,mcp}.json` and check the settings file on disk is the generated one, byte for
    /// byte. Returns the settings' sha256. Rewritten on every ask, so an edit between asks never matters; the check
    /// closes the gap between writing and spawning.
    fn write_lead_files(&self, dir: &Path, repo: &Path, run_id: &str) -> Result<String, LeadError> {
        if !self.data.is_absolute() {
            return Err(LeadError::GuardMissing("the data dir must be an absolute path".into()));
        }
        fsx::private_dir(dir).map_err(io)?;
        let want = lead_settings::settings(&self.home, repo, &self.data)?;
        let mcp = json!({"mcpServers": {"vajra-desktop": {
            "command": self.lead.exe.to_string_lossy(),
            "args": ["--mcp"],
            "env": {"VAJRA_DESKTOP_DATA": self.data.to_string_lossy(), crate::control::mcp::RUN_ENV: run_id},
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
    fn ensure_selftest(&self, bin: &Path, env: &[(&'static str, OsString)], dir: &Path, repo: &Path, settings_sha: &str, run_id: &str, h: &Arc<Handle>) -> Result<(), LeadError> {
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
            let bare = lead_settings::settings_no_hooks(&self.home, repo, &self.data)?;
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
                if done.permission_denied {
                    return Ok(());
                }
                if done.tool_attempted {
                    return Err(fail("the read failed, but not because a permission rule denied it"));
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
    fn rebuild_lead_stream(&self, t: &Target) {
        let Some(sid) = &t.lead_session_id else { return };
        let stream = format!("lead-{}", t.run_id);
        if self.bus.tail(&stream, 0, 1).head_seq > 0 {
            return;
        }
        let path = self.claude.join("projects").join(project_slug(&t.cwd)).join(format!("{sid}.jsonl"));
        let Some(body) = read_tail(&path, REBUILD_BYTES) else { return };
        let red = t.nonces.clone();
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
    let tamper = || LeadError::GuardMissing("the Lead settings file does not match what was generated".into());
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
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
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
