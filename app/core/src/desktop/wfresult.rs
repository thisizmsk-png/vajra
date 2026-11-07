//! The workflow's final result (ADR-14), read from the file Claude Code writes itself:
//! `<claude>/projects/<cwd-slug>/<session_id>/workflows/wf_<id>.json`.
//!
//! That file is NOT signed and the same user (so a worker) can write it. So it is read with the held-bytes
//! discipline and used only as untrusted display data, labelled "from the workflow result":
//! the directory is resolved through the allowlist, opened as a directory fd (no symlink), the file is opened with
//! `openat(O_NOFOLLOW|O_NONBLOCK)` relative to that fd, and the *open descriptor* is checked (regular file, one link,
//! this user's, mode 0600, size cap). The JSON is parsed once from the bytes read from that descriptor. The
//! `<task-notification>` is never data (it is cut at about 8 KB) and the `/tmp/.../tasks/*.output` file is never read.

use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use serde_json::Value;

use crate::control::lead::sanitise;
use crate::crew::redact::Redactor;
use crate::vajra::allowlist::Allowlist;

/// Largest result file read. A real `vajra-deliver` result is a few KB; anything bigger is refused, not truncated.
pub const MAX_BYTES: u64 = 256 * 1024;
/// Files whose contents are read: the newest this many (by mtime) at or after the run's launch.
const MAX_FILES: usize = 200;
/// Directory entries examined. More than this is refused (nothing actionable) instead of trusting a partial view.
const MAX_NAMES: usize = 4096;
const TEXT_MAX: usize = 500;
/// `ApprovedPlanTurn::MAX_INDEX + 1`: a result with more entries is refused whole, never partly shown.
const MAX_PLANS: usize = crate::sessions::ApprovedPlanTurn::MAX_INDEX + 1;
/// A plan entry bigger than this (as written) is shown in full but cannot be approved.
const PLAN_MAX_BYTES: usize = 64 * 1024;
/// `S.Action` of `workflows/vajra-deliver.js`.
const ACTIONS: [&str; 9] = ["push", "pull_request", "deploy", "migration", "data_deletion", "secrets", "spend", "external_message", "other"];

#[derive(Debug, PartialEq, Eq)]
pub enum WfError {
    NotAllowed,
    Missing,
    /// Not a regular file, more than one link, not ours, not 0600, or over the cap.
    Unsafe(&'static str),
    Io,
}

/// What a workflow result says is waiting on the human. Every string is plain text (controls dropped, capped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub campaign: String,
    pub modified: i64,
    pub question: Option<String>,
    /// One per `pending_approval[]` entry, keeping the entry's own index in the workflow result.
    pub plans: Vec<Plan>,
}

/// One `pending_approval[]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Position in the workflow's `pending_approval[]`. Ids and the delivered turn use this, never a filtered position.
    pub index: usize,
    pub action: String,
    /// The entry exactly as the workflow wrote it. This is what is hashed (`plan_sha256`).
    pub raw: Value,
    /// The same entry with every control, bidi and invisible character shown as a visible `\u{..}` escape. Nothing is
    /// shortened or dropped, so what is shown and what is hashed describe the same entry.
    pub shown: Value,
    /// Why this entry cannot be approved (it can still be rejected), if so.
    pub invalid: Option<&'static str>,
}

fn open_dir(real: &Path) -> Result<OwnedFd, WfError> {
    let c = CString::new(real.as_os_str().as_bytes()).map_err(|_| WfError::Io)?;
    // SAFETY: `c` is a valid NUL-terminated path; the returned fd is owned by `OwnedFd`.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(WfError::Io);
    }
    // SAFETY: `fd` is a fresh descriptor we own.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn me() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

/// What `fstat` said about the open descriptor.
pub struct Facts {
    pub is_file: bool,
    pub nlink: u64,
    pub uid: u32,
    pub mode: u32,
    pub len: u64,
}

/// Why a result file may not be read, or `None` when it is a plain, single-link, 0600 file of ours within the cap.
pub fn unsafe_reason(f: &Facts, me: u32) -> Option<&'static str> {
    if !f.is_file || f.nlink != 1 {
        Some("not a plain file")
    } else if f.uid != me {
        Some("owned by another user")
    } else if f.mode & 0o777 != 0o600 {
        Some("mode is not 0600")
    } else if f.len > MAX_BYTES {
        Some("file is too large")
    } else {
        None
    }
}

