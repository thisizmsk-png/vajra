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
    c.set_workers(SID, vec![worker(round)]);
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
    let bad = RunInput { ask: String::new(), max_rounds: 1, base: "x".into(), max_in_flight: 1, campaign: None, provider: None };
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
        lock(&app.control).set_workers(SID, vec![worker(3)]);
        app.control_stop_expecting("bhima", None, e.expect_generation)
    });
    assert_eq!((state_of(&r), r["receipt"]["reason"].as_str(), r["receipt"]["retry"].as_str()), ("rejected", Some("stale_generation"), Some("after_refresh")));
    assert_eq!(r["error"]["current_generation"], 3);
    // The receipt itself carries the generation the effect found, and so does its replay.
    assert_eq!(r["receipt"]["current_generation"], 3);
    let again = app.act(&e, Caller::Ui, true, "stop_worker", "bhima", Some(2), || panic!("never re-run"));
    assert_eq!(again["receipt"]["current_generation"], 3);
    assert_eq!(again["receipt"], r["receipt"]);
    let l = app.ledger().unwrap();
    assert!(!l["records"].as_array().unwrap().iter().any(|x| x["kind"] == "stop"), "attempt 3 was not stopped");
}

// Review P2: counters survive a restart. After a successful start (generation 1), a reopened app still reports
// generation 1, rejects an action expecting the old 0, and never repeats a revision a cached client holds.
#[test]
fn generations_and_revisions_survive_a_restart() {
    let t = Tmp::new();
    let (old_rev, old_receipt_rev) = {
        let app = t.app();
        let r = app.act(&env("run-0012-llll", Some(0)), Caller::Ui, false, "start_run", "proj-a", Some(0), || Ok(json!({})));
        assert_eq!(state_of(&r), "done");
        let ps = app.project_state("proj-a", None).unwrap();
        assert_eq!(ps["generations"]["project"], 1);
        (ps["rev"].as_u64().unwrap(), r["receipt"]["rev"].as_u64().unwrap())
    };
    let app = t.app();
    let ps = app.project_state("proj-a", None).unwrap();
    assert_eq!(ps["generations"]["project"], 1, "the project generation is recovered");
    let rev = ps["rev"].as_u64().unwrap();
    assert!(rev > old_rev, "a revision a client cached before the restart is never reused");
    assert!(ps["receipts"].as_array().unwrap().iter().all(|r| r["rev"].as_u64().unwrap() <= rev), "receipts never run ahead of their projection");
    assert_eq!(app.project_state("proj-a", Some(old_rev)).unwrap()["unchanged"], false);
    assert!(old_receipt_rev <= rev);
    // The delayed action that still expects generation 0 is stale; the one expecting 1 goes through.
    let bad = app.act_start_run(&env("run-0013-mmmm", Some(0)), RunInput { ask: String::new(), max_rounds: 1, base: "x".into(), max_in_flight: 1, campaign: None, provider: None }, "/nonexistent");
    assert_eq!(bad["receipt"]["reason"], "stale_generation");
    let ok = app.act(&env("run-0014-nnnn", Some(1)), Caller::Ui, false, "start_run", "proj-a", Some(1), || Ok(json!({})));
    assert_eq!(state_of(&ok), "done");
}

// A first quiet sighting never bumps: a reopen with a clock that has not moved (the same millisecond, or one that went
// back) still starts past every revision a client saw, from the durable receipts alone.
#[test]
fn a_same_ms_reopen_never_reuses_a_revision_a_client_saw() {
    let t = Tmp::new();
    let seen = {
        let app = t.app();
        let r = app.act(&env("run-0016-pppp", Some(0)), Caller::Ui, false, "start_run", "proj-a", Some(0), || Ok(json!({})));
        assert_eq!(state_of(&r), "done");
        let first = app.project_state("proj-a", None).unwrap()["rev"].as_u64().unwrap();
        let again = app.project_state("proj-a", None).unwrap()["rev"].as_u64().unwrap();
        assert_eq!(first, again, "a steady read moves nothing");
        assert_eq!(first, r["receipt"]["rev"].as_u64().unwrap(), "a first read lands on the durable receipt revision");
        first
    };
    let app = t.app();
    *lock(&app.acts) = state::Hub::open_at(&t.0.join("data"), 0).unwrap();
    let ps = app.project_state("proj-a", Some(seen)).unwrap();
    assert!(ps["rev"].as_u64().unwrap() > seen, "a clock that has not moved never hands out a revision a client saw");
    assert_eq!(ps["unchanged"], false);
}

