use std::fs;

use serde_json::Value;

use super::*;

fn setup(name: &str) -> (std::path::PathBuf, App) {
    let root = fs::canonicalize(std::env::temp_dir()).unwrap().join(format!("vajra-projcmd-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let home = root.join("home");
    fs::create_dir_all(home.join(".claude")).unwrap();
    let app = App::open(&root.join("data"), &home).unwrap();
    (root, app)
}

fn repo(root: &std::path::Path, rel: &str) -> String {
    let p = root.join("home").join(rel);
    fs::create_dir_all(p.join(".git/objects")).unwrap();
    fs::write(p.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    p.to_string_lossy().into_owned()
}

fn err(r: Res) -> Value {
    r.expect_err("expected a refusal")
}

#[test]
fn app_info_names_the_record_versions() {
    let (root, app) = setup("info");
    let v = app.app_info().unwrap();
    assert_eq!(v["schema"], "vajra.app_info/1");
    assert_eq!(v["contracts"]["project"], 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn add_list_rename_archive_through_the_commands() {
    let (root, app) = setup("flow");
    let a = repo(&root, "src/alpha");
    let added = app.project_add(&a, false).unwrap();
    assert_eq!(added["outcome"], "created");
    let id = added["project"]["id"].as_str().unwrap().to_string();
    assert_eq!(app.project_add(&a, false).unwrap()["outcome"], "existing");
    assert_eq!(app.project_list(false).unwrap()["projects"].as_array().unwrap().len(), 1);
    let renamed = app.project_rename(&id, "Alpha", Some(1)).unwrap();
    assert_eq!(renamed["project"]["rev"], 2);
    let stale = err(app.project_rename(&id, "Other", Some(1)));
    assert_eq!((stale["error"].as_str(), stale["current_rev"].as_u64()), (Some("conflict"), Some(2)));
    app.project_archive(&id, None).unwrap();
    assert!(app.project_list(false).unwrap()["projects"].as_array().unwrap().is_empty());
    assert_eq!(app.project_list(true).unwrap()["projects"].as_array().unwrap().len(), 1);
    assert!(app.project_sessions(&id).unwrap()["sessions"].as_array().unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn refusals_carry_a_code_the_window_can_act_on() {
    let (root, app) = setup("refuse");
    let outer = repo(&root, "src/outer");
    let inner = repo(&root, "src/outer/vendor/inner");
    app.project_add(&outer, false).unwrap();
    let nested = err(app.project_add(&inner, false));
    assert_eq!(nested["error"], "nested");
    assert_eq!(nested["with"]["name"], "outer");
    assert_eq!(app.project_add(&inner, true).unwrap()["outcome"], "created");
    let home = root.join("home").to_string_lossy().into_owned();
    assert_eq!(err(app.project_add(&home, false))["error"], "refused_folder");
    assert_eq!(err(app.project_add("relative/dir", false))["error"], "invalid_args");
    assert_eq!(err(app.project_add(&format!("{home}/nope"), false))["error"], "invalid_args");
    assert_eq!(err(app.project_rename("p_01JC5X8Z3A9K2M4N6P7Q8R9S0T", "x", None))["error"], "not_found");
    let _ = fs::remove_dir_all(root);
}