/// Open `name` inside the held directory and check the descriptor itself (plain file, one link, ours, 0600, within the cap).
fn open_checked(dir: &OwnedFd, name: &str) -> Result<(File, i64), WfError> {
    let c = CString::new(name).map_err(|_| WfError::Io)?;
    // SAFETY: `dir` is a live directory fd and `c` a valid NUL-terminated relative name.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), c.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound { WfError::Missing } else { WfError::Unsafe("cannot open (a symlink?)") });
    }
    // SAFETY: `fd` is a fresh descriptor we own.
    let f = unsafe { File::from_raw_fd(fd) };
    let m = f.metadata().map_err(|_| WfError::Io)?;
    unsafe_reason(&Facts { is_file: m.is_file(), nlink: m.nlink(), uid: m.uid(), mode: m.mode(), len: m.len() }, me()).map_or(Ok(()), |r| Err(WfError::Unsafe(r)))?;
    Ok((f, m.mtime()))
}

/// The checked file's bytes plus mtime.
fn read_held(dir: &OwnedFd, name: &str) -> Result<(Vec<u8>, i64), WfError> {
    let (mut f, mtime) = open_checked(dir, name)?;
    let mut buf = Vec::new();
    f.by_ref().take(MAX_BYTES + 1).read_to_end(&mut buf).map_err(|_| WfError::Io)?;
    if buf.len() as u64 > MAX_BYTES {
        return Err(WfError::Unsafe("file is too large"));
    }
    Ok((buf, mtime))
}

fn plain(v: Option<&Value>, cap: usize) -> String {
    sanitise(v.and_then(Value::as_str).unwrap_or_default(), &Redactor::default()).chars().take(cap).collect()
}

/// Make unsafe characters visible instead of dropping them.
fn visible(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let hidden = c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{feff}' | '\u{2028}' | '\u{2029}');
        if hidden {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

fn show(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(visible(s)),
        Value::Array(a) => Value::Array(a.iter().map(show).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (visible(k), show(x))).collect()),
        other => other.clone(),
    }
}

fn strings(v: Option<&Value>) -> bool {
    v.and_then(Value::as_array).is_some_and(|a| a.iter().all(Value::is_string))
}

/// `S.DeployPlan`: target, commands[], rollback[], health_checks[], risk.
fn deploy_plan_ok(p: Option<&Value>) -> bool {
    let Some(p) = p else { return false };
    p.get("target").is_some_and(Value::is_string)
        && strings(p.get("commands"))
        && strings(p.get("rollback"))
        && strings(p.get("health_checks"))
        && matches!(p.get("risk").and_then(Value::as_str), Some("low" | "medium" | "high"))
}

fn plan_entry(index: usize, p: &Value) -> Plan {
    let action = p.get("action").and_then(Value::as_str).unwrap_or_default().to_string();
    let invalid = if !p.is_object() || !ACTIONS.contains(&action.as_str()) {
        Some("not a known action")
    } else if !p.get("why").is_some_and(Value::is_string) {
        Some("has no reason")
    } else if action == "deploy" && !deploy_plan_ok(p.get("plan")) {
        Some("a deploy needs a complete plan (target, commands, rollback, health checks, risk)")
    } else if serde_json::to_vec(p).map_or(true, |b| b.len() > PLAN_MAX_BYTES) {
        Some("too large to review")
    } else {
        None
    };
    Plan { index, action, raw: p.clone(), shown: show(p), invalid }
}

/// One `vajra-deliver` result file, read.
struct Env {
    modified: i64,
    /// The harness says the workflow finished (`status: "completed"`) and the script returned an object.
    terminal: bool,
    result: Option<Value>,
}

fn envelope(bytes: &[u8], modified: i64) -> Option<Env> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    if v.get("workflowName").and_then(Value::as_str) != Some("vajra-deliver") {
        return None;
    }
    let result = v.get("result").filter(|r| r.is_object()).cloned();
    Some(Env { modified, terminal: v.get("status").and_then(Value::as_str) == Some("completed") && result.is_some(), result })
}

