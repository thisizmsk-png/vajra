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
        let real = self.resolve(path)?;
        self.open_resolved(&real)
    }

    /// Open an already-resolved path, then check what was actually opened. `resolve()` validated a path *name*; by
    /// the time the file is opened a component may have been swapped for a symlink (a worker can write under
    /// `~/.claude/projects`), so the name is resolved again by the kernel. So: the final component is never followed,
    /// nothing blocks on a FIFO, the open descriptor is asked which path it really is (that path must be allowlisted),
    /// and the file itself must be a plain single-link regular file that is not the protected key (a hard link to the
    /// key placed inside an allowed directory has an allowed *name* but the key's data).
    pub fn open_resolved(&self, real: &Path) -> Result<File, Denied> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(real)
            .map_err(Denied::Io)?;
        let at = fd_path(&file).map_err(Denied::Io)?; // fails closed where the OS cannot say
        if at.file_name().is_some_and(|n| n == DENIED_NAME) {
            return Err(Denied::SecretKey);
        }
        if !self.permits(&at) {
            return Err(Denied::NotAllowed);
        }
        let md = file.metadata().map_err(Denied::Io)?;
        if !md.is_file() || md.nlink() != 1 {
            return Err(Denied::NotAllowed);
        }
        if let Ok(key) = std::fs::symlink_metadata(self.claude.join("vajra").join(DENIED_NAME))
            && (key.dev(), key.ino()) == (md.dev(), md.ino())
        {
            return Err(Denied::SecretKey);
        }
        // Reads are ordinary blocking reads from here on.
        // SAFETY: F_GETFL/F_SETFL on a descriptor this function owns.
        unsafe {
            let fl = libc::fcntl(file.as_raw_fd(), libc::F_GETFL);
            if fl == -1 || libc::fcntl(file.as_raw_fd(), libc::F_SETFL, fl & !libc::O_NONBLOCK) == -1 {
                return Err(Denied::Io(io::Error::last_os_error()));
            }
        }
        Ok(file)
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

/// The path an open descriptor refers to, as the OS reports it (not the name it was opened by).
#[cfg(target_os = "macos")]
fn fd_path(f: &File) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let mut buf = [0u8; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes a NUL-terminated path of at most PATH_MAX bytes into `buf`.
    let rc = unsafe { libc::fcntl(f.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) };
    if rc == -1 {
        return Err(io::Error::last_os_error());
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..n])))
}

#[cfg(target_os = "linux")]
fn fd_path(f: &File) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd()))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn fd_path(_: &File) -> io::Result<PathBuf> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "cannot verify an open descriptor on this platform"))
}
