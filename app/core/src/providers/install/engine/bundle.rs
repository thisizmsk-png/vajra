//! The bundled engine, read and checked in memory before anything is written (D7 design E2; bhishma C1, C10).
//!
//! The resource is read with `O_NOFOLLOW` under a size cap, its sha256 must equal the hash compiled into this binary,
//! and the archive must be exactly what `engine_pack` writes: ustar regular files only, safe names, each once (case
//! folded), holding exactly the files its own `manifest.json` lists with those sha256s. Extraction later writes these
//! verified bytes; nothing is read back from disk.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

use super::pack::{self, manifest_files, safe_name, sha256_hex, MANIFEST};
use super::Pin;

/// The resource's name under the app's resource directory (`tauri.conf.json` `bundle.resources`).
pub const RESOURCE: &str = "engine/engine.tgz";
/// The most the resource may be on disk.
const MAX_TGZ: u64 = 64 << 20;

/// One file of the engine: its name, mode (0o644 or 0o755) and bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub mode: u32,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "File({}, {:o}, {} bytes)", self.name, self.mode, self.bytes.len())
    }
}

/// A verified engine: every file, the manifest's own map, and the version from its `package.json`.
#[derive(Debug, Clone)]
pub struct Engine {
    pub files: Vec<File>,
    pub manifest: BTreeMap<String, String>,
    pub version: String,
}

impl Engine {
    pub fn get(&self, name: &str) -> Option<&File> {
        self.files.iter().find(|f| f.name == name)
    }
}

/// Why a bundle was refused. Each is a manual reason on the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The build's files didn't match `manifest.json`, so nothing was pinned (bhishma C1).
    NotPinned,
    /// The resource is missing, not a regular file, or too big.
    Missing,
    /// Its sha256 isn't the one compiled into this binary.
    Mismatch,
    /// It hashes right but isn't a well-formed engine archive (a build bug: never expected).
    Malformed,
}

impl Refused {
    pub fn code(self) -> &'static str {
        match self {
            Refused::NotPinned => "engine_not_pinned",
            Refused::Missing => "engine_bundle_missing",
            Refused::Mismatch => "engine_bundle_mismatch",
            Refused::Malformed => "engine_bundle_malformed",
        }
    }
}

/// Read the resource at `path` (no symlink, a regular file, at most 64 MiB).
pub fn read_resource(path: &Path) -> Result<Vec<u8>, Refused> {
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path).map_err(|_| Refused::Missing)?;
    let meta = f.metadata().map_err(|_| Refused::Missing)?;
    if !meta.is_file() || meta.len() > MAX_TGZ {
        return Err(Refused::Missing);
    }
    let mut out = Vec::with_capacity(meta.len() as usize);
    f.take(MAX_TGZ + 1).read_to_end(&mut out).map_err(|_| Refused::Missing)?;
    if out.len() as u64 > MAX_TGZ {
        return Err(Refused::Missing);
    }
    Ok(out)
}

/// Load and verify the engine bundled at `resource_dir/engine/engine.tgz` against `pin`.
pub fn load(resource_dir: &Path, pin: &Pin) -> Result<Engine, Refused> {
    if !pin.tree_ok {
        return Err(Refused::NotPinned);
    }
    verify(&read_resource(&resource_dir.join(RESOURCE))?, pin)
}

/// Verify bundle bytes against `pin`: the hash first, then the archive.
pub fn verify(tgz: &[u8], pin: &Pin) -> Result<Engine, Refused> {
    if sha256_hex(tgz) != pin.sha256 {
        return Err(Refused::Mismatch);
    }
    // Twice the build's own size, and its own file count: the archive can't be more than what was packed (C10).
    let cap = pin.unpacked.saturating_mul(2).max(1 << 20);
    let files = read_archive(tgz, cap, pin.files).ok_or(Refused::Malformed)?;
    check(files).ok_or(Refused::Malformed)
}

