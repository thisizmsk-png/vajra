//! bhishma C1: the engine pack holds exactly the manifest's files, refuses anything that isn't a plain regular file in
//! the repository, and gives the same bytes for the same tree.

use std::path::{Path, PathBuf};

use super::pack::{self, pack, safe_name};
use super::Pin;

fn repo(tag: &str, files: &[(&str, &str, bool)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vajra-engine-pack-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let mut listed = serde_json::Map::new();
    for (name, body, exec) in files {
        let p = d.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        if *exec {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        listed.insert(name.to_string(), pack::sha256_hex(body.as_bytes()).into());
    }
    write_manifest(&d, listed);
    d
}

fn write_manifest(d: &Path, files: serde_json::Map<String, serde_json::Value>) {
    std::fs::write(d.join("manifest.json"), serde_json::to_vec(&serde_json::json!({"version": "0.6.2", "files": files})).unwrap()).unwrap();
}

const BASE: [(&str, &str, bool); 3] = [("package.json", r#"{"version":"0.6.2"}"#, false), ("scripts/a.sh", "echo a\n", true), ("SKILL.md", "# s\n", false)];

/// The entries of a pack: (name, mode, bytes), in archive order.
fn entries(tgz: &[u8]) -> Vec<(String, u32, Vec<u8>)> {
    let mut tar = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(tgz), &mut tar).unwrap();
    let mut out = Vec::new();
    let mut at = 0;
    while at + 512 <= tar.len() && tar[at] != 0 {
        let h = &tar[at..at + 512];
        let field = |a: usize, b: usize| String::from_utf8_lossy(&h[a..b]).trim_end_matches('\0').to_string();
        let prefix = field(345, 500);
        let name = if prefix.is_empty() { field(0, 100) } else { format!("{prefix}/{}", field(0, 100)) };
        let size = u64::from_str_radix(field(124, 135).trim(), 8).unwrap() as usize;
        let mode = u32::from_str_radix(field(100, 107).trim(), 8).unwrap();
        assert_eq!(h[156], b'0', "{name}: regular files only");
        out.push((name, mode, tar[at + 512..at + 512 + size].to_vec()));
        at += 512 + size.div_ceil(512) * 512;
    }
    out
}

#[test]
fn the_pack_is_exactly_the_manifest_files_with_fixed_modes_and_is_reproducible() {
    let d = repo("exact", &BASE);
    std::fs::write(d.join("not-listed.txt"), "x").unwrap();
    let a = pack(&d).unwrap();
    let b = pack(&d).unwrap();
    assert_eq!(a.tgz, b.tgz, "the same tree gives the same bytes");
    assert_eq!(a.sha256, pack::sha256_hex(&a.tgz));
    assert!(a.tree_ok && a.mismatched.is_empty());
    assert_eq!((a.version.as_str(), a.files), ("0.6.2", 4));
    let e = entries(&a.tgz);
    let names: Vec<&str> = e.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(names, ["SKILL.md", "manifest.json", "package.json", "scripts/a.sh"], "manifest files plus the manifest, in byte order; nothing else");
    assert_eq!(e.iter().map(|x| x.1).collect::<Vec<_>>(), [0o644, 0o644, 0o644, 0o755]);
    assert_eq!(e[3].2, b"echo a\n");
    // Touching a file's time changes nothing.
    let f = std::fs::File::options().write(true).open(d.join("SKILL.md")).unwrap();
    f.set_modified(std::time::SystemTime::now()).unwrap();
    assert_eq!(pack(&d).unwrap().sha256, a.sha256);
}

#[test]
fn a_file_that_differs_from_the_manifest_clears_tree_ok_and_is_named() {
    let d = repo("differs", &BASE);
    std::fs::write(d.join("scripts/a.sh"), "echo changed\n").unwrap();
    let p = pack(&d).unwrap();
    assert!(!p.tree_ok);
    assert_eq!(p.mismatched, ["scripts/a.sh"]);
}

#[test]
fn a_listed_symlink_or_a_file_reached_through_one_fails_the_build() {
    let d = repo("link", &BASE);
    let outside = std::env::temp_dir().join(format!("vajra-engine-pack-secret-{}", std::process::id()));
    std::fs::write(&outside, "secret").unwrap();
    std::fs::remove_file(d.join("SKILL.md")).unwrap();
    std::os::unix::fs::symlink(&outside, d.join("SKILL.md")).unwrap();
    assert!(pack(&d).unwrap_err().contains("symlink"), "a listed symlink");
    let d = repo("dirlink", &BASE);
    let real = d.join("real");
    std::fs::rename(d.join("scripts"), &real).unwrap();
    std::os::unix::fs::symlink(&real, d.join("scripts")).unwrap();
    assert!(pack(&d).unwrap_err().contains("symlink"), "a file reached through a symlinked folder");
}

#[test]
fn an_unsafe_manifest_name_fails_the_build() {
    for bad in ["../x", "/etc/passwd", "a/../b", "./a", "a//b", "a/", "", "a\\b", "caf\u{e9}"] {
        assert!(!safe_name(bad), "{bad:?}");
        let d = repo(&format!("bad{}", bad.len()), &BASE);
        let mut files = serde_json::Map::new();
        files.insert(bad.into(), "0".repeat(64).into());
        files.insert("package.json".into(), pack::sha256_hex(br#"{"version":"0.6.2"}"#).into());
        write_manifest(&d, files);
        assert!(pack(&d).unwrap_err().contains("unsafe name"), "{bad:?}");
    }
    assert!(safe_name("bundled-skills/x y/SKILL.md"), "a space is fine");
}

#[test]
fn two_names_that_differ_only_in_case_fail_the_build() {
    let d = repo("case", &BASE);
    let mut files = serde_json::Map::new();
    for n in ["package.json", "Readme.md", "README.md"] {
        files.insert(n.into(), "0".repeat(64).into());
    }
    write_manifest(&d, files);
    assert!(pack(&d).unwrap_err().contains("only in case"));
}

#[test]
fn the_version_range_is_the_tested_one() {
    assert_eq!(pack::engine_version(br#"{"version":"0.6.2"}"#), Some(("0.6.2".into(), true)));
    assert_eq!(pack::engine_version(br#"{"version":"0.5.9"}"#).map(|v| v.1), Some(false));
    assert_eq!(pack::engine_version(br#"{"version":"1.0.0"}"#).map(|v| v.1), Some(false));
    assert_eq!(pack::engine_version(br#"{"version":"x"}"#), None);
}

#[test]
fn this_binary_carries_the_pin_of_the_engine_it_was_built_from() {
    let p = Pin::built();
    assert_eq!(p.sha256.len(), 64);
    assert!(p.files > 100 && p.unpacked > 0, "{p:?}");
    let resource = Path::new(env!("CARGO_MANIFEST_DIR")).join("gen/engine/engine.tgz");
    assert_eq!(pack::sha256_hex(&std::fs::read(resource).unwrap()), p.sha256, "the resource build.rs wrote is the one pinned");
    assert!(!p.release, "tests are never a release build");
}
