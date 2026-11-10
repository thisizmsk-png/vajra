//! FG-02c c3 (decision Q2): the Director-General's actions through action receipts. The fake CLI and the fixture are
//! `lead_tests`'s.

use super::*;
use crate::desktop::state::Envelope;

fn env(action_id: &str, project_id: &str, expect: Option<u64>) -> Envelope {
    Envelope { action_id: action_id.into(), project_id: project_id.into(), expect_generation: expect }
}

fn receipt_ids(app: &App, pid: &str) -> Vec<String> {
    app.project_state(pid, None).unwrap()["receipts"].as_array().unwrap().iter().map(|r| r["action_id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn an_ask_with_a_receipt_runs_once_and_a_retry_replays_it() {
    let t = Tmp::new();
    let app = t.app();
    let e = env("a_ask_once_0001", &pid(&t), Some(1));
    let first = app.act_ask_lead(&e, RUN_ID, "plan the next step");
    assert_eq!((first["receipt"]["state"].as_str(), first["receipt"]["action"].as_str()), (Some("done"), Some("ask_lead")), "{first}");
    assert!(first["result"]["ask_id"].is_string(), "{first}");
    wait_until("the ask", || idle(&app));
    let again = app.act_ask_lead(&e, RUN_ID, "plan the next step");
    assert_eq!(again["replayed"], true, "{again}");
    assert_eq!(again["receipt"], first["receipt"]);
    assert_eq!(t.count("ask.count"), 1, "the retry asked nothing");
    assert!(receipt_ids(&app, &pid(&t)).contains(&"a_ask_once_0001".to_string()));
}

#[test]
fn a_click_aimed_at_an_older_conversation_is_stale_and_two_new_conversation_clicks_make_one() {
    let t = Tmp::new();
    let app = t.app();
    let p = pid(&t);
    app.lead_status(RUN_ID).unwrap();
    let nc = app.act_new_conversation(&env("a_newconv_0001", &p, Some(1)), RUN_ID);
    assert_eq!((nc["receipt"]["state"].as_str(), nc["result"]["generation"].as_u64()), (Some("done"), Some(2)), "{nc}");
    // The second click, also aimed at conversation 1, starts nothing.
    let twice = app.act_new_conversation(&env("a_newconv_0002", &p, Some(1)), RUN_ID);
    assert_eq!((twice["receipt"]["state"].as_str(), twice["receipt"]["reason"].as_str()), (Some("rejected"), Some("stale_generation")), "{twice}");
    assert_eq!((twice["receipt"]["current_generation"].as_u64(), twice["receipt"]["retry"].as_str()), (Some(2), Some("after_refresh")));
    assert_eq!(app.lead_status(RUN_ID).unwrap()["generation"], 2, "one new conversation");
    // An ask aimed at conversation 1 is refused, and nothing runs.
    let old = app.act_ask_lead(&env("a_ask_stale_001", &p, Some(1)), RUN_ID, "hi");
    assert_eq!((old["receipt"]["reason"].as_str(), old["receipt"]["current_generation"].as_u64()), (Some("stale_generation"), Some(2)), "{old}");
    assert_eq!(t.count("ask.count"), 0);
    // project_state carries the Director-General's generation.
    assert_eq!(app.project_state(&p, None).unwrap()["generations"]["orchestrator"], 2);
    let ok = app.act_ask_lead(&env("a_ask_fresh_001", &p, Some(2)), RUN_ID, "hi");
    assert_eq!(ok["receipt"]["state"], "done", "{ok}");
    wait_until("the ask", || idle(&app));
}

/// #105 review [P2]: a client following the revision contract never keeps a stale conversation. With an idle project, a
/// Director-General created by `lead_status` (no generation, then 1) and a plain `new_conversation` (1, then 2) each move
/// the revision, so a read with the saved `since_rev` is a full one carrying the new generation. A read with nothing
/// changed is still `unchanged`.
#[test]
fn a_new_or_renewed_conversation_moves_the_project_revision() {
    let t = Tmp::new();
    let app = t.app();
    let p = pid(&t);
    let first = app.project_state(&p, None).unwrap();
    assert!(first["generations"]["orchestrator"].is_null(), "{first}");
    let rev = first["rev"].as_u64().unwrap();
    assert_eq!(app.project_state(&p, Some(rev)).unwrap()["unchanged"], true, "nothing changed yet");
    app.lead_status(RUN_ID).unwrap();
    let created = app.project_state(&p, Some(rev)).unwrap();
    assert_eq!((created["unchanged"].as_bool(), created["generations"]["orchestrator"].as_u64()), (Some(false), Some(1)), "created: {created}");
    let rev = created["rev"].as_u64().unwrap();
    assert_eq!(app.project_state(&p, Some(rev)).unwrap()["unchanged"], true);
    app.new_conversation(RUN_ID).unwrap();
    let renewed = app.project_state(&p, Some(rev)).unwrap();
    assert_eq!((renewed["unchanged"].as_bool(), renewed["generations"]["orchestrator"].as_u64()), (Some(false), Some(2)), "renewed: {renewed}");
    assert!(renewed["rev"].as_u64().unwrap() > rev);
    assert_eq!(app.project_state(&p, renewed["rev"].as_u64()).unwrap()["unchanged"], true, "and steady after");
}

/// A "New conversation" landing between the receipt's check and the ask lock: the ask is still stale, never a turn of
/// the new conversation it was not aimed at.
#[test]
fn a_new_conversation_racing_a_receipt_ask_makes_it_stale() {
    let t = Tmp::new();
    let app = t.app();
    let p = pid(&t);
    app.lead_status(RUN_ID).unwrap();
    let (dir, pp, control) = (orch_dir(&t), p.clone(), app.control.clone());
    *lock(&app.lead.before_ask_lock) = Some(Box::new(move || {
        let _ = std::fs::remove_file(dir.join("session"));
        std::fs::write(dir.join("generation"), "2").unwrap();
        lock(&control).set_generation(&pp, 2).unwrap();
    }));
    let r = app.act_ask_lead(&env("a_ask_race_0001", &p, Some(1)), RUN_ID, "hi");
    *lock(&app.lead.before_ask_lock) = None;
    assert_eq!((r["receipt"]["state"].as_str(), r["receipt"]["reason"].as_str()), (Some("rejected"), Some("stale_generation")), "{r}");
    assert_eq!(r["receipt"]["current_generation"], 2);
    assert!(idle(&app) && t.count("ask.count") == 0, "nothing ran and the slot is free");
}

#[test]
fn an_envelope_naming_another_project_stores_nothing() {
    let t = Tmp::new();
    let app = t.app();
    let other = crate::desktop::test_project(&t.data(), &t.home(), &t.0.join("other"));
    for r in [
        app.act_ask_lead(&env("a_mismatch_001", &other, None), RUN_ID, "hi"),
        app.act_new_conversation(&env("a_mismatch_002", &other, None), RUN_ID),
    ] {
        assert_eq!((r["receipt"].is_null(), code(&r["error"])), (true, "project_mismatch"), "{r}");
    }
    assert!(receipt_ids(&app, &other).is_empty() && receipt_ids(&app, &pid(&t)).is_empty());
    assert_eq!(t.count("ask.count"), 0);
}

/// Every path and file under `dir`, with its bytes.
fn tree(dir: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                todo.push(path.clone());
                out.push((path, Vec::new()));
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap_or_default()));
            }
        }
    }
    out.sort();
    out
}

