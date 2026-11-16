//! The Vajra engine shipped inside the app (ADR-17 item 10 step 2, D7; bhishma C1, C10). Shared by `build.rs`, which
//! packs it at build time and compiles its hash into the binary, and by the core, which checks a bundle read at run
//! time against that hash and the limits below. Std, flate2, sha2 and serde_json only: it runs inside the build script.
//!
//! The pack is exactly the files `manifest.json` lists, plus `manifest.json` itself, as a plain ustar archive with
//! regular files only (no directory entries, no links), in byte order of their names, owner 0, a fixed mtime and mode
//! 0644 or 0755. So the same tree gives the same bytes.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Component, Path};

use sha2::{Digest, Sha256};

/// The engine versions this core was tested with (check 6). A release build refuses to bundle one outside them.
pub const ENGINE_MIN: (u32, u32) = (0, 6);
pub const ENGINE_MAX: (u32, u32) = (0, 999);

/// The manifest's own name: packed next to the files it lists, and not listed in itself.
pub const MANIFEST: &str = "manifest.json";
/// A fixed mtime for every entry (2026-01-01), so the bytes don't depend on checkout times.
const MTIME: u64 = 1_767_225_600;
/// The pinned gzip level (bhishma C1).
const GZIP_LEVEL: u32 = 9;
/// The longest name ustar can hold (155-byte prefix, `/`, 100-byte name).
const MAX_NAME: usize = 255;

pub struct Packed {
    pub tgz: Vec<u8>,
    pub sha256: String,
    pub version: String,
    /// Every listed file matched its manifest sha256 when it was packed.
    pub tree_ok: bool,
    /// The files that didn't, for the build log.
    pub mismatched: Vec<String>,
    /// Bytes once unpacked, and the number of files (both include `manifest.json`).
    pub unpacked: u64,
    pub files: usize,
}

impl std::fmt::Debug for Packed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Packed").field("sha256", &self.sha256).field("version", &self.version).field("tree_ok", &self.tree_ok).field("mismatched", &self.mismatched).field("files", &self.files).finish_non_exhaustive()
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A name the engine may hold: relative, normal components only, ASCII printable, no backslash, short enough for ustar.
/// Non-ASCII is refused outright, so no two names can collide under Unicode folding (bhishma C10).
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name.bytes().all(|b| (0x21..0x7f).contains(&b) && b != b'\\' || b == b' ')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.contains("//")
        && Path::new(name).components().all(|c| matches!(c, Component::Normal(_)))
        && name.split('/').all(|s| s != "." && s != "..")
}

/// The manifest's file map, refusing anything that isn't a plain `{ "files": { name: sha256 } }` with safe names that
/// stay distinct when case is ignored (APFS is case-insensitive by default).
pub fn manifest_files(manifest: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let v: serde_json::Value = serde_json::from_slice(manifest).map_err(|e| format!("manifest.json is not JSON: {e}"))?;
    let files = v.get("files").and_then(|f| f.as_object()).ok_or("manifest.json has no files object")?;
    let mut out = BTreeMap::new();
    let mut folded = std::collections::BTreeSet::new();
    for (name, hash) in files {
        if !safe_name(name) || name == MANIFEST {
            return Err(format!("manifest.json lists an unsafe name: {name:?}"));
        }
        let hash = hash.as_str().filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).ok_or_else(|| format!("manifest.json has a bad sha256 for {name}"))?;
        if !folded.insert(name.to_ascii_lowercase()) {
            return Err(format!("manifest.json lists two names that differ only in case: {name}"));
        }
        out.insert(name.clone(), hash.to_ascii_lowercase());
    }
    if out.is_empty() {
        return Err("manifest.json lists no files".into());
    }
    Ok(out)
}

/// `package.json`'s `version`, and whether its major.minor is in the tested range.
pub fn engine_version(package_json: &[u8]) -> Option<(String, bool)> {
    let v: serde_json::Value = serde_json::from_slice(package_json).ok()?;
    let s = v.get("version")?.as_str()?.to_string();
    let mut p = s.split('.').map(|n| n.parse::<u32>().ok());
    let mm = (p.next()??, p.next()??);
    Some((s.clone(), (ENGINE_MIN..=ENGINE_MAX).contains(&mm)))
}

