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

/// #103 review: a run launched from a SUBFOLDER of a plain project records that subfolder, while its Director-General runs in
/// the store's root; an actual ask from it is delivered and answered.
#[test]
fn an_ask_from_a_subfolder_of_a_plain_project_is_answered() {
    let t = Tmp::new();
    let app = t.app();
    let sub = t.repo().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    live_run(&app, SID, RUN_ID, sub.clone());
    app.record_run(RUN_ID, &sub, SID, "deliver-x", true, 1).unwrap();
    app.ask_lead(RUN_ID, "hi").unwrap();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    assert_eq!(t.ask_states(), ["queued:", "delivered:", "done:"]);
}

/// #103 review: the registered root counts as the run's own only as the store recorded it. A root moved away and replaced at
/// the same path by another folder, or by a link, is not the project's folder, and the ask from a subfolder starts nothing.
#[test]
fn a_registered_root_replaced_by_another_folder_or_a_link_is_refused() {
    for link in [false, true] {
        let t = Tmp::new();
        let app = t.app();
        let sub = t.repo().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        live_run(&app, SID, RUN_ID, sub.clone());
        app.record_run(RUN_ID, &sub, SID, "deliver-x", true, 1).unwrap();
        let moved = t.0.join("moved");
        std::fs::rename(t.repo(), &moved).unwrap();
        if link {
            std::os::unix::fs::symlink(&moved, t.repo()).unwrap();
        } else {
            std::fs::create_dir_all(t.repo().join("sub")).unwrap();
        }
        assert!(app.ask_lead(RUN_ID, "hi").is_err(), "link={link}: refused");
        assert!(t.count("ask.count") == 0 && t.ask_states().is_empty(), "link={link}: nothing was spawned or recorded");
    }
}

