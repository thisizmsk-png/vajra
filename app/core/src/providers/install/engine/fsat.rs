//! Directory-fd filesystem calls for the engine install (bhishma C2). Every create, rename and remove is relative to a
//! directory this process holds open and checked, one path component at a time with `O_NOFOLLOW`, so a symlink planted
//! by a same-uid process anywhere on the way makes the call fail instead of writing somewhere else.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

fn c(name: &str) -> io::Result<CString> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "one plain path component"));
    }
    CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in name"))
}

fn check(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

/// What `fstatat(..., AT_SYMLINK_NOFOLLOW)` says about an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub kind: Kind,
    pub uid: u32,
    pub mode: u32,
    pub nlink: u64,
    pub size: u64,
    pub dev: u64,
    pub ino: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    Link,
    Other,
}

impl Stat {
    fn of(s: &libc::stat) -> Stat {
        let kind = match s.st_mode & libc::S_IFMT {
            libc::S_IFDIR => Kind::Dir,
            libc::S_IFREG => Kind::File,
            libc::S_IFLNK => Kind::Link,
            _ => Kind::Other,
        };
        Stat { kind, uid: s.st_uid, mode: u32::from(s.st_mode) & 0o7777, nlink: u64::from(s.st_nlink), size: s.st_size as u64, dev: s.st_dev as u64, ino: s.st_ino }
    }

    /// A real directory owned by `uid`, not writable by group or others.
    pub fn private_dir(&self, uid: u32) -> bool {
        self.kind == Kind::Dir && self.uid == uid && self.mode & 0o022 == 0
    }
}

/// An open directory, checked when it was opened.
#[derive(Debug)]
pub struct Dir {
    fd: OwnedFd,
}