// Review P2: an interrupted action changes the projection after restart, so a client holding the old revision
// must not be told `unchanged`.
#[test]
fn recovered_unknown_receipts_invalidate_cached_revisions() {
    let t = Tmp::new();
    let cached = {
        let app = t.app();
        let mut h = lock(&app.acts);
        let rev = h.rev("proj-a");
        let q = Request { action_id: "click-0015-oooo", project_id: "proj-a", action: "start_run", target: "proj-a", expect_generation: None, caller: Caller::Ui, human_only: false };
        assert!(matches!(h.store.begin(&q, None, rev, now_ms()).unwrap(), Begin::Run(_)));
        rev
    };
    let app = t.app();
    let ps = app.project_state("proj-a", Some(cached)).unwrap();
    assert_eq!(ps["unchanged"], false);
    assert_eq!(ps["receipts"][0]["state"], "unknown");
}

// Review P2: receipt transitions notify subscribers on their own, with no bus traffic, for accepted, terminal
// and pre-effect rejections; a replay changes nothing and says nothing.
#[test]
fn receipt_transitions_emit_changed_without_bus_output() {
    let t = Tmp::new();
    let app = t.app();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let s2 = seen.clone();
    app.set_change_sink(Box::new(move |p| s2.lock().unwrap().push(p)));
    let revs = || -> Vec<u64> { seen.lock().unwrap().iter().flat_map(|p| p["projects"].as_array().unwrap().iter().map(|x| x["rev"].as_u64().unwrap()).collect::<Vec<_>>()).collect() };
    // Accepted then done: two notifications, strictly increasing.
    app.act(&env("note-0016-pppp", None), Caller::Ui, false, "start_run", "proj-a", None, || Ok(json!({})));
    let r = revs();
    assert_eq!(r.len(), 2);
    assert!(r[0] < r[1]);
    assert_eq!(seen.lock().unwrap()[0]["projects"][0]["project_id"], "proj-a");
    // A stale-generation rejection (never reaches the effect) notifies too.
    seed_session(&app, 3);
    app.act_stop_worker(&env("note-0017-qqqq", Some(2)), "bhima", None);
    assert_eq!(revs().len(), 3);
    // Replaying either id emits nothing.
    app.act(&env("note-0016-pppp", None), Caller::Ui, false, "start_run", "proj-a", None, || Ok(json!({})));
    app.act_stop_worker(&env("note-0017-qqqq", Some(2)), "bhima", None);
    assert_eq!(revs().len(), 3);
    // And the revision the sink reported is the one project_state serves.
    assert_eq!(app.project_state("proj-a", None).unwrap()["rev"].as_u64().unwrap(), *revs().last().unwrap());
}

// Review P2: the project generation is read with the receipt decision, and starts are decided one at a time.
// A second start (new action id, same expected generation) that arrives while the first is running waits, then
// is judged against the generation the first left behind: stale, and its effect never runs.
#[test]
fn a_second_start_with_the_same_expected_generation_is_stale_after_the_first() {
    let t = Tmp::new();
    let app = t.app();
    let second_ran = std::sync::atomic::AtomicBool::new(false);
    let second = std::thread::scope(|s| {
        let h = std::cell::RefCell::new(None);
        let first = app.act(&env("run-0018-rrrr", Some(0)), Caller::Ui, false, "start_run", "proj-a", super::state::Gen::ProjectRuns, || {
            let j = s.spawn(|| {
                app.act(&env("run-0019-ssss", Some(0)), Caller::Ui, false, "start_run", "proj-a", super::state::Gen::ProjectRuns, || {
                    second_ran.store(true, Ordering::SeqCst);
                    Ok(json!({}))
                })
            });
            std::thread::sleep(std::time::Duration::from_millis(150));
            *h.borrow_mut() = Some(j);
            Ok(json!({}))
        });
        assert_eq!(state_of(&first), "done");
        h.into_inner().unwrap().join().unwrap()
    });
    assert!(!second_ran.load(Ordering::SeqCst), "the stale start never ran");
    assert_eq!((state_of(&second), second["receipt"]["reason"].as_str(), second["receipt"]["current_generation"].as_u64()), ("rejected", Some("stale_generation"), Some(1)));
}

