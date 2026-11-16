//! bhishma C3: the core writes the workflows itself and never writes through a link.

use super::super::fsat::{euid, Dir};
use super::*;

fn dir(tag: &str) -> (std::path::PathBuf, Dir) {
    let d = std::env::temp_dir().canonicalize().unwrap().join(format!("vajra-wf-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir(&d).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
    let held = Dir::open_path(&d, euid()).unwrap();
    (d, held)
}

fn want() -> BTreeMap<String, Vec<u8>> {
    [("vajra-deliver.js".to_string(), b"new deliver".to_vec()), ("vajra-review.js".to_string(), b"new review".to_vec())].into()
}

#[test]
fn only_the_engines_own_workflows_are_shipped() {
    use super::super::bundle::{Engine, File};
    let f = |n: &str| File { name: n.into(), mode: 0o644, bytes: n.as_bytes().to_vec() };
    let e = Engine { files: vec![f("workflows/vajra-deliver.js"), f("workflows/other.js"), f("workflows/sub/vajra-x.js"), f("scripts/vajra-x.js")], manifest: Default::default(), version: "0.6.2".into() };
    assert_eq!(shipped(&e).keys().collect::<Vec<_>>(), ["vajra-deliver.js"]);
}

#[test]
fn it_writes_what_differs_and_restores_exactly() {
    let (d, held) = dir("write");
    std::fs::write(d.join("vajra-deliver.js"), "old deliver").unwrap();
    std::fs::write(d.join("user-own.js"), "mine").unwrap();
    let snap = snapshot(&held, want().into_keys()).unwrap();
    assert_eq!(changes(&snap, &want()), ["vajra-deliver.js", "vajra-review.js"]);
    write(&held, &want(), &snap).unwrap();
    assert_eq!(std::fs::read_to_string(d.join("vajra-deliver.js")).unwrap(), "new deliver");
    assert_eq!(changes(&snapshot(&held, want().into_keys()).unwrap(), &want()), Vec::<String>::new(), "the installer then finds nothing to copy");
    restore(&held, &snap).unwrap();
    assert_eq!(std::fs::read_to_string(d.join("vajra-deliver.js")).unwrap(), "old deliver");
    assert!(!d.join("vajra-review.js").exists(), "a workflow that wasn't there is removed again");
    assert_eq!(std::fs::read_to_string(d.join("user-own.js")).unwrap(), "mine", "other files untouched");
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 2, "no temp files left");
}

#[test]
fn a_hard_linked_workflow_is_refused_and_its_target_is_untouched() {
    let (d, held) = dir("hard");
    let settings = d.join("settings.json");
    std::fs::write(&settings, "{\"keep\":true}").unwrap();
    std::fs::hard_link(&settings, d.join("vajra-deliver.js")).unwrap();
    use std::os::unix::fs::MetadataExt;
    let ino = std::fs::metadata(&settings).unwrap().ino();
    assert_eq!(snapshot(&held, want().into_keys()).unwrap_err(), Refused::Link);
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{\"keep\":true}");
    assert_eq!(std::fs::metadata(&settings).unwrap().ino(), ino);
}

#[test]
fn a_symlinked_workflow_or_folder_is_refused() {
    let (d, held) = dir("soft");
    std::fs::write(d.join("target"), "t").unwrap();
    std::os::unix::fs::symlink(d.join("target"), d.join("vajra-review.js")).unwrap();
    assert_eq!(snapshot(&held, want().into_keys()).unwrap_err(), Refused::Link);
    assert_eq!(std::fs::read_to_string(d.join("target")).unwrap(), "t");
    // The workflows folder itself a link: refused when it is opened.
    let (d2, claude) = dir("folder");
    std::fs::create_dir(d2.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(d2.join("elsewhere"), d2.join("workflows")).unwrap();
    assert_eq!(open(&claude, euid()).unwrap_err(), Refused::NotPrivate);
}