/// A target that is no project's (a run the app never launched, an ask not running, a proposal not open) files no
/// receipt under the envelope's project: its recent list stays as it was.
#[test]
fn an_unbound_target_files_no_receipt_under_the_envelopes_project() {
    let t = Tmp::new();
    let app = t.app();
    let p = pid(&t);
    app.lead_status(RUN_ID).unwrap();
    assert_eq!(app.act_new_conversation(&env("a_seed_000001", &p, Some(1)), RUN_ID)["receipt"]["state"], "done");
    let recent = receipt_ids(&app, &p);
    let ulid = "01J0000000000000000000000Z";
    for r in [
        app.act_ask_lead(&env("a_unbound_001", &p, None), "20990101-0000", "hi"),
        app.act_new_conversation(&env("a_unbound_002", &p, None), "../x"),
        app.act_cancel_ask(&env("a_unbound_003", &p, None), ulid),
        app.act_start_proposal(&env("a_unbound_004", &p, None), "p_nope", &"0".repeat(64)),
        app.act_dismiss_proposal(&env("a_unbound_005", &p, None), "p_nope"),
    ] {
        assert!(r["receipt"].is_null() && r["error"].is_object(), "{r}");
    }
    assert_eq!(receipt_ids(&app, &p), recent, "the recent list is unchanged");
    assert_eq!(t.count("ask.count"), 0);
}

