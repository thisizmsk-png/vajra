use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::*;

fn me() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Self {
        let d = std::env::temp_dir().join(format!("vajra-admin-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("lib/node_modules/x/node_modules/y")).unwrap();
        std::fs::create_dir_all(d.join("bin")).unwrap();
        std::fs::write(d.join("lib/node_modules/x/node_modules/y/index.js"), "1").unwrap();
        std::fs::write(d.join("bin/x"), "1").unwrap();
        Dir(d.canonicalize().unwrap())
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(self.0.join("bin"), std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn targets(d: &Dir) -> Targets {
    Targets { dirs: vec![d.0.clone(), d.0.join("lib/node_modules"), d.0.join("bin")], trees: vec![d.0.join("lib/node_modules/x")], files: vec![d.0.join("bin/x")] }
}

#[test]
fn a_prefix_the_user_owns_and_can_write_needs_no_admin() {
    let d = Dir::new("ok");
    assert_eq!(check(&targets(&d), me(), 10_000), None);
}

#[test]
fn a_root_owned_prefix_needs_admin() {
    let t = Targets { dirs: vec![PathBuf::from("/var/empty")], trees: vec![], files: vec![] };
    let b = check(&t, me(), 10_000).expect("root-owned");
    assert_eq!((b.path, b.reason), (PathBuf::from("/var/empty"), "root_owned"));
}

#[test]
fn a_folder_the_user_cannot_write_needs_admin() {
    let d = Dir::new("ro");
    std::fs::set_permissions(d.0.join("bin"), std::fs::Permissions::from_mode(0o555)).unwrap();
    let b = check(&targets(&d), me(), 10_000).expect("read-only bin");
    assert_eq!((b.path, b.reason), (d.0.join("bin"), "not_writable"));
}

#[test]
fn anything_inside_the_existing_package_owned_by_someone_else_needs_admin() {
    // Every entry belongs to the test user; checking as another uid stands in for a root-owned file deep inside.
    let d = Dir::new("other");
    let t = Targets { dirs: vec![], trees: vec![d.0.join("lib/node_modules/x")], files: vec![] };
    let b = check(&t, me() + 4242, 10_000).expect("not ours");
    assert_eq!(b.reason, "not_owned");
}

#[test]
fn the_binary_link_owned_by_someone_else_needs_admin() {
    let d = Dir::new("bin");
    let t = Targets { dirs: vec![], trees: vec![], files: vec![d.0.join("bin/x")] };
    assert_eq!(check(&t, me() + 4242, 10_000).map(|b| b.reason), Some("not_owned"));
}

#[test]
fn the_walk_does_not_follow_symlinks_and_is_bounded() {
    let d = Dir::new("walk");
    std::os::unix::fs::symlink("/usr", d.0.join("lib/node_modules/x/escape")).unwrap();
    assert_eq!(check(&targets(&d), me(), 10_000), None, "the link is ours; /usr behind it is never walked");
    let b = check(&targets(&d), me(), 2).expect("too many entries");
    assert_eq!(b.reason, "too_many_files");
}

#[test]
fn missing_targets_are_skipped() {
    let t = Targets { dirs: vec![PathBuf::from("/nonexistent/vajra")], trees: vec![PathBuf::from("/nonexistent/vajra/x")], files: vec![PathBuf::from("/nonexistent/vajra/bin/x")] };
    assert_eq!(check(&t, me(), 10), None);
}

#[test]
fn a_target_folder_that_is_a_dangling_link_needs_admin() {
    let d = Dir::new("dangling");
    let link = d.0.join("gone");
    std::os::unix::fs::symlink(d.0.join("nowhere"), &link).unwrap();
    let t = Targets { dirs: vec![link.clone()], trees: vec![], files: vec![] };
    assert_eq!(check(&t, me(), 100).map(|b| (b.path, b.reason)), Some((link, "not_writable")));
}
