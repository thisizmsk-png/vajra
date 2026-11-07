//! "Ask the Lead" at the app level (ADR-13): the three commands `ask_lead`, `cancel_ask` and `lead_status`.
//!
//! One ask is one headless `claude -p` turn (see `control::lead_run`). This file decides whether it may run (the run
//! is one this app launched, no ask is in flight, the Vajra guard is in place and proven), records it in the ledger
//! *before* it spawns, and streams the reply into the bus stream `lead-<run_id>`.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};

use super::{App, Res, Run, project_slug, runindex};
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
use crate::vajra::bridge::{Request, run_process};
use crate::vajra::preflight::find_on_path;

/// `read_only` detail for a run this app did not launch.
const NOT_OURS: &str = "run not launched by this app";
/// How long a passing `vajra-doctor.sh` answer is reused.
const DOCTOR_TTL: Duration = Duration::from_secs(60);
/// Most of a Lead transcript read back after a restart.
const REBUILD_BYTES: u64 = 4 * 1024 * 1024;
/// Where the guard script lives under `$HOME`.
const GUARD_SCRIPT: &str = ".claude/skills/vajra/hooks/pre-tool-use.sh";

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
    version: Mutex<Option<(PathBuf, String)>>,
    doctor: Mutex<Option<(Instant, bool)>>,
    limits: Mutex<Limits>,
    /// The app's own executable, for the `--mcp` server entry in `mcp.json`.
    exe: PathBuf,
    #[cfg(test)]
    pub(super) after_write: Mutex<Option<Box<dyn Fn(&Path) + Send>>>,
}