#[test]
fn a_cancel_with_a_receipt_names_the_asks_project() {
    let t = Tmp::new();
    let app = t.app();
    let p = pid(&t);
    t.set("mode", "slow");
    let ask = app.ask_lead(RUN_ID, "long").unwrap()["ask_id"].as_str().unwrap().to_string();
    let other = crate::desktop::test_project(&t.data(), &t.home(), &t.0.join("other"));
    assert_eq!(code(&app.act_cancel_ask(&env("a_cancel_0001", &other, None), &ask)["error"]), "project_mismatch");
    assert!(!idle(&app), "not cancelled by the mismatched click");
    let r = app.act_cancel_ask(&env("a_cancel_0002", &p, None), &ask);
    assert_eq!(r["receipt"]["state"], "done", "{r}");
    wait_until("cancelled", || idle(&app));
    // Nothing left to cancel: refused with no receipt (the ask is no project's now), not a crash.
    let gone = app.act_cancel_ask(&env("a_cancel_0003", &p, None), &ask);
    assert_eq!((gone["receipt"].is_null(), code(&gone["error"])), (true, "ask_not_found"), "{gone}");
    // The receipt of the cancel that ran still replays.
    assert_eq!(app.act_cancel_ask(&env("a_cancel_0002", &p, None), &ask)["replayed"], true);
}

#[test]
fn lead_status_by_project_needs_no_run_and_refuses_an_archived_project() {
    let t = Tmp::new();
    let app = t.app();
    // The fixture's run is alive: its process is up, not just its entry kept for the crew view (#82 review).
    app.sup.spawn(&format!("run-{SID}"), crate::sessions::SpawnSpec { argv: vec!["sleep".into(), "30".into()], cwd: t.repo(), rows: 24, cols: 80, redact: Default::default() }).unwrap();
    let p = pid(&t);
    assert!(app.project_state(&p, None).unwrap()["generations"]["orchestrator"].is_null(), "no Director-General yet");
    let quiet = crate::desktop::test_project(&t.data(), &t.home(), &t.0.join("quiet"));
    let before = tree(&t.data());
    let s = app.lead_status_for_project(&p).unwrap();
    assert_eq!((s["available"].as_bool(), s["project_id"].as_str(), s["generation"].as_u64()), (Some(true), Some(p.as_str()), Some(1)), "{s}");
    assert_eq!(s["state"], "running", "the fixture's run is going: {s}");
    assert_eq!(s["stream"], stream_name(&t));
    // A project with no run and no question yet: not started.
    let q = app.lead_status_for_project(&quiet).unwrap();
    assert_eq!((q["state"].as_str(), q["generation"].as_u64()), (Some("not_started"), Some(1)), "{q}");
    assert_eq!(tree(&t.data()), before, "a status read creates nothing");
    assert!(app.project_state(&quiet, None).unwrap()["generations"]["orchestrator"].is_null());
    assert!(app.project_state(&p, None).unwrap()["generations"]["orchestrator"].is_null(), "and carries nothing over");
    // Once asked, it reads the recorded conversation.
    app.lead_status(RUN_ID).unwrap();
    assert_eq!(app.project_state(&p, None).unwrap()["generations"]["orchestrator"], 1);
    assert_eq!(code(&app.lead_status_for_project("../x").unwrap_err()), "invalid_args");
    app.projects.archive(&p, None).unwrap();
    let off = app.lead_status_for_project(&p).unwrap();
    assert_eq!((off["available"].as_bool(), off["error"].as_str()), (Some(false), Some("project_archived")), "{off}");
}
