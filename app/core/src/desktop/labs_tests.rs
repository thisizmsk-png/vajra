use super::*;

const P1: &str = "p_01HZX5Q8B3K9M2N4P6R8T0V2W4";
const P2: &str = "p_01HZX5Q8B3K9M2N4P6R8T0V2W5";

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vajra-labs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn labs_is_per_project_and_off_by_default() {
    let d = tmp("per");
    let l = Labs::new(&d);
    const { assert!(LABS_BUILD, "tests run in a developer build") };
    assert!(!l.on(P1) && !l.on(P2), "no file: Labs is off everywhere");
    l.set(P1, true).unwrap();
    assert!(l.on(P1) && !l.on(P2));
    l.set(P1, true).unwrap();
    l.set(P2, true).unwrap();
    l.set(P1, false).unwrap();
    assert!(!l.on(P1) && l.on(P2));
    assert_eq!(l.set("not a project", true), Err("invalid_project"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_release_build_has_no_labs_even_with_the_file_on_disk() {
    let d = tmp("rel");
    Labs::new(&d).set(P1, true).unwrap();
    let r = Labs::release(&d);
    assert!(!r.on(P1), "a release build never reads the switch");
    assert_eq!(r.set(P1, true), Err("labs_unavailable"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_bad_or_linked_file_is_labs_off() {
    let d = tmp("bad");
    let l = Labs::new(&d);
    for body in ["not json", r#"{"v": 2, "projects": ["p_01HZX5Q8B3K9M2N4P6R8T0V2W4"]}"#, r#"{"v": 1, "projects": ["../x"]}"#] {
        std::fs::write(d.join("labs.json"), body).unwrap();
        assert!(!l.on(P1), "{body}");
    }
    let real = d.join("real.json");
    std::fs::write(&real, format!(r#"{{"v": 1, "projects": ["{P1}"]}}"#)).unwrap();
    std::fs::remove_file(d.join("labs.json")).unwrap();
    std::os::unix::fs::symlink(&real, d.join("labs.json")).unwrap();
    assert!(!l.on(P1), "a linked switch file is not read");
    let _ = std::fs::remove_dir_all(&d);
}