/// Pack the engine from the repository at `repo`. A listed name that is unsafe, missing, a symlink, not a regular file,
/// or that resolves outside the repository is an error (the build fails); a file whose bytes differ from the manifest
/// only clears `tree_ok`.
pub fn pack(repo: &Path) -> Result<Packed, String> {
    let root = repo.canonicalize().map_err(|e| format!("repo root: {e}"))?;
    let manifest = std::fs::read(root.join(MANIFEST)).map_err(|e| format!("manifest.json: {e}"))?;
    let listed = manifest_files(&manifest)?;
    let mut entries: Vec<(String, Vec<u8>, u32)> = Vec::with_capacity(listed.len() + 1);
    let mut mismatched = Vec::new();
    for (name, want) in &listed {
        let path = root.join(name);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{name}: {e}"))?;
        if !meta.file_type().is_file() {
            return Err(format!("{name} is a symlink or not a regular file; the engine pack holds regular files only"));
        }
        // Every directory on the way must be real too: no listed file may be reached through a symlinked folder.
        let real = path.canonicalize().map_err(|e| format!("{name}: {e}"))?;
        if real != path || !real.starts_with(&root) {
            return Err(format!("{name} is reached through a symlink or leaves the repository"));
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{name}: {e}"))?;
        if sha256_hex(&bytes) != *want {
            mismatched.push(name.clone());
        }
        use std::os::unix::fs::PermissionsExt;
        let mode = if meta.permissions().mode() & 0o111 != 0 { 0o755 } else { 0o644 };
        entries.push((name.clone(), bytes, mode));
    }
    entries.push((MANIFEST.to_string(), manifest, 0o644));
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let package = entries.iter().find(|e| e.0 == "package.json").ok_or("the manifest doesn't list package.json")?;
    let (version, _) = engine_version(&package.1).ok_or("package.json has no major.minor version")?;
    let unpacked = entries.iter().map(|e| e.1.len() as u64).sum();
    let files = entries.len();
    let tar = ustar(&entries)?;
    let mut gz = flate2::GzBuilder::new().mtime(0).write(Vec::new(), flate2::Compression::new(GZIP_LEVEL));
    gz.write_all(&tar).map_err(|e| e.to_string())?;
    let tgz = gz.finish().map_err(|e| e.to_string())?;
    Ok(Packed { sha256: sha256_hex(&tgz), tgz, version, tree_ok: mismatched.is_empty(), mismatched, unpacked, files })
}

/// The gzip'd ustar of `entries` as given (name, bytes, mode), with no checks: the core's tests use it to build archives
/// the core must refuse.
#[allow(dead_code)]
pub fn raw_tgz(entries: &[(String, Vec<u8>, u32)], kind: u8) -> Vec<u8> {
    let mut tar = ustar_kind(entries, kind).expect("short names");
    let mut gz = flate2::GzBuilder::new().mtime(0).write(Vec::new(), flate2::Compression::new(GZIP_LEVEL));
    gz.write_all(&tar).expect("in memory");
    tar.clear();
    gz.finish().expect("in memory")
}

/// A ustar archive of regular files, then the two zero blocks.
fn ustar(entries: &[(String, Vec<u8>, u32)]) -> Result<Vec<u8>, String> {
    ustar_kind(entries, b'0')
}

fn ustar_kind(entries: &[(String, Vec<u8>, u32)], kind: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for (name, bytes, mode) in entries {
        let mut h = [0u8; 512];
        let (prefix, base) = split_name(name).ok_or_else(|| format!("{name} is too long for the engine pack"))?;
        h[..base.len()].copy_from_slice(base.as_bytes());
        octal(&mut h[100..108], u64::from(*mode));
        octal(&mut h[108..116], 0);
        octal(&mut h[116..124], 0);
        octal(&mut h[124..136], bytes.len() as u64);
        octal(&mut h[136..148], MTIME);
        h[148..156].fill(b' ');
        h[156] = kind;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
        let sum: u64 = h.iter().map(|&b| u64::from(b)).sum();
        octal(&mut h[148..155], sum);
        h[155] = b' ';
        out.extend_from_slice(&h);
        out.extend_from_slice(bytes);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    Ok(out)
}

/// Split a name into ustar's prefix (up to 155 bytes) and name (up to 100), at a `/`.
fn split_name(name: &str) -> Option<(&str, &str)> {
    if name.len() <= 100 {
        return Some(("", name));
    }
    name.match_indices('/').map(|(i, _)| i).rfind(|&i| i <= 155 && name.len() - i - 1 <= 100).map(|i| (&name[..i], &name[i + 1..]))
}

/// Zero-padded octal, NUL-terminated, filling the field.
fn octal(field: &mut [u8], v: u64) {
    let s = format!("{v:0width$o}", width = field.len() - 1);
    field[..s.len()].copy_from_slice(s.as_bytes());
    field[field.len() - 1] = 0;
}