/// The files of an engine archive, or None if it isn't exactly the format `engine_pack` writes.
pub(crate) fn read_archive(tgz: &[u8], max_unpacked: u64, max_files: usize) -> Option<Vec<File>> {
    if !tgz.starts_with(&[0x1f, 0x8b]) {
        return None;
    }
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(tgz).take(max_unpacked + 1).read_to_end(&mut raw).ok()?;
    if raw.len() as u64 > max_unpacked {
        return None;
    }
    let mut files = Vec::new();
    let mut names = BTreeSet::new();
    let mut at = 0usize;
    loop {
        let h = raw.get(at..at + 512)?;
        if h.iter().all(|&b| b == 0) {
            // The end: a second zero block, then only zeros.
            return raw.get(at + 512..at + 1024).filter(|z| z.iter().all(|&b| b == 0)).and(raw[at..].iter().all(|&b| b == 0).then_some(files));
        }
        if &h[257..263] != b"ustar\0" || &h[263..265] != b"00" || h[156] != b'0' || !checksum_ok(h) {
            return None;
        }
        let name = {
            let base = field(&h[0..100])?;
            let prefix = field(&h[345..500])?;
            if prefix.is_empty() { base.to_string() } else { format!("{prefix}/{base}") }
        };
        if !safe_name(&name) || !names.insert(name.to_ascii_lowercase()) || files.len() >= max_files {
            return None;
        }
        let mode = octal(&h[100..108])? as u32;
        if mode != 0o644 && mode != 0o755 {
            return None;
        }
        let size = usize::try_from(octal(&h[124..136])?).ok()?;
        let body = raw.get(at + 512..at + 512 + size)?;
        files.push(File { name, mode, bytes: body.to_vec() });
        at += 512 + size.div_ceil(512) * 512;
    }
}

/// The archive holds exactly its manifest's files, with those hashes, and a `package.json` with a version.
fn check(files: Vec<File>) -> Option<Engine> {
    let manifest_bytes = &files.iter().find(|f| f.name == MANIFEST)?.bytes;
    let manifest = manifest_files(manifest_bytes).ok()?;
    let listed: BTreeSet<&str> = manifest.keys().map(String::as_str).chain([MANIFEST]).collect();
    let held: BTreeSet<&str> = files.iter().map(|f| f.name.as_str()).collect();
    if listed != held || held.len() != files.len() {
        return None;
    }
    if files.iter().filter(|f| f.name != MANIFEST).any(|f| manifest.get(&f.name).map(String::as_str) != Some(sha256_hex(&f.bytes).as_str())) {
        return None;
    }
    let (version, _) = pack::engine_version(&files.iter().find(|f| f.name == "package.json")?.bytes)?;
    Some(Engine { files, manifest, version })
}

/// A NUL-terminated (or full) ASCII field.
fn field(b: &[u8]) -> Option<&str> {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    if b[end..].iter().any(|&c| c != 0) {
        return None;
    }
    std::str::from_utf8(&b[..end]).ok()
}

/// A numeric field: octal digits, then NUL or space padding only (tar's checksum is "digits NUL space").
fn octal(b: &[u8]) -> Option<u64> {
    let end = b.iter().position(|&c| c == 0 || c == b' ').unwrap_or(b.len());
    if end == 0 || !b[..end].iter().all(|c| (b'0'..=b'7').contains(c)) || !b[end..].iter().all(|&c| c == 0 || c == b' ') {
        return None;
    }
    u64::from_str_radix(std::str::from_utf8(&b[..end]).ok()?, 8).ok()
}

fn checksum_ok(h: &[u8]) -> bool {
    let want = octal(&h[148..156]);
    let sum: u64 = h.iter().enumerate().map(|(i, &b)| if (148..156).contains(&i) { u64::from(b' ') } else { u64::from(b) }).sum();
    want == Some(sum)
}

#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;
