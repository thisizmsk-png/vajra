//! Owner-only file primitives for the control dirs: dirs 0700, files 0600,
//! never following a symlink at the final path component.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Create `dir` (and parents) and set it to 0700. A symlink in its place is
/// refused instead of followed.
pub fn private_dir(dir: &Path) -> io::Result<()> {
    if let Ok(m) = fs::symlink_metadata(dir)
        && !m.is_dir()
    {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "control dir path is not a directory"));
    }
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

/// O_CREAT | O_EXCL | O_NOFOLLOW, mode 0600, write-only. Fails if anything
/// (file or symlink) already exists at `path`.
pub fn create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true) // O_CREAT | O_EXCL
        .custom_flags(libc::O_NOFOLLOW)
        .mode(0o600)
        .open(path)
}

/// Open for append (creating at 0600), never through a symlink, and only a
/// regular file. Existing files are tightened to 0600.
pub fn open_append(path: &Path) -> io::Result<File> {
    let f = OpenOptions::new()
        .append(true)
        .create(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(0o600)
        .open(path)?;
    let m = f.metadata()?;
    if !m.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
    }
    if m.permissions().mode() & 0o777 != 0o600 {
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(f)
}

/// Open for reading without following a symlink and without blocking, then require a regular file on the descriptor
/// itself. O_NONBLOCK matters: a worker (same user) can plant a FIFO in the control directory, and a blocking open of
/// it would hang the caller (the watcher, which holds the service mutex) until some writer appeared.
fn open_regular(path: &Path) -> io::Result<fs::File> {
    let f = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    if !f.metadata()?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
    }
    Ok(f)
}

/// Read a regular file without following a symlink.
pub fn read_nofollow(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let mut f = open_regular(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Like `read_nofollow`, but a file longer than `cap` bytes is an `InvalidData` error and at most `cap + 1` bytes are
/// ever read. For files a worker (same user) can write into the control directory.
pub fn read_nofollow_capped(path: &Path, cap: u64) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let f = open_regular(path)?;
    let meta = f.metadata()?;
    if meta.len() > cap {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
    }
    let mut buf = Vec::new();
    f.take(cap + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
    }
    Ok(buf)
}

/// Write `bytes` to `dir/name` atomically: a 0600 temp file created with
/// O_EXCL | O_NOFOLLOW, then rename. Readers (the hook) see all or nothing.
/// The temp name starts with a dot so the hook's `*.json` glob skips it.
pub fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let res = (|| {
        let mut f = create_new(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        let dst = dir.join(name);
        fs::rename(&tmp, &dst)?;
        Ok(dst)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}
