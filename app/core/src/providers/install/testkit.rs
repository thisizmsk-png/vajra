//! Test fixtures for the install module: a tarball builder and a fake npm toolchain on a temp folder. The fake `node`
//! runs its first argument as a shell script, so `[node, npm-cli.js, ...]` drives a scripted npm that records every call.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::tools::{Npm, Tool};

fn header(name: &str, size: usize) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(b"0000644\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    h[136..148].copy_from_slice(b"00000000000\0");
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

/// A gzip'd tarball of `(path, text)` files.
pub fn tgz(files: &[(&str, &str)]) -> Vec<u8> {
    let mut raw = Vec::new();
    for (name, data) in files {
        raw.extend_from_slice(&header(name, data.len()));
        raw.extend_from_slice(data.as_bytes());
        raw.resize(raw.len().div_ceil(512) * 512, 0);
    }
    raw.extend_from_slice(&[0u8; 1024]);
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(&raw).unwrap();
    e.finish().unwrap()
}

/// A scripted npm under `dir`: `tool/` holds node and npm-cli.js, `prefix/` is the global prefix, `log` records calls.
pub struct FakeNpm {
    pub dir: PathBuf,
}

fn exe(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

impl FakeNpm {
    /// `tarball` is what `npm pack` writes; `ls` what `npm ls -g` prints. `prefix` overrides `npm config get prefix`.
    pub fn new(tag: &str, tarball: &[u8], ls: &str, prefix: Option<&str>) -> Self {
        let dir = std::env::temp_dir().join(format!("vajra-npm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("prefix/lib/node_modules")).unwrap();
        std::fs::create_dir_all(dir.join("prefix/bin")).unwrap();
        let dir = dir.canonicalize().unwrap();
        std::fs::write(dir.join("fixture.tgz"), tarball).unwrap();
        std::fs::write(dir.join("ls.json"), ls).unwrap();
        let d = dir.display();
        let prefix = prefix.map_or_else(|| format!("{d}/prefix"), str::to_string);
        exe(&dir.join("tool/bin/node"), "#!/bin/sh\ns=\"$1\"; shift; exec /bin/sh \"$s\" \"$@\"\n");
        exe(
            &dir.join("tool/lib/node_modules/npm/bin/npm-cli.js"),
            &format!(
                "#!/bin/sh\necho \"$*\" >> {d}/log\nenv > {d}/env.$1\ncase \"$1\" in\n\
                 pack) dest=\"\"; while [ $# -gt 0 ]; do [ \"$1\" = --pack-destination ] && dest=\"$2\"; shift; done; \
                 cp {d}/fixture.tgz \"$dest/pkg.tgz\"; [ -f {d}/extra ] && cp {d}/extra \"$dest/extra\"; echo '[{{\"filename\":\"../../elsewhere.tgz\"}}]';;\n\
                 config) case \"$3\" in prefix) echo {prefix};; *registry) echo https://mirror.example/;; esac;;\n\
                 ls) cat {d}/ls.json;;\n\
                 install) [ -f {d}/install.ls ] && cp {d}/install.ls {d}/ls.json; [ -f {d}/install.fail ] && exit 1; true;;\n\
                 uninstall) echo '{{\"dependencies\":{{}}}}' > {d}/ls.json;;\n\
                 esac\n"
            ),
        );
        FakeNpm { dir }
    }

    pub fn npm(&self) -> Npm {
        let t = |p: &str| Tool { path: self.dir.join(p), label: None, owned_by_you: true };
        Npm { node: t("tool/bin/node"), cli: t("tool/lib/node_modules/npm/bin/npm-cli.js"), prefix: self.dir.join("tool") }
    }

    /// Every npm call so far, one line each (its arguments).
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("log")).unwrap_or_default()
    }
}

/// A scripted `brew` under `dir`: `info` prints `info.json`, `--prefix` prints `dir/prefix`, `install` copies
/// `after.json` over `info.json` (the installed version), `log` records calls.
pub struct FakeBrew {
    pub dir: PathBuf,
}

impl FakeBrew {
    pub fn new(tag: &str, info: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vajra-brew-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["prefix/bin", "prefix/Cellar", "prefix/Caskroom"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let dir = dir.canonicalize().unwrap();
        std::fs::write(dir.join("info.json"), info).unwrap();
        let d = dir.display();
        exe(
            &dir.join("brew/bin/brew"),
            &format!(
                "#!/bin/sh\necho \"$*\" >> {d}/log\ncase \"$1\" in\n\
                 info) cat {d}/info.json;;\n--prefix) echo {d}/prefix;;\n\
                 install) [ -f {d}/after.json ] && cp {d}/after.json {d}/info.json; [ -f {d}/install.fail ] && exit 1; true;;\n\
                 esac\n"
            ),
        );
        FakeBrew { dir }
    }

    pub fn brew(&self) -> Tool {
        Tool { path: self.dir.join("brew/bin/brew"), label: None, owned_by_you: true }
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("log")).unwrap_or_default()
    }
}

impl Drop for FakeBrew {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Drop for FakeNpm {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
