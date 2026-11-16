//! D7 E2 / bhishma C1, C10: a bundle is used only if it is byte for byte the one this binary pinned, and holds exactly
//! the regular files its manifest lists.

use super::super::pack::{raw_tgz, sha256_hex};
use super::*;

fn manifest(files: &[(&str, &[u8])]) -> Vec<u8> {
    let m: serde_json::Map<String, serde_json::Value> = files.iter().map(|(n, b)| (n.to_string(), sha256_hex(b).into())).collect();
    serde_json::to_vec(&serde_json::json!({ "files": m })).unwrap()
}

const PKG: &[u8] = br#"{"version":"0.6.2"}"#;
const SH: &[u8] = b"echo hi\n";

/// A good engine archive and the pin for it.
fn good() -> (Vec<u8>, Pin) {
    let m = manifest(&[("package.json", PKG), ("scripts/a.sh", SH)]);
    archive(vec![("manifest.json", m, 0o644), ("package.json", PKG.to_vec(), 0o644), ("scripts/a.sh", SH.to_vec(), 0o755)], b'0')
}

fn archive(entries: Vec<(&str, Vec<u8>, u32)>, kind: u8) -> (Vec<u8>, Pin) {
    let e: Vec<(String, Vec<u8>, u32)> = entries.into_iter().map(|(n, b, m)| (n.to_string(), b, m)).collect();
    let unpacked = e.iter().map(|x| x.1.len() as u64).sum();
    let files = e.len();
    let tgz = raw_tgz(&e, kind);
    let pin = Pin { sha256: sha256_hex(&tgz), version: "0.6.2".into(), tree_ok: true, unpacked, files, release: false };
    (tgz, pin)
}

#[test]
fn a_good_bundle_gives_every_file_with_its_mode() {
    let (tgz, pin) = good();
    let e = verify(&tgz, &pin).unwrap();
    assert_eq!(e.version, "0.6.2");
    assert_eq!(e.get("scripts/a.sh").map(|f| (f.mode, f.bytes.as_slice())), Some((0o755, SH)));
    assert_eq!(e.files.len(), 3);
}

#[test]
fn one_flipped_byte_is_a_mismatch_before_anything_is_parsed() {
    let (mut tgz, pin) = good();
    let last = tgz.len() - 1;
    tgz[last] ^= 1;
    assert_eq!(verify(&tgz, &pin).unwrap_err(), Refused::Mismatch);
}

#[test]
fn an_unpinned_build_never_offers_its_bundle() {
    let (tgz, mut pin) = good();
    pin.tree_ok = false;
    let d = std::env::temp_dir().join(format!("vajra-engine-unpinned-{}", std::process::id()));
    std::fs::create_dir_all(d.join("engine")).unwrap();
    std::fs::write(d.join(RESOURCE), &tgz).unwrap();
    assert_eq!(load(&d, &pin).unwrap_err(), Refused::NotPinned);
    pin.tree_ok = true;
    assert!(load(&d, &pin).is_ok());
}