// Audit: a panicking effect leaves an `unknown` receipt (and the start gate usable), not an `accepted` one.
#[test]
fn a_panicking_effect_reads_unknown_and_does_not_wedge_later_starts() {
    let t = Tmp::new();
    let app = t.app();
    let r = app.act(&env("run-0020-tttt", Some(0)), Caller::Ui, false, "start_run", "proj-a", super::state::Gen::ProjectRuns, || panic!("boom"));
    assert_eq!((state_of(&r), r["receipt"]["retry"].as_str()), ("unknown", Some("after_refresh")));
    let replay = app.act(&env("run-0020-tttt", Some(0)), Caller::Ui, false, "start_run", "proj-a", super::state::Gen::ProjectRuns, || Ok(json!({})));
    assert_eq!(replay["receipt"]["state"], "unknown", "never re-run");
    let next = app.act(&env("run-0021-uuuu", Some(0)), Caller::Ui, false, "start_run", "proj-a", super::state::Gen::ProjectRuns, || Ok(json!({})));
    assert_eq!(state_of(&next), "done");
}

// Review P2: a worker list that changes with nothing on the bus (a status update from the journal) advances the
// revision, after the list is published, and notifies; an unchanged list does not.
#[test]
fn a_quiet_worker_change_advances_the_revision_and_notifies() {
    let t = Tmp::new();
    let app = t.app();
    seed_session(&app, 1);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let s2 = seen.clone();
    app.set_change_sink(Box::new(move |p| s2.lock().unwrap().push(p)));
    let rev0 = app.project_state("proj-a", None).unwrap()["rev"].as_u64().unwrap();
    app.workers_published(); // first sighting of the list
    let rev1 = app.project_state("proj-a", None).unwrap()["rev"].as_u64().unwrap();
    assert!(rev1 > rev0);
    let n = seen.lock().unwrap().len();
    app.workers_published(); // nothing changed
    assert_eq!(seen.lock().unwrap().len(), n);
    assert_eq!(app.project_state("proj-a", Some(rev1)).unwrap()["unchanged"], true);
    // The worker moves to attempt 2: no bus line, but the projection changed.
    lock(&app.control).set_workers(SID, vec![worker(2)]);
    app.workers_published();
    let ps = app.project_state("proj-a", Some(rev1)).unwrap();
    assert_eq!(ps["unchanged"], false);
    assert_eq!(ps["workers"][0]["round"], 2, "the state served at the new revision has the new list");
    assert!(seen.lock().unwrap().len() > n);
}

// Review P2: a project's generation outlives a restart even after its receipts have aged out.
#[test]
fn project_generation_is_durable_independent_of_receipts() {
    let t = Tmp::new();
    {
        let app = t.app();
        let r = app.act(&env("run-0022-vvvv", Some(0)), Caller::Ui, false, "start_run", "proj-a", super::state::Gen::ProjectRuns, || Ok(json!({})));
        assert_eq!(state_of(&r), "done");
    }
    // Age the receipt out of the dedupe window by rewriting its ts in the ledger, as seven days passing would.
    let p = t.0.join("data/control/ledger.jsonl");
    let old = std::fs::read_to_string(&p).unwrap();
    let aged: String = old
        .lines()
        .map(|l| {
            let mut v: Value = serde_json::from_str(l).unwrap();
            if v["kind"] == "receipt" {
                v["receipt"]["ts_ms"] = json!(1_000);
            }
            format!("{v}\n")
        })
        .collect();
    std::fs::write(&p, aged).unwrap();
    let app = t.app();
    assert_eq!(app.project_state("proj-a", None).unwrap()["receipts"].as_array().unwrap().len(), 0, "the receipts aged out");
    assert_eq!(app.project_state("proj-a", None).unwrap()["generations"]["project"], 1);
    let stale = app.act_start_run(&env("run-0023-wwww", Some(0)), RunInput { ask: String::new(), max_rounds: 1, base: "x".into(), max_in_flight: 1, campaign: None, provider: None }, "/nonexistent");
    assert_eq!(stale["receipt"]["reason"], "stale_generation");
}

// ---- FG-07: answer a waiting task from the inbox ------------------------------------------------------------------

