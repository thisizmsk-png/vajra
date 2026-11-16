//! bhishma C2: every write is through a held directory, never following a link.

use super::*;

fn tmp(tag: &str) -> (std::path::PathBuf, Dir) {
    let d = std::env::temp_dir().canonicalize().unwrap().join(format!("vajra-fsat-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir(&d).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
    let dir = Dir::open_path(&d, euid()).unwrap();
    (d, dir)
}

#[test]
fn open_path_refuses_a_symlink_anywhere_on_the_way() {
    let (d, _) = tmp("openlink");
    std::fs::create_dir(d.join("real")).unwrap();
    std::os::unix::fs::symlink(d.join("real"), d.join("link")).unwrap();
    assert!(Dir::open_path(&d.join("real"), euid()).is_ok());
    assert!(Dir::open_path(&d.join("link"), euid()).is_err(), "a link as the last part");
    std::fs::create_dir(d.join("real/inner")).unwrap();
    assert!(Dir::open_path(&d.join("link/inner"), euid()).is_err(), "a link in the middle");
}

#[test]
fn open_path_refuses_a_group_writable_folder() {
    let (d, _) = tmp("gw");
    std::fs::create_dir(d.join("open")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(d.join("open"), std::fs::Permissions::from_mode(0o775)).unwrap();
    assert_eq!(Dir::open_path(&d.join("open"), euid()).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn write_new_never_reuses_or_follows_an_existing_entry() {
    let (d, dir) = tmp("excl");
    let target = d.join("target");
    std::fs::write(&target, "keep").unwrap();
    std::os::unix::fs::symlink(&target, d.join("planted")).unwrap();
    assert!(dir.write_new("planted", b"evil", 0o644).is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep", "the link's target is untouched");
    dir.write_new("fresh", b"ok", 0o755).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(d.join("fresh")).unwrap().permissions().mode() & 0o777, 0o755, "exact mode, whatever the umask");
    assert!(dir.write_new("fresh", b"again", 0o644).is_err());
}

#[test]
fn open_dir_refuses_a_link_and_make_dir_sets_the_exact_mode() {
    let (d, dir) = tmp("dirs");
    std::fs::create_dir(d.join("real")).unwrap();
    std::os::unix::fs::symlink(d.join("real"), d.join("link")).unwrap();
    assert!(dir.open_dir("link").is_err());
    let made = dir.make_dir("private", 0o700).unwrap();
    assert_eq!(made.stat().unwrap().mode, 0o700);
    assert!(dir.make_dir("private", 0o700).is_err(), "never reuses");
    assert!(dir.open_or_make_dir("private", 0o700).is_ok());
}

#[test]
fn names_must_be_one_plain_component() {
    let (_, dir) = tmp("names");
    for bad in ["", ".", "..", "a/b", "../x"] {
        assert!(dir.write_new(bad, b"x", 0o644).is_err(), "{bad:?}");
    }
}

#[test]
fn remove_tree_removes_a_link_inside_but_never_what_it_points_to() {
    let (d, dir) = tmp("rmtree");
    let outside = d.join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("precious"), "x").unwrap();
    std::fs::create_dir_all(d.join("tree/sub")).unwrap();
    std::fs::write(d.join("tree/sub/f"), "x").unwrap();
    std::os::unix::fs::symlink(&outside, d.join("tree/escape")).unwrap();
    dir.remove_tree("tree").unwrap();
    assert!(!d.join("tree").exists());
    assert!(outside.join("precious").exists(), "the link was removed, not followed");
}

#[test]
fn rename_swap_exchanges_and_exclusive_refuses_an_existing_target() {
    let (d, dir) = tmp("rename");
    std::fs::create_dir(d.join("a")).unwrap();
    std::fs::write(d.join("a/which"), "a").unwrap();
    std::fs::create_dir(d.join("b")).unwrap();
    std::fs::write(d.join("b/which"), "b").unwrap();
    dir.rename("a", &dir, "b", Rename::Swap).unwrap();
    assert_eq!(std::fs::read_to_string(d.join("b/which")).unwrap(), "a");
    assert_eq!(std::fs::read_to_string(d.join("a/which")).unwrap(), "b");
    assert!(dir.rename("a", &dir, "b", Rename::Exclusive).is_err());
    dir.rename("a", &dir, "c", Rename::Exclusive).unwrap();
    assert!(d.join("c/which").exists() && !d.join("a").exists());
}

#[test]
fn read_refuses_a_link_a_hard_link_or_a_big_file() {
    let (d, dir) = tmp("read");
    std::fs::write(d.join("f"), "hello").unwrap();
    assert_eq!(dir.read("f", 10).unwrap(), b"hello");
    assert!(dir.read("f", 2).is_err(), "over the limit");
    std::fs::hard_link(d.join("f"), d.join("hard")).unwrap();
    assert!(dir.read("f", 10).is_err(), "two links");
    std::os::unix::fs::symlink(d.join("hard"), d.join("soft")).unwrap();
    assert!(dir.read("soft", 10).is_err());
}

#[test]
fn stat_at_does_not_follow_and_lists_names() {
    let (d, dir) = tmp("stat");
    std::fs::write(d.join("f"), "x").unwrap();
    std::os::unix::fs::symlink(d.join("f"), d.join("l")).unwrap();
    assert_eq!(dir.stat_at("l").unwrap().unwrap().kind, Kind::Link);
    assert_eq!(dir.stat_at("f").unwrap().unwrap().kind, Kind::File);
    assert_eq!(dir.stat_at("nope").unwrap(), None);
    assert_eq!(dir.list().unwrap(), ["f", "l"]);
    assert!(dir.free_bytes().unwrap() > 0);
}
