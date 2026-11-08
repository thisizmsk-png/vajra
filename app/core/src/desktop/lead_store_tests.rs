//! FG-02c c1: the Director-General on the project store (fg02-plan sections 3.1, 3.2 and 3.4). The fake CLI and the
//! fixture are `lead_tests`'s.

use super::*;
use crate::control::service::ControlError;
use crate::projects::contract::{SessionKind, valid_project_id};
use std::io::Write;

fn project_of(t: &Tmp, dir: &Path) -> String {
    crate::desktop::test_project(&t.data(), &t.home(), dir)
}

fn live_run(app: &App, sid: &str, run: &str, cwd: PathBuf) {
    lock(&app.runs).insert(format!("run-{sid}"), Run { session_id: sid.into(), run_id: run.into(), campaign: "deliver-y".into(), cwd, nonces: Redactor::default() });
}

fn orchestrators(app: &App, pid: &str) -> Vec<crate::projects::contract::Session> {
    app.projects.sessions(pid).unwrap().sessions.into_iter().filter(|s| s.kind == SessionKind::Orchestrator).collect()
}

/// A stand-in (`k_`) orchestrator dir as FG-02a/b left it.
fn standin(t: &Tmp, root: &Path, generation: u32, session: &str) -> PathBuf {
    let k = super::super::orchestrator::project_ref(root).0;
    let dir = t.data().join("control/projects").join(&k).join("orchestrator");
    for d in [t.data().join("control"), t.data().join("control/projects"), dir.parent().unwrap().to_path_buf(), dir.clone()] {
        crate::control::fsx::private_dir(&d).unwrap();
    }
    for (f, v) in [("root", root.to_string_lossy().into_owned()), ("generation", generation.to_string()), ("session", session.to_string())] {
        crate::control::fsx::write_atomic(&dir, f, v.as_bytes()).unwrap();
    }
    dir
}

/// 3.1 tests 1-2: a run is pinned to its folder's store project, and a subfolder or a symlinked path of that folder
/// belongs to the same project, so they share one Director-General.
#[test]
fn a_run_is_pinned_to_its_store_project_and_its_subfolders_share_it() {
    let t = Tmp::new();
    let app = t.app();
    app.record_run(RUN_ID, &t.repo(), SID, "deliver-x", true, 1).unwrap();
    let pinned = std::fs::read_to_string(t.data().join("control/lead").join(RUN_ID).join("project")).unwrap();
    assert!(valid_project_id(&pinned), "{pinned}");
    assert_eq!(pinned, pid(&t));
    std::fs::create_dir_all(t.repo().join("src")).unwrap();
    let link = t.0.join("link");
    std::os::unix::fs::symlink(t.repo(), &link).unwrap();
    live_run(&app, "3c8fad5b-d9cb-469f-a165-70867728950e", "20261106-0930", t.repo().join("src"));
    live_run(&app, "4c8fad5b-d9cb-469f-a165-70867728950e", "20261106-0931", link.join("src"));
    app.projects.rename(&pinned, "Harbor", None).unwrap();
    for run in [RUN_ID, "20261106-0930", "20261106-0931"] {
        let s = app.lead_status(run).unwrap();
        assert_eq!((s["available"].as_bool(), s["project_id"].as_str(), s["project_name"].as_str()), (Some(true), Some(pinned.as_str()), Some("Harbor")), "{run}: {s}");
    }
    let r = app.ask_lead(RUN_ID, "hi").unwrap();
    assert_eq!((r["project_id"].as_str(), r["project_name"].as_str()), (Some(pinned.as_str()), Some("Harbor")));
    wait_until("the ask", || idle(&app));
}

/// 3.1 test 3 (decision Q1): a run in a folder that is not a project gets `not_a_project` with the add action and the
/// folder, and nothing runs.
#[test]
fn a_run_in_a_folder_that_is_not_a_project_is_refused_and_nothing_runs() {
    let t = Tmp::new();
    let app = t.app();
    let loose = t.0.join("loose");
    std::fs::create_dir_all(&loose).unwrap();
    live_run(&app, "3c8fad5b-d9cb-469f-a165-70867728950e", "20261106-0930", loose.clone());
    let e = app.ask_lead("20261106-0930", "hi").unwrap_err();
    assert_eq!((code(&e), e["action"].as_str(), e["path"].as_str()), ("not_a_project", Some("add_project"), loose.to_str()), "{e}");
    assert!(!e["detail"].as_str().unwrap().contains(loose.to_str().unwrap()), "the folder is not in the detail text");
    let s = app.lead_status("20261106-0930").unwrap();
    assert_eq!((s["available"].as_bool(), s["error"].as_str(), s["action"].as_str(), s["path"].as_str()), (Some(false), Some("not_a_project"), Some("add_project"), loose.to_str()), "{s}");
    assert!(t.count("ask.count") == 0 && t.ask_states().is_empty(), "nothing was spawned or recorded");
}

