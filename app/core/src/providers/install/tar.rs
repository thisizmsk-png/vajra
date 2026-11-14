//! An in-memory reader for the npm tarball whose sha512 was just checked (bhishma condition 3). Nothing is extracted to
//! disk: it streams the gzip, keeps only the manifests the closure check needs, and refuses anything ambiguous.
//! ustar, pax (`x`) and GNU long names (`L`) are read; the single root folder (`package/`) is stripped.

use std::collections::HashSet;
use std::io::Read;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Bytes of tar data after gunzip.
    pub unpacked: u64,
    /// File, folder and link entries (pax and long-name records are not counted).
    pub entries: usize,
    /// Bytes of one inspected manifest.
    pub manifest: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { unpacked: 1 << 30, entries: 200_000, manifest: 8 << 20 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TarError {
    Gzip,
    Truncated,
    Header,
    Path,
    Roots,
    Duplicate,
    Link,
    Kind,
    TooBig,
    TooMany,
    NoManifest,
}

impl TarError {
    pub fn code(self) -> &'static str {
        match self {
            TarError::Gzip => "tar_gzip",
            TarError::Truncated => "tar_truncated",
            TarError::Header => "tar_header",
            TarError::Path => "tar_path",
            TarError::Roots => "tar_roots",
            TarError::Duplicate => "tar_duplicate",
            TarError::Link => "tar_link",
            TarError::Kind => "tar_kind",
            TarError::TooBig => "tar_too_big",
            TarError::TooMany => "tar_too_many",
            TarError::NoManifest => "tar_no_manifest",
        }
    }
}

/// What the closure check reads from a package tarball.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    /// `package.json` at the root.
    pub manifest: Vec<u8>,
    /// `npm-shrinkwrap.json` at the root, if any.
    pub shrinkwrap: Option<Vec<u8>>,
    /// Every bundled package's folder (`node_modules/a`, `node_modules/@s/b`, nested too) and its `package.json`.
    pub nested: Vec<(String, Vec<u8>)>,
}

/// Read a gzip'd tarball.
pub fn read_tgz(gz: &[u8], lim: Limits) -> Result<Pack, TarError> {
    if !gz.starts_with(&[0x1f, 0x8b]) {
        return Err(TarError::Gzip);
    }
    parse(flate2::read::GzDecoder::new(gz), lim, TarError::Gzip)
}

/// Read an uncompressed tarball (the fuzz test's second door).
pub fn parse_tar(raw: &[u8], lim: Limits) -> Result<Pack, TarError> {
    parse(raw, lim, TarError::Truncated)
}

/// Counts what it reads, so the unpacked cap holds however the archive is shaped.
struct Counted<R> {
    inner: R,
    read: u64,
    cap: u64,
    io_err: TarError,
}

impl<R: Read> Counted<R> {
    fn exact(&mut self, buf: &mut [u8]) -> Result<(), TarError> {
        let mut at = 0;
        while at < buf.len() {
            match self.inner.read(&mut buf[at..]) {
                Ok(0) => return Err(TarError::Truncated),
                Ok(n) => at += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Err(TarError::Truncated),
                Err(_) => return Err(self.io_err),
            }
        }
        self.read += buf.len() as u64;
        if self.read > self.cap {
            return Err(TarError::TooBig);
        }
        Ok(())
    }
    /// Read `size` bytes of entry data plus padding, keeping them only when `keep`.
    fn data(&mut self, size: u64, keep: bool) -> Result<Vec<u8>, TarError> {
        let padded = size.div_ceil(512) * 512;
        if self.read.saturating_add(padded) > self.cap {
            return Err(TarError::TooBig);
        }
        let mut out = Vec::new();
        let mut left = padded;
        let mut block = [0u8; 512];
        while left > 0 {
            self.exact(&mut block)?;
            if keep {
                let want = (size - (padded - left)).min(512) as usize;
                out.extend_from_slice(&block[..want]);
            }
            left -= 512;
        }
        Ok(out)
    }
}

fn octal(field: &[u8]) -> Result<u64, TarError> {
    let s = field.iter().take_while(|&&b| b != 0).copied().collect::<Vec<u8>>();
    let s = std::str::from_utf8(&s).map_err(|_| TarError::Header)?.trim();
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).map_err(|_| TarError::Header)
}

fn checksum_ok(h: &[u8; 512]) -> bool {
    let sum: u64 = h.iter().enumerate().map(|(i, &b)| if (148..156).contains(&i) { 32 } else { u64::from(b) }).sum();
    octal(&h[148..156]).is_ok_and(|v| v == sum)
}

fn cstr(field: &[u8]) -> Result<String, TarError> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8(field[..end].to_vec()).map_err(|_| TarError::Header)
}