/// The run index line that says campaign `slug` was launched as run `run`.
fn launched(t: &Tmp, slug: &str, run: &str) {
    let e = runindex::Entry { campaign: slug.into(), session_id: SID.into(), run_id: run.into(), cwd: t.0.join("repo"), lead_session_id: None, provider: None };
    runindex::append(&t.0.join("data/runs.jsonl"), &e).unwrap();
}

fn waiting_in(t: &Tmp, slug: &str, status: &str) {
    let d = t.0.join("home/.claude/vajra/campaigns").join(slug);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("campaign.json"), json!({"slug": slug, "goal": "g", "status": "active", "created": "2026-11-01T00:00:00Z", "updated": "2026-11-01T00:00:00Z"}).to_string()).unwrap();
    fs::write(d.join("tasks.json"), json!({"campaign": slug, "schema": "vajra.tasks/v1", "tasks": [{"id": "t1", "status": status, "title": "Which key?"}]}).to_string()).unwrap();
}

/// Campaign c1, launched as the run the seeded worker belongs to (`20261105-1810`), with its task waiting.
fn waiting_task(t: &Tmp, status: &str) {
    launched(t, "c1", "20261105-1810");
    let d = t.0.join("home/.claude/vajra/campaigns/c1");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("campaign.json"), json!({"slug": "c1", "goal": "g", "status": "active", "created": "2026-11-01T00:00:00Z", "updated": "2026-11-01T00:00:00Z"}).to_string()).unwrap();
    fs::write(d.join("tasks.json"), json!({"campaign": "c1", "schema": "vajra.tasks/v1", "tasks": [{"id": "t1", "status": status, "title": "Which key?"}]}).to_string()).unwrap();
}

fn queued_messages(app: &App) -> Vec<Value> {
    app.ledger().unwrap()["records"].as_array().unwrap().iter().filter(|r| r["kind"] == "message").cloned().collect()
}

// Given a worker's task is waiting with a question, When the human answers through a receipt, Then the answer is queued
// to that worker as a message, the receipt is done, and a second click with the same action id queues nothing more.
#[test]
fn answering_a_question_messages_the_worker_once() {
    let t = Tmp::new();
    let app = t.app();
    seed_session(&app, 1);
    waiting_task(&t, "question");
    let e = env("ans-0001-aaaa", None);
    let r = app.act_answer_question(&e, "task:c1.t1", "use the key id; $(rm -rf /) is just text");
    assert_eq!(state_of(&r), "done", "{r}");
    assert_eq!(r["receipt"]["action"], "answer_question");
    assert_eq!(queued_messages(&app).len(), 1);
    assert!(queued_messages(&app)[0].to_string().contains("$(rm -rf /) is just text"), "the text is kept as plain text");
    let again = app.act_answer_question(&e, "task:c1.t1", "use the key id");
    assert_eq!(again["replayed"], true);
    assert_eq!(queued_messages(&app).len(), 1, "no second message");
}

// Given the worker moved to another attempt since the human looked, Then the answer is refused as stale.
#[test]
fn an_answer_for_an_older_attempt_is_refused() {
    let t = Tmp::new();
    let app = t.app();
    seed_session(&app, 3);
    waiting_task(&t, "question");
    let r = app.act_answer_question(&env("ans-0002-bbbb", Some(2)), "task:c1.t1", "x");
    assert_eq!((state_of(&r), r["receipt"]["reason"].as_str()), ("rejected", Some("stale_generation")));
    assert!(queued_messages(&app).is_empty());
}

// Given the worker is gone or the item is no longer waiting, Then the receipt says so and no message is queued.
#[test]
fn an_answer_with_no_worker_or_no_waiting_item_is_a_rejected_receipt() {
    let t = Tmp::new();
    let app = t.app();
    seed_session(&app, 1);
    waiting_task(&t, "question");
    let gone = app.act_answer_question(&env("ans-0003-cccc", None), "task:c1.t7", "x");
    assert_eq!(gone["receipt"]["reason"], "not_found");
    waiting_task(&t, "in_progress");
    let moved = app.act_answer_question(&env("ans-0004-dddd", None), "task:c1.t1", "x");
    assert_eq!(moved["receipt"]["reason"], "not_found");
    assert!(queued_messages(&app).is_empty());
}

// A run-level question has no worker to message: it is refused with a clear code (answered in the conversation).
#[test]
fn a_run_level_question_is_not_answerable_from_here() {
    let t = Tmp::new();
    let app = t.app();
    let r = app.act_answer_question(&env("ans-0005-eeee", None), "question:20261106-0911", "x");
    assert_eq!(r["receipt"]["reason"], "not_answerable");
}

