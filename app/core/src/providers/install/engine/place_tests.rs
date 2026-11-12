//! D7 E3/E4, bhishma C2, C5, C6: whose copy it is, staging, the swap, the journal and the backup rule.

use std::path::{Path, PathBuf};

use super::super::bundle::{Engine, File};
use super::super::pack::sha256_hex;
use super::*;

struct World {
    home: PathBuf,
    data: PathBuf,
}

impl World {
    fn new(tag: &str) -> World {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!("vajra-place-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, data) = (root.join("home"), root.join("data"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        use std::os::unix::fs::PermissionsExt;
        for d in [&root, &home, &data] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        World { home, data }
    }
    fn place(&self) -> Place {
        Place::open(&self.home, &self.data, uid()).unwrap()
    }
    fn skill(&self) -> PathBuf {
        self.home.join(".claude/skills/vajra")
    }
}

fn engine(body: &str) -> Engine {
    let files = [("package.json", r#"{"version":"0.6.2"}"#), ("scripts/a.sh", body), ("hooks/x/deep.sh", "deep")];
    let manifest: serde_json::Map<String, serde_json::Value> = files.iter().map(|(n, b)| (n.to_string(), sha256_hex(b.as_bytes()).into())).collect();
    let mut all: Vec<File> = files.iter().map(|(n, b)| File { name: n.to_string(), mode: if n.ends_with(".sh") { 0o755 } else { 0o644 }, bytes: b.as_bytes().to_vec() }).collect();
    all.push(File { name: "manifest.json".into(), mode: 0o644, bytes: serde_json::to_vec(&serde_json::json!({ "files": manifest })).unwrap() });
    Engine { manifest: manifest.into_iter().map(|(k, v)| (k, v.as_str().unwrap().to_string())).collect(), version: "0.6.2".into(), files: all }
}

/// A full fresh install, committed: the state after a good first run.
fn installed(w: &World, body: &str) -> Place {
    let p = w.place();
    let staged = p.stage(&engine(body), "aa").unwrap();
    let j = p.swap_in(&staged, &Found::Absent).unwrap();
    assert_eq!(p.commit(&j, "0.6.2", "aa").unwrap(), None);
    p
}

#[test]
fn a_fresh_install_lands_every_file_with_its_mode_and_the_work_area_is_private() {
    let w = World::new("fresh");
    let p = installed(&w, "echo a");
    assert_eq!(std::fs::read_to_string(w.skill().join("scripts/a.sh")).unwrap(), "echo a");
    assert_eq!(std::fs::read_to_string(w.skill().join("hooks/x/deep.sh")).unwrap(), "deep");
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode(&w.skill().join("scripts/a.sh")), 0o755);
    assert_eq!(mode(&w.skill().join("package.json")), 0o644);
    assert_eq!(mode(&w.skill()), 0o755);
    assert_eq!(mode(&w.home.join(".claude/.vajra-install")), 0o700);
    assert!(matches!(p.found(), Found::Ours { ref version, .. } if version == "0.6.2"));
    assert!(p.journal().is_none(), "a committed install leaves no journal");
    assert!(std::fs::read_dir(w.home.join(".claude/skills")).unwrap().count() == 1, "nothing else under skills/");
}

#[test]
fn a_git_checkout_a_link_or_an_unrecorded_copy_is_the_users_and_never_touched() {
    // .git as a folder, and as a file (a worktree).
    for git in ["dir", "file"] {
        let w = World::new(&format!("git{git}"));
        let p = installed(&w, "echo a");
        let g = w.skill().join(".git");
        if git == "dir" { std::fs::create_dir(&g).unwrap() } else { std::fs::write(&g, "gitdir: x").unwrap() }
        assert_eq!(p.found(), Found::UserCopy, ".git {git}");
    }
    let w = World::new("link");
    let p = w.place();
    std::fs::create_dir_all(w.home.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(w.home.join("elsewhere"), w.skill()).unwrap();
    assert_eq!(p.found(), Found::UserCopy);
    let w = World::new("nomarker");
    let p = w.place();
    std::fs::create_dir_all(w.skill()).unwrap();
    assert_eq!(p.found(), Found::UserCopy, "a folder the app has no record of");
}

#[test]
fn a_forged_marker_without_the_apps_record_is_still_the_users() {
    let w = World::new("forged");
    let p = w.place();
    std::fs::create_dir_all(w.skill()).unwrap();
    std::fs::write(w.skill().join(MARKER), serde_json::to_vec(&Marker { version: "0.6.2".into(), tgz_sha256: "aa".into() }).unwrap()).unwrap();
    assert_eq!(p.found(), Found::UserCopy);
}

#[test]
fn a_record_for_another_folder_does_not_make_a_copy_ours() {
    // The marker is copied into a replacement folder: same text, but a different (dev, ino) than the app recorded.
    let w = World::new("moved");
    let p = installed(&w, "echo a");
    let marker = std::fs::read(w.skill().join(MARKER)).unwrap();
    std::fs::rename(w.skill(), w.home.join("old")).unwrap();
    std::fs::create_dir(w.skill()).unwrap();
    std::fs::write(w.skill().join(MARKER), marker).unwrap();
    assert_eq!(p.found(), Found::UserCopy);
}

#[test]
fn something_planted_at_the_target_after_the_plan_fails_a_fresh_install() {
    let w = World::new("planted");
    let p = w.place();
    let staged = p.stage(&engine("echo a"), "aa").unwrap();
    std::fs::create_dir(w.skill()).unwrap();
    assert_eq!(p.swap_in(&staged, &Found::Absent).unwrap_err(), Unsafe::TargetChanged);
    assert_eq!(std::fs::read_dir(w.skill()).unwrap().count(), 0, "the planted folder is untouched");
}

#[test]
fn an_update_swaps_atomically_so_the_target_always_exists() {
    let w = World::new("swap");
    let p = installed(&w, "echo old");
    let Found::Ours { .. } = p.found() else { panic!("ours") };
    let found = p.found();
    let staged = p.stage(&engine("echo new"), "bb").unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let probe = {
        let (stop, target) = (stop.clone(), w.skill().join("package.json"));
        std::thread::spawn(move || {
            let mut missing = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                if !target.exists() {
                    missing += 1;
                }
            }
            missing
        })
    };
    let j = p.swap_in(&staged, &found).unwrap();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(probe.join().unwrap(), 0, "vajra was never missing");
    assert_eq!(std::fs::read_to_string(w.skill().join("scripts/a.sh")).unwrap(), "echo new");
    assert!(j.previous.is_some());
    assert_eq!(p.commit(&j, "0.6.2", "bb").unwrap(), None, "an unmodified previous copy is deleted");
}

#[test]
fn a_modified_previous_copy_is_kept_as_a_backup_never_deleted() {
    let w = World::new("backup");
    let p = installed(&w, "echo old");
    std::fs::write(w.skill().join("scripts/a.sh"), "my own edit").unwrap();
    std::fs::write(w.skill().join("notes.txt"), "mine").unwrap();
    let found = p.found();
    let staged = p.stage(&engine("echo new"), "bb").unwrap();
    let j = p.swap_in(&staged, &found).unwrap();
    let kept = p.commit(&j, "0.6.2", "bb").unwrap().expect("a backup");
    let backup = w.home.join(".claude/.vajra-install").join(&kept);
    assert_eq!(std::fs::read_to_string(backup.join("scripts/a.sh")).unwrap(), "my own edit");
    assert_eq!(std::fs::read_to_string(backup.join("notes.txt")).unwrap(), "mine");
}

#[test]
fn a_rollback_restores_the_previous_copy_byte_for_byte() {
    let w = World::new("rollback");
    let p = installed(&w, "echo old");
    let found = p.found();
    let staged = p.stage(&engine("echo new"), "bb").unwrap();
    let j = p.swap_in(&staged, &found).unwrap();
    p.roll_back(&j).unwrap();
    assert_eq!(std::fs::read_to_string(w.skill().join("scripts/a.sh")).unwrap(), "echo old");
    assert!(matches!(p.found(), Found::Ours { .. }), "the restored copy is still ours (same folder identity)");
    assert_eq!(p.work.list().unwrap(), Vec::<String>::new(), "nothing left in the work area");
    // A fresh install rolled back leaves no target.
    let w = World::new("rollback-fresh");
    let p = w.place();
    let staged = p.stage(&engine("echo a"), "aa").unwrap();
    let j = p.swap_in(&staged, &Found::Absent).unwrap();
    p.roll_back(&j).unwrap();
    assert!(!w.skill().exists());
}

#[test]
fn recovery_acts_only_on_what_the_journal_names() {
    // A crash after the swap: the journal says Swapped; recovery puts the previous copy back.
    let w = World::new("recover");
    let p = installed(&w, "echo old");
    let found = p.found();
    let staged = p.stage(&engine("echo new"), "bb").unwrap();
    p.swap_in(&staged, &found).unwrap();
    // A folder a worker planted in the work area, named like a previous copy.
    std::fs::create_dir(w.home.join(".claude/.vajra-install/previous-planted")).unwrap();
    assert_eq!(p.strays(), ["previous-planted"]);
    assert!(p.recover().unwrap());
    assert_eq!(std::fs::read_to_string(w.skill().join("scripts/a.sh")).unwrap(), "echo old");
    assert!(w.home.join(".claude/.vajra-install/previous-planted").exists(), "not in the journal: left alone");
    assert!(!p.recover().unwrap(), "no journal, nothing to do");
}

#[test]
fn recovery_refuses_a_previous_copy_whose_identity_changed() {
    let w = World::new("recover-swapped-dir");
    let p = installed(&w, "echo old");
    let found = p.found();
    let staged = p.stage(&engine("echo new"), "bb").unwrap();
    let j = p.swap_in(&staged, &found).unwrap();
    let (prev, ..) = j.previous.clone().unwrap();
    let prev = w.home.join(".claude/.vajra-install").join(prev);
    std::fs::remove_dir_all(&prev).unwrap();
    std::fs::create_dir(&prev).unwrap();
    assert_eq!(p.recover().unwrap_err(), Unsafe::TargetChanged, "a different folder under the same name is never moved in");
    assert_eq!(std::fs::read_to_string(w.skill().join("scripts/a.sh")).unwrap(), "echo new");
}

#[test]
fn a_linked_or_open_claude_folder_is_refused() {
    let w = World::new("claude-link");
    std::fs::create_dir(w.home.join("other")).unwrap();
    std::os::unix::fs::symlink(w.home.join("other"), w.home.join(".claude")).unwrap();
    assert_eq!(Place::open(&w.home, &w.data, uid()).err(), Some(Unsafe::NotPrivate));
    let w = World::new("skills-link");
    std::fs::create_dir_all(w.home.join(".claude")).unwrap();
    std::fs::create_dir(w.home.join("other")).unwrap();
    std::os::unix::fs::symlink(w.home.join("other"), w.home.join(".claude/skills")).unwrap();
    assert_eq!(Place::open(&w.home, &w.data, uid()).err(), Some(Unsafe::NotPrivate));
    let w = World::new("claude-gw");
    std::fs::create_dir_all(w.home.join(".claude")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(w.home.join(".claude"), std::fs::Permissions::from_mode(0o770)).unwrap();
    assert_eq!(Place::open(&w.home, &w.data, uid()).err(), Some(Unsafe::NotPrivate));
}

#[test]
fn staging_that_fails_partway_leaves_nothing_behind() {
    let w = World::new("stage-fail");
    let p = w.place();
    let mut e = engine("echo a");
    // A file whose parent is also a file: the second write must fail.
    e.files.push(File { name: "package.json/inner".into(), mode: 0o644, bytes: b"x".to_vec() });
    assert!(p.stage(&e, "aa").is_err());
    assert_eq!(p.work.list().unwrap(), Vec::<String>::new());
    assert!(p.has_space(1));
    assert!(!p.has_space(u64::MAX / 4));
}
