//! Plan #215 L1 and L2: the list of proven builds is read once, from one no-follow descriptor, and only a plain file
//! this user owns, that no one else can write and that has one link counts. Anything else is an empty list, with a
//! plain reason. Temp data folders only.

use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("vajra-canary-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("data/control")).unwrap();
        Self(p.canonicalize().unwrap())
    }
    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn body(pairs: &[(&str, &str)]) -> String {
    json!({"pairs": pairs.iter().map(|(v, s)| json!({"version": v, "shape": s, "test": claude_selftest::VERSION})).collect::<Vec<_>>()}).to_string()
}

const PAIR: (&str, &str) = ("9.9.9 (Claude Code)", "shape-a");

#[test]
fn the_list_add_pair_writes_is_read_back() {
    let t = Tmp::new("roundtrip");
    add_pair(&t.data(), PAIR.0, PAIR.1).unwrap();
    assert_eq!(local_pairs(&t.data()), vec![(PAIR.0.to_string(), PAIR.1.to_string())]);
    let m = fs::metadata(list_path(&t.data())).unwrap();
    assert_eq!(m.permissions().mode() & 0o777, 0o600, "the list is written private");
}

#[test]
fn a_symlinked_list_counts_as_empty() {
    let t = Tmp::new("symlink");
    let elsewhere = t.0.join("planted.json");
    fs::write(&elsewhere, body(&[PAIR])).unwrap();
    fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&elsewhere, list_path(&t.data())).unwrap();
    assert!(read_list(&list_path(&t.data())).unwrap_err().contains("is a link"));
    assert!(local_pairs(&t.data()).is_empty());
}

#[test]
fn a_group_or_world_writable_list_counts_as_empty() {
    let t = Tmp::new("groupw");
    let p = list_path(&t.data());
    for mode in [0o620, 0o602, 0o666] {
        fs::write(&p, body(&[PAIR])).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
        assert!(read_list(&p).unwrap_err().contains("can be changed by other users"), "{mode:o}");
        assert!(local_pairs(&t.data()).is_empty(), "{mode:o}");
    }
    // The control: the same list at 0600 counts.
    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(local_pairs(&t.data()).len(), 1);
}

#[test]
fn a_hardlinked_list_counts_as_empty() {
    let t = Tmp::new("hardlink");
    let p = list_path(&t.data());
    fs::write(&p, body(&[PAIR])).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&p, t.0.join("second-name.json")).unwrap();
    assert!(read_list(&p).unwrap_err().contains("hard link"));
    assert!(local_pairs(&t.data()).is_empty());
}

#[test]
fn a_directory_or_fifo_list_counts_as_empty_without_stalling() {
    let t = Tmp::new("notfile");
    let p = list_path(&t.data());
    fs::create_dir(&p).unwrap();
    assert!(local_pairs(&t.data()).is_empty());
    fs::remove_dir(&p).unwrap();
    let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let start = Instant::now();
    assert!(read_list(&p).unwrap_err().contains("isn't a regular file"));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_missing_list_is_empty_with_no_reason() {
    let t = Tmp::new("missing");
    assert_eq!(read_list(&list_path(&t.data())), Ok(None));
    assert!(local_pairs(&t.data()).is_empty());
}
