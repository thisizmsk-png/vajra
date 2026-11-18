//! The Files panel's read-only view of one project folder (or one run worktree of it): list a directory, read a file.
//!
//! Every path the UI sends is relative to a root the core picked (the UI never names a root). A path is refused when it
//! is absolute, holds `..`, `.` or an empty part, a NUL, a `.git` part or the signing key's name. Otherwise it is
//! resolved through links and must land inside the root and outside every denied folder (the app's data folder and
//! the engine's state folder). The resolved path is then opened again one part at a time from the open root with
//! `O_NOFOLLOW`, so a link swapped in after the check is refused rather than followed. A file is opened only if it is a
//! regular file that is not the signing key (by device and inode, so a hard link to the key under another name is
//! refused too); a read is capped and binary content is never sent.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};

use crate::vajra::allowlist::DENIED_NAME;

pub mod diff;
pub mod link;

/// Largest text file sent whole.
pub const TEXT_CAP: u64 = 1 << 20;
/// Largest image sent.
pub const IMAGE_CAP: u64 = 5 << 20;
/// Entries listed from one directory at most.
pub const LIST_CAP: usize = 5000;
/// Longest relative path accepted.
pub const PATH_MAX_LEN: usize = 4096;

/// Why a path was refused. `code()` is the UI's `error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The path itself is malformed: absolute, `..`, an empty part, NUL, too long.
    BadPath(&'static str),
    /// It resolves outside the root (a link out, typically).
    Outside,
    /// `.git`, the signing key, the app's data folder or the engine's state folder.
    Protected,
    NotFound,
    NotAFile,
    NotADirectory,
    Io(String),
}

impl Refused {
    pub fn code(&self) -> &'static str {
        match self {
            Refused::BadPath(_) => "invalid_args",
            Refused::Outside => "outside_root",
            Refused::Protected => "protected",
            Refused::NotFound => "not_found",
            Refused::NotAFile => "not_a_file",
            Refused::NotADirectory => "not_a_directory",
            Refused::Io(_) => "io_error",
        }
    }

    pub fn to_json(&self) -> Value {
        let detail = match self {
            Refused::BadPath(why) => format!("path: {why}"),
            Refused::Outside => "the path leads outside the project".into(),
            Refused::Protected => "the app does not show this file".into(),
            Refused::NotFound => "no such file or folder".into(),
            Refused::NotAFile => "not a regular file".into(),
            Refused::NotADirectory => "not a folder".into(),
            Refused::Io(e) => e.clone(),
        };
        json!({"error": self.code(), "detail": detail})
    }
}

/// Check the shape of a relative path, `""` being the root. Returns its parts. Nothing is touched on disk.
pub fn check_rel(rel: &str) -> Result<Vec<&str>, Refused> {
    if rel.len() > PATH_MAX_LEN {
        return Err(Refused::BadPath("too long"));
    }
    if rel.contains('\0') {
        return Err(Refused::BadPath("contains NUL"));
    }
    if rel.is_empty() {
        return Ok(Vec::new());
    }
    if rel.starts_with('/') || Path::new(rel).is_absolute() {
        return Err(Refused::BadPath("must be relative to the project"));
    }
    let parts: Vec<&str> = rel.split('/').collect();
    for p in &parts {
        match *p {
            "" => return Err(Refused::BadPath("has an empty part")),
            "." | ".." => return Err(Refused::BadPath("has a . or .. part")),
            _ if protected_name(p) => return Err(Refused::Protected),
            _ => {}
        }
    }
    Ok(parts)
}

/// A name the panel never lists or opens, wherever it is: git's own folder (or a worktree's `.git` file) and the key.
fn protected_name(name: &str) -> bool {
    name == ".git" || name == DENIED_NAME
}

/// One listed entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// `dir`, `file`, `link` (a link that leads outside the root or nowhere: listed, never opened) or `other`.
    pub kind: &'static str,
    /// Through a link that stays inside the root.
    pub link: bool,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub truncated: bool,
    /// Entries left out because their names are not UTF-8.
    pub skipped: usize,
}

