//! Canary A's local half: a Claude Code build that is not on the built-in proven list (`profiles::CLAUDE_WORKER_PROVEN`)
//! is self-tested once on this Mac (`adapter::claude_selftest`). Only a PASS adds its (version, flag shape) pair to
//! this machine's own list, `<data>/control/claude-proven.json`; a failure is remembered for `RETRY_AFTER`, so a
//! failing build is not re-tested on every dispatch. One self-test runs at a time. The list is in the data dir, which
//! every worker's deny rules and the guard protect, so a worker can't add itself to it.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::adapter::{claude_selftest, profiles};

/// How long a failed self-test keeps the build refused before it is tried again.
pub const RETRY_AFTER: Duration = Duration::from_secs(600);

/// The failures one data dir's self-tests remember: (version, shape) -> when. Its mutex is that data dir's
/// one-at-a-time lock (the app has one data dir; tests run many side by side, and don't queue on each other).
type Failures = HashMap<(String, String), Instant>;

fn state(data: &Path) -> Arc<Mutex<Failures>> {
    static S: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<Failures>>>>> = OnceLock::new();
    crate::lock(S.get_or_init(Mutex::default)).entry(data.to_path_buf()).or_default().clone()
}

fn list_path(data: &Path) -> PathBuf {
    data.join("control").join("claude-proven.json")
}

/// The list's largest size: a few pairs are a few hundred bytes.
const LIST_MAX: u64 = 64 * 1024;

/// The list's bytes, read once from one descriptor (plan #215 L2): opened with O_NOFOLLOW (a link fails) and
/// O_NONBLOCK (a FIFO can't stall a dispatch), then checked on that same descriptor, so nothing can be swapped in
/// between the check and the read (plan #215 L1): a regular file, one link (a hard link is a second name another folder
/// could hold), owned by this user, and writable by no one else. `Ok(None)` when there's no list. `Err` says in plain
/// words why the list is ignored, which then counts as empty.
fn read_list(p: &Path) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let f = match std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(p) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("the list of Claude Code builds proven on this Mac is a link or can't be opened, so it is ignored".into()),
    };
    let m = f.metadata().map_err(|_| "the list of Claude Code builds proven on this Mac can't be checked, so it is ignored".to_string())?;
    // SAFETY: getuid has no preconditions.
    let me = unsafe { libc::getuid() };
    let why = if !m.is_file() {
        Some("isn't a regular file")
    } else if m.nlink() != 1 {
        Some("has a second name (a hard link)")
    } else if m.uid() != me {
        Some("belongs to another user")
    } else if m.mode() & 0o022 != 0 {
        Some("can be changed by other users")
    } else if m.len() > LIST_MAX {
        Some("is too large")
    } else {
        None
    };
    if let Some(why) = why {
        return Err(format!("the list of Claude Code builds proven on this Mac {why}, so it is ignored and those builds are self-tested again"));
    }
    let mut buf = Vec::new();
    f.take(LIST_MAX + 1).read_to_end(&mut buf).map_err(|_| "the list of Claude Code builds proven on this Mac can't be read, so it is ignored".to_string())?;
    Ok(Some(buf))
}

/// The pairs this Mac's self-tests proved, the current self-test only (`claude_selftest::VERSION`). A pair an older,
/// weaker test recorded, or one with no test number at all, isn't proof: that build is self-tested again. A missing,
/// odd or unsafe file (`read_list`) is an empty list (fail closed); an unsafe one is reported once per reason.
fn local_pairs(data: &Path) -> Vec<(String, String)> {
    let bytes = match read_list(&list_path(data)) {
        Ok(Some(b)) => b,
        Ok(None) => return Vec::new(),
        Err(why) => {
            static SAID: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
            if crate::lock(SAID.get_or_init(Mutex::default)).insert(why.clone()) {
                eprintln!("vajra: {why}");
            }
            return Vec::new();
        }
    };
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let current = |p: &Value| p["test"].as_u64() == Some(u64::from(claude_selftest::VERSION));
    v["pairs"].as_array().into_iter().flatten().filter(|p| current(p)).filter_map(|p| Some((p["version"].as_str()?.to_string(), p["shape"].as_str()?.to_string()))).collect()
}

#[cfg(test)]
#[path = "claude_canary_tests.rs"]
mod tests;

/// Records a pass of the current self-test. Pairs an older test recorded are dropped as the list is rewritten.
fn add_pair(data: &Path, version: &str, shape: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = data.join("control");
    crate::control::fsx::private_dir(&dir)?;
    let mut pairs = local_pairs(data);
    pairs.push((version.to_string(), shape.to_string()));
    let body = json!({"pairs": pairs.iter().map(|(v, s)| json!({"version": v, "shape": s, "test": claude_selftest::VERSION})).collect::<Vec<_>>()});
    let tmp = dir.join(format!("claude-proven.json.tmp-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut f = crate::control::fsx::create_new(&tmp)?;
    f.write_all(body.to_string().as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, list_path(data))
}

/// Whether `version` with today's `shape` is proven here: on this Mac's list, or by a self-test run now. `probe` asks
/// the binary for its version again; a PASS is recorded only if it still says `version` (#195 bhishma: an update in
/// place during the test would otherwise record the old version for a test of the new binary).
pub(super) fn proven_here(claude: &Path, home: &Path, data: &Path, version: &str, shape: &str, probe: &dyn Fn() -> Option<String>) -> Result<(), &'static str> {
    let refused = profiles::CLAUDE_SELFTEST_FAILED.reason;
    let have = |data: &Path| local_pairs(data).iter().any(|(v, s)| v == version && s == shape);
    if have(data) {
        return Ok(());
    }
    // One at a time: a dispatch that waited here finds the pair the one before it proved.
    let state = state(data);
    let mut failed = crate::lock(&state);
    if have(data) {
        return Ok(());
    }
    let key = (version.to_string(), shape.to_string());
    if failed.get(&key).is_some_and(|at| at.elapsed() < RETRY_AFTER) {
        return Err(refused);
    }
    match claude_selftest::run(claude, home, shape, claude_selftest::TIMEOUT) {
        Ok(()) if probe().as_deref() != Some(version) => {
            // The binary changed under the test: record nothing, remember nothing; the next dispatch tests it afresh.
            eprintln!("vajra: Claude Code changed during its worker self-test; Claude workers stay off for now");
            Err(refused)
        }
        Ok(()) => {
            failed.remove(&key);
            add_pair(data, version, shape).map_err(|_| refused)
        }
        Err(why) => {
            eprintln!("vajra: Claude Code {version} failed the worker self-test ({why}); Claude workers stay off");
            failed.insert(key, Instant::now());
            Err(refused)
        }
    }
}
