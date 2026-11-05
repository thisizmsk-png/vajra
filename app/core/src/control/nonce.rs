//! The per-session steering nonce (design.md 2, "Control channel trust").
//!
//! 128 bits from /dev/urandom, 32 lowercase hex chars, one per run session,
//! rotated on resume. The file `<session dir>/nonce` is created
//! O_CREAT | O_EXCL | O_NOFOLLOW at 0600 inside a 0700 dir, which is exactly
//! what `desktop-control.sh` `load_nonce` accepts. The nonce is never written
//! anywhere else by C6: not the ledger, queue, acks, app log or MCP results.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use super::fsx;
use super::sha256::hex;

/// The only randomness source. There is no fallback: a failed read is an error.
pub const SOURCE: &str = "/dev/urandom";
pub const NONCE_BYTES: usize = 16;
pub const FILE: &str = "nonce";

/// Hex of `n` random bytes from /dev/urandom.
pub fn random_hex(n: usize) -> io::Result<String> {
    let mut buf = vec![0u8; n];
    File::open(SOURCE)?.read_exact(&mut buf)?;
    Ok(hex(&buf))
}

/// A nonce from 16 bytes of `r` (a short read is an error).
pub fn from_reader(r: &mut impl Read) -> io::Result<String> {
    let mut buf = [0u8; NONCE_BYTES];
    r.read_exact(&mut buf)?;
    Ok(hex(&buf))
}

pub fn generate() -> io::Result<String> {
    from_reader(&mut File::open(SOURCE)?)
}

/// Make `dir` 0700 and create `dir/nonce` with a fresh nonce. Never
/// overwrites: an existing file or symlink there is an error.
pub fn create(dir: &Path) -> io::Result<String> {
    fsx::private_dir(dir)?;
    let n = generate()?;
    let path = dir.join(FILE);
    let mut f = fsx::create_new(&path)?;
    let res = f.write_all(format!("{n}\n").as_bytes()).and_then(|_| f.sync_all());
    if let Err(e) = res {
        let _ = fs::remove_file(&path);
        return Err(e);
    }
    Ok(n)
}

/// Replace the nonce: unlink the old file (a symlink is removed, not
/// followed), then `create`. Between the two the hook finds no nonce and
/// delivers nothing, which is the safe side.
pub fn rotate(dir: &Path) -> io::Result<String> {
    match fs::remove_file(dir.join(FILE)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    create(dir)
}

/// Read the nonce back under the hook's rules: a regular file (no symlink),
/// owned by us, mode 0600, 32-128 lowercase hex.
pub fn read(dir: &Path) -> io::Result<String> {
    let path = dir.join(FILE);
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let meta = fs::symlink_metadata(&path)?;
    if !meta.file_type().is_file() {
        return Err(bad("nonce is not a regular file"));
    }
    if meta.uid() != unsafe { libc::getuid() } || meta.permissions().mode() & 0o777 != 0o600 {
        return Err(bad("nonce file owner or mode is wrong"));
    }
    let body = fsx::read_nofollow(&path)?;
    let n = String::from_utf8(body).map_err(|_| bad("nonce is not text"))?;
    let n = n.trim_end_matches('\n');
    if !(32..=128).contains(&n.len()) || !n.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(bad("nonce is not hex"));
    }
    Ok(n.to_string())
}
