//! Canary A's local half: a Claude Code build that is not on the built-in proven list (`profiles::CLAUDE_WORKER_PROVEN`)
//! is self-tested once on this Mac (`adapter::claude_selftest`). Only a PASS adds its (version, flag shape) pair to
//! this machine's own list, `<data>/control/claude-proven.json`; a failure is remembered for `RETRY_AFTER`, so a
//! failing build is not re-tested on every dispatch. One self-test runs at a time. The list is in the data dir, which
//! every worker's deny rules and the guard protect, so a worker can't add itself to it.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::adapter::{claude_selftest, profiles};

/// How long a failed self-test keeps the build refused before it is tried again.
pub const RETRY_AFTER: Duration = Duration::from_secs(600);

/// The failures the self-test remembers: (data dir, version, shape) -> when. Its mutex is the one-at-a-time lock.
type Failures = HashMap<(PathBuf, String, String), Instant>;

fn state() -> &'static Mutex<Failures> {
    static S: OnceLock<Mutex<Failures>> = OnceLock::new();
    S.get_or_init(Mutex::default)
}

fn list_path(data: &Path) -> PathBuf {
    data.join("control").join("claude-proven.json")
}

/// The pairs this Mac's self-tests proved. A missing, odd or non-regular file is an empty list (fail closed).
fn local_pairs(data: &Path) -> Vec<(String, String)> {
    let p = list_path(data);
    if !std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) {
        return Vec::new();
    }
    let v: Value = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    v["pairs"].as_array().into_iter().flatten().filter_map(|p| Some((p["version"].as_str()?.to_string(), p["shape"].as_str()?.to_string()))).collect()
}

fn add_pair(data: &Path, version: &str, shape: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = data.join("control");
    crate::control::fsx::private_dir(&dir)?;
    let mut pairs = local_pairs(data);
    pairs.push((version.to_string(), shape.to_string()));
    let body = json!({"pairs": pairs.iter().map(|(v, s)| json!({"version": v, "shape": s})).collect::<Vec<_>>()});
    let tmp = dir.join(format!("claude-proven.json.tmp-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut f = crate::control::fsx::create_new(&tmp)?;
    f.write_all(body.to_string().as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, list_path(data))
}

/// Whether `version` with today's `shape` is proven here: on this Mac's list, or by a self-test run now.
pub(super) fn proven_here(claude: &Path, home: &Path, data: &Path, version: &str, shape: &str) -> Result<(), &'static str> {
    let refused = profiles::CLAUDE_SELFTEST_FAILED.reason;
    let have = |data: &Path| local_pairs(data).iter().any(|(v, s)| v == version && s == shape);
    if have(data) {
        return Ok(());
    }
    // One at a time: a dispatch that waited here finds the pair the one before it proved.
    let mut failed = crate::lock(state());
    if have(data) {
        return Ok(());
    }
    let key = (data.to_path_buf(), version.to_string(), shape.to_string());
    if failed.get(&key).is_some_and(|at| at.elapsed() < RETRY_AFTER) {
        return Err(refused);
    }
    match claude_selftest::run(claude, home, claude_selftest::TIMEOUT) {
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
