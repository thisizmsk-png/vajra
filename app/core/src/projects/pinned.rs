//! A project's folder, opened once and then used only through that descriptor. `projects/<id>` is opened with
//! O_DIRECTORY | O_NOFOLLOW, so a symlink in its place is refused, and every read, append and rewrite inside it is
//! `openat`/`renameat` relative to the descriptor. Swapping the folder for a symlink after the open cannot redirect
//! them, and nothing is ever created outside the folder that was opened. Files inside are opened O_NOFOLLOW |
//! O_NONBLOCK and must be regular (a planted FIFO is refused, not waited on).

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;

pub struct Pinned(OwnedFd);

fn cstr(s: &str) -> io::Result<CString> {
    CString::new(s).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "name contains NUL"))
}

fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

impl Pinned {
    /// Open the folder at `path` (never through a symlink), creating it at 0700 first when `create`.
    pub fn open(path: &Path, create: bool) -> io::Result<Self> {
        use std::os::unix::ffi::OsStrExt;
        let p = CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
        if create {
            // SAFETY: `p` is a valid NUL-terminated string.
            if unsafe { libc::mkdir(p.as_ptr(), 0o700) } < 0 {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::AlreadyExists {
                    return Err(e);
                }
            }
        }
        // SAFETY: `p` is valid; the returned descriptor is owned below.
        let fd = cvt(unsafe { libc::open(p.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) })?;
        // SAFETY: `fd` is a fresh descriptor that nothing else owns.
        let me = Self(unsafe { OwnedFd::from_raw_fd(fd) });
        if create {
            // SAFETY: the descriptor is open.
            cvt(unsafe { libc::fchmod(me.0.as_raw_fd(), 0o700) })?;
        }
        Ok(me)
    }

    /// A subfolder of this one, opened (and with `create`, made at 0700) relative to the held descriptor: this folder
    /// being renamed away or replaced by a symlink afterwards does not redirect it.
    pub fn child(&self, name: &str, create: bool) -> io::Result<Self> {
        let n = cstr(name)?;
        if create {
            // SAFETY: valid descriptor and NUL-terminated name.
            if unsafe { libc::mkdirat(self.0.as_raw_fd(), n.as_ptr(), 0o700) } < 0 {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::AlreadyExists {
                    return Err(e);
                }
            }
        }
        // SAFETY: as above; the result is owned immediately.
        let fd = cvt(unsafe { libc::openat(self.0.as_raw_fd(), n.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) })?;
        // SAFETY: `fd` is a fresh descriptor that nothing else owns.
        let me = Self(unsafe { OwnedFd::from_raw_fd(fd) });
        if create {
            // SAFETY: the descriptor is open.
            cvt(unsafe { libc::fchmod(me.0.as_raw_fd(), 0o700) })?;
        }
        Ok(me)
    }

    /// Is there an entry `name` in this folder (of any type, a link counted as itself, never followed)?
    pub fn exists(&self, name: &str) -> bool {
        let Ok(n) = cstr(name) else { return false };
        // SAFETY: valid descriptor and NUL-terminated name; `st` is written by the call before it is read.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        unsafe { libc::fstatat(self.0.as_raw_fd(), n.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) == 0 }
    }

    /// Entry names (UTF-8, no `.` or `..`), read from the held descriptor.
    pub fn names(&self) -> io::Result<Vec<String>> {
        crate::control::fsx::DirHandle::from_file(File::from(self.0.try_clone()?)).names()
    }

    /// Every entry name as bytes, UTF-8 or not (no `.` or `..`).
    pub fn raw_names(&self) -> io::Result<Vec<Vec<u8>>> {
        crate::control::fsx::DirHandle::from_file(File::from(self.0.try_clone()?)).raw_names()
    }