/// 3.1 test 5: a run pinned before the store (a stand-in id) resolves to its folder's store project while the folder
/// still gives that stand-in; a stand-in that does not match its folder is refused.
#[test]
fn a_stand_in_pin_resolves_through_its_folder_or_fails_closed() {
    let t = Tmp::new();
    let app = t.app();
    app.record_run(RUN_ID, &t.repo(), SID, "deliver-x", true, 1).unwrap();
    let rec = t.data().join("control/lead").join(RUN_ID);
    crate::control::fsx::write_atomic(&rec, "project", super::super::orchestrator::project_ref(&t.repo()).0.as_bytes()).unwrap();
    assert_eq!(app.lead_status(RUN_ID).unwrap()["project_id"], pid(&t).as_str());
    crate::control::fsx::write_atomic(&rec, "project", b"k_ffffffffffffffffffffffffffffffff").unwrap();
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "lead_guard_missing");
    assert_eq!(t.count("ask.count"), 0);
}

/// 3.2 tests 6, 8 and 10: an archived project gets nothing (no ask, no tool call), and once restored the same
/// conversation resumes.
#[test]
fn an_archived_project_gets_nothing_and_resumes_the_same_conversation_when_restored() {
    let t = Tmp::new();
    let app = t.app();
    let first = app.ask_lead(RUN_ID, "remember this").unwrap()["lead_session_id"].as_str().unwrap().to_string();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    let asks = t.count("ask.count");
    let p = pid(&t); // read before archiving: adding the folder again would restore it
    app.projects.archive(&p, None).unwrap();
    let e = app.ask_lead(RUN_ID, "still there?").unwrap_err();
    assert_eq!(code(&e), "project_archived", "{e}");
    assert_eq!(t.count("ask.count"), asks, "no process started");
    assert!(matches!(lock(&app.control).call_tool_for_project(&p, 1, "list_workers", &json!({})), Err(ControlError::ProjectArchived)));
    assert_eq!(app.lead_status(RUN_ID).unwrap()["available"], false);
    assert_eq!(app.projects.add(&t.repo(), true).unwrap().outcome(), "restored");
    let r = app.ask_lead(RUN_ID, "back again").unwrap();
    assert_eq!((r["generation"].as_u64(), r["lead_session_id"].as_str()), (Some(1), Some(first.as_str())), "{r}");
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 6);
    assert!(argvs(&t).iter().any(|a| a.contains("--resume") && a.contains(&first)), "the old conversation was resumed");
}

/// 3.2 test 7: the project is checked again when the ask takes its slot, not only when the request came in.
#[test]
fn a_project_archived_while_an_ask_is_on_its_way_starts_nothing() {
    let t = Tmp::new();
    let app = t.app();
    let (store, p) = (app.projects.clone(), pid(&t));
    *lock(&app.lead.before_ask_lock) = Some(Box::new(move || {
        store.archive(&p, None).unwrap();
    }));
    let e = app.ask_lead(RUN_ID, "hi").unwrap_err();
    *lock(&app.lead.before_ask_lock) = None;
    assert_eq!(code(&e), "project_archived", "{e}");
    assert!(idle(&app), "the slot was given back");
    assert!(t.count("ask.count") == 0 && t.ask_states().is_empty());
}

/// 3.2 test 9: a project gone from the store gets `project_missing` everywhere, and its orchestrator dir is kept.
#[test]
fn a_project_gone_from_the_store_is_missing_and_its_orchestrator_dir_is_kept() {
    let t = Tmp::new();
    let app = t.app();
    app.ask_lead(RUN_ID, "hello").unwrap();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    let p = pid(&t);
    std::fs::remove_dir_all(t.data().join("projects").join(&p)).unwrap();
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "project_missing");
    assert!(matches!(lock(&app.control).call_tool_for_project(&p, 1, "list_workers", &json!({})), Err(ControlError::ProjectMissing)));
    assert!(t.data().join("control/projects").join(&p).join("orchestrator/session").is_file(), "the orchestrator dir is untouched");
}

/// 3.4 tests 16, 17 and 19: the project's sessions.jsonl holds one orchestrator session across asks, a new line per
/// conversation, and is never what `--resume` reads.
#[test]
fn the_orchestrator_is_one_session_record_and_the_log_never_drives_resume() {
    let t = Tmp::new();
    let app = t.app();
    let p = pid(&t);
    let s1 = app.ask_lead(RUN_ID, "one").unwrap()["lead_session_id"].as_str().unwrap().to_string();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    app.ask_lead(RUN_ID, "two").unwrap();
    wait_until("the second ask", || idle(&app) && t.ask_states().len() == 6);
    let o = orchestrators(&app, &p);
    assert_eq!(o.len(), 1, "one orchestrator session: {o:?}");
    assert_eq!((o[0].native_session_id.as_deref(), o[0].generation, o[0].parent.as_deref()), (Some(s1.as_str()), 1, None));
    let record = std::fs::read_to_string(orch_dir(&t).join("record")).unwrap();
    assert_eq!(o[0].id, record);
    // A forged newest line naming another native session changes nothing.
    let forged = "9a8fad5b-d9cb-469f-a165-70867728950e";
    let mut line = o[0].clone();
    line.native_session_id = Some(forged.into());
    let mut bytes = serde_json::to_vec(&line).unwrap();
    bytes.push(b'\n');
    std::fs::OpenOptions::new().append(true).open(t.data().join("projects").join(&p).join("sessions.jsonl")).unwrap().write_all(&bytes).unwrap();
    app.ask_lead(RUN_ID, "three").unwrap();
    wait_until("the third ask", || idle(&app) && t.ask_states().len() == 9);
    assert!(argvs(&t).iter().all(|a| !a.contains(forged)), "the forged id is in no argv");
    assert!(argvs(&t).iter().filter(|a| a.contains("--resume")).all(|a| a.contains(&s1)));
    // New conversation: a line on generation 2, same orchestrator session id.
    app.new_conversation(RUN_ID).unwrap();
    let o = orchestrators(&app, &p);
    assert_eq!((o.len(), o[0].id.as_str(), o[0].generation), (1, record.as_str(), 2));
}

