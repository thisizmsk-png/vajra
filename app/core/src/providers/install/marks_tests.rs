use super::*;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vajra-marks-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("bin")).unwrap();
    std::fs::write(d.join("bin/codex"), b"").unwrap();
    d
}

#[test]
fn a_mark_holds_while_the_binary_and_version_are_the_same() {
    let d = tmp("hold");
    let m = Marks::at(&d);
    let bin = d.join("bin/codex");
    m.set("codex", &bin, Some("0.167.1"), "closure_mismatch").unwrap();
    // `--version` working again is not enough: same binary, same version, still Broken.
    assert_eq!(m.active("codex", &bin, Some("0.167.1")).as_deref(), Some("closure_mismatch"));
    assert_eq!(Marks::at(&d).active("codex", &bin, Some("0.167.1")).as_deref(), Some("closure_mismatch"), "it survives a restart");
    assert_eq!(m.active("claude", &bin, Some("0.167.1")), None, "marks are per harness");
}

#[test]
fn a_mark_clears_on_a_new_version_or_binary_and_stays_cleared() {
    let d = tmp("change");
    let m = Marks::at(&d);
    let bin = d.join("bin/codex");
    m.set("codex", &bin, Some("0.167.1"), "closure_mismatch").unwrap();
    assert_eq!(m.active("codex", &bin, Some("0.168.0")), None, "a new version");
    assert_eq!(m.active("codex", &bin, Some("0.167.1")), None, "the change cleared it for good");
    m.set("codex", &bin, Some("0.167.1"), "closure_mismatch").unwrap();
    std::fs::write(d.join("bin/codex2"), b"").unwrap();
    assert_eq!(m.active("codex", &d.join("bin/codex2"), Some("0.167.1")), None, "another binary");
}

#[test]
fn a_symlink_to_the_same_binary_is_the_same_binary() {
    let d = tmp("link");
    let m = Marks::at(&d);
    let bin = d.join("bin/codex");
    std::os::unix::fs::symlink(&bin, d.join("bin/link")).unwrap();
    m.set("codex", &bin, Some("1.0.0"), "rollback_failed").unwrap();
    assert_eq!(m.active("codex", &d.join("bin/link"), Some("1.0.0")).as_deref(), Some("rollback_failed"));
}

#[test]
fn a_passing_closure_check_clears_it() {
    let d = tmp("pass");
    let m = Marks::at(&d);
    let bin = d.join("bin/codex");
    m.set("codex", &bin, Some("1.0.0"), "closure_mismatch").unwrap();
    m.clear("codex").unwrap();
    assert_eq!(m.active("codex", &bin, Some("1.0.0")), None);
}

#[test]
fn an_unreadable_state_file_marks_nothing_and_is_private() {
    let d = tmp("bad");
    std::fs::write(d.join(FILE), b"{not json").unwrap();
    let m = Marks::at(&d);
    assert_eq!(m.active("codex", &d.join("bin/codex"), None), None);
    m.set("codex", &d.join("bin/codex"), None, "closure_mismatch").unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(d.join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(Marks::none().active("codex", &d.join("bin/codex"), None).is_none());
}
