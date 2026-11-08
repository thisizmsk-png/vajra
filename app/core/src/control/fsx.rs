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

/// A directory opened with O_DIRECTORY | O_NOFOLLOW, so a symlink in its place (a worker can swap one in) is an
/// `InvalidInput` error. The entries are read from this descriptor (fdopendir), and removals go through it
/// (unlinkat), never through the path again: a symlink swapped in after the open is not followed.
#[derive(Debug)]
pub struct DirHandle(File);

impl DirHandle {
    pub fn open(path: &Path) -> io::Result<Self> {
        OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW).open(path).map(Self).map_err(|e| {
            if e.raw_os_error() == Some(libc::ELOOP) || e.raw_os_error() == Some(libc::ENOTDIR) {
                io::Error::new(io::ErrorKind::InvalidInput, "control subdirectory is not a directory")
            } else {
                e
            }
        })
    }

    /// The entry names (UTF-8 only, no `.` or `..`), read from the held descriptor.
    pub fn names(&self) -> io::Result<Vec<String>> {
        use std::os::fd::AsRawFd;
        // SAFETY: dup gives fdopendir its own descriptor to own; the DIR is closed before returning.
        let fd = unsafe { libc::dup(self.0.as_raw_fd()) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let dir = unsafe { libc::fdopendir(fd) };
        if dir.is_null() {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        unsafe { libc::rewinddir(dir) };
        let mut out = Vec::new();
        let result = loop {
            unsafe { *errno_location() = 0 };
            let ent = unsafe { libc::readdir(dir) };
            if ent.is_null() {
                let code = unsafe { *errno_location() };
                break if code == 0 { Ok(()) } else { Err(io::Error::from_raw_os_error(code)) };
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()) };
            if let Ok(n) = name.to_str() {
                if n != "." && n != ".." {
                    out.push(n.to_string());
                }
            }
        };
        unsafe { libc::closedir(dir) };
        result.map(|()| out)
    }

    /// Remove a non-directory entry by name, relative to the held descriptor.
    pub fn unlink(&self, name: &str) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        let c = std::ffi::CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        if unsafe { libc::unlinkat(self.0.as_raw_fd(), c.as_ptr(), 0) } == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
}

#[cfg(target_os = "macos")]
unsafe fn errno_location() -> *mut libc::c_int {
    unsafe { libc::__error() }
}
#[cfg(not(target_os = "macos"))]
unsafe fn errno_location() -> *mut libc::c_int {
    unsafe { libc::__errno_location() }
}

/// Read a regular file without following a symlink.
pub fn read_nofollow(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let mut f = open_regular(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

/// True when the file is empty or its last byte is a newline (a crash mid-append leaves neither).
pub fn ends_with_newline(path: &Path) -> io::Result<bool> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = open_regular(path)?;
    if f.metadata()?.len() == 0 {
        return Ok(true);
    }
    f.seek(SeekFrom::End(-1))?;
    let mut b = [0u8; 1];
    f.read_exact(&mut b)?;
    Ok(b[0] == b'\n')
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

#[cfg(test)]
mod dir_handle_tests {
    use super::*;

    /// The entries come from the directory that was checked and opened, not from the path afterwards: swap the
    /// directory for a symlink to another one after the open and the handle still lists the original.
    #[test]
    fn names_are_read_from_the_opened_directory_not_the_path() {
        let base = std::env::temp_dir().join(format!("vajra-dirhandle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (real, other) = (base.join("real"), base.join("other"));
        fs::create_dir_all(&real).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(real.join("mine.json"), "").unwrap();
        fs::write(other.join("theirs.json"), "").unwrap();
        let h = DirHandle::open(&real).unwrap();
        fs::rename(&real, base.join("moved")).unwrap();
        std::os::unix::fs::symlink(&other, &real).unwrap();
        assert_eq!(h.names().unwrap(), vec!["mine.json".to_string()]);
        assert_eq!(DirHandle::open(&real).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        h.unlink("mine.json").unwrap();
        assert!(base.join("moved").read_dir().unwrap().next().is_none(), "the unlink went through the descriptor");
        assert!(other.join("theirs.json").exists());
        let _ = fs::remove_dir_all(&base);
    }
}
