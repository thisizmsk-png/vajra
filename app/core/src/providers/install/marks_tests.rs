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
fn concurrent_runs_with_their_own_marks_never_lose_each_others_marks() {
    // Each run builds its own Marks for the same file (as flow::run does). Unlocked, these read the same state and
    // overwrite each other, or one unlinks the other's temp file; every mark set must survive, and only the clears go.
    let d = tmp("race");
    let bin = d.join("bin/codex");
    let ids: Vec<String> = (0..24).map(|i| format!("h{i}")).collect();
    for round in 0..5 {
        std::thread::scope(|s| {
            for (i, id) in ids.iter().enumerate() {
                let (d, bin) = (&d, &bin);
                s.spawn(move || {
                    let m = Marks::at(d);
                    if i % 3 == 0 {
                        m.clear(id).unwrap();
                    } else {
                        m.set(id, bin, Some("1.0.0"), "closure_mismatch").unwrap();
                    }
                });
            }
        });
        let m = Marks::at(&d);
        for (i, id) in ids.iter().enumerate() {
            let want = (i % 3 != 0).then_some("closure_mismatch");
            assert_eq!(m.active(id, &bin, Some("1.0.0")).as_deref(), want, "round {round}: {id}");
        }
    }
    let left: Vec<_> = std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.ends_with(".tmp")).collect();
    assert!(left.is_empty(), "no temp files left behind: {left:?}");
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
