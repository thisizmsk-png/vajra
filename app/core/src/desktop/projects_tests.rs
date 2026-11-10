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

#[test]
fn migration_suggests_then_confirms_once_through_the_commands() {
    let (root, app) = setup("migrate");
    let a = repo(&root, "src/alpha");
    // A campaign dir the reader lists, and a launch line that names alpha as its folder.
    let camp = root.join("home/.claude/vajra/campaigns/old-one");
    fs::create_dir_all(&camp).unwrap();
    fs::write(root.join("data/runs.jsonl"), format!("{{\"campaign\":\"old-one\",\"session_id\":\"0f8fad5b-d9cb-469f-a165-70867728950e\",\"run_id\":\"20261101-0900\",\"cwd\":\"{a}\"}}\n")).unwrap();
    let s = app.project_migration_suggest().unwrap();
    assert_eq!(s["done"], false);
    let groups = s["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "{s}");
    assert_eq!(groups[0]["root"], a.as_str());
    let out = app.project_migration_confirm(std::slice::from_ref(&a)).unwrap();
    assert_eq!(out["projects"][0]["sessions"], 1);
    assert_eq!(app.project_migration_suggest().unwrap()["done"], true);
    assert_eq!(err(app.project_migration_confirm(&[a]))["error"], "already_migrated");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn project_sessions_carries_the_truncated_flag() {
    let (root, app) = setup("trunc");
    let a = repo(&root, "src/alpha");
    let id = app.project_add(&a, false).unwrap()["project"]["id"].as_str().unwrap().to_string();
    let v = app.project_sessions(&id).unwrap();
    assert_eq!((v["truncated"].clone(), v["skipped"].clone(), v["unsupported"].clone()), (false.into(), 0.into(), false.into()));
    // The response has exactly the fields the shared fixture (also read by the UI tests) documents.
    let fx: Value = serde_json::from_slice(&fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contract/project_sessions.v1.json")).unwrap()).unwrap();
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(keys(&v), keys(&fx));
    let _ = fs::remove_dir_all(root);
}

/// #77 follow-up: the app opens with a symlinked projects folder; the project commands refuse with what is wrong, the
/// path and what to do, and preflight shows it as a row (without disabling unrelated checks' start button).
#[test]
fn a_symlinked_projects_folder_does_not_stop_the_app_opening() {
    let root = fs::canonicalize(std::env::temp_dir()).unwrap().join(format!("vajra-projcmd-unavail-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let (home, data, elsewhere) = (root.join("home"), root.join("data"), root.join("elsewhere"));
    for d in [home.join(".claude"), data.clone(), elsewhere.clone()] {
        fs::create_dir_all(d).unwrap();
    }
    std::os::unix::fs::symlink(&elsewhere, data.join("projects")).unwrap();
    let app = App::open(&data, &home).expect("the app opens");
    let e = err(app.project_list(false));
    assert_eq!(e["error"], "projects_unavailable", "{e}");
    assert_eq!(e["path"], data.join("projects").to_str().unwrap());
    assert!(e["detail"].as_str().unwrap().contains("symbolic link") && e["fix"].as_str().is_some_and(|f| !f.is_empty()), "{e}");
    assert_eq!(err(app.project_add(&repo(&root, "src/a"), false))["error"], "projects_unavailable");
    // #106 review: the migration was never done (no marker could be written), so neither command may say it was.
    assert_eq!(err(app.project_migration_suggest())["error"], "projects_unavailable");
    assert_eq!(err(app.project_migration_confirm(&[]))["error"], "projects_unavailable");
    let row = app.projects_row().expect("a preflight row");
    assert_eq!((row["ok"].as_bool(), row["name"].as_str()), (Some(false), Some("Projects folder")), "{row}");
    assert!(row["detail"].as_str().unwrap().contains(data.join("projects").to_str().unwrap()), "{row}");
    // #106 review: every start reads the store, so Home does not offer Start, and a start, a resume and an ask each say
    // what is wrong and what to do (never a bare io_error or a record that "cannot be read").
    let pf = app.preflight().unwrap();
    assert_eq!(pf["start_enabled"], false, "{pf}");
    assert!(pf["rows"].as_array().unwrap().iter().any(|r| r["name"] == "Projects folder" && r["ok"] == false), "{pf}");
    let cwd_s = repo(&root, "src/a");
    let cwd = std::path::Path::new(&cwd_s);
    let unavailable = |e: Value, what: &str| {
        assert_eq!(e["error"], "projects_unavailable", "{what}: {e}");
        assert!(e["fix"].as_str().is_some_and(|f| f.contains(data.join("projects").to_str().unwrap())), "{what}: {e}");
        assert_eq!(e["path"], data.join("projects").to_str().unwrap(), "{what}: {e}");
    };
    let input = crate::desktop::RunInput { ask: "x".into(), max_rounds: 1, base: "ea73cb05d639b945e21c667e17103e56404861a8".into(), max_in_flight: 1, campaign: None, provider: None };
    unavailable(err(app.start_run(input, &cwd_s)), "start");
    unavailable(err(app.resume_run("0f8fad5b-d9cb-469f-a165-70867728950e", "c", &cwd_s, None)), "resume");
    unavailable(app.launch_project(cwd).unwrap_err(), "launch_project");
    unavailable(err(app.ask_lead("20261106-0911", "hi")), "ask");
    unavailable(app.isolation("20261106-0911", cwd).unwrap_err().to_json(), "isolation");
    unavailable(app.orchestrator_for_run("20261106-0911", cwd).map(|_| json!(null)).map_err(|e| e.to_json()).unwrap_err(), "the run's Director-General");
    // The Director-General's tool gate says the same (detail only: a tool result never carries a path).
    let gate = crate::desktop::orchestrator::project_gate(&app.projects, "p_01M6YKE1Q5YEQZWJSJ91A6DQYT").unwrap_err().to_json();
    assert_eq!(gate["error"], "projects_unavailable", "{gate}");
    assert!(gate["detail"].as_str().is_some_and(|d| d.contains("symbolic link")) && gate.get("path").is_none(), "{gate}");
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    let _ = fs::remove_dir_all(root);
}

/// #106 review: with every other check passing, a refused projects folder alone holds up Start; without it Start is on.
#[test]
fn a_refused_projects_folder_alone_disables_start() {
    let rows = vec![crate::vajra::preflight::Row { name: "claude".into(), ok: true, detail: "found".into(), fix: String::new() }];
    assert_eq!(crate::desktop::preflight_json(&rows, None)["start_enabled"], true);
    let row = json!({"name": "Projects folder", "ok": false, "detail": "refused", "fix": "move it"});
    let pf = crate::desktop::preflight_json(&rows, Some(row));
    assert_eq!(pf["start_enabled"], false, "{pf}");
    assert_eq!(pf["rows"].as_array().unwrap().len(), 2, "{pf}");
}

// A run started from a symlink to a registered project, or from a linked worktree of its repository, belongs to that
// project: the start receipt must be keyed by the project the store finds, not by a literal path comparison in the UI.
#[test]
fn a_folder_resolves_to_its_project_through_a_symlink_and_a_linked_worktree() {
    let (root, app) = setup("for-folder");
    let main = repo(&root, "src/api");
    let id = app.project_add(&main, false).unwrap()["project"]["id"].as_str().unwrap().to_string();
    let id_of = |p: &str| app.project_for_folder(p).unwrap()["project_id"].as_str().unwrap().to_string();
    assert_eq!(id_of(&main), id);
    let link = root.join("home/link-to-api");
    std::os::unix::fs::symlink(&main, &link).unwrap();
    assert_eq!(id_of(link.to_str().unwrap()), id, "a symlink to the project folder");
    let wt_git = std::path::Path::new(&main).join(".git/worktrees/fix");
    fs::create_dir_all(&wt_git).unwrap();
    fs::write(wt_git.join("commondir"), "../..\n").unwrap();
    fs::write(wt_git.join("HEAD"), "ref: refs/heads/fix\n").unwrap();
    let wt = root.join("home/worktrees/api-fix");
    fs::create_dir_all(&wt).unwrap();
    fs::write(wt_git.join("gitdir"), format!("{}\n", wt.join(".git").display())).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
    assert_eq!(id_of(wt.to_str().unwrap()), id, "a linked worktree of the project's repository");
    let other = repo(&root, "src/other");
    assert_eq!(id_of(&other), "p_unsorted", "a folder no project owns is Unsorted, and is not registered by asking");
    assert_eq!(app.project_list(false).unwrap()["projects"].as_array().unwrap().len(), 1);
    assert_eq!(id_of("/no/such/folder"), "p_unsorted");
    assert_eq!(err(app.project_for_folder("relative/dir"))["error"], "invalid_args");
    let _ = fs::remove_dir_all(root);
}