// The answer is bounded and plain: empty and oversize text are invalid and nothing is queued.
#[test]
fn an_answer_must_be_text_of_a_sane_size() {
    let t = Tmp::new();
    let app = t.app();
    seed_session(&app, 1);
    waiting_task(&t, "question");
    for (i, bad) in ["", "   ", &"x".repeat(100_000)].into_iter().enumerate() {
        let r = app.act_answer_question(&env(&format!("ans-1000-{i:04}"), None), "task:c1.t1", bad);
        assert_eq!(r["receipt"]["state"], "rejected", "{r}");
    }
    assert!(queued_messages(&app).is_empty());
}

// Answering is a human decision: the MCP caller is refused before anything is queued.
#[test]
fn answering_is_human_only() {
    let t = Tmp::new();
    let app = t.app();
    seed_session(&app, 1);
    waiting_task(&t, "question");
    let r = app.act(&env("ans-0006-ffff", None), Caller::Mcp, true, "answer_question", "task:c1.t1", None, || app.control_message("t1", "x"));
    assert_eq!(r["receipt"]["reason"], "human_only");
    assert!(queued_messages(&app).is_empty());
}

fn worker_of(agent: &str, run: &str, round: u32) -> WorkerInfo {
    let mut w = worker(round);
    w.worker.agent_id = agent.into();
    w.task_id = Some(format!("{run}.t1"));
    w
}

fn message_workers(app: &App) -> Vec<String> {
    queued_messages(app).iter().map(|r| r["worker"].as_str().unwrap_or_default().to_string()).collect()
}

// Given two campaigns that both have a task t1 waiting and only the OTHER campaign's worker is live, When the human
// answers the first campaign's t1, Then it is refused (no worker of that campaign) and nothing reaches the other one.
#[test]
fn an_answer_never_reaches_another_campaigns_worker_with_the_same_task_name() {
    let t = Tmp::new();
    let app = t.app();
    seed_session(&app, 1); // the worker of run 20261105-1810 (campaign c2 below)
    waiting_in(&t, "c1", "question");
    launched(&t, "c1", "20261104-0900"); // c1 ran earlier; no worker of it is live
    waiting_in(&t, "c2", "question");
    launched(&t, "c2", "20261105-1810");
    let r = app.act_answer_question(&env("ans-2000-aaaa", None), "task:c1.t1", "for c1");
    assert_eq!((state_of(&r), r["receipt"]["reason"].as_str()), ("rejected", Some("not_found")), "{r}");
    assert!(queued_messages(&app).is_empty(), "nothing was queued to c2's worker");
    let ok = app.act_answer_question(&env("ans-2001-bbbb", None), "task:c2.t1", "for c2");
    assert_eq!(state_of(&ok), "done", "{ok}");
    assert_eq!(queued_messages(&app).len(), 1);
}

// Given both campaigns have a live t1, Then each answer goes to its own campaign's worker, never ambiguous.
#[test]
fn two_campaigns_sharing_a_task_name_are_answered_separately() {
    let t = Tmp::new();
    let app = t.app();
    {
        let mut c = lock(&app.control);
        c.start_session(SID, "20261105-1810").unwrap();
        c.set_workers(SID, vec![worker_of("aaaaaaaaaaaaaaaaa", "20261105-1810", 1), worker_of("bbbbbbbbbbbbbbbbb", "20261104-0900", 1)]);
    }
    waiting_in(&t, "c1", "question");
    launched(&t, "c1", "20261104-0900");
    waiting_in(&t, "c2", "question");
    launched(&t, "c2", "20261105-1810");
    assert_eq!(state_of(&app.act_answer_question(&env("ans-2002-cccc", None), "task:c1.t1", "one")), "done");
    assert_eq!(state_of(&app.act_answer_question(&env("ans-2003-dddd", None), "task:c2.t1", "two")), "done");
    assert_eq!(message_workers(&app), ["bbbbbbbbbbbbbbbbb", "aaaaaaaaaaaaaaaaa"]);
}

