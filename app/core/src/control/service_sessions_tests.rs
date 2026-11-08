//! ADR-15 2.6 step 1: one control service steers several run sessions at once, and each session stays its own: its
//! nonce, its queue and stop dirs, its workers and its pending actions. A Lead bound to one run never sees or steers
//! another run's workers.

use super::*;
use crate::crew::tree::{Status, Worker};
use std::fs;

const A: &str = "62278db8-d6e8-48f7-b03e-14d8a8c989f2";
const B: &str = "8c1f4e2a-3b5d-4c6e-9f70-0a1b2c3d4e5f";
const RUN_A: &str = "20261108-0900";
const RUN_B: &str = "20261108-0901";

fn worker(agent: &str, task: &str, cwd: &Path) -> WorkerInfo {
    WorkerInfo {
        worker: Worker {
            agent_id: agent.into(),
            engine_id: "bhima".into(),
            label: format!("task.{task}.r1"),
            prompt_first_line: None,
            branch: None,
            status: Status::InProgress,
        },
        task_id: Some(task.into()),
        title: None,
        round: Some(1),
        live: true,
        last_activity: None,
        cwd: Some(cwd.to_string_lossy().into_owned()),
        lines: vec![],
    }
}

/// Two live sessions, each with one worker on task `t1`.
fn two() -> (Service, PathBuf) {
    let dir = std::env::temp_dir().join(format!("vajra-svc2-{}-{}", std::process::id(), nonce::random_hex(4).unwrap()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("wa")).unwrap();
    fs::create_dir_all(dir.join("wb")).unwrap();
    let mut s = Service::open(&dir, dir.join("audit.log"), None).unwrap();
    s.start_session(A, RUN_A).unwrap();
    s.set_workers(A, vec![worker("aaaa0000aaaa0000a", "t1", &dir.join("wa"))]);
    s.start_session(B, RUN_B).unwrap();
    s.set_workers(B, vec![worker("bbbb0000bbbb0000b", "t1", &dir.join("wb"))]);
    (s, dir)
}

fn count(dir: &Path, sid: &str, sub: &str) -> usize {
    fs::read_dir(dir.join("control").join(sid).join(sub))
        .map(|r| r.filter_map(|e| e.ok()).filter(|e| !e.file_name().to_string_lossy().starts_with('.')).count())
        .unwrap_or(0)
}

#[test]
fn starting_a_second_session_keeps_the_first_with_its_own_nonce() {
    let (s, dir) = two();
    let (na, nb) = (s.nonce(A).unwrap().to_string(), s.nonce(B).unwrap().to_string());
    assert_ne!(na, nb, "each session has its own nonce");
    let mut ids = s.session_ids();
    ids.sort();
    assert_eq!(ids, vec![A.to_string(), B.to_string()]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_redactor_removes_every_live_nonce() {
    let (s, dir) = two();
    let text = format!("a={} b={}", s.nonce(A).unwrap(), s.nonce(B).unwrap());
    let clean = s.redactor().clean(&text);
    assert!(!clean.contains(s.nonce(A).unwrap()) && !clean.contains(s.nonce(B).unwrap()), "{clean}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_message_lands_in_its_own_sessions_queue_and_ledger_line() {
    let (mut s, dir) = two();
    s.message_worker(Actor::Human, "aaaa0000aaaa0000a", "hello").unwrap();
    assert_eq!((count(&dir, A, "queue"), count(&dir, B, "queue")), (1, 0));
    let recs = Ledger::read_all(&dir.join("control/ledger.jsonl")).unwrap();
    let q = recs.iter().find(|r| str_field(r, "state") == "queued").unwrap();
    assert_eq!((str_field(q, "session_id"), str_field(q, "run_id")), (A, RUN_A));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_name_that_matches_workers_in_two_sessions_is_ambiguous() {
    let (mut s, dir) = two();
    assert!(matches!(s.message_worker(Actor::Human, "t1", "hi"), Err(ControlError::WorkerAmbiguous)));
    assert_eq!(count(&dir, A, "queue") + count(&dir, B, "queue"), 0);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_lead_bound_to_run_a_never_sees_or_steers_run_b() {
    let (mut s, dir) = two();
    let list = s.call_tool_for_run(Some(RUN_A), "list_workers", &json!({})).unwrap();
    let names: Vec<&str> = list["workers"].as_array().unwrap().iter().map(|w| w["worker"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["aaaa0000aaaa0000a"]);
    assert!(s.call_tool_for_run(Some(RUN_A), "list_workers", &json!({"run_id": RUN_B})).unwrap()["workers"].as_array().unwrap().is_empty());
    for (tool, args) in [
        ("inspect_worker", json!({"worker": "bbbb0000bbbb0000b"})),
        ("message_worker", json!({"worker": "bbbb0000bbbb0000b", "text": "x"})),
        ("redirect_worker", json!({"worker": "bbbb0000bbbb0000b", "text": "x"})),
        ("stop_worker", json!({"worker": "bbbb0000bbbb0000b"})),
    ] {
        assert!(matches!(s.call_tool_for_run(Some(RUN_A), tool, &args), Err(ControlError::WorkerNotFound)), "{tool}");
    }
    // Its own run's `t1` is not ambiguous for it: the other session is out of its scope.
    s.call_tool_for_run(Some(RUN_A), "message_worker", &json!({"worker": "t1", "text": "x"})).unwrap();
    assert_eq!((count(&dir, A, "queue"), count(&dir, B, "queue")), (1, 0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_unbound_lead_call_with_two_sessions_is_refused() {
    let (mut s, dir) = two();
    assert!(matches!(s.call_tool("list_workers", &json!({})), Err(ControlError::RunNotCurrent)));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_lead_stop_confirmed_later_is_written_to_its_own_session() {
    let (mut s, dir) = two();
    let r = s.call_tool_for_run(Some(RUN_B), "stop_worker", &json!({"worker": "bbbb0000bbbb0000b"})).unwrap();
    s.confirm_stop(Actor::Human, r["action_id"].as_str().unwrap()).unwrap();
    assert_eq!((count(&dir, A, "stops"), count(&dir, B, "stops")), (0, 1));
    assert!(s.signals(B).stopped.contains("bbbb0000bbbb0000b"));
    assert!(s.signals(A).stopped.is_empty(), "another session's stop is not this session's overlay");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ending_one_session_expires_only_its_own_queued_messages() {
    let (mut s, dir) = two();
    let a = s.message_worker(Actor::Human, "aaaa0000aaaa0000a", "to a").unwrap();
    let b = s.message_worker(Actor::Human, "bbbb0000bbbb0000b", "to b").unwrap();
    s.end_session(A);
    assert_eq!(s.session_ids(), vec![B.to_string()]);
    assert!(!s.pending.contains_key(a["action_id"].as_str().unwrap()), "A's message expired");
    assert!(s.pending.contains_key(b["action_id"].as_str().unwrap()), "B's message is untouched");
    assert!(s.signals(B).message_pending.contains("bbbb0000bbbb0000b"));
    // A's workers left with it: they can no longer be steered.
    assert!(matches!(s.message_worker(Actor::Human, "aaaa0000aaaa0000a", "late"), Err(ControlError::WorkerNotFound)));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resuming_one_session_rotates_only_its_nonce() {
    let (mut s, dir) = two();
    let (na, nb) = (s.nonce(A).unwrap().to_string(), s.nonce(B).unwrap().to_string());
    s.resume_session(A).unwrap();
    assert_ne!(s.nonce(A).unwrap(), na);
    assert_eq!(s.nonce(B).unwrap(), nb);
    assert!(matches!(s.resume_session("00000000-0000-4000-8000-000000000000"), Err(ControlError::ControlDisabled)));
    let _ = fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------ ADR-15 2.3: a Director-General is bound to its project

const PA: &str = "k_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PB: &str = "k_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn a_director_general_sees_and_steers_only_its_own_projects_runs() {
    let (mut s, dir) = two();
    s.set_project(A, PA).unwrap();
    s.set_project(B, PB).unwrap();
    s.set_generation(PA, 1).unwrap();
    let list = s.call_tool_for_project(PA, 1, "list_workers", &json!({})).unwrap();
    let names: Vec<&str> = list["workers"].as_array().unwrap().iter().map(|w| w["worker"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["aaaa0000aaaa0000a"]);
    for (tool, args) in [
        ("inspect_worker", json!({"worker": "bbbb0000bbbb0000b"})),
        ("message_worker", json!({"worker": "bbbb0000bbbb0000b", "text": "x"})),
        ("stop_worker", json!({"worker": "bbbb0000bbbb0000b"})),
    ] {
        assert!(matches!(s.call_tool_for_project(PA, 1, tool, &args), Err(ControlError::WorkerNotFound)), "{tool}");
    }
    // `t1` names only its own project's worker.
    s.call_tool_for_project(PA, 1, "message_worker", &json!({"worker": "t1", "text": "x"})).unwrap();
    assert_eq!((count(&dir, A, "queue"), count(&dir, B, "queue")), (1, 0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_stale_or_unknown_generation_gets_nothing() {
    let (mut s, dir) = two();
    s.set_project(A, PA).unwrap();
    // No generation was set for the project: nothing the app started can be calling.
    assert!(matches!(s.call_tool_for_project(PA, 1, "list_workers", &json!({})), Err(ControlError::StaleLeadGeneration)));
    s.set_generation(PA, 1).unwrap();
    s.set_generation(PA, 2).unwrap();
    // A process of the replaced conversation can neither read nor steer.
    for (tool, args) in [("list_workers", json!({})), ("message_worker", json!({"worker": "t1", "text": "x"}))] {
        assert!(matches!(s.call_tool_for_project(PA, 1, tool, &args), Err(ControlError::StaleLeadGeneration)), "{tool}");
    }
    assert_eq!(count(&dir, A, "queue"), 0);
    s.call_tool_for_project(PA, 2, "list_workers", &json!({})).unwrap();
    assert!(matches!(s.call_tool_for_project("k_../x", 2, "list_workers", &json!({})), Err(ControlError::InvalidArgs(_))));
    assert!(s.set_project(A, "p_not-a-ref").is_err() && s.set_generation("../x", 1).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_director_general_with_no_live_run_sees_no_workers_and_never_falls_back_to_all() {
    let (mut s, dir) = two();
    // Neither session belongs to PA: the scope is empty, not "every session".
    s.set_generation(PA, 1).unwrap();
    assert!(s.call_tool_for_project(PA, 1, "list_workers", &json!({})).unwrap()["workers"].as_array().unwrap().is_empty());
    assert!(matches!(s.call_tool_for_project(PA, 1, "message_worker", &json!({"worker": "aaaa0000aaaa0000a", "text": "x"})), Err(ControlError::ControlDisabled)));
    assert_eq!(count(&dir, A, "queue") + count(&dir, B, "queue"), 0);
    let _ = fs::remove_dir_all(&dir);
}
