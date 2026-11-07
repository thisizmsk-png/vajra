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

use serde_json::{Value, json};

use crate::control::lead::sanitise;
use crate::crew::redact::Redactor;
use crate::vajra::allowlist::Allowlist;

/// Largest result file read. A real `vajra-deliver` result is a few KB; anything bigger is refused, not truncated.
pub const MAX_BYTES: u64 = 256 * 1024;
const MAX_FILES: usize = 200;
const MAX_ITEMS: usize = 20;
const WHY_MAX: usize = 300;
const TEXT_MAX: usize = 500;

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
    /// Each `{action, why[, plan]}` exactly as shown, in order.
    pub plans: Vec<Value>,
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

/// Open `name` inside the held directory and return its bytes plus mtime, after checking the descriptor itself.
fn read_held(dir: &OwnedFd, name: &str) -> Result<(Vec<u8>, i64), WfError> {
    let c = CString::new(name).map_err(|_| WfError::Io)?;
    // SAFETY: `dir` is a live directory fd and `c` a valid NUL-terminated relative name.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), c.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound { WfError::Missing } else { WfError::Unsafe("cannot open (a symlink?)") });
    }
    // SAFETY: `fd` is a fresh descriptor we own.
    let mut f = unsafe { File::from_raw_fd(fd) };
    let m = f.metadata().map_err(|_| WfError::Io)?;
    unsafe_reason(&Facts { is_file: m.is_file(), nlink: m.nlink(), uid: m.uid(), mode: m.mode(), len: m.len() }, me()).map_or(Ok(()), |r| Err(WfError::Unsafe(r)))?;
    let mut buf = Vec::new();
    f.by_ref().take(MAX_BYTES + 1).read_to_end(&mut buf).map_err(|_| WfError::Io)?;
    if buf.len() as u64 > MAX_BYTES {
        return Err(WfError::Unsafe("file is too large"));
    }
    Ok((buf, m.mtime()))
}

fn plain(v: Option<&Value>, cap: usize) -> String {
    sanitise(v.and_then(Value::as_str).unwrap_or_default(), &Redactor::default()).chars().take(cap).collect()
}

/// One `pending_approval` entry as it will be shown (and hashed). `None` for anything that is not a known action.
fn plan_item(p: &Value) -> Option<Value> {
    let action = p.get("action").and_then(Value::as_str)?;
    if !matches!(action, "push" | "pr" | "deploy") {
        return None;
    }
    let mut out = json!({"action": action, "why": plain(p.get("why"), WHY_MAX)});
    if action == "deploy" {
        let plan = p.get("plan");
        out["plan"] = json!({
            "commands": plan.and_then(|x| x.get("commands")).and_then(Value::as_array).map(|a| a.iter().take(MAX_ITEMS).map(|c| plain(Some(c), TEXT_MAX)).collect::<Vec<_>>()).unwrap_or_default(),
            "rollback": plain(plan.and_then(|x| x.get("rollback")), TEXT_MAX),
            "risk": plain(plan.and_then(|x| x.get("risk")), TEXT_MAX),
        });
    }
    Some(out)
}

/// What a `vajra-deliver` result file asks of the human, or `None` when it is some other workflow, did not complete,
/// or has nothing waiting. A failed workflow (`result: null`) never yields items.
pub fn parse(bytes: &[u8], modified: i64) -> Option<Waiting> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    if v.get("workflowName").and_then(Value::as_str) != Some("vajra-deliver") || v.get("status").and_then(Value::as_str) != Some("completed") {
        return None;
    }
    let r = v.get("result").filter(|r| r.is_object())?;
    let campaign = r.get("artifacts").and_then(|a| a.get("campaign")).and_then(Value::as_str).filter(|c| crate::bus::valid_stream_id(c))?.to_string();
    let (question, plans) = match r.get("status").and_then(Value::as_str)? {
        "need_input" => (Some(plain(r.get("question"), TEXT_MAX)), Vec::new()),
        "done" => (None, r.get("pending_approval").and_then(Value::as_array).map(|a| a.iter().filter_map(plan_item).take(MAX_ITEMS).collect()).unwrap_or_default()),
        _ => return None,
    };
    Some(Waiting { campaign, modified, question, plans })
}

/// The newest `vajra-deliver` result in `dir` (a session's `workflows/` directory) that names `campaign`.
pub fn waiting_for(allow: &Allowlist, dir: &Path, campaign: &str) -> Result<Option<Waiting>, WfError> {
    let real = allow.resolve(dir).map_err(|_| WfError::NotAllowed)?;
    let held = open_dir(&real)?;
    let d = File::from(held.try_clone().map_err(|_| WfError::Io)?);
    if d.metadata().map_err(|_| WfError::Io)?.uid() != me() {
        return Err(WfError::Unsafe("directory owned by another user"));
    }
    let mut names: Vec<String> = std::fs::read_dir(&real)
        .map_err(|_| WfError::Io)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("wf_") && n.ends_with(".json") && n.len() <= 80)
        .take(MAX_FILES)
        .collect();
    names.sort();
    let mut best: Option<Waiting> = None;
    for n in names {
        // An unsafe or unreadable file is skipped, never trusted. It cannot hide a good one.
        let Ok((bytes, mtime)) = read_held(&held, &n) else { continue };
        if let Some(w) = parse(&bytes, mtime).filter(|w| w.campaign == campaign)
            && best.as_ref().is_none_or(|b| w.modified >= b.modified)
        {
            best = Some(w);
        }
    }
    Ok(best)
}

/// Strict read of one named file, for tests and the refusal contract: any unsafe file is an error, not a skip.
#[cfg(test)]
pub fn read_one(allow: &Allowlist, dir: &Path, name: &str) -> Result<Vec<u8>, WfError> {
    let real = allow.resolve(dir).map_err(|_| WfError::NotAllowed)?;
    read_held(&open_dir(&real)?, name).map(|(b, _)| b)
}