    fn openat(&self, name: &str, flags: libc::c_int, mode: libc::mode_t) -> io::Result<File> {
        let n = cstr(name)?;
        // SAFETY: the directory descriptor and name are valid; the result is owned immediately.
        let fd = cvt(unsafe { libc::openat(self.0.as_raw_fd(), n.as_ptr(), flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC, libc::c_uint::from(mode)) })?;
        // SAFETY: `fd` is a fresh descriptor that nothing else owns.
        let f = unsafe { File::from_raw_fd(fd) };
        if !f.metadata()?.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
        }
        Ok(f)
    }

    /// Like `read`, and the file must have exactly one link (checked on the open file): a hard link would let a name
    /// here reach a file kept somewhere else.
    pub fn read_single_link(&self, name: &str, cap: u64) -> io::Result<Vec<u8>> {
        use std::os::unix::fs::MetadataExt;
        let f = self.openat(name, libc::O_RDONLY, 0)?;
        let m = f.metadata()?;
        if m.nlink() != 1 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file has more than one link"));
        }
        if m.len() > cap {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
        }
        let mut buf = Vec::new();
        f.take(cap + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 > cap {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
        }
        Ok(buf)
    }

    /// The whole file, which must be at most `cap` bytes.
    pub fn read(&self, name: &str, cap: u64) -> io::Result<Vec<u8>> {
        let f = self.openat(name, libc::O_RDONLY, 0)?;
        if f.metadata()?.len() > cap {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
        }
        let mut buf = Vec::new();
        f.take(cap + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 > cap {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
        }
        Ok(buf)
    }

    /// The last `cap` bytes (all of it if shorter) and whether the file was longer, i.e. the start was cut off. When cut,
    /// the first line is probably partial.
    pub fn read_tail(&self, name: &str, cap: u64) -> io::Result<(Vec<u8>, bool)> {
        let mut f = self.openat(name, libc::O_RDONLY, 0)?;
        let len = f.metadata()?.len();
        f.seek(SeekFrom::Start(len.saturating_sub(cap)))?;
        let mut buf = Vec::new();
        f.take(cap).read_to_end(&mut buf)?;
        Ok((buf, len > cap))
    }

    /// Append to `name` (created at 0600, tightened to 0600 if it exists wider).
    pub fn append(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::fs::FileExt;
        let mut f = self.openat(name, libc::O_RDWR | libc::O_APPEND | libc::O_CREAT, 0o600)?;
        if f.metadata()?.permissions().mode() & 0o777 != 0o600 {
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        // A crash mid-append leaves a fragment with no newline; the next record would join it and both would be lost.
        // Seal the fragment onto its own (malformed, skipped) line first, in the same write as the new record.
        let torn = f.metadata()?.len() > 0 && {
            let mut last = [0u8; 1];
            f.read_at(&mut last, f.metadata()?.len() - 1)? == 1 && last[0] != b'\n'
        };
        if torn {
            let mut buf = Vec::with_capacity(bytes.len() + 1);
            buf.push(b'\n');
            buf.extend_from_slice(bytes);
            return f.write_all(&buf);
        }
        f.write_all(bytes)
    }

    /// Replace `name` atomically: a 0600 temp file created O_EXCL in this folder, then `renameat` within it.
    pub fn write_atomic(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let tmp = format!(".{name}.{}.tmp", std::process::id());
        let t = cstr(&tmp)?;
        let n = cstr(name)?;
        // SAFETY: valid descriptor and NUL-terminated names.
        unsafe { libc::unlinkat(self.0.as_raw_fd(), t.as_ptr(), 0) };
        let res = (|| {
            let mut f = self.openat(&tmp, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL, 0o600)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            // SAFETY: valid descriptor and names; both are relative to the same pinned folder.
            cvt(unsafe { libc::renameat(self.0.as_raw_fd(), t.as_ptr(), self.0.as_raw_fd(), n.as_ptr()) }).map(|_| ())
        })();
        if res.is_err() {
            // SAFETY: as above.
            unsafe { libc::unlinkat(self.0.as_raw_fd(), t.as_ptr(), 0) };
        }
        res
    }
}