// Given the worker moved to a newer attempt between the receipt's check and the message's publication, Then the
// message path itself refuses (the check is under the control lock), and nothing is queued.
#[test]
fn an_answer_is_checked_against_the_attempt_at_publication() {
    let t = Tmp::new();
    let app = t.app();
    seed_session(&app, 2); // the worker is now on attempt 2
    waiting_task(&t, "question");
    // The receipt's own pre-check saw attempt 1 (the stale read it was given), so it lets the effect run...
    let r = app.act(&env("ans-4000-aaaa", Some(1)), Caller::Ui, true, "answer_question", "task:c1.t1", Some(1), || app.control_message_expecting("t1", "old answer", Some(1)));
    // ...and the message path refuses it.
    assert_eq!((state_of(&r), r["receipt"]["reason"].as_str()), ("rejected", Some("stale_generation")), "{r}");
    assert!(queued_messages(&app).is_empty());
    let ok = app.act_answer_question(&env("ans-4001-bbbb", Some(2)), "task:c1.t1", "new answer");
    assert_eq!(state_of(&ok), "done", "{ok}");
}

// The workflow writes tasks.json ids already namespaced (`<run>.t1`), so a real inbox item is `task:<slug>.<run>.t1`.
// Given such an item, Then the answer reaches the worker whose task id is that same `<run>.t1`.
#[test]
fn an_answer_to_a_workflow_namespaced_task_reaches_its_worker() {
    let t = Tmp::new();
    let app = t.app();
    seed_session(&app, 1); // worker task id 20261105-1810.t1
    launched(&t, "c1", "20261105-1810");
    let d = t.0.join("home/.claude/vajra/campaigns/c1");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("campaign.json"), json!({"slug": "c1", "goal": "g", "status": "active", "created": "2026-11-01T00:00:00Z", "updated": "2026-11-01T00:00:00Z"}).to_string()).unwrap();
    fs::write(d.join("tasks.json"), json!({"campaign": "c1", "schema": "vajra.tasks/v1", "tasks": [{"id": "20261105-1810.t1", "status": "question", "title": "Which key?"}]}).to_string()).unwrap();
    let r = app.act_answer_question(&env("ans-5000-aaaa", None), "task:c1.20261105-1810.t1", "use the key id");
    assert_eq!(state_of(&r), "done", "{r}");
    assert_eq!(queued_messages(&app).len(), 1);
    // Another campaign's run never matches, namespaced or not.
    let other = app.act_answer_question(&env("ans-5001-bbbb", None), "task:c2.20261105-1810.t1", "x");
    assert_eq!(other["receipt"]["reason"], "not_found");
}

// #107 review sweep: the worker list changed with no bump of its own (here straight on the control service, as
// `end_session` does) still moves the revision, so a `since_rev` poll never keeps an old list; steady stays unchanged.
#[test]
fn a_worker_list_changed_without_a_bump_moves_the_revision() {
    let t = Tmp::new();
    let app = t.app();
    let rev0 = app.project_state("proj-w", None).unwrap()["rev"].as_u64().unwrap();
    assert_eq!(app.project_state("proj-w", Some(rev0)).unwrap()["unchanged"], true);
    seed_session(&app, 2);
    let after = app.project_state("proj-w", Some(rev0)).unwrap();
    assert_eq!(after["unchanged"], false, "{after}");
    assert!(!after["workers"].as_array().unwrap().is_empty(), "{after}");
    assert_eq!(app.project_state("proj-w", after["rev"].as_u64()).unwrap()["unchanged"], true, "steady again");
    lock(&app.control).end_session(SID);
    assert_eq!(app.project_state("proj-w", after["rev"].as_u64()).unwrap()["unchanged"], false, "the session ended");
}

// Batch-3 repair: a project_state read whose quiet fields did not move sends no `changed` event (an empty one woke every
// subscriber on every read), and no `changed` payload ever names zero projects.
#[test]
fn a_steady_project_state_read_emits_nothing() {
    let t = Tmp::new();
    let app = t.app();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let s2 = seen.clone();
    app.set_change_sink(Box::new(move |p| s2.lock().unwrap().push(p)));
    let rev = app.project_state("proj-steady", None).unwrap()["rev"].as_u64().unwrap();
    let after_first = seen.lock().unwrap().len();
    for _ in 0..3 {
        assert_eq!(app.project_state("proj-steady", Some(rev)).unwrap()["unchanged"], true);
    }
    let got = seen.lock().unwrap().clone();
    assert_eq!(got.len(), after_first, "{got:?}");
    assert!(got.iter().all(|p| !p["projects"].as_array().unwrap().is_empty()), "{got:?}");
}
