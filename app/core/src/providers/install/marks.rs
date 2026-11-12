//! Broken marks (bhishma condition 1). An install whose result failed the closure check, or whose rollback could not be
//! confirmed, marks the harness Broken in `<data>/setup_state.json`. `--version` working is not enough to clear it: only
//! a passing closure check (`clear`) or a different canonical binary or version does.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE: &str = "setup_state.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Mark {
    path: String,
    version: Option<String>,
    reason: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    broken: BTreeMap<String, Mark>,
}

#[derive(Debug, Clone)]
pub struct Marks {
    file: Option<PathBuf>,
}

fn canon(p: &Path) -> String {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().into_owned()
}

impl Marks {
    /// The marks in a data dir.
    pub fn at(data: &Path) -> Self {
        Self { file: Some(data.join(FILE)) }
    }

    /// No marks (callers without a data dir).
    pub fn none() -> Self {
        Self { file: None }
    }

    fn read(&self) -> State {
        self.file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn write(f: &Path, s: &State) -> std::io::Result<()> {
        // A temp name of its own per write, so no writer unlinks another's (the lock already orders them).
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = f.with_extension(format!("json.{}.{n}.tmp", std::process::id()));
        use std::os::unix::fs::OpenOptionsExt;
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&tmp)?;
        let r = out
            .write_all(&serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?)
            .and_then(|_| out.sync_all())
            .and_then(|_| std::fs::rename(&tmp, f));
        if r.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        r
    }

    /// One read-modify-write of the marks, under an exclusive `flock` on `setup_state.lock` next to them, so concurrent
    /// runs (other harnesses, other `Marks`, other processes) never lose each other's marks. `change` returns whether it
    /// changed anything; only then is the file rewritten. `T` is what `change` reports back.
    fn update<T>(&self, change: impl FnOnce(&mut State) -> (bool, T)) -> std::io::Result<T> {
        let Some(f) = &self.file else { return Ok(change(&mut State::default()).1) };
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(f.with_extension("lock"))?;
        // SAFETY: a valid open fd; the lock is released when `lock` is dropped (closed) at the end of this function.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut s = self.read();
        let (changed, out) = change(&mut s);
        if changed {
            Self::write(f, &s)?;
        }
        Ok(out)
    }

    /// Mark `id` Broken for the binary at `path` reporting `version`.
    pub fn set(&self, id: &str, path: &Path, version: Option<&str>, reason: &str) -> std::io::Result<()> {
        let mark = Mark { path: canon(path), version: version.map(str::to_string), reason: reason.into() };
        self.update(|s| (s.broken.insert(id.into(), mark.clone()).as_ref() != Some(&mark), ()))
    }

    /// A passing closure check: `id` is not Broken.
    pub fn clear(&self, id: &str) -> std::io::Result<()> {
        self.update(|s| (s.broken.remove(id).is_some(), ()))
    }

    /// The reason `id` is Broken, whatever its path or version, never clearing it (bhishma C9: the engine's mark ends
    /// only with a passing engine install, not because someone edited `package.json`).
    pub fn any(&self, id: &str) -> Option<String> {
        self.read().broken.get(id).map(|m| m.reason.clone())
    }

    /// The reason `id` is still Broken, when the binary at `path` and its `version` are the ones that were marked. A
    /// different binary or version clears the mark for good. The check and the clear are one locked step, so a mark
    /// set meanwhile for the new binary is never cleared by a stale comparison.
    pub fn active(&self, id: &str, path: &Path, version: Option<&str>) -> Option<String> {
        self.read().broken.get(id)?;
        let here = canon(path);
        self.update(|s| match s.broken.get(id) {
            None => (false, None),
            Some(m) if m.path == here && m.version.as_deref() == version => (false, Some(m.reason.clone())),
            Some(_) => (s.broken.remove(id).is_some(), None),
        })
        // The lock could not be taken: fail closed. A mark that exists still counts, and nothing is cleared.
        .unwrap_or_else(|_| self.read().broken.get(id).map(|m| m.reason.clone()))
    }
}

#[cfg(test)]
#[path = "marks_tests.rs"]
mod tests;
