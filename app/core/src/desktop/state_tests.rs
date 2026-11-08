//! Receipt-wrapped actions and the project_state projection (FG-11, ADR-15 section 3).
use super::*;
use crate::control::receipt::{Begin, Caller, Request};
use crate::control::service::WorkerInfo;
use crate::crew::tree::{Status, Worker};
use super::state::now_ms;
use std::cell::Cell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);
const SID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Self {
        let d = std::env::temp_dir().join(format!("vajra-state-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("home/.claude/vajra/campaigns")).unwrap();
        Self(d)
    }
    fn app(&self) -> App {
        App::open(&self.0.join("data"), &self.0.join("home")).unwrap()
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn env(id: &str, expect: Option<u64>) -> Envelope {
    Envelope { action_id: id.into(), project_id: "proj-a".into(), expect_generation: expect }
}

fn worker(round: u32) -> WorkerInfo {
    WorkerInfo {
        worker: Worker { agent_id: "aa870dd40540481f0".into(), engine_id: "bhima".into(), label: format!("task.t1.r{round}"), prompt_first_line: None, branch: None, status: Status::InProgress },
        task_id: Some("20261105-1810.t1".into()),
        title: None,
        round: Some(round),
        live: true,
        last_activity: None,
        cwd: None,
        lines: vec![],
    }
}

fn seed_session(app: &App, round: u32) {
    let mut c = lock(&app.control);
    c.start_session(SID, "20261105-1810").unwrap();
    c.set_workers(vec![worker(round)]);
}

fn state_of(v: &Value) -> &str {
    v["receipt"]["state"].as_str().unwrap_or("")
}

// Given a button is clicked twice with the same action_id, When the effect is slow to answer,
// Then it runs once; the second answer is the stored receipt (accepted then done), never a second effect.
#[test]
fn double_click_runs_the_effect_once() {
    let t = Tmp::new();
    let app = t.app();
    let runs = Cell::new(0);
    let go = || {
        runs.set(runs.get() + 1);
        Ok(json!({"ok": true}))
    };
    let first = app.act(&env("click-0001-aaaa", None), Caller::Ui, false, "start_run", "proj-a", None, go);
    assert_eq!(state_of(&first), "done");
    let second = app.act(&env("click-0001-aaaa", None), Caller::Ui, false, "start_run", "proj-a", None, || {
        runs.set(runs.get() + 1);
        Ok(json!({}))
    });
    assert_eq!(runs.get(), 1);
    assert_eq!(second["replayed"], true);
    assert_eq!(second["receipt"], first["receipt"]);
}

// Given the app died after "accepted" and before the outcome, When the same action_id is retried after restart,
// Then it answers unknown, runs nothing, and project_state shows the unknown receipt.
#[test]
fn retry_after_unknown_does_not_rerun_and_is_visible_in_project_state() {
    let t = Tmp::new();
    {
        let app = t.app();
        let mut h = lock(&app.acts);
        let rev = h.rev("proj-a");
        let q = Request { action_id: "click-0002-bbbb", project_id: "proj-a", action: "start_run", target: "proj-a", expect_generation: None, caller: Caller::Ui, human_only: false };
        assert!(matches!(h.store.begin(&q, None, rev, now_ms()).unwrap(), Begin::Run(_)));
    } // crash before the effect reported
    let app = t.app();
    let ran = Cell::new(false);
    let r = app.act(&env("click-0002-bbbb", None), Caller::Ui, false, "start_run", "proj-a", None, || {
        ran.set(true);
        Ok(json!({}))
    });
    assert!(!ran.get(), "an unknown action is never re-run");
    assert_eq!((state_of(&r), r["receipt"]["retry"].as_str()), ("unknown", Some("after_refresh")));
    let ps = app.project_state("proj-a", None).unwrap();
    assert_eq!(ps["receipts"][0]["state"], "unknown");
}

// Given the worker is on attempt 3, When a stop carrying attempt 2 arrives, Then it is rejected stale_generation
// and no stop is written; with the right generation it goes through.
#[test]
fn stale_worker_generation_is_rejected_and_the_current_one_is_accepted() {
    let t = Tmp::new();
    let app = t.app();
    seed_session(&app, 3);
    let stale = app.act_stop_worker(&env("stop-0003-cccc", Some(2)), "bhima", None);
    assert_eq!((state_of(&stale), stale["receipt"]["reason"].as_str()), ("rejected", Some("stale_generation")));
    assert_eq!(stale["receipt"]["current_generation"], 3);
    assert!(stale.get("result").is_none() && stale.get("error").is_none(), "the effect never ran");
    let l = app.ledger().unwrap();
    assert!(!l["records"].as_array().unwrap().iter().any(|r| r["kind"] == "stop"), "no stop line was recorded");
    let ok = app.act_stop_worker(&env("stop-0004-dddd", Some(3)), "bhima", None);
    assert_eq!(state_of(&ok), "done", "{ok}");
}

// Given the Lead (MCP) asks for a human-only action, Then it is refused before the effect.
#[test]
fn mcp_cannot_take_human_only_actions() {
    let t = Tmp::new();
    let app = t.app();
    let ran = Cell::new(false);
    let r = app.act(&env("mcp-0005-eeee", None), Caller::Mcp, true, "stop_worker", "w", None, || {
        ran.set(true);
        Ok(json!({}))
    });
    assert!(!ran.get());
    assert_eq!(r["receipt"]["reason"], "human_only");
}

// A refusal by the effect (run_active, control_disabled) is a rejected receipt carrying the code, not a failure.
#[test]
fn an_effect_refusal_is_a_rejected_receipt_with_its_code() {
    let t = Tmp::new();
    let app = t.app();
    let r = app.act_stop_worker(&env("stop-0006-ffff", None), "w1", None);
    assert_eq!((state_of(&r), r["receipt"]["reason"].as_str()), ("rejected", Some("control_disabled")));
    assert_eq!(r["error"]["error"], "control_disabled");
}

// All surfaces read one projection: the revision moves with every action and `since_rev` answers unchanged.
#[test]
fn project_state_is_revisioned() {
    let t = Tmp::new();
    let app = t.app();
    let a = app.project_state("proj-a", None).unwrap();
    let rev = a["rev"].as_u64().unwrap();
    let same = app.project_state("proj-a", Some(rev)).unwrap();
    assert_eq!(same["unchanged"], true);
    app.act(&env("rev-0007-gggg", None), Caller::Ui, false, "start_run", "proj-a", None, || Ok(json!({})));
    let b = app.project_state("proj-a", Some(rev)).unwrap();
    assert_eq!(b["unchanged"], false);
    assert!(b["rev"].as_u64().unwrap() > rev);
    // Another project is unaffected.
    assert_eq!(app.project_state("proj-b", None).unwrap()["receipts"].as_array().unwrap().len(), 0);
    // A bus change bumps every known project and says so.
    let bumped = app.bump_all_projects();
    assert!(bumped.iter().any(|p| p["project_id"] == "proj-a"));
    assert!(app.project_state("proj-a", Some(b["rev"].as_u64().unwrap())).unwrap()["unchanged"] == false);
}

#[test]
fn start_run_generation_guards_a_second_start_that_saw_the_old_state() {
    let t = Tmp::new();
    let app = t.app();
    // A first start through a receipt (bad input: refused, so the project generation does not move).
    let bad = RunInput { ask: String::new(), max_rounds: 1, base: "x".into(), max_in_flight: 1, campaign: None };
    let r = app.act_start_run(&env("run-0008-hhhh", Some(0)), bad.clone(), "/nonexistent");
    assert_eq!(state_of(&r), "rejected");
    let ps = app.project_state("proj-a", None).unwrap();
    assert_eq!(ps["generations"]["project"], 0);
    // A start that saw generation 5 is stale against generation 0.
    let r2 = app.act_start_run(&env("run-0009-iiii", Some(5)), bad, "/nonexistent");
    assert_eq!(r2["receipt"]["reason"], "stale_generation");
}

// Approve plan through a receipt: a refused approval is a rejected receipt, and clicking again with the same id
// replays that receipt instead of re-deciding.
#[test]
fn approve_plan_receipt_replays_on_a_second_click() {
    let t = Tmp::new();
    let app = t.app();
    let a = app.act_approve_plan(&env("appr-0010-jjjj", None), "plan:not-a-plan", &"a".repeat(64));
    assert_eq!(state_of(&a), "rejected");
    assert_eq!(a["error"]["error"], "invalid_args");
    let b = app.act_approve_plan(&env("appr-0010-jjjj", None), "plan:not-a-plan", &"a".repeat(64));
    assert_eq!((b["replayed"].clone(), b["receipt"].clone()), (json!(true), a["receipt"].clone()));
}

// Review P1: the attempt is checked under the control lock, together with resolving the worker and publishing the
// stop. A refresh that swaps attempt 2 for attempt 3 between the receipt being accepted and the effect must be
// rejected, and must not stop the replacement.
#[test]
fn a_worker_replaced_between_acceptance_and_effect_is_not_stopped() {
    let t = Tmp::new();
    let app = t.app();
    seed_session(&app, 2);
    let e = env("stop-0011-kkkk", Some(2));
    // The generation read at begin sees attempt 2; the replacement lands before the effect runs.
    let r = app.act(&e, Caller::Ui, true, "stop_worker", "bhima", Some(2), || {
        lock(&app.control).set_workers(vec![worker(3)]);
        app.control_stop_expecting("bhima", None, e.expect_generation)
    });
    assert_eq!((state_of(&r), r["receipt"]["reason"].as_str(), r["receipt"]["retry"].as_str()), ("rejected", Some("stale_generation"), Some("after_refresh")));
    assert_eq!(r["error"]["current_generation"], 3);
    let l = app.ledger().unwrap();
    assert!(!l["records"].as_array().unwrap().iter().any(|x| x["kind"] == "stop"), "attempt 3 was not stopped");
}