#[test]
fn a_symlinked_or_missing_resource_is_refused() {
    let (tgz, pin) = good();
    let d = std::env::temp_dir().join(format!("vajra-engine-reslink-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("engine")).unwrap();
    assert_eq!(load(&d, &pin).unwrap_err(), Refused::Missing);
    let real = d.join("real.tgz");
    std::fs::write(&real, &tgz).unwrap();
    std::os::unix::fs::symlink(&real, d.join(RESOURCE)).unwrap();
    assert_eq!(load(&d, &pin).unwrap_err(), Refused::Missing, "O_NOFOLLOW");
}

#[test]
fn links_and_other_kinds_are_refused_even_when_pinned() {
    let m = manifest(&[("package.json", PKG)]);
    for kind in *b"123456xL" {
        let (tgz, pin) = archive(vec![("manifest.json", m.clone(), 0o644), ("package.json", PKG.to_vec(), 0o644)], kind);
        assert_eq!(verify(&tgz, &pin).unwrap_err(), Refused::Malformed, "kind {}", kind as char);
    }
}

#[test]
fn unsafe_duplicate_or_case_folded_names_are_refused() {
    let m = manifest(&[("package.json", PKG)]);
    for extra in ["../evil", "/abs", "a/./b", "Package.json", "package.json"] {
        let (tgz, pin) = archive(vec![("manifest.json", m.clone(), 0o644), ("package.json", PKG.to_vec(), 0o644), (extra, b"x".to_vec(), 0o644)], b'0');
        assert_eq!(verify(&tgz, &pin).unwrap_err(), Refused::Malformed, "{extra}");
    }
}

#[test]
fn a_file_missing_from_or_extra_to_the_manifest_or_with_another_hash_is_refused() {
    let m = manifest(&[("package.json", PKG), ("scripts/a.sh", SH)]);
    let missing = archive(vec![("manifest.json", m.clone(), 0o644), ("package.json", PKG.to_vec(), 0o644)], b'0');
    let extra = archive(vec![("manifest.json", m.clone(), 0o644), ("package.json", PKG.to_vec(), 0o644), ("scripts/a.sh", SH.to_vec(), 0o755), ("x", b"x".to_vec(), 0o644)], b'0');
    let other = archive(vec![("manifest.json", m.clone(), 0o644), ("package.json", PKG.to_vec(), 0o644), ("scripts/a.sh", b"echo bye\n".to_vec(), 0o755)], b'0');
    for (what, (tgz, pin)) in [("missing", missing), ("extra", extra), ("other hash", other)] {
        assert_eq!(verify(&tgz, &pin).unwrap_err(), Refused::Malformed, "{what}");
    }
}

#[test]
fn a_setuid_or_odd_mode_is_refused() {
    let m = manifest(&[("package.json", PKG)]);
    for mode in [0o4755, 0o2755, 0o1755, 0o777, 0o600] {
        let (tgz, pin) = archive(vec![("manifest.json", m.clone(), 0o644), ("package.json", PKG.to_vec(), mode)], b'0');
        assert_eq!(verify(&tgz, &pin).unwrap_err(), Refused::Malformed, "{mode:o}");
    }
}

#[test]
fn an_archive_bigger_than_its_pin_is_refused() {
    let (tgz, mut pin) = good();
    pin.files = 2;
    assert_eq!(verify(&tgz, &pin).unwrap_err(), Refused::Malformed, "more entries than packed");
    let big = vec![b'a'; 3 << 20];
    let m = manifest(&[("package.json", PKG), ("big", &big)]);
    let (tgz, mut pin) = archive(vec![("manifest.json", m, 0o644), ("package.json", PKG.to_vec(), 0o644), ("big", big.clone(), 0o644)], b'0');
    pin.unpacked = 1 << 20;
    assert_eq!(verify(&tgz, &pin).unwrap_err(), Refused::Malformed, "more bytes than twice the packed size");
}

#[test]
fn trailing_data_after_the_end_is_refused() {
    let (tgz, _) = good();
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(tgz.as_slice()), &mut raw).unwrap();
    raw.extend_from_slice(b"hidden");
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut gz, &raw).unwrap();
    let tgz = gz.finish().unwrap();
    let pin = Pin { sha256: sha256_hex(&tgz), version: "0.6.2".into(), tree_ok: true, unpacked: raw.len() as u64, files: 3, release: false };
    assert_eq!(verify(&tgz, &pin).unwrap_err(), Refused::Malformed);
}

#[test]
fn the_real_bundle_verifies_exactly_when_its_files_matched_the_manifest() {
    // The manifest is operator-regenerated, so on a feature branch it can be stale: then the build pins nothing, and
    // even forcing tree_ok, the bundle's own hashes disagree and it is refused. On a matching tree it verifies whole.
    let built = Pin::built();
    let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("gen");
    if !built.tree_ok {
        assert_eq!(load(&res, &built).unwrap_err(), Refused::NotPinned);
        assert_eq!(load(&res, &Pin { tree_ok: true, ..built }).unwrap_err(), Refused::Malformed, "stale hashes never pass");
        return;
    }
    let e = load(&res, &built).expect("the bundle build.rs packed");
    assert_eq!(e.files.len(), built.files);
    for f in ["scripts/vajra-campaign.sh", "scripts/vajra-verify-work.sh", "scripts/vajra_tasks.py", "scripts/vajra_kb.py", "scripts/vajra-doctor.sh", "workflows/vajra-deliver.js"] {
        assert!(e.get(f).is_some(), "{f} is in the bundle");
    }
}

#[test]
fn a_bundle_packed_from_a_matching_tree_verifies_and_holds_the_engine_files() {
    // The same as a release would see, from a fake repo whose manifest matches (the packer's own output, end to end).
    let d = std::env::temp_dir().join(format!("vajra-engine-roundtrip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let names = ["package.json", "scripts/vajra-campaign.sh", "workflows/vajra-deliver.js"];
    let mut listed = serde_json::Map::new();
    for n in names {
        let body: &[u8] = if n == "package.json" { PKG } else { SH };
        std::fs::create_dir_all(d.join(n).parent().unwrap()).unwrap();
        std::fs::write(d.join(n), body).unwrap();
        listed.insert(n.into(), sha256_hex(body).into());
    }
    std::fs::write(d.join("manifest.json"), serde_json::to_vec(&serde_json::json!({ "files": listed })).unwrap()).unwrap();
    let p = super::super::pack::pack(&d).unwrap();
    let pin = Pin { sha256: p.sha256.clone(), version: p.version.clone(), tree_ok: p.tree_ok, unpacked: p.unpacked, files: p.files, release: false };
    let e = verify(&p.tgz, &pin).unwrap();
    assert_eq!(e.files.len(), 4);
    assert!(names.iter().all(|n| e.get(n).is_some()));
}