/// #103 review: a project registered inside a Git checkout (its root below the checkout's top), with the run launched
/// below the project root: the run records the checkout, its Director-General runs in the project's root, and the ask is
/// answered.
#[test]
fn an_ask_from_below_a_project_registered_inside_a_git_checkout_is_answered() {
    let t = Tmp::new();
    let app = t.app();
    let g = t.0.join("checkout");
    std::fs::create_dir_all(g.join(".git/objects")).unwrap();
    std::fs::write(g.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let p = g.join("proj");
    let sub = p.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let id = project_of(&t, &p);
    assert_eq!(app.projects.get(&id).unwrap().root, p.to_string_lossy(), "the project's root is its own folder");
    live_run(&app, SID, RUN_ID, sub.clone());
    app.record_run(RUN_ID, &sub, SID, "deliver-x", true, 1).unwrap();
    assert_eq!(app.lead_status(RUN_ID).unwrap()["project_id"].as_str(), Some(id.as_str()));
    app.ask_lead(RUN_ID, "hi").unwrap();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    assert_eq!(t.ask_states(), ["queued:", "delivered:", "done:"]);
}

/// #103 review: accepting the registered root opens nothing else. Another project's folder is still denied to this
/// Director-General, and asking as if from that folder, or from this project's subfolder rather than its root, is refused.
#[test]
fn with_a_subfolder_run_another_projects_folder_is_still_denied() {
    let t = Tmp::new();
    let app = t.app();
    let sub = t.repo().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    live_run(&app, SID, RUN_ID, sub.clone());
    app.record_run(RUN_ID, &sub, SID, "deliver-x", true, 1).unwrap();
    let other = t.0.join("sibling");
    project_of(&t, &other);
    app.record_run("20261109-1200", &other, "3c8fad5b-d9cb-469f-a165-70867728950e", "deliver-z", true, 1).unwrap();
    let iso = app.isolation(RUN_ID, &t.repo()).unwrap();
    assert!(iso.deny.contains(&other), "the other project's folder is denied: {:?}", iso.deny);
    assert!(!iso.deny.iter().any(|d| d.starts_with(t.repo())), "nothing of its own is denied: {:?}", iso.deny);
    assert!(app.isolation(RUN_ID, &other).is_err(), "the other project's folder is not this run's");
    let elsewhere = t.0.join("loose");
    std::fs::create_dir_all(&elsewhere).unwrap();
    assert!(app.isolation(RUN_ID, &elsewhere).is_err(), "an unrelated folder is not this run's");
}

/// #103 review: runs of the same store project are one project whatever folders they started in. With the root run AND a
/// later subfolder run both kept, an ask from either is answered, and neither's folder is denied to the other as another
/// project's; a run pinned to another store project is still another project, even in this project's folders.
#[test]
fn a_root_run_and_a_subfolder_run_of_one_project_are_both_its_own() {
    let t = Tmp::new();
    let app = t.app();
    let sub = t.repo().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    // The fixture's run (RUN_ID) is at the root; a second, finished run of the same project started in the subfolder.
    let (sid2, run2) = ("3c8fad5b-d9cb-469f-a165-70867728950e", "20261109-1300");
    live_run(&app, sid2, run2, sub.clone());
    app.record_run(run2, &sub, sid2, "deliver-y", true, 1).unwrap();
    assert_eq!(app.pinned_project(run2), app.pinned_project(RUN_ID), "both are pinned to one store project");
    assert_ne!(crate::desktop::caps::project_key(&sub), crate::desktop::caps::project_key(&t.repo()), "under different keys");
    let sibling = t.0.join("sibling");
    project_of(&t, &sibling);
    app.record_run("20261109-1302", &sibling, "5c8fad5b-d9cb-469f-a165-70867728950e", "deliver-s", true, 1).unwrap();
    for run in [RUN_ID, run2] {
        let iso = app.isolation(run, &t.repo()).unwrap();
        assert!(!iso.deny.iter().any(|d| d.starts_with(t.repo())), "{run}: nothing of its own is denied: {:?}", iso.deny);
        assert!(iso.deny.contains(&sibling), "{run}: another project's folder is still denied: {:?}", iso.deny);
    }
    app.ask_lead(run2, "from the subfolder").unwrap();
    wait_until("the subfolder ask", || idle(&app) && t.ask_states().len() == 3);
    app.ask_lead(RUN_ID, "from the root").unwrap();
    wait_until("the root ask", || idle(&app) && t.ask_states().len() == 6);
    assert_eq!(t.ask_states(), ["queued:", "delivered:", "done:", "queued:", "delivered:", "done:"]);
    // A run pinned to another store project is still another project's, even recorded in this project's subfolder.
    let other = project_of(&t, &t.0.join("elsewhere"));
    app.record_run("20261109-1301", &sub, "4c8fad5b-d9cb-469f-a165-70867728950e", "deliver-z", true, 1).unwrap();
    crate::control::fsx::write_atomic(&t.data().join("control/lead/20261109-1301"), "project", other.as_bytes()).unwrap();
    assert!(app.isolation(RUN_ID, &t.repo()).is_err(), "a folder claimed by two projects refuses");
}

/// #103 review: a run from before the store, launched in a SUBFOLDER of a plain project, keeps its stand-in pin. Once
/// that pin validates (its guarded folder still gives the same stand-in, and that folder is a store project) it is that
/// project's run: the ask from it is answered, the project's other run is its own, and another project is still denied.
#[test]
fn a_stand_in_run_in_a_subfolder_is_answered_once_its_pin_validates() {
    let t = Tmp::new();
    let app = t.app();
    let sub = t.repo().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let (sid2, run2) = ("3c8fad5b-d9cb-469f-a165-70867728950e", "20261109-1400");
    live_run(&app, sid2, run2, sub.clone());
    app.record_run(run2, &sub, sid2, "deliver-y", true, 1).unwrap();
    let k = super::super::orchestrator::project_ref(&sub).0;
    crate::control::fsx::write_atomic(&t.data().join("control/lead").join(run2), "project", k.as_bytes()).unwrap();
    assert_eq!(app.pinned_project(run2).as_deref(), Some(k.as_str()), "a stand-in pin");
    let sibling = t.0.join("sibling");
    project_of(&t, &sibling);
    app.record_run("20261109-1402", &sibling, "5c8fad5b-d9cb-469f-a165-70867728950e", "deliver-s", true, 1).unwrap();
    for run in [run2, RUN_ID] {
        let iso = app.isolation(run, &t.repo()).unwrap();
        assert!(!iso.deny.iter().any(|d| d.starts_with(t.repo())), "{run}: nothing of its own is denied: {:?}", iso.deny);
        assert!(iso.deny.contains(&sibling), "{run}: another project's folder is still denied: {:?}", iso.deny);
    }
    // Another record whose stand-in pin claims this project's folder but does not validate (its own folder gives another
    // stand-in) is no project's: it is never this project's run, and its folder stays denied.
    let forged = t.0.join("forged");
    std::fs::create_dir_all(&forged).unwrap();
    app.record_run("20261109-1403", &forged, "6c8fad5b-d9cb-469f-a165-70867728950e", "deliver-f", true, 1).unwrap();
    crate::control::fsx::write_atomic(&t.data().join("control/lead/20261109-1403"), "project", k.as_bytes()).unwrap();
    let iso = app.isolation(run2, &t.repo()).unwrap();
    assert!(iso.deny.contains(&forged), "a record whose pin does not validate is another's: {:?}", iso.deny);
    app.ask_lead(run2, "from a stand-in run in the subfolder").unwrap();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    assert_eq!(t.ask_states(), ["queued:", "delivered:", "done:"]);
}

/// #103 review: a stand-in pin that does not validate (it names another folder's stand-in) is no project's. Its own ask
/// is refused, by the launch check and by the isolation check itself, and nothing starts.
#[test]
fn a_stand_in_pin_that_does_not_validate_is_refused() {
    let t = Tmp::new();
    let app = t.app();
    let sub = t.repo().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    live_run(&app, SID, RUN_ID, sub.clone());
    app.record_run(RUN_ID, &sub, SID, "deliver-x", true, 1).unwrap();
    let k = super::super::orchestrator::project_ref(&t.0.join("elsewhere")).0;
    crate::control::fsx::write_atomic(&t.data().join("control/lead").join(RUN_ID), "project", k.as_bytes()).unwrap();
    assert!(app.isolation(RUN_ID, &t.repo()).is_err(), "the isolation check refuses it");
    assert!(app.isolation(RUN_ID, &sub).is_err(), "from its own folder too");
    assert!(app.ask_lead(RUN_ID, "hi").is_err(), "the ask is refused");
    assert!(t.count("ask.count") == 0 && t.ask_states().is_empty(), "nothing was spawned or recorded");
}

/// #103 review, the reported case: the project's only run is a stand-in run from before the store, in a SUBFOLDER. Its
/// validated pin carries into the registered-root check, so the ask in the project's root is answered.
#[test]
fn a_lone_stand_in_run_in_a_subfolder_is_answered() {
    let t = Tmp::new();
    let app = t.app();
    let sub = t.repo().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    live_run(&app, SID, RUN_ID, sub.clone());
    app.record_run(RUN_ID, &sub, SID, "deliver-x", true, 1).unwrap();
    let k = super::super::orchestrator::project_ref(&sub).0;
    crate::control::fsx::write_atomic(&t.data().join("control/lead").join(RUN_ID), "project", k.as_bytes()).unwrap();
    assert_eq!(app.lead_status(RUN_ID).unwrap()["available"], true);
    app.ask_lead(RUN_ID, "hi").unwrap();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    assert_eq!(t.ask_states(), ["queued:", "delivered:", "done:"]);
}

/// #103 review: two store projects registered inside ONE Git checkout, with a run from each and both records kept. Both
/// records name the checkout as their location and main checkout (shared Git bookkeeping), which is no claim of ownership:
/// an ask from each is answered, each sibling's folder is denied to the other, and the checkout gets the secret-file rules
/// only. A folder one of them owns, recorded by a run pinned to the other, still refuses.
#[test]
fn two_store_projects_inside_one_git_checkout_each_answer_with_both_records_kept() {
    let t = Tmp::new();
    let app = t.app();
    let c = t.0.join("checkout");
    std::fs::create_dir_all(c.join(".git/objects")).unwrap();
    std::fs::write(c.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let (a, b) = (c.join("a"), c.join("b"));
    let (pa, pb) = (project_of(&t, &a), project_of(&t, &b));
    assert_ne!(pa, pb);
    let (sa, ra) = ("3c8fad5b-d9cb-469f-a165-70867728950e", "20261109-1600");
    let (sb, rb) = ("4c8fad5b-d9cb-469f-a165-70867728950e", "20261109-1601");
    live_run(&app, sa, ra, a.clone());
    // b's run is in a subfolder of b: b's root is b's through the store alone.
    let bsub = b.join("sub");
    std::fs::create_dir_all(&bsub).unwrap();
    live_run(&app, sb, rb, bsub.clone());
    app.record_run(ra, &a, sa, "deliver-a", true, 1).unwrap();
    app.record_run(rb, &bsub, sb, "deliver-b", true, 1).unwrap();
    assert_eq!((app.pinned_project(ra), app.pinned_project(rb)), (Some(pa.clone()), Some(pb.clone())));
    for (run, own, other) in [(ra, &a, &b), (rb, &b, &a)] {
        let iso = app.isolation(run, own).unwrap();
        assert!(iso.deny.contains(other), "{run}: the sibling project is denied: {iso:?}");
        assert!(!iso.deny.iter().any(|d| own.starts_with(d)), "{run}: nothing holding its own folder is denied wholesale: {iso:?}");
    }
    app.ask_lead(ra, "from a").unwrap();
    wait_until("a's ask", || idle(&app) && t.ask_states().len() == 3);
    app.ask_lead(rb, "from b").unwrap();
    wait_until("b's ask", || idle(&app) && t.ask_states().len() == 6);
    assert_eq!(t.ask_states(), ["queued:", "delivered:", "done:", "queued:", "delivered:", "done:"]);
    // A run pinned to `a` but recorded in `b`'s folder: a folder one project owns, claimed by the other, refuses both.
    app.record_run("20261109-1602", &b, "5c8fad5b-d9cb-469f-a165-70867728950e", "deliver-x", true, 1).unwrap();
    crate::control::fsx::write_atomic(&t.data().join("control/lead/20261109-1602"), "project", pa.as_bytes()).unwrap();
    assert!(app.isolation(ra, &a).is_err(), "a's Director-General refuses");
    assert!(app.isolation(rb, &b).is_err(), "b's Director-General refuses");
}

/// #103 review: a project's validated store root is what it owns, whatever runs it has: another project's run whose
/// recorded checkout location is this project's root refuses (the redirection protection kept).
#[test]
fn a_store_root_claimed_by_another_projects_record_refuses() {
    let t = Tmp::new();
    let app = t.app();
    std::fs::create_dir_all(t.repo().join(".git/objects")).unwrap();
    std::fs::write(t.repo().join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let (lib, sub) = (t.repo().join("lib"), t.repo().join("sub"));
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::create_dir_all(&sub).unwrap();
    // This project's only run is in a subfolder: its root is owned through the store alone.
    live_run(&app, SID, RUN_ID, sub.clone());
    app.record_run(RUN_ID, &sub, SID, "deliver-x", true, 1).unwrap();
    assert!(app.isolation(RUN_ID, &t.repo()).is_ok(), "healthy");
    let other = project_of(&t, &t.0.join("elsewhere"));
    app.record_run("20261109-1700", &lib, "3c8fad5b-d9cb-469f-a165-70867728950e", "deliver-o", true, 1).unwrap();
    crate::control::fsx::write_atomic(&t.data().join("control/lead/20261109-1700"), "project", other.as_bytes()).unwrap();
    assert_eq!(app.read_run_loc("20261109-1700"), Some(t.repo()), "the other record's location is this project's root");
    assert!(app.isolation(RUN_ID, &t.repo()).is_err(), "this project's root claimed by another project refuses");
}

/// #103 review (wire): every store root is listed, so another project's root is owned by it even with no run of its own;
/// a record pinned to this project but recorded in that root refuses, and the root is denied.
#[test]
fn another_projects_root_without_runs_is_still_owned_and_denied() {
    let t = Tmp::new();
    let app = t.app();
    let sib = t.0.join("sibling");
    project_of(&t, &sib);
    assert!(app.isolation(RUN_ID, &t.repo()).unwrap().deny.contains(&sib), "denied with no run of its own");
    app.record_run("20261109-1701", &sib, "4c8fad5b-d9cb-469f-a165-70867728950e", "deliver-x", true, 1).unwrap();
    crate::control::fsx::write_atomic(&t.data().join("control/lead/20261109-1701"), "project", pid(&t).as_bytes()).unwrap();
    assert!(app.isolation(RUN_ID, &t.repo()).is_err(), "a folder another project owns, recorded as this project's, refuses");
}

/// #103 review (wire): shared Git bookkeeping is no ownership claim only between records whose store identity validates.
/// A record whose pin does not validate, recorded in the same checkout, has nothing to vouch for whose checkout it names,
/// so its location still counts as its own and the shared checkout refuses.
#[test]
fn a_record_without_a_validated_identity_sharing_the_checkout_still_refuses() {
    let t = Tmp::new();
    let app = t.app();
    let c = t.0.join("checkout");
    std::fs::create_dir_all(c.join(".git/objects")).unwrap();
    std::fs::write(c.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let (a, x) = (c.join("a"), c.join("x"));
    std::fs::create_dir_all(&x).unwrap();
    project_of(&t, &a);
    let (sa, ra) = ("3c8fad5b-d9cb-469f-a165-70867728950e", "20261109-1800");
    live_run(&app, sa, ra, a.clone());
    app.record_run(ra, &a, sa, "deliver-a", true, 1).unwrap();
    assert!(app.isolation(ra, &a).is_ok(), "healthy");
    app.record_run("20261109-1801", &x, "4c8fad5b-d9cb-469f-a165-70867728950e", "deliver-x", true, 1).unwrap();
    let k = super::super::orchestrator::project_ref(&t.0.join("elsewhere")).0;
    crate::control::fsx::write_atomic(&t.data().join("control/lead/20261109-1801"), "project", k.as_bytes()).unwrap();
    assert_eq!(app.read_run_loc("20261109-1801"), Some(c.clone()), "it records the shared checkout");
    assert!(app.isolation(ra, &a).is_err(), "an unidentified record naming the shared checkout refuses");
}
