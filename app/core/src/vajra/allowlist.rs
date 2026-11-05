//! The one place that decides which files under `~/.claude` (and the app data
//! dir) the app may open. Design section 8.

use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Name of the key file that must never be opened. Only this module names it.
const DENIED_NAME: &str = ".hmac-key";

#[derive(Debug)]
pub enum Denied {
    /// The path is, or resolves to, the HMAC key.
    SecretKey,
    /// The raw path contains `..`.
    Traversal,
    /// The resolved path is not on the allowlist (includes symlink escapes).
    NotAllowed,
    Io(io::Error),
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Denied::SecretKey => write!(f, "denied: protected key file"),
            Denied::Traversal => write!(f, "denied: path contains .."),
            Denied::NotAllowed => write!(f, "denied: path is not on the allowlist"),
            Denied::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for Denied {}

impl From<Denied> for io::Error {
    fn from(d: Denied) -> Self {
        match d {
            Denied::Io(e) => e,
            other => io::Error::new(io::ErrorKind::PermissionDenied, other.to_string()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Allowlist {
    /// Canonical `~/.claude`.
    claude: PathBuf,
    /// Canonical app data dir.
    app_data: PathBuf,
}

impl Allowlist {
    /// `claude_dir` is `~/.claude`; both dirs must exist.
    pub fn new(claude_dir: &Path, app_data: &Path) -> io::Result<Self> {
        Ok(Self {
            claude: claude_dir.canonicalize()?,
            app_data: app_data.canonicalize()?,
        })
    }

    pub fn vajra_dir(&self) -> PathBuf {
        self.claude.join("vajra")
    }

    pub fn campaigns_dir(&self) -> PathBuf {
        self.vajra_dir().join("campaigns")
    }

    pub fn workflow_path(&self) -> PathBuf {
        self.claude.join("workflows/vajra-deliver.js")
    }

    /// Check `path` and return its canonical form. The deny list runs first,
    /// on the raw name and again on the resolved name.
    pub fn resolve(&self, path: &Path) -> Result<PathBuf, Denied> {
        if path.file_name().is_some_and(|n| n == DENIED_NAME) {
            return Err(Denied::SecretKey);
        }
        if path.components().any(|c| c == Component::ParentDir) {
            return Err(Denied::Traversal);
        }
        let real = path.canonicalize().map_err(Denied::Io)?;
        if real.file_name().is_some_and(|n| n == DENIED_NAME) {
            return Err(Denied::SecretKey);
        }
        if self.permits(&real) {
            Ok(real)
        } else {
            Err(Denied::NotAllowed)
        }
    }

    /// Open through `opener` after every check passed. Tests inject an opener
    /// to record what would have been opened.
    pub fn open_with<T>(
        &self,
        path: &Path,
        opener: impl FnOnce(&Path) -> io::Result<T>,
    ) -> Result<T, Denied> {
        let real = self.resolve(path)?;
        opener(&real).map_err(Denied::Io)
    }

    pub fn open_allowlisted(&self, path: &Path) -> Result<File, Denied> {
        self.open_with(path, |p| File::open(p))
    }

    /// `real` is canonical, so it can be matched against canonical roots.
    fn permits(&self, real: &Path) -> bool {
        if real.starts_with(&self.app_data) {
            return true;
        }
        let Ok(rel) = real.strip_prefix(&self.claude) else {
            return false;
        };
        let parts: Vec<&str> = rel.components().filter_map(|c| c.as_os_str().to_str()).collect();
        if parts.len() != rel.components().count() {
            return false; // non-UTF-8 component
        }
        match parts.as_slice() {
            ["vajra", "audit.log"] | ["vajra", "routing-log.jsonl"] => true,
            ["vajra", "campaigns"] | ["vajra", "campaigns", ".active"] => true,
            ["vajra", "campaigns", slug] => safe_name(slug),
            ["vajra", "campaigns", slug, f] => {
                safe_name(slug)
                    && matches!(
                        *f,
                        "campaign.json" | "features.json" | "tasks.json" | "progress.md" | "checkpoints"
                    )
            }
            ["vajra", "campaigns", slug, "checkpoints", name] => safe_name(slug) && safe_name(name),
            ["projects", slug, rest @ ..] => safe_name(slug) && rest.iter().all(|p| safe_name(p)),
            ["workflows", "vajra-deliver.js"] => true,
            _ => false,
        }
    }
}

fn safe_name(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s != DENIED_NAME
}