/// What a read returns. Content only for `Text` and `Image`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(String),
    Image { mime: &'static str, bytes: Vec<u8> },
    Binary,
    TooLarge { cap: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRead {
    pub size: u64,
    pub content: Content,
}

/// One root the panel reads from, with what it must never show.
#[derive(Debug)]
pub struct Scope {
    root: PathBuf,
    /// The root folder's device and inode when the scope was made.
    root_id: (u64, u64),
    deny: Vec<PathBuf>,
    key: Vec<(u64, u64)>,
}

/// A read buffer wiped (with volatile writes, so not optimised away) when dropped: bytes read but not sent (a file
/// that grew past the cap, a binary file) don't linger in freed memory. Sent bytes are moved out first.
pub struct Wiped(Vec<u8>);

impl From<Vec<u8>> for Wiped {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

impl std::ops::Deref for Wiped {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Wiped {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            // SAFETY: a valid, aligned, exclusive reference into the buffer.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

impl Scope {
    /// `root` must be an existing directory. `deny` are folders never shown (missing ones are skipped). `key` is the
    /// signing key's path: only `stat`ed for its identity, never opened or read (NFR-SEC-2).
    pub fn new(root: &Path, deny: &[PathBuf], key: &Path) -> Result<Self, Refused> {
        let root = root.canonicalize().map_err(io)?;
        let md = std::fs::metadata(&root).map_err(io)?;
        if !md.is_dir() {
            return Err(Refused::NotADirectory);
        }
        let root_id = (md.dev(), md.ino());
        let deny = deny.iter().filter_map(|d| d.canonicalize().ok()).collect();
        let mut keyids: Vec<(u64, u64)> =
            [std::fs::symlink_metadata(key), std::fs::metadata(key)].into_iter().flatten().map(|m| (m.dev(), m.ino())).collect();
        keyids.dedup();
        Ok(Self { root, root_id, deny, key: keyids })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A canonical path's parts below the root, or why it may not be shown.
    fn inside<'a>(&self, real: &'a Path) -> Result<Vec<&'a str>, Refused> {
        let rest = real.strip_prefix(&self.root).map_err(|_| Refused::Outside)?;
        if self.deny.iter().any(|d| real.starts_with(d)) {
            return Err(Refused::Protected);
        }
        let mut parts = Vec::new();
        for c in rest.components() {
            let Component::Normal(s) = c else { return Err(Refused::Outside) };
            let s = s.to_str().ok_or(Refused::BadPath("not UTF-8"))?;
            if protected_name(s) {
                return Err(Refused::Protected);
            }
            parts.push(s);
        }
        Ok(parts)
    }

    /// Check `rel`, resolve it through links, and return its canonical path below the root.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, Refused> {
        let parts = check_rel(rel)?;
        let joined = parts.iter().fold(self.root.clone(), |p, c| p.join(c));
        let real = match joined.canonicalize() {
            Ok(r) => r,
            Err(e) => {
                // A missing path answers as its nearest existing folder does, so a link out can't be used to ask
                // whether some file outside the root exists.
                if let Some(up) = joined.ancestors().skip(1).find_map(|a| a.canonicalize().ok()) {
                    self.inside(&up)?;
                }
                return Err(io(e));
            }
        };
        self.inside(&real)?;
        Ok(real)
    }

    /// The root, opened from `/` one part at a time without following a link (the root is canonical, so it has none),
    /// and checked to be the same folder (device and inode) `new` found. A folder above the root swapped for a link
    /// since then is refused, not followed: `O_NOFOLLOW` on a whole path guards only its last part.
    fn open_root(&self) -> Result<File, Refused> {
        let mut at = open_path(Path::new("/"), libc::O_RDONLY | libc::O_DIRECTORY)?;
        for c in self.root.components() {
            match c {
                Component::RootDir => {}
                Component::Normal(s) => {
                    let s = s.to_str().ok_or(Refused::BadPath("not UTF-8"))?;
                    at = open_at(&at, s, libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW)?;
                }
                _ => return Err(Refused::Outside),
            }
        }
        let md = at.metadata().map_err(io)?;
        if (md.dev(), md.ino()) != self.root_id {
            return Err(Refused::Outside);
        }
        Ok(at)
    }

    /// Open the canonical `real` (from `resolve`) one part at a time from the root, never following a link. `dir`
    /// asks for a directory; otherwise the last part is opened without blocking (a FIFO can't hang the read).
    fn open_walk(&self, real: &Path, dir: bool) -> Result<File, Refused> {
        let parts = self.inside(real)?;
        // `real` was checked to have no link in it, so a link (ELOOP) or a non-folder (ENOTDIR) met here was swapped in
        // since: refused as leading outside, whichever error the OS gives for it.
        let changed = |e: Refused| if matches!(e, Refused::NotADirectory) { Refused::Outside } else { e };
        let mut at = self.open_root().map_err(changed)?;
        for (i, p) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let flags = if !last || dir { libc::O_DIRECTORY } else { libc::O_NONBLOCK };
            at = open_at(&at, p, libc::O_RDONLY | libc::O_NOFOLLOW | flags).map_err(changed)?;
        }
        Ok(at)
    }

    /// The entries of the folder `rel` (`""` is the root): folders first, then by name. `.git`, the key, and denied
    /// folders are left out.
    pub fn list(&self, rel: &str) -> Result<Listing, Refused> {
        let real = self.resolve(rel)?;
        if !real.is_dir() {
            return Err(Refused::NotADirectory);
        }
        let dir = self.open_walk(&real, true)?;
        let (names, skipped, truncated) = entry_names(&dir)?;
        let mut entries = Vec::new();
        for name in names {
            if protected_name(&name) || self.deny.iter().any(|d| real.join(&name).starts_with(d)) {
                continue;
            }
            let Some(st) = stat_at(&dir, &name) else { continue };
            let (mut kind, mut link, mut size) = (kind_of(st.st_mode), false, None);
            if kind == "link" {
                // A link is shown as what it leads to only when that is inside the root and may be shown.
                if let Ok(target) = real.join(&name).canonicalize() {
                    if self.inside(&target).is_ok() {
                        if let Ok(m) = std::fs::metadata(&target) {
                            link = true;
                            kind = if m.is_dir() { "dir" } else if m.is_file() { "file" } else { "other" };
                            size = m.is_file().then_some(m.len());
                        }
                    }
                }
            } else if kind == "file" {
                size = Some(st.st_size as u64);
            }
            entries.push(Entry { name, kind, link, size });
        }
        entries.sort_by(|a, b| (a.kind != "dir").cmp(&(b.kind != "dir")).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.name.cmp(&b.name)));
        Ok(Listing { entries, truncated, skipped })
    }

    /// Open `rel` as `read` would: resolved inside the root, walked without links, a regular file that is not the key.
    fn open_file(&self, rel: &str) -> Result<(File, std::fs::Metadata), Refused> {
        let real = self.resolve(rel)?;
        let f = self.open_walk(&real, false)?;
        let md = f.metadata().map_err(io)?;
        if !md.is_file() {
            return Err(Refused::NotAFile);
        }
        if self.key.contains(&(md.dev(), md.ino())) {
            return Err(Refused::Protected);
        }
        Ok((f, md))
    }

    /// Whether `rel` may be shown, by every rule `read` applies, without reading it. For a caller that hands the path
    /// to another reader (git diff): `NotFound` means the path is missing and its nearest existing folder may be shown.
    pub fn check_file(&self, rel: &str) -> Result<(), Refused> {
        self.open_file(rel).map(|_| ())
    }

    /// Read the file `rel`: text up to `TEXT_CAP`, an image up to `IMAGE_CAP`; anything else is only described.
    pub fn read(&self, rel: &str) -> Result<FileRead, Refused> {
        let (f, md) = self.open_file(rel)?;
        let size = md.len();
        let image = image_mime(rel);
        let cap = if image.is_some() { IMAGE_CAP } else { TEXT_CAP };
        if size > cap {
            return Ok(FileRead { size, content: Content::TooLarge { cap } });
        }
        clear_nonblock(&f)?;
        let mut buf = Wiped(Vec::with_capacity(size as usize));
        // One byte past the cap tells a file that grew since the stat.
        f.take(cap + 1).read_to_end(&mut buf.0).map_err(io)?;
        if buf.0.len() as u64 > cap {
            return Ok(FileRead { size: buf.0.len() as u64, content: Content::TooLarge { cap } });
        }
        if let Some(mime) = image {
            return Ok(FileRead { size, content: Content::Image { mime, bytes: std::mem::take(&mut buf.0) } });
        }
        if looks_binary(&buf.0) || std::str::from_utf8(&buf.0).is_err() {
            return Ok(FileRead { size, content: Content::Binary });
        }
        let text = String::from_utf8(std::mem::take(&mut buf.0)).map_err(|_| Refused::Io("not UTF-8".into()))?;
        Ok(FileRead { size, content: Content::Text(text) })
    }
}

/// A NUL in the first 8 KiB: what git calls binary.
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|&b| b == 0)
}