/// 3.4 test 20 (decision Q3): a stand-in conversation for this folder carries over once, nothing is deleted.
#[test]
fn a_stand_in_conversation_of_the_same_folder_carries_over_once() {
    let t = Tmp::new();
    let app = t.app();
    let old = "6a8fad5b-d9cb-469f-a165-70867728950e";
    let kdir = standin(&t, &t.repo(), 3, old);
    let r = app.ask_lead(RUN_ID, "do you remember?").unwrap();
    assert_eq!((r["generation"].as_u64(), r["lead_session_id"].as_str()), (Some(3), Some(old)), "{r}");
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    assert!(argvs(&t).iter().any(|a| a.contains("--resume") && a.contains(old)));
    let k = kdir.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(std::fs::read_to_string(orch_dir(&t).join("mapped-from")).unwrap(), k);
    assert_eq!(std::fs::read_to_string(kdir.join("mapped-to")).unwrap(), pid(&t));
    for f in ["root", "generation", "session"] {
        assert!(kdir.join(f).is_file(), "the stand-in's {f} is kept");
    }
    // Once only: a later change to the stand-in is not taken again.
    std::fs::write(kdir.join("generation"), "7").unwrap();
    let r = app.ask_lead(RUN_ID, "again").unwrap();
    assert_eq!(r["generation"].as_u64(), Some(3));
    wait_until("the second ask", || idle(&app));
}

/// 3.4 test 21: a stand-in whose recorded root is another folder lends nothing.
#[test]
fn a_stand_in_recorded_for_another_folder_is_not_carried_over() {
    let t = Tmp::new();
    let app = t.app();
    let elsewhere = t.0.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let kdir = standin(&t, &t.repo(), 3, "6a8fad5b-d9cb-469f-a165-70867728950e");
    std::fs::write(kdir.join("root"), elsewhere.to_string_lossy().as_bytes()).unwrap();
    let r = app.ask_lead(RUN_ID, "hi").unwrap();
    assert_eq!(r["generation"].as_u64(), Some(1), "{r}");
    assert_ne!(r["lead_session_id"], "6a8fad5b-d9cb-469f-a165-70867728950e");
    wait_until("the ask", || idle(&app));
    assert!(!orch_dir(&t).join("mapped-from").exists() && !kdir.join("mapped-to").exists());
}

/// 3.3 test 12: another project's Director-General naming this project's worker gets `not_found`, never a hint.
#[test]
fn another_projects_director_general_cannot_name_this_projects_worker() {
    let t = Tmp::new();
    let app = t.app();
    let other = project_of(&t, &t.0.join("other"));
    let mut svc = lock(&app.control);
    svc.start_session(SID, RUN_ID).unwrap();
    svc.set_project(SID, &pid(&t)).unwrap();
    svc.set_generation(&other, 1).unwrap();
    let e = svc.call_tool_for_project(&other, 1, "stop_worker", &json!({"worker": "bhima", "reason": "x"})).unwrap_err();
    assert!(matches!(e, ControlError::ControlDisabled | ControlError::WorkerNotFound), "{e:?}");
    svc.end_session(SID);
}

/// Plan item 7: every ask line in the ledger (queued, delivered, done, and a new conversation's) names the project and
/// the conversation it went to.
#[test]
fn every_ask_line_names_the_project_and_its_conversation() {
    let t = Tmp::new();
    let app = t.app();
    let p = pid(&t);
    app.ask_lead(RUN_ID, "one").unwrap();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    app.new_conversation(RUN_ID).unwrap();
    app.ask_lead(RUN_ID, "two").unwrap();
    wait_until("the second ask", || idle(&app) && t.ask_states().len() == 6);
    let asks: Vec<Value> = t.ledger().into_iter().filter(|r| r["kind"] == "ask").collect();
    assert!(asks.iter().all(|r| r["project_id"] == p.as_str()), "{asks:?}");
    let gens: Vec<u64> = asks.iter().map(|r| r["generation"].as_u64().unwrap()).collect();
    assert_eq!(gens, [1, 1, 1, 2, 2, 2]);
}