impl Dir {
    /// Open an absolute path from `/`, one component at a time, never following a link. Every directory on the way
    /// must be owned by root or by `uid` and not writable by group or others, except the sticky `/tmp`-style folders
    /// the OS owns (so tests under the temp dir work); the last must be owned by `uid`.
    pub fn open_path(path: &std::path::Path, uid: u32) -> io::Result<Dir> {
        if !path.is_absolute() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "absolute path"));
        }
        let root = CString::new("/").expect("no NUL");
        // SAFETY: a NUL-terminated path; the fd is owned below.
        let mut dir = Dir { fd: unsafe { OwnedFd::from_raw_fd(check(libc::open(root.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC))?) } };
        let parts: Vec<String> = path.components().skip(1).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        for (i, part) in parts.iter().enumerate() {
            dir = dir.open_dir(part)?;
            let s = dir.stat()?;
            let last = i + 1 == parts.len();
            let sticky_root = s.uid == 0 && s.mode & 0o1000 != 0;
            let ok = if last { s.uid == uid && s.mode & 0o022 == 0 } else { (s.uid == 0 || s.uid == uid) && (s.mode & 0o022 == 0 || sticky_root) };
            if !ok {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{part} is not a private directory")));
            }
        }
        Ok(dir)
    }

    pub fn raw(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    pub fn stat(&self) -> io::Result<Stat> {
        // SAFETY: a valid fd and an out pointer to a zeroed stat.
        let mut s: libc::stat = unsafe { std::mem::zeroed() };
        check(unsafe { libc::fstat(self.raw(), &mut s) })?;
        Ok(Stat::of(&s))
    }

    /// The entry `name` here, without following a link; None when it doesn't exist.
    pub fn stat_at(&self, name: &str) -> io::Result<Option<Stat>> {
        let n = c(name)?;
        // SAFETY: as above.
        let mut s: libc::stat = unsafe { std::mem::zeroed() };
        match check(unsafe { libc::fstatat(self.raw(), n.as_ptr(), &mut s, libc::AT_SYMLINK_NOFOLLOW) }) {
            Ok(_) => Ok(Some(Stat::of(&s))),
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Open the subdirectory `name` (never a link).
    pub fn open_dir(&self, name: &str) -> io::Result<Dir> {
        let n = c(name)?;
        // SAFETY: as above; O_NOFOLLOW refuses a final symlink, O_DIRECTORY anything but a directory.
        let fd = check(unsafe { libc::openat(self.raw(), n.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) })?;
        Ok(Dir { fd: unsafe { OwnedFd::from_raw_fd(fd) } })
    }

    /// Open `parts` below this directory, one component at a time, never following a link. An empty list is this
    /// directory again.
    pub fn open_dir_path(&self, parts: &[&str]) -> io::Result<Dir> {
        // SAFETY: dup of a valid fd; owned below.
        let dup = check(unsafe { libc::fcntl(self.raw(), libc::F_DUPFD_CLOEXEC, 0) })?;
        let mut cur = Dir { fd: unsafe { OwnedFd::from_raw_fd(dup) } };
        for p in parts {
            cur = cur.open_dir(p)?;
        }
        Ok(cur)
    }

    /// Create the subdirectory `name` with `mode` (fails if anything is already there), and open it.
    pub fn make_dir(&self, name: &str, mode: u32) -> io::Result<Dir> {
        let n = c(name)?;
        // SAFETY: as above.
        check(unsafe { libc::mkdirat(self.raw(), n.as_ptr(), mode as libc::mode_t) })?;
        let d = self.open_dir(name)?;
        // mkdir honours the umask; set the exact mode on the fd we hold.
        check(unsafe { libc::fchmod(d.raw(), mode as libc::mode_t) })?;
        Ok(d)
    }

    /// Open `name` if it is a directory, else create it with `mode`.
    pub fn open_or_make_dir(&self, name: &str, mode: u32) -> io::Result<Dir> {
        match self.make_dir(name, mode) {
            Err(e) if e.raw_os_error() == Some(libc::EEXIST) => self.open_dir(name),
            r => r,
        }
    }

    /// Write a new file `name` (fails if anything is already there) with `bytes` and `mode`, synced.
    pub fn write_new(&self, name: &str, bytes: &[u8], mode: u32) -> io::Result<()> {
        let n = c(name)?;
        // SAFETY: as above; O_EXCL with O_CREAT never follows or reuses an existing entry.
        let fd = check(unsafe { libc::openat(self.raw(), n.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC, mode as libc::c_uint) })?;
        let mut f = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
        let r = (|| {
            std::io::Write::write_all(&mut f, bytes)?;
            check(unsafe { libc::fchmod(f.as_raw_fd(), mode as libc::mode_t) })?;
            f.sync_all()
        })();
        if r.is_err() {
            let _ = self.remove_file(name);
        }
        r
    }

    /// Read the regular file `name` (never a link, a single link count), at most `max` bytes.
    pub fn read(&self, name: &str, max: u64) -> io::Result<Vec<u8>> {
        let n = c(name)?;
        // SAFETY: as above.
        let fd = check(unsafe { libc::openat(self.raw(), n.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC) })?;
        let f = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
        let m = f.metadata()?;
        use std::os::unix::fs::MetadataExt;
        if !m.is_file() || m.nlink() != 1 || m.len() > max {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{name} is not a single regular file under the limit")));
        }
        let mut out = Vec::with_capacity(m.len() as usize);
        std::io::Read::read_to_end(&mut std::io::Read::take(f, max + 1), &mut out)?;
        Ok(out)
    }

    /// The names in this directory (no `.` or `..`).
    pub fn list(&self) -> io::Result<Vec<String>> {
        // SAFETY: dup the fd so closedir doesn't close ours; fdopendir takes ownership of the dup.
        let dup = check(unsafe { libc::fcntl(self.raw(), libc::F_DUPFD_CLOEXEC, 0) })?;
        let dp = unsafe { libc::fdopendir(dup) };
        if dp.is_null() {
            unsafe { libc::close(dup) };
            return Err(io::Error::last_os_error());
        }
        unsafe { libc::rewinddir(dp) };
        let mut out = Vec::new();
        loop {
            // SAFETY: readdir returns null at the end; the entry is valid until the next call.
            let e = unsafe { libc::readdir(dp) };
            if e.is_null() {
                break;
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*e).d_name.as_ptr()) }.to_string_lossy().into_owned();
            if name != "." && name != ".." {
                out.push(name);
            }
        }
        unsafe { libc::closedir(dp) };
        out.sort();
        Ok(out)
    }

    pub fn remove_file(&self, name: &str) -> io::Result<()> {
        let n = c(name)?;
        check(unsafe { libc::unlinkat(self.raw(), n.as_ptr(), 0) }).map(|_| ())
    }

    /// Remove the directory tree `name` here, never following a link (a link inside is removed as a link).
    pub fn remove_tree(&self, name: &str) -> io::Result<()> {
        match self.stat_at(name)? {
            None => Ok(()),
            Some(s) if s.kind == Kind::Dir => {
                let d = self.open_dir(name)?;
                for child in d.list()? {
                    d.remove_tree(&child)?;
                }
                let n = c(name)?;
                check(unsafe { libc::unlinkat(self.raw(), n.as_ptr(), libc::AT_REMOVEDIR) }).map(|_| ())
            }
            Some(_) => self.remove_file(name),
        }
    }

    /// Rename `from` here to `to` in `dest`, atomically: `Swap` exchanges two existing entries, `Exclusive` fails if
    /// `to` exists, `Replace` replaces a file.
    pub fn rename(&self, from: &str, dest: &Dir, to: &str, how: Rename) -> io::Result<()> {
        let (f, t) = (c(from)?, c(to)?);
        let flags = match how {
            Rename::Swap => libc::RENAME_SWAP,
            Rename::Exclusive => libc::RENAME_EXCL,
            Rename::Replace => 0,
        };
        // SAFETY: valid fds and NUL-terminated names.
        check(unsafe { libc::renameatx_np(self.raw(), f.as_ptr(), dest.raw(), t.as_ptr(), flags) }).map(|_| ())
    }

    /// Flush this directory's entries to disk.
    pub fn sync(&self) -> io::Result<()> {
        check(unsafe { libc::fsync(self.raw()) }).map(|_| ())
    }

    /// Free bytes on this directory's filesystem, for an unprivileged user.
    pub fn free_bytes(&self) -> io::Result<u64> {
        // SAFETY: an out pointer to a zeroed statvfs.
        let mut v: libc::statvfs = unsafe { std::mem::zeroed() };
        check(unsafe { libc::fstatvfs(self.raw(), &mut v) })?;
        Ok(v.f_bavail as u64 * v.f_frsize as u64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rename {
    Swap,
    Exclusive,
    Replace,
}

/// The effective uid.
pub fn euid() -> u32 {
    // SAFETY: always succeeds.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
#[path = "fsat_tests.rs"]
mod tests;