/// The image types the viewer shows, by extension. SVG is shown only as an image (`<img>` runs no script in it).
pub fn image_mime(rel: &str) -> Option<&'static str> {
    let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

fn io(e: std::io::Error) -> Refused {
    match e.raw_os_error() {
        Some(libc::ENOENT) => Refused::NotFound,
        Some(libc::ENOTDIR) => Refused::NotADirectory,
        // A link where the walk expected none: something changed since the check.
        Some(libc::ELOOP) => Refused::Outside,
        _ => Refused::Io(e.to_string()),
    }
}

fn open_path(p: &Path, flags: i32) -> Result<File, Refused> {
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(p.as_os_str().as_bytes()).map_err(|_| Refused::BadPath("contains NUL"))?;
    // SAFETY: a NUL-terminated path; on success the descriptor is ours.
    let fd = unsafe { libc::open(c.as_ptr(), flags | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    // SAFETY: a fresh descriptor nothing else owns.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn open_at(dir: &File, name: &str, flags: i32) -> Result<File, Refused> {
    let c = CString::new(name).map_err(|_| Refused::BadPath("contains NUL"))?;
    // SAFETY: a directory descriptor we hold and a NUL-terminated single name.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), c.as_ptr(), flags | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    // SAFETY: a fresh descriptor nothing else owns.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn clear_nonblock(f: &File) -> Result<(), Refused> {
    // SAFETY: F_GETFL/F_SETFL on a descriptor this function's caller owns.
    unsafe {
        let fl = libc::fcntl(f.as_raw_fd(), libc::F_GETFL);
        if fl == -1 || libc::fcntl(f.as_raw_fd(), libc::F_SETFL, fl & !libc::O_NONBLOCK) == -1 {
            return Err(io(std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

fn stat_at(dir: &File, name: &str) -> Option<libc::stat> {
    let c = CString::new(name).ok()?;
    // SAFETY: zeroed is a valid `stat`; fstatat fills it for a name under a directory we hold, not following links.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::fstatat(dir.as_raw_fd(), c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
    (rc == 0).then_some(st)
}

fn kind_of(mode: libc::mode_t) -> &'static str {
    match mode & libc::S_IFMT {
        libc::S_IFDIR => "dir",
        libc::S_IFREG => "file",
        libc::S_IFLNK => "link",
        _ => "other",
    }
}

/// The entry names of a held directory, read through its descriptor: `(names, non-UTF-8 skipped, truncated)`.
fn entry_names(dir: &File) -> Result<(Vec<String>, usize, bool), Refused> {
    // SAFETY: a close-on-exec copy of a valid descriptor; on success `fdopendir` owns it and `closedir` closes it.
    let fd = unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    // SAFETY: `fd` is a directory descriptor we own.
    let handle = unsafe { libc::fdopendir(fd) };
    if handle.is_null() {
        // SAFETY: `fdopendir` failed, so `fd` is still ours to close.
        unsafe { libc::close(fd) };
        return Err(io(std::io::Error::last_os_error()));
    }
    let (mut names, mut skipped, mut truncated) = (Vec::new(), 0, false);
    loop {
        // SAFETY: `handle` is an open DIR; the entry is read before the next call.
        let ent = unsafe { libc::readdir(handle) };
        if ent.is_null() {
            break;
        }
        // SAFETY: `d_name` is a NUL-terminated name inside the entry.
        let name = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) };
        match name.to_str() {
            Ok("." | "..") => {}
            Ok(n) if names.len() < LIST_CAP => names.push(n.to_owned()),
            Ok(_) => {
                truncated = true;
                break;
            }
            Err(_) => skipped += 1,
        }
    }
    // SAFETY: closes the DIR and the duplicated descriptor with it.
    unsafe { libc::closedir(handle) };
    Ok((names, skipped, truncated))
}

#[cfg(test)]
mod tests;