/// What a finished result asks of the human. The campaign comes from the run's guarded record, not from the result:
/// the early `need_input` returns of the workflow carry no campaign slug (`artifacts.understanding`, or the campaign
/// state object). If the result does name a slug, it must be this campaign.
fn waiting(env: &Env, campaign: &str) -> Option<Waiting> {
    let r = env.result.as_ref()?;
    match r.get("artifacts").and_then(|a| a.get("campaign")) {
        Some(Value::String(c)) if c != campaign => return None,
        None | Some(Value::String(_)) | Some(Value::Object(_)) | Some(Value::Null) => {}
        Some(_) => return None,
    }
    let (question, plans) = match r.get("status").and_then(Value::as_str)? {
        "need_input" => (Some(plain(r.get("question"), TEXT_MAX)), Vec::new()),
        "done" => {
            // A finished delivery always names its campaign.
            r.get("artifacts").and_then(|a| a.get("campaign")).and_then(Value::as_str)?;
            let entries = r.get("pending_approval").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            if entries.len() > MAX_PLANS {
                return None;
            }
            (None, entries.iter().enumerate().map(|(i, p)| plan_entry(i, p)).collect())
        }
        _ => return None,
    };
    Some(Waiting { campaign: campaign.to_string(), modified: env.modified, question, plans })
}

/// What the CURRENT `vajra-deliver` of a session's `workflows/` directory asks of the human. Result files older than
/// the run's actual launch time (`since`, unix seconds, from the guarded run record; never derived from the run id) belong to earlier workflows and are ignored. Of the rest the newest one is the
/// current workflow: if it is still running, or failed, nothing is actionable, even when an older file holds pending
/// approvals (fail closed while the workflow is not terminal).
pub fn waiting_for(allow: &Allowlist, dir: &Path, campaign: &str, since: i64) -> Result<Option<Waiting>, WfError> {
    let real = allow.resolve(dir).map_err(|_| WfError::NotAllowed)?;
    let held = open_dir(&real)?;
    let d = File::from(held.try_clone().map_err(|_| WfError::Io)?);
    if d.metadata().map_err(|_| WfError::Io)?.uid() != me() {
        return Err(WfError::Unsafe("directory owned by another user"));
    }
    let names: Vec<String> = std::fs::read_dir(&real)
        .map_err(|_| WfError::Io)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("wf_") && n.ends_with(".json") && n.len() <= 80)
        .take(MAX_NAMES + 1)
        .collect();
    // `read_dir` has no order, so every candidate's mtime is looked at (cheaply, without reading) before choosing which
    // to read. A directory too big to examine whole is refused: an incomplete view could offer an old approval.
    if names.len() > MAX_NAMES {
        return Ok(None);
    }
    let mut dated: Vec<(i64, String)> = names
        .into_iter()
        // An unsafe or unreadable file is skipped, never trusted.
        .filter_map(|n| open_checked(&held, &n).ok().map(|(_, mtime)| (mtime, n)))
        .filter(|(mtime, _)| *mtime >= since)
        .collect();
    dated.sort_by(|a, b| b.cmp(a));
    let mut files: Vec<Env> = Vec::new();
    for (_, n) in dated.into_iter().take(MAX_FILES) {
        let Ok((bytes, mtime)) = read_held(&held, &n) else { continue };
        if let Some(env) = envelope(&bytes, mtime).filter(|e| e.modified >= since) {
            files.push(env);
        }
    }
    let Some(newest) = files.iter().map(|e| e.modified).max() else { return Ok(None) };
    // Equal times: any file that is not finished makes the whole tie not actionable.
    let tied: Vec<&Env> = files.iter().filter(|e| e.modified == newest).collect();
    if tied.iter().any(|e| !e.terminal) {
        return Ok(None);
    }
    Ok(tied.iter().find_map(|e| waiting(e, campaign)))
}

/// Strict read of one named file, for tests and the refusal contract: any unsafe file is an error, not a skip.
#[cfg(test)]
pub fn read_one(allow: &Allowlist, dir: &Path, name: &str) -> Result<Vec<u8>, WfError> {
    let real = allow.resolve(dir).map_err(|_| WfError::NotAllowed)?;
    read_held(&open_dir(&real)?, name).map(|(b, _)| b)
}
