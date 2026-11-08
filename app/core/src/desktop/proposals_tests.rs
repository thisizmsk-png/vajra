//! ADR-15 2.4 (D4 option A): a Director-General proposes, a human starts. No real `claude` is ever started here: the
//! success path takes a recording starter, and the held path stops at the caps, before any spawn.

use super::*;
use crate::control::ledger::Ledger;
use crate::control::service::ControlError;
use crate::desktop::Run;
use crate::crew::redact::Redactor;
use crate::sessions::SpawnSpec;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);
const SID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
const RUN: &str = "20261108-1200";
const SHA: &str = "ea73cb05d639b945e21c667e17103e56404861a8";

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        let d = std::env::temp_dir().join(format!("vajra-proposals-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("home/.claude/vajra/campaigns")).unwrap();
        fs::create_dir_all(d.join("repo/.git")).unwrap();
        let d = d.canonicalize().unwrap();
        super::super::test_project(&d.join("data"), &d.join("home"), &d.join("repo"));
        Self(d)
    }
    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
    fn repo(&self) -> PathBuf {
        self.0.join("repo")
    }
    fn ledger(&self) -> Vec<Value> {
        Ledger::read_all(&self.0.join("data/control/ledger.jsonl")).unwrap()
    }
    fn audit(&self) -> String {
        fs::read_to_string(self.home().join(".claude/vajra/audit.log")).unwrap_or_default()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// An app whose project has a Director-General record (as after its first ask) and one live control session, and
/// whose Director-General proposed `asks`. Returns the app and the project id.
fn proposed(t: &Tmp, asks: &[&str]) -> (App, String) {
    proposed_at(t, &t.repo(), asks)
}

/// As `proposed`, for a project whose repo is at `repo`.
fn proposed_at(t: &Tmp, repo: &Path, asks: &[&str]) -> (App, String) {
    let pid = super::super::test_project(&t.0.join("data"), &t.home(), repo);
    let app = App::open(&t.0.join("data"), &t.home()).unwrap();
    let root = repo.canonicalize().unwrap();
    app.orchestrator(&pid, &root).unwrap();
    {
        let mut svc = lock(&app.control);
        svc.start_session(SID, RUN).unwrap();
        svc.set_project(SID, &pid).unwrap();
        svc.set_generation(&pid, 1).unwrap();
        for a in asks {
            svc.call_tool_for_project(&pid, 1, "propose_run", &json!({"ask": a, "base": SHA, "rounds": 1})).unwrap();
        }
    }
    (app, pid)
}

fn proposals(app: &App) -> Vec<Value> {
    app.inbox().unwrap()["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "run_proposal").cloned().collect()
}

fn code(v: &Value) -> &str {
    v["error"].as_str().unwrap_or("")
}

fn states(t: &Tmp, kind: &str) -> Vec<String> {
    t.ledger().iter().filter(|r| r["kind"] == kind).map(|r| r["state"].as_str().unwrap().to_string()).collect()
}

#[test]
fn the_inbox_shows_the_exact_ask_and_start_runs_exactly_it_in_the_project_folder() {
    let t = Tmp::new();
    let ask = "add a CHANGELOG entry\n\tfor the caps work";
    let (app, pid) = proposed(&t, &[ask]);
    let items = proposals(&app);
    assert_eq!(items.len(), 1);
    let it = &items[0];
    assert_eq!((it["ask"].as_str(), it["body"].as_str(), it["base"].as_str(), it["rounds"].as_u64()), (Some(ask), Some(ask), Some(SHA), Some(1)));
    let root = t.repo().to_string_lossy().into_owned();
    assert_eq!((it["project_id"].as_str(), it["project"].as_str(), it["project_name"].as_str()), (Some(pid.as_str()), Some(root.as_str()), Some("repo")));
    assert_eq!(it["source"].as_str(), Some(FROM_DIRECTOR_GENERAL));
    assert_eq!(it["actions"], json!(["start", "dismiss"]));
    let id = it["id"].as_str().unwrap().to_string();
    let sha = it["proposal_sha256"].as_str().unwrap().to_string();

    // A hash other than the one shown starts nothing and writes nothing.
    let seen: RefCell<Vec<(RunInput, String)>> = RefCell::new(Vec::new());
    let starter = |i: RunInput, cwd: &str| {
        seen.borrow_mut().push((i, cwd.to_string()));
        Ok(json!({"run_id": "20261108-1205", "session_id": "x"}))
    };
    let e = app.start_proposal_with(&id, &"0".repeat(64), starter).unwrap_err();
    assert_eq!(code(&e), "proposal_changed");
    assert!(seen.borrow().is_empty() && states(&t, "approve").is_empty());

    let r = app.start_proposal_with(&id, &sha, |i, cwd| {
        seen.borrow_mut().push((i, cwd.to_string()));
        Ok(json!({"run_id": "20261108-1205", "session_id": "x"}))
    });
    let r = r.unwrap();
    assert_eq!(r["state"], "started");
    let got = seen.borrow();
    let (input, cwd) = &got[0];
    assert_eq!((input.ask.as_str(), input.base.as_str(), input.max_rounds, input.max_in_flight, input.campaign.as_deref()), (ask, SHA, 1, caps::DEFAULT_RUN_WORKERS, None));
    assert_eq!(Path::new(cwd), t.repo(), "it runs in the project folder");
    // Requested first, then delivered with the run it started; the audit line names the item.
    assert_eq!(states(&t, "approve"), ["requested", "delivered"]);
    let fin = t.ledger().into_iter().find(|r| r["kind"] == "approve" && r["state"] == "delivered").unwrap();
    assert_eq!((fin["item"].as_str(), fin["started_run_id"].as_str(), fin["proposal_sha256"].as_str()), (Some(id.as_str()), Some("20261108-1205"), Some(sha.as_str())));
    assert!(t.audit().contains(&format!("approve id={} item={id}", r["action_id"].as_str().unwrap())));
    assert!(proposals(&app).is_empty(), "a started proposal leaves the inbox");
    // Once only.
    drop(got);
    let again = app.start_proposal_with(&id, &sha, |_, _| panic!("must not start twice")).unwrap_err();
    assert_eq!(code(&again), "already_decided");
}

#[test]
fn a_held_start_leaves_the_proposal_open() {
    let t = Tmp::new();
    let (app, _) = proposed(&t, &["build the thing"]);
    // A run of the same project is live: the real start path is held by the caps before anything spawns.
    let stream = "run-1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    app.sup.spawn(stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: t.repo(), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.into(), Run { session_id: "1b4e28ba-2fa1-41d2-883f-0016d3cca427".into(), run_id: "20261108-1100".into(), campaign: "c".into(), cwd: t.repo(), nonces: Redactor::default() });
    let it = proposals(&app).remove(0);
    let (id, sha) = (it["id"].as_str().unwrap(), it["proposal_sha256"].as_str().unwrap());
    let held = app.start_proposal(id, sha).unwrap_err();
    assert_eq!((code(&held), held["reason"].as_str()), ("run_held", Some("project_busy")), "{held}");
    assert_eq!(states(&t, "approve"), ["requested", "failed"]);
    assert!(!t.audit().contains("approve id="), "a failed start has no audit line");
    assert_eq!(proposals(&app).len(), 1, "still waiting for the human");
    app.sup.kill(stream).unwrap();
}

#[test]
fn dismiss_closes_a_proposal_and_runs_nothing() {
    let t = Tmp::new();
    let (app, _) = proposed(&t, &["one", "two"]);
    let items = proposals(&app);
    let id = items[0]["id"].as_str().unwrap();
    assert_eq!(app.dismiss_proposal(id).unwrap()["state"], "dismissed");
    assert_eq!(proposals(&app).len(), 1);
    assert_eq!(code(&app.dismiss_proposal(id).unwrap_err()), "already_decided");
    assert_eq!(code(&app.start_proposal(id, items[0]["proposal_sha256"].as_str().unwrap()).unwrap_err()), "already_decided");
    assert_eq!(states(&t, "approve"), Vec::<String>::new(), "nothing was started");
    for bad in ["", "plan:20261108-1200.0", "proposal:../x", "proposal:a0"] {
        assert_eq!(code(&app.dismiss_proposal(bad).unwrap_err()), "invalid_args", "{bad}");
    }
    assert_eq!(code(&app.dismiss_proposal(&format!("proposal:a{}", "f".repeat(28))).unwrap_err()), "not_found");
}

#[test]
fn a_proposal_line_whose_fields_do_not_match_its_hash_is_never_started() {
    let t = Tmp::new();
    let (app, pid) = proposed(&t, &[]);
    // A line whose stored hash is for another ask (as if the ask were edited after the hash was taken).
    let item = &format!("proposal:a{}", "0".repeat(28));
    let honest = crate::control::service::proposal_sha(&pid, "harmless", SHA, 1);
    let line = json!({"v": 1, "ts": "2026-11-08T12:00:00.000Z", "action_id": &item[9..], "kind": "propose", "state": "awaiting_human_confirm", "actor": "lead", "via": "mcp", "source": "lead",
        "item": item, "project": pid, "generation": 1, "ask": "rm -rf everything", "base": SHA, "rounds": 1, "proposal_sha256": honest});
    Ledger::open(&t.0.join("data/control/ledger.jsonl")).unwrap().append(&line).unwrap();
    let it = proposals(&app).into_iter().find(|i| i["id"] == item.as_str()).expect("listed, so the human can see and dismiss it");
    let e = app.start_proposal_with(item, it["proposal_sha256"].as_str().unwrap(), |_, _| panic!("must not start")).unwrap_err();
    assert_eq!(code(&e), "proposal_changed");
}

#[test]
fn a_lead_bound_to_another_project_cannot_reach_this_projects_proposals() {
    let t = Tmp::new();
    let (app, pid) = proposed(&t, &["ours"]);
    let other = &super::super::test_project(&t.0.join("data"), &t.home(), &t.0.join("other"));
    let mut svc = lock(&app.control);
    svc.set_generation(other, 1).unwrap();
    // It may propose for its own project only; start and dismiss are not tools at all.
    svc.call_tool_for_project(other, 1, "propose_run", &json!({"ask": "theirs"})).unwrap();
    assert_eq!(svc.open_proposals(Some(&pid)).unwrap().len(), 1);
    assert!(matches!(svc.call_tool_for_project(other, 1, "start_proposal", &json!({})), Err(ControlError::UnknownTool)));
    assert!(matches!(svc.call_tool_for_project(&pid, 2, "propose_run", &json!({"ask": "stale"})), Err(ControlError::StaleLeadGeneration)));
}

/// #90 review: the UI matches a proposal to its project by the project's full recorded root, so `project` is that root
/// (not the folder name, which two projects can share) and `project_name` is the short label.
#[test]
fn a_proposal_names_its_project_by_the_full_recorded_root() {
    let t = Tmp::new();
    let nested = t.0.join("work/clients/harbor-api");
    fs::create_dir_all(nested.join(".git")).unwrap();
    // Another project with the same folder name elsewhere: the label alone could not tell them apart.
    fs::create_dir_all(t.0.join("other/harbor-api/.git")).unwrap();
    let (app, pid) = proposed_at(&t, &nested, &["tidy the README"]);
    let items = proposals(&app);
    assert_eq!(items.len(), 1);
    let it = &items[0];
    let recorded = app.orchestrator_root(&pid).expect("the recorded root");
    assert_eq!(recorded, nested, "the recorded root is the nested repo");
    assert_eq!(it["project"].as_str(), Some(nested.to_string_lossy().as_ref()), "{it}");
    assert_eq!(it["project_name"].as_str(), Some("harbor-api"));
    assert_eq!(it["project_id"].as_str(), Some(pid.as_str()));
    assert_ne!(super::super::orchestrator::project_ref(&t.0.join("other/harbor-api")).0, pid, "same name, different project");
}

/// A `propose` line as FG-02b wrote it, for project `project` (a stand-in id from before the store, here).
fn old_proposal(t: &Tmp, project: &str, n: char) -> String {
    let item = format!("proposal:a{}", n.to_string().repeat(28));
    let sha = crate::control::service::proposal_sha(project, "tidy the README", SHA, 1);
    let line = json!({"v": 1, "ts": "2026-11-08T12:00:00.000Z", "action_id": &item[9..], "kind": "propose", "state": "awaiting_human_confirm", "actor": "lead", "via": "mcp", "source": "lead",
        "item": item, "project": project, "generation": 1, "ask": "tidy the README", "base": SHA, "rounds": 1, "proposal_sha256": sha});
    Ledger::open(&t.0.join("data/control/ledger.jsonl")).unwrap().append(&line).unwrap();
    item
}

/// FG-02c 3.5 tests 22-24: a proposal names its store project (id, root and the store's name); an archived project's
/// proposal starts nothing and stays open; one from before the store is dismiss-only and can never start, and is
/// listed under the project its stand-in's conversation went to.
#[test]
fn proposals_belong_to_store_projects_and_old_ones_can_only_be_dismissed() {
    let t = Tmp::new();
    let (app, pid) = proposed(&t, &["ours"]);
    app.projects.rename(&pid, "Harbor", None).unwrap();
    let it = proposals(&app).into_iter().next().unwrap();
    assert_eq!((it["project_id"].as_str(), it["project_name"].as_str(), it["actions"].clone()), (Some(pid.as_str()), Some("Harbor"), json!(["start", "dismiss"])), "{it}");
    // Archived: nothing starts, nothing is recorded, and the proposal is still open.
    app.projects.archive(&pid, None).unwrap();
    let e = app.start_proposal_with(it["id"].as_str().unwrap(), it["proposal_sha256"].as_str().unwrap(), |_, _| panic!("must not start")).unwrap_err();
    assert_eq!(code(&e), "project_archived", "{e}");
    assert_eq!(states(&t, "approve"), Vec::<String>::new());
    assert_eq!(proposals(&app).len(), 1, "still open");
    app.projects.add(&t.repo(), true).unwrap();
    // From before the store, not carried over: dismiss only, never started.
    let k = "k_0123456789abcdef0123456789abcdef";
    let item = old_proposal(&t, k, '1');
    let old = proposals(&app).into_iter().find(|i| i["id"] == item.as_str()).unwrap();
    assert_eq!((old["actions"].clone(), old["pre_store"].as_bool(), old["project_id"].as_str()), (json!(["dismiss"]), Some(true), Some(k)), "{old}");
    let e = app.start_proposal_with(&item, old["proposal_sha256"].as_str().unwrap(), |_, _| panic!("must not start")).unwrap_err();
    assert_eq!(code(&e), "proposal_changed");
    assert_eq!(app.dismiss_proposal(&item).unwrap()["state"], "dismissed");
    // Carried over (the stand-in's conversation went to this project): listed under it, still dismiss-only.
    let kdir = t.0.join("data/control/projects").join(k).join("orchestrator");
    fs::create_dir_all(&kdir).unwrap();
    fs::write(kdir.join("mapped-to"), &pid).unwrap();
    let item = old_proposal(&t, k, '2');
    let old = proposals(&app).into_iter().find(|i| i["id"] == item.as_str()).unwrap();
    assert_eq!((old["project_id"].as_str(), old["project_name"].as_str(), old["actions"].clone()), (Some(pid.as_str()), Some("Harbor"), json!(["dismiss"])), "{old}");
}

/// FG-02c c3 (decision Q2): start and dismiss through receipts. A retried dismiss replays its receipt; a start that
/// does not match what was shown is a rejected receipt and the proposal stays open; the envelope must name the
/// project the inbox lists the proposal under (for an old one, the project its conversation went to).
#[test]
fn proposal_decisions_through_receipts_run_once_and_name_the_listed_project() {
    use crate::desktop::state::Envelope;
    let t = Tmp::new();
    let (app, pid) = proposed(&t, &["one", "two"]);
    let env = |a: &str, p: &str| Envelope { action_id: a.into(), project_id: p.into(), expect_generation: None };
    let items = proposals(&app);
    let (one, two) = (items[0]["id"].as_str().unwrap(), items[1]["id"].as_str().unwrap());
    let other = super::super::test_project(&t.0.join("data"), &t.home(), &t.0.join("other"));
    let r = app.act_dismiss_proposal(&env("a_dismiss_0001", &other), one);
    assert_eq!((r["receipt"].is_null(), code(&r["error"])), (true, "project_mismatch"), "{r}");
    let r = app.act_dismiss_proposal(&env("a_dismiss_0002", &pid), one);
    assert_eq!((r["receipt"]["state"].as_str(), r["result"]["state"].as_str()), (Some("done"), Some("dismissed")), "{r}");
    let again = app.act_dismiss_proposal(&env("a_dismiss_0002", &pid), one);
    assert_eq!((again["replayed"].as_bool(), &again["receipt"]), (Some(true), &r["receipt"]), "a retry replays, it does not say already_decided");
    // A start with a hash other than the one shown: rejected, nothing recorded, still open.
    let bad = app.act_start_proposal(&env("a_start_bad_001", &pid), two, &"0".repeat(64));
    assert_eq!((bad["receipt"]["state"].as_str(), bad["receipt"]["reason"].as_str()), (Some("rejected"), Some("proposal_changed")), "{bad}");
    assert_eq!(states(&t, "approve"), Vec::<String>::new());
    assert_eq!(proposals(&app).len(), 1);
    // An old proposal carried over to this project: the receipt names this project, and it is dismiss-only.
    let k = "k_0123456789abcdef0123456789abcdef";
    let kdir = t.0.join("data/control/projects").join(k).join("orchestrator");
    fs::create_dir_all(&kdir).unwrap();
    fs::write(kdir.join("mapped-to"), &pid).unwrap();
    let item = old_proposal(&t, k, '3');
    let sha = proposals(&app).into_iter().find(|i| i["id"] == item.as_str()).unwrap()["proposal_sha256"].as_str().unwrap().to_string();
    assert_eq!(code(&app.act_start_proposal(&env("a_start_old_001", k), &item, &sha)["error"]), "project_mismatch");
    let s = app.act_start_proposal(&env("a_start_old_002", &pid), &item, &sha);
    assert_eq!(s["receipt"]["reason"], "proposal_changed", "{s}");
    let d = app.act_dismiss_proposal(&env("a_dismiss_old_01", &pid), &item);
    assert_eq!(d["receipt"]["state"], "done", "{d}");
}