/// The `path` value of a pax record block ("<len> key=value\n" records).
fn pax_path(data: &[u8]) -> Result<Option<String>, TarError> {
    let mut rest = data;
    let mut path = None;
    while !rest.is_empty() && rest[0] != 0 {
        let sp = rest.iter().position(|&b| b == b' ').ok_or(TarError::Header)?;
        let len: usize = std::str::from_utf8(&rest[..sp]).ok().and_then(|s| s.parse().ok()).ok_or(TarError::Header)?;
        if len <= sp + 1 || len > rest.len() || rest[len - 1] != b'\n' {
            return Err(TarError::Header);
        }
        let rec = std::str::from_utf8(&rest[sp + 1..len - 1]).map_err(|_| TarError::Header)?;
        if let Some(v) = rec.strip_prefix("path=") {
            path = Some(v.to_string());
        }
        rest = &rest[len..];
    }
    Ok(path)
}

/// `node_modules/<name>` or `node_modules/@scope/<name>` (at any depth) when `rel` is that package's `package.json`.
fn bundled_dir(rel: &str) -> Option<String> {
    let c: Vec<&str> = rel.split('/').collect();
    let n = c.len();
    if n < 3 || c[n - 1] != "package.json" {
        return None;
    }
    let ok = c[n - 3] == "node_modules" || (n >= 4 && c[n - 3].starts_with('@') && c[n - 4] == "node_modules");
    (ok && c[n - 2] != "node_modules").then(|| c[..n - 1].join("/"))
}

fn parse<R: Read>(inner: R, lim: Limits, io_err: TarError) -> Result<Pack, TarError> {
    let mut r = Counted { inner, read: 0, cap: lim.unpacked, io_err };
    let mut long: Option<String> = None;
    let mut seen = HashSet::new();
    let mut root: Option<String> = None;
    let mut entries = 0usize;
    let (mut manifest, mut shrinkwrap, mut nested) = (None, None, Vec::new());
    loop {
        let mut h = [0u8; 512];
        r.exact(&mut h)?;
        if h.iter().all(|&b| b == 0) {
            break;
        }
        if !checksum_ok(&h) {
            return Err(TarError::Header);
        }
        if h[124] & 0x80 != 0 {
            return Err(TarError::TooBig); // base-256 size: larger than any manifest or cap
        }
        let size = octal(&h[124..136])?;
        let kind = h[156];
        match kind {
            b'x' | b'L' => {
                if size > 1 << 20 {
                    return Err(TarError::Header);
                }
                let data = r.data(size, true)?;
                let p = if kind == b'x' { pax_path(&data)? } else { Some(cstr(&data)?) };
                if p.is_some() {
                    long = p;
                }
                continue;
            }
            b'g' | b'K' => {
                r.data(size, false)?;
                continue;
            }
            _ => {}
        }
        let name = match long.take() {
            Some(n) => n,
            None => {
                let (name, prefix) = (cstr(&h[0..100])?, cstr(&h[345..500])?);
                if &h[257..262] == b"ustar" && !prefix.is_empty() { format!("{prefix}/{name}") } else { name }
            }
        };
        entries += 1;
        if entries > lim.entries {
            return Err(TarError::TooMany);
        }
        if name.is_empty() || name.starts_with('/') {
            return Err(TarError::Path);
        }
        let comps: Vec<&str> = name.split('/').filter(|c| !c.is_empty() && *c != ".").collect();
        if comps.is_empty() || comps.contains(&"..") {
            return Err(TarError::Path);
        }
        match &root {
            None => root = Some(comps[0].to_string()),
            Some(r0) if r0 != comps[0] => return Err(TarError::Roots),
            _ => {}
        }
        let rel = comps[1..].join("/");
        if !seen.insert(comps.join("/")) {
            return Err(TarError::Duplicate);
        }
        let nested_dir = bundled_dir(&rel);
        let inspected = rel == "package.json" || rel == "npm-shrinkwrap.json" || nested_dir.is_some();
        match kind {
            0 | b'0' | b'7' => {
                if inspected && size > lim.manifest as u64 {
                    return Err(TarError::TooBig);
                }
                let data = r.data(size, inspected)?;
                if rel == "package.json" {
                    manifest = Some(data);
                } else if rel == "npm-shrinkwrap.json" {
                    shrinkwrap = Some(data);
                } else if let Some(d) = nested_dir {
                    nested.push((d, data));
                }
            }
            b'5' => {
                r.data(size, false)?;
            }
            b'1' | b'2' => {
                if inspected {
                    return Err(TarError::Link);
                }
                r.data(size, false)?;
            }
            _ => return Err(TarError::Kind),
        }
    }
    Ok(Pack { manifest: manifest.ok_or(TarError::NoManifest)?, shrinkwrap, nested })
}

#[cfg(test)]
#[path = "tar_tests.rs"]
mod tests;
