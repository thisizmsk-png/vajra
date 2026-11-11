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

    fn write(&self, s: &State) -> std::io::Result<()> {
        let Some(f) = &self.file else { return Ok(()) };
        let tmp = f.with_extension("json.tmp");
        let _ = std::fs::remove_file(&tmp);
        use std::os::unix::fs::OpenOptionsExt;
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&tmp)?;
        out.write_all(&serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?)?;
        out.sync_all()?;
        std::fs::rename(&tmp, f)
    }

    /// Mark `id` Broken for the binary at `path` reporting `version`.
    pub fn set(&self, id: &str, path: &Path, version: Option<&str>, reason: &str) -> std::io::Result<()> {
        let mut s = self.read();
        s.broken.insert(id.into(), Mark { path: canon(path), version: version.map(str::to_string), reason: reason.into() });
        self.write(&s)
    }

    /// A passing closure check: `id` is not Broken.
    pub fn clear(&self, id: &str) -> std::io::Result<()> {
        let mut s = self.read();
        if s.broken.remove(id).is_some() {
            self.write(&s)?;
        }
        Ok(())
    }

    /// The reason `id` is still Broken, when the binary at `path` and its `version` are the ones that were marked. A
    /// different binary or version clears the mark for good.
    pub fn active(&self, id: &str, path: &Path, version: Option<&str>) -> Option<String> {
        let m = self.read().broken.get(id).cloned()?;
        if m.path == canon(path) && m.version.as_deref() == version {
            return Some(m.reason);
        }
        let _ = self.clear(id);
        None
    }
}

#[cfg(test)]
#[path = "marks_tests.rs"]
mod tests;