impl LeadState {
    pub(super) fn new(home: &Path) -> Self {
        let path_dirs = crate::vajra::preflight::path_dirs();
        Self {
            bin: Mutex::new(resolve_claude(home, &path_dirs)),
            env: Mutex::new(scrubbed_env(|k| std::env::var_os(k))),
            active: Arc::new(Mutex::new(HashMap::new())),
            proven: Mutex::new(HashSet::new()),
            version: Mutex::new(None),
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
    campaign: String,
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
        let Some(t) = self.lead_target(run_id) else {
            return Ok(json!({"available": false, "read_only_reason": NOT_OURS, "busy": false}));
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

    /// Ask the Lead one question about a run. Returns as soon as the Lead is spawned; the reply streams into
    /// `lead-<run_id>`.
    pub fn ask_lead(&self, run_id: &str, text: &str) -> Res {
        self.ask_lead_inner(run_id, text).map_err(|e| e.to_json())
    }

    fn ask_lead_inner(&self, run_id: &str, text: &str) -> Result<Value, LeadError> {
        if !ids::valid_run_id(run_id) {
            return Err(LeadError::Invalid("run_id: must be YYYYMMDD-HHMM".into()));
        }
        let target = self.lead_target(run_id).ok_or(LeadError::ReadOnly(NOT_OURS))?;
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
    fn lead_target(&self, run_id: &str) -> Option<Target> {
        let latest = runindex::latest(&self.data.join("runs.jsonl"));
        let lead_of = |campaign: &str| latest.get(campaign).filter(|e| e.run_id == run_id).and_then(|e| e.lead_session_id.clone());
        let live: Option<Run> = lock(&self.runs).values().find(|r| r.run_id == run_id).cloned();
        if let Some(r) = live {
            return Some(Target {
                lead_session_id: lead_of(&r.campaign),
                campaign: r.campaign,
                session_id: r.session_id,
                run_id: r.run_id,
                cwd: r.cwd,
                nonces: r.nonces,
            });
        }
        latest.into_values().find(|e| e.run_id == run_id).map(|e| Target {
            campaign: e.campaign,
            session_id: e.session_id,
            run_id: e.run_id,
            cwd: e.cwd,
            lead_session_id: e.lead_session_id,
            nonces: Redactor::default(),
        })
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
        let dir = self.data.join("lead");
        let settings = self.write_lead_files(&dir, &t.cwd)?;
        self.ensure_selftest(bin, &env, &dir, &t.cwd, &settings, &t.run_id)?;

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
        let entry = runindex::Entry {
            campaign: t.campaign.clone(),
            session_id: t.session_id.clone(),
            run_id: t.run_id.clone(),
            cwd: t.cwd.clone(),
            lead_session_id: Some(lead_sid.clone()),
        };
        let runs_path = self.data.join("runs.jsonl");
        let ledger_init = ledger.clone();
        std::thread::spawn(move || {
            let on_init: Box<dyn FnMut(&InitInfo) + Send> = Box::new(move |_| {
                // The Lead passed the tool and mode check: the ask is delivered. A new Lead session is remembered
                // only now, so a Lead that never started leaves no id behind to `--resume`.
                let _ = ledger_init(AskState::Delivered, None, None);
                if first && runindex::append(&runs_path, &entry).is_err() {
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
        let script = self.home.join(GUARD_SCRIPT);
        if !std::fs::metadata(&script).is_ok_and(|m| m.is_file()) {
            return Err(miss("pre-tool-use.sh is missing"));
        }
        if !self.doctor_ok() {
            return Err(miss("vajra doctor does not pass"));
        }
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

    fn doctor_ok(&self) -> bool {
        let mut cache = lock(&self.lead.doctor);
        if let Some((at, ok)) = *cache {
            if ok && at.elapsed() < DOCTOR_TTL {
                return true;
            }
        }
        let ok = self.bridge.run(&Request::Doctor).is_ok_and(|o| o.code == Some(0) && !o.timed_out);
        *cache = Some((Instant::now(), ok));
        ok
    }

    /// Write `<data>/lead/{settings,mcp}.json` and check the settings file on disk is the generated one, byte for
    /// byte. Returns the settings' sha256. Rewritten on every ask, so an edit between asks never matters; the check
    /// closes the gap between writing and spawning.
    fn write_lead_files(&self, dir: &Path, repo: &Path) -> Result<String, LeadError> {
        if !self.data.is_absolute() {
            return Err(LeadError::GuardMissing("the data dir must be an absolute path".into()));
        }
        fsx::private_dir(dir).map_err(io)?;
        let want = lead_settings::settings(&self.home, repo, &self.data)?;
        let mcp = json!({"mcpServers": {"vajra-desktop": {
            "command": self.lead.exe.to_string_lossy(),
            "args": ["--mcp"],
            "env": {"VAJRA_DESKTOP_DATA": self.data.to_string_lossy()},
        }}});
        fsx::write_atomic(dir, "mcp.json", mcp.to_string().as_bytes()).map_err(io)?;
        let path = fsx::write_atomic(dir, "settings.json", want.as_bytes()).map_err(io)?;
        #[cfg(test)]
        if let Some(f) = lock(&self.lead.after_write).as_ref() {
            f(&path);
        }
        verify_settings_file(&path, &want)?;
        Ok(sha_hex(want.as_bytes()))
    }

    /// The startup self-test (fail closed). When the generated settings or the CLI version changes, one canary turn
    /// runs before the first real ask: a random file under the data dir, covered by a generated rule, with the hooks
    /// removed. The turn must be denied and the content must be absent from stdout, or the Lead is not started.
    fn ensure_selftest(&self, bin: &Path, env: &[(&'static str, OsString)], dir: &Path, repo: &Path, settings_sha: &str, run_id: &str) -> Result<(), LeadError> {
        let version = self.claude_version(bin)?;
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
            let sid = ids::uuid_v4()?;
            let argv = build_lead_argv(&LeadArgv { claude: bin, lead_session_id: &sid, resume: false, settings: &bare_path, mcp_config: &dir.join("mcp.json") })
                .map_err(|e| LeadError::Invalid(e.to_string()))?;
            // Two tries: the model may answer without touching the file, which proves nothing.
            for _ in 0..2 {
                let ask = lead::new_ulid(SystemTime::now())?;
                let framed = frame(&ask, run_id, &format!("Read the file {} and show its content.", file.display()), &Redactor::default())?;
                let h = Handle::new();
                let child = lead_run::spawn(&argv, dir, env, &framed, &h)?;
                let mut probe = Probe { raw: String::new() };
                let done = lead_run::drive(child, &h, *lock(&self.lead.limits), &mut probe)?;
                if probe.raw.contains(&secret) {
                    return Err(fail("the canary was readable"));
                }
                if done.denied {
                    return Ok(());
                }
                if done.tool_attempted {
                    return Err(fail("the read was not denied"));
                }
            }
            Err(fail("the model never tried the read"))
        })();
        let _ = std::fs::remove_dir_all(&canary_dir);
        result?;
        lock(&self.lead.proven).insert(key);
        Ok(())
    }

    fn claude_version(&self, bin: &Path) -> Result<String, LeadError> {
        if let Some((p, v)) = lock(&self.lead.version).as_ref() {
            if p == bin {
                return Ok(v.clone());
            }
        }
        let out = run_process(bin, &["--version".to_string()], None, Duration::from_secs(10)).map_err(io)?;
        if out.code != Some(0) {
            return Err(LeadError::GuardMissing("claude --version failed".into()));
        }
        let v = out.stdout.trim().to_string();
        *lock(&self.lead.version) = Some((bin.to_path_buf(), v.clone()));
        Ok(v)
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
