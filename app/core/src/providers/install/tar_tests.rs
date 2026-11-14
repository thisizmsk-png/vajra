use std::io::Write;

use super::*;

/// One ustar header block for `name` (≤ 100 bytes), type `kind`, `size` bytes of data.
fn header(name: &str, kind: u8, size: usize) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(b"0000644\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    h[136..148].copy_from_slice(b"00000000000\0");
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

#[derive(Default)]
struct Tar(Vec<u8>);

impl Tar {
    fn entry(mut self, name: &str, kind: u8, data: &[u8]) -> Self {
        self.0.extend_from_slice(&header(name, kind, data.len()));
        self.0.extend_from_slice(data);
        self.0.resize(self.0.len().div_ceil(512) * 512, 0);
        self
    }
    fn file(self, name: &str, data: &str) -> Self {
        self.entry(name, b'0', data.as_bytes())
    }
    fn pax(self, path: &str) -> Self {
        let body = |n: usize| format!("{n} path={path}\n");
        let mut n = body(0).len();
        while body(n).len() != n {
            n = body(n).len();
        }
        self.entry("PaxHeaders/x", b'x', body(n).as_bytes())
    }
    fn gnu_long(self, path: &str) -> Self {
        self.entry("././@LongLink", b'L', format!("{path}\0").as_bytes())
    }
    fn end(mut self) -> Vec<u8> {
        self.0.extend_from_slice(&[0u8; 1024]);
        self.0
    }
}

fn gz(raw: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(raw).unwrap();
    e.finish().unwrap()
}

const PKG: &str = r#"{"name":"x","version":"1.0.0"}"#;

fn basic() -> Tar {
    Tar::default().file("package/package.json", PKG).file("package/cli.js", "console.log(1)")
}

#[test]
fn reads_the_manifest_and_strips_the_root_folder() {
    let p = read_tgz(&gz(&basic().end()), Limits::default()).unwrap();
    assert_eq!(p.manifest, PKG.as_bytes());
    assert_eq!(p.shrinkwrap, None);
    assert!(p.nested.is_empty());
}

#[test]
fn collects_the_shrinkwrap_and_every_nested_and_scoped_bundled_manifest() {
    let t = basic()
        .file("package/npm-shrinkwrap.json", "{}")
        .file("package/node_modules/a/package.json", "{\"name\":\"a\"}")
        .file("package/node_modules/@s/b/package.json", "{\"name\":\"@s/b\"}")
        .file("package/node_modules/a/node_modules/c/package.json", "{\"name\":\"c\"}")
        .file("package/node_modules/a/lib/package.json", "{\"not\":\"a manifest\"}")
        .end();
    let p = read_tgz(&gz(&t), Limits::default()).unwrap();
    assert_eq!(p.shrinkwrap.as_deref(), Some(b"{}".as_slice()));
    let mut names: Vec<String> = p.nested.iter().map(|(n, _)| n.clone()).collect();
    names.sort();
    assert_eq!(names, ["node_modules/@s/b", "node_modules/a", "node_modules/a/node_modules/c"]);
}

#[test]
fn reads_pax_and_gnu_long_names() {
    let long = format!("package/node_modules/{}/package.json", "n".repeat(120));
    let t = Tar::default().file("package/package.json", PKG).pax(&long).file("package/short1", "{}").gnu_long(&long.replace("nnn", "mmm")).file("package/short2", "{}").end();
    let p = read_tgz(&gz(&t), Limits::default()).unwrap();
    assert_eq!(p.nested.len(), 2, "{:?}", p.nested.iter().map(|n| &n.0).collect::<Vec<_>>());
}

#[test]
fn refuses_a_second_root_folder() {
    let t = basic().file("other/x", "1").end();
    assert_eq!(read_tgz(&gz(&t), Limits::default()).unwrap_err(), TarError::Roots);
}

#[test]
fn refuses_absolute_and_dot_dot_paths() {
    for bad in ["/package/x", "package/../x", "package/a/../../x"] {
        let t = basic().file(bad, "1").end();
        assert_eq!(read_tgz(&gz(&t), Limits::default()).unwrap_err(), TarError::Path, "{bad}");
    }
}

#[test]
fn refuses_duplicate_entries() {
    let t = basic().file("package/package.json", PKG).end();
    assert_eq!(read_tgz(&gz(&t), Limits::default()).unwrap_err(), TarError::Duplicate);
}

#[test]
fn refuses_a_link_at_an_inspected_manifest() {
    let sym = Tar::default().entry("package/package.json", b'2', b"").end();
    assert_eq!(read_tgz(&gz(&sym), Limits::default()).unwrap_err(), TarError::Link);
    let hard = basic().entry("package/node_modules/a/package.json", b'1', b"").end();
    assert_eq!(read_tgz(&gz(&hard), Limits::default()).unwrap_err(), TarError::Link);
}

#[test]
fn refuses_devices_and_fifos() {
    for kind in *b"346" {
        let t = basic().entry("package/dev", kind, b"").end();
        assert_eq!(read_tgz(&gz(&t), Limits::default()).unwrap_err(), TarError::Kind);
    }
}

#[test]
fn caps_the_unpacked_size_and_the_entry_count() {
    let big = basic().file("package/big", &"x".repeat(4096)).end();
    assert_eq!(read_tgz(&gz(&big), Limits { unpacked: 2048, entries: 100, manifest: 1 << 20 }).unwrap_err(), TarError::TooBig);
    let many = (0..20).fold(basic(), |t, i| t.file(&format!("package/f{i}"), "1")).end();
    assert_eq!(read_tgz(&gz(&many), Limits { unpacked: 1 << 20, entries: 10, manifest: 1 << 20 }).unwrap_err(), TarError::TooMany);
}

#[test]
fn refuses_a_tarball_with_no_manifest_and_a_bad_checksum() {
    let none = Tar::default().file("package/cli.js", "1").end();
    assert_eq!(read_tgz(&gz(&none), Limits::default()).unwrap_err(), TarError::NoManifest);
    let mut bad = basic().end();
    bad[148] = b'7';
    assert_eq!(read_tgz(&gz(&bad), Limits::default()).unwrap_err(), TarError::Header);
    assert_eq!(read_tgz(b"not gzip", Limits::default()).unwrap_err(), TarError::Gzip);
}

/// A small deterministic PRNG so the fuzz run is the same every time.
struct XorShift(u64);
impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn the_reader_never_panics_on_mutated_or_random_input() {
    let raw = basic().file("package/npm-shrinkwrap.json", "{}").pax(&format!("package/{}", "p".repeat(150))).file("package/x", "1").end();
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    let small = Limits { unpacked: 1 << 20, entries: 1000, manifest: 1 << 16 };
    for i in 0..3000 {
        let mut t = raw.clone();
        for _ in 0..(1 + rng.next() % 8) {
            let at = (rng.next() as usize) % t.len();
            t[at] = rng.next() as u8;
        }
        if i % 7 == 0 {
            t.truncate((rng.next() as usize) % t.len());
        }
        let _ = parse_tar(&t, small);
        let _ = read_tgz(&gz(&t), small);
        let noise: Vec<u8> = (0..(rng.next() % 2048)).map(|_| rng.next() as u8).collect();
        let _ = read_tgz(&noise, small);
        let _ = parse_tar(&noise, small);
    }
}
