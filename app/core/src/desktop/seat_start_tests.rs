//! ADR-16 16.1 to 16.3: starting a proposal with worker picks. Temp HOME, fake CLIs on a temp PATH, a fake Keychain store:
//! never the real Keychain, `~/.codex` or `~/.claude`.

use super::*;
use crate::control::ledger::Ledger;
use crate::desktop::{App, lock};
use crate::preferences::{Patch, Store as Prefs};
use crate::providers::keychain::{Error as KeyError, Key, Keys, Store};
use crate::sessions::argv::RunInput;
use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);
const SID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
const RUN: &str = "20261108-1200";
const SHA: &str = "ea73cb05d639b945e21c667e17103e56404861a8";

struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Self {
        let d = std::env::temp_dir().join(format!("vajra-seat-start-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("repo/.git")).unwrap();
        fs::create_dir_all(d.join("home/.claude/vajra/campaigns")).unwrap();
        let skills = d.join("home/.claude/skills/vajra");
        fs::create_dir_all(skills.join("scripts")).unwrap();
        for f in ["scripts/vajra-campaign.sh", "scripts/vajra-verify-work.sh", "scripts/vajra_tasks.py", "scripts/vajra_kb.py"] {
            fs::write(skills.join(f), "").unwrap();
        }
        fs::write(skills.join("package.json"), r#"{"version": "0.6.2"}"#).unwrap();
        let bin = d.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for (n, v) in [("claude", "2.1.296 (Claude Code)"), ("codex", "codex-cli 0.166.0")] {
            fs::write(bin.join(n), format!("#!/bin/sh\ncase \"$1\" in --version) echo '{v}';; esac\n")).unwrap();
            fs::set_permissions(bin.join(n), fs::Permissions::from_mode(0o755)).unwrap();
        }
        let d = d.canonicalize().unwrap();
        crate::desktop::test_project(&d.join("data"), &d.join("home"), &d.join("repo"));
        Self(d)
    }
    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
    fn ledger(&self) -> Vec<Value> {
        Ledger::read_all(&self.data().join("control/ledger.jsonl")).unwrap()
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A Keychain that holds nothing and can only answer `exists`.
struct Empty;
impl Store for Empty {
    fn exists(&self, _: &str, _: &str) -> Result<bool, KeyError> {
        Ok(false)
    }
    fn add(&self, _: &str, _: &str, _: &Key) -> Result<(), KeyError> {
        panic!("no key is written")
    }
    fn delete(&self, _: &str, _: &str) -> Result<bool, KeyError> {
        panic!("no key is deleted")
    }
}

/// An app with a Director-General session, the CLIs on a temp PATH, the route on, and codex allowed next to claude.
fn app_for(t: &Tmp, route: bool) -> (App, String) {
    let pid = crate::desktop::test_project(&t.data(), &t.0.join("home"), &t.0.join("repo"));
    let app = App::open(&t.data(), &t.0.join("home")).unwrap();
    app.orchestrator(&pid, &t.0.join("repo")).unwrap();
    app.seat_env.for_test(vec![t.0.join("bin")], Keys::new(Box::new(Empty)), route);
    let patch: Patch = serde_json::from_value(json!({"agents": {"default": {"account": "claude"}, "roles": {}, "allowed": [{"account": "claude"}, {"account": "codex"}]}})).unwrap();
    Prefs::new(t.data()).update(None, 0, patch, false).unwrap();
    let mut svc = lock(&app.control);
    svc.start_session(SID, RUN).unwrap();
    svc.set_project(SID, &pid).unwrap();
    svc.set_generation(&pid, 1).unwrap();
    svc.set_seats_enabled(true);
    drop(svc);
    (app, pid)
}

fn propose(app: &App, pid: &str, args: Value) -> Result<Value, crate::control::service::ControlError> {
    let mut a = json!({"ask": "tidy the README", "base": SHA, "rounds": 1});
    a.as_object_mut().unwrap().extend(args.as_object().unwrap().clone());
    lock(&app.control).call_tool_for_project(pid, 1, "propose_run", &a)
}

fn w(slot: &str, role: &str, account: &str) -> Value {
    json!({"slot": slot, "role": role, "task": "Do the thing", "account": account})
}

fn open_one(app: &App) -> (String, String) {
    let o = lock(&app.control).open_proposals(None).unwrap();
    let p = o.last().unwrap();
    (p.item.clone(), p.proposal_sha256.clone())
}

fn code(v: &Value) -> &str {
    v["error"].as_str().unwrap_or("")
}

fn states(t: &Tmp) -> Vec<String> {
    t.ledger().iter().filter(|r| r["kind"] == "approve").map(|r| r["state"].as_str().unwrap().to_string()).collect()
}

fn starter<'a>(seen: &'a RefCell<Vec<RunInput>>) -> impl FnOnce(RunInput, &str, Option<(u64, String)>) -> crate::desktop::Res + 'a {
    move |i, _, taken| {
        seen.borrow_mut().push(i);
        Ok(json!({"run_id": taken.map(|t| t.1).unwrap_or_default(), "session_id": "x"}))
    }
}

fn seats_file(t: &Tmp, run: &str) -> Value {
    serde_json::from_slice(&fs::read(t.data().join("control/lead").join(run).join("seats.json")).unwrap()).unwrap()
}

#[test]
fn while_the_core_route_is_off_a_proposal_with_seats_cannot_be_made_or_started() {
    let t = Tmp::new();
    let (app, pid) = app_for(&t, false);
    let e = propose(&app, &pid, json!({"workers": [w("w1", "docs", "claude")]})).unwrap_err();
    let crate::control::service::ControlError::Refusals(list) = e else { panic!("{e:?}") };
    assert!(list.iter().all(|r| r["code"] == "core_route_unavailable"), "{list:?}");
    assert!(list.iter().any(|r| r["seat"] == "lead") && list.iter().any(|r| r["seat"] == "w1"));
    // And the DG is not offered the fields at all in this build: a fresh app (the build's own route) has them off.
    let fresh = App::open(&t.data(), &t.0.join("home")).unwrap();
    assert!(!fresh.seat_env.route());
}

#[test]
fn a_seats_start_checks_again_writes_seats_json_then_requested_and_uses_max_in_flight() {
    let t = Tmp::new();
    let (app, pid) = app_for(&t, true);
    propose(&app, &pid, json!({"max_in_flight": 7, "workers": [w("w1", "backend", "claude"), w("w2", "tests", "codex")]})).unwrap();
    let (id, sha) = open_one(&app);
    let seen = RefCell::new(Vec::new());
    let r = app.start_proposal_inner(&id, &sha, StartOptions::default(), starter(&seen)).unwrap();
    assert_eq!(seen.borrow()[0].max_in_flight, 7, "the proposal's max_in_flight, not DEFAULT_RUN_WORKERS");
    let run = r["run"]["run_id"].as_str().unwrap();
    let file = seats_file(&t, run);
    let bytes = fs::read(t.data().join("control/lead").join(run).join("seats.json")).unwrap();
    // The requested line carries the seats and the sha256 of exactly the bytes in seats.json, and it came after the write.
    let req = t.ledger().into_iter().find(|x| x["kind"] == "approve" && x["state"] == "requested").unwrap();
    assert_eq!(req["seats_sha256"].as_str(), Some(crate::control::sha256::hex_digest(&bytes).as_str()));
    assert_eq!(req["seats"], file);
    assert_eq!(states(&t), ["requested", "delivered"]);
    // The receipt has one entry per seat; the added verifier and reviewer are role_default; Codex is allowed (no key_exposed).
    let seats = r["seats"].as_array().unwrap();
    let by = |s: &str| seats.iter().find(|x| x["seat"] == s).unwrap();
    assert_eq!(seats.len(), 5, "{seats:?}");
    assert_eq!((by("w2")["requested"]["harness"].as_str(), by("w2")["protection"].as_str(), by("w1")["protection"].as_str()), (Some("codex"), Some("sandbox_worktree"), Some("guard")));
    assert_eq!((by("w3")["role"].as_str(), by("w3")["source"].as_str()), (Some("verifier"), Some("role_default")));
    assert_eq!(by("lead")["requested"]["network"], false);
    assert!(!serde_json::to_string(&file).unwrap().contains("key_exposed"));
}

#[test]
fn every_start_refusal_is_worker_unavailable_before_the_requested_line_and_the_proposal_stays_open() {
    let t = Tmp::new();
    let (app, pid) = app_for(&t, true);
    propose(&app, &pid, json!({"workers": [w("w1", "backend", "claude")]})).unwrap();
    let (id, sha) = open_one(&app);
    // The route turns off again, a CLI vanishes, and the engine goes: every seat is checked again at start.
    app.seat_env.for_test(vec![t.0.join("nowhere")], Keys::new(Box::new(Empty)), true);
    fs::remove_file(t.0.join("home/.claude/skills/vajra/scripts/vajra_kb.py")).unwrap();
    let e = app.start_proposal_inner(&id, &sha, StartOptions::default(), |_, _, _| panic!("must not start")).unwrap_err();
    assert_eq!(code(&e), "worker_unavailable");
    let list = e["refusals"].as_array().unwrap();
    assert!(list.iter().filter(|r| r["code"] == "harness_missing").count() >= 3 && list.iter().any(|r| r["seat"] == "engine" && r["code"] == "engine_missing"), "{list:?}");
    assert!(states(&t).is_empty(), "no requested line");
    assert!(!t.data().join("control/lead").read_dir().map(|mut d| d.any(|e| e.unwrap().path().join("seats.json").exists())).unwrap_or(false), "no seats.json");
    assert_eq!(lock(&app.control).open_proposals(None).unwrap().len(), 1);
}

#[test]
fn a_seats_json_that_cannot_be_written_refuses_the_start_and_leaves_the_proposal_open() {
    let t = Tmp::new();
    let (app, pid) = app_for(&t, true);
    propose(&app, &pid, json!({"workers": [w("w1", "backend", "claude")]})).unwrap();
    let (id, sha) = open_one(&app);
    let lead = t.data().join("control/lead");
    let _ = fs::remove_dir_all(&lead);
    fs::write(&lead, b"not a dir").unwrap();
    let e = app.start_proposal_inner(&id, &sha, StartOptions::default(), |_, _, _| panic!("must not start")).unwrap_err();
    assert_eq!(code(&e), "io");
    assert!(states(&t).is_empty(), "no requested line");
    assert_eq!(lock(&app.control).open_proposals(None).unwrap().len(), 1, "still open");
}

#[test]
fn overrides_change_only_account_model_effort_and_network_and_are_checked_again() {
    let t = Tmp::new();
    let (app, pid) = app_for(&t, true);
    propose(&app, &pid, json!({"workers": [w("w1", "backend", "claude")]})).unwrap();
    let (id, sha) = open_one(&app);
    let run_with = |overrides: Vec<Value>| app.start_proposal_inner(&id, &sha, StartOptions { overrides, approve_checker: false }, |_, _, _| panic!("must not start"));
    let refused = |o: Value| {
        let e = run_with(vec![o.clone()]).unwrap_err();
        assert_eq!(code(&e), "override_invalid", "{o}: {e}");
    };
    refused(json!({"seat": "w1", "role": "docs"}));
    refused(json!({"seat": "w1", "security": true}));
    refused(json!({"seat": "lead", "network": true}));
    refused(json!({"seat": "w9", "account": "codex"}));
    refused(json!({"account": "codex"}));
    refused(json!({"seat": "w1", "model": "sk proj abc"}));
    refused(json!({"seat": "w1", "network": "yes"}));
    let e = run_with(vec![json!({"seat": "w1", "effort": "low"}), json!({"seat": "w1", "effort": "high"})]).unwrap_err();
    assert_eq!(code(&e), "override_invalid", "twice");
    // An override to an account outside the ceiling or a harness no adapter runs fails the seat check at start.
    let e = run_with(vec![json!({"seat": "w1", "account": "gemini"})]).unwrap_err();
    assert_eq!((code(&e), e["refusals"][0]["seat"].as_str(), e["refusals"][0]["code"].as_str()), ("worker_unavailable", Some("w1"), Some("outside_allowed")));
    let e = run_with(vec![json!({"seat": "w1", "account": "openrouter"})]).unwrap_err();
    assert_eq!(e["refusals"][0]["code"], "outside_allowed");
    assert!(states(&t).is_empty());
    // A good override applies, is recorded as `user`, and its network value reaches seats.json and the receipt.
    let seen = RefCell::new(Vec::new());
    let r = app.start_proposal_inner(&id, &sha, StartOptions { overrides: vec![json!({"seat": "w1", "account": "codex", "model": null, "effort": "low", "network": true}), json!({"seat": "lead", "effort": "high"})], approve_checker: false }, starter(&seen)).unwrap();
    let file = seats_file(&t, r["run"]["run_id"].as_str().unwrap());
    let seat = |s: &str| file["seats"].as_array().unwrap().iter().find(|x| x["seat"] == s).unwrap().clone();
    assert_eq!((seat("w1")["account"].as_str(), seat("w1")["effort"].as_str(), seat("w1")["network"].as_bool(), seat("w1")["source"].as_str()), (Some("codex"), Some("low"), Some(true), Some("user")));
    assert_eq!((seat("lead")["effort"].as_str(), seat("lead")["source"].as_str(), seat("w2")["source"].as_str()), (Some("high"), Some("user"), Some("role_default")));
    let rs = r["seats"].as_array().unwrap().iter().find(|x| x["seat"] == "w1").unwrap();
    assert_eq!((rs["requested"]["network"].as_bool(), rs["source"].as_str(), rs["requested"]["harness"].as_str()), (Some(true), Some("user"), Some("codex")));
    // The overrides sit outside the hash: the stored proposal line is unchanged.
    assert_eq!(lock(&app.control).open_proposals(None).unwrap().len(), 0);
}

#[test]
fn the_evidence_seat_is_always_the_verifier_and_the_trust_and_slug_follow_its_harness() {
    for (lead, verifier, others, approve, trust, og) in [
        ("claude", "claude", "codex", false, "guarded", false),
        // bhishma #216 H1: a Codex Lead's run is never guarded, whatever the verifier.
        ("codex", "claude", "codex", false, "outside_guard", true),
        ("codex", "claude", "claude", true, "guarded_by_approval", false),
        ("claude", "codex", "claude", false, "outside_guard", true),
        ("codex", "codex", "codex", false, "outside_guard", true),
        ("claude", "codex", "claude", true, "guarded_by_approval", false),
        ("claude", "claude", "claude", true, "guarded", false),
    ] {
        let t = Tmp::new();
        let (app, pid) = app_for(&t, true);
        propose(&app, &pid, json!({"lead": {"account": lead}, "workers": [w("w1", "backend", others), w("w2", "verifier", verifier), w("w3", "reviewer", others)]})).unwrap();
        let (id, sha) = open_one(&app);
        let seen = RefCell::new(Vec::new());
        let r = app.start_proposal_inner(&id, &sha, StartOptions { overrides: vec![], approve_checker: approve }, starter(&seen)).unwrap();
        let run = r["run"]["run_id"].as_str().unwrap();
        let slug = if og { format!("deliver-{run}-og") } else { format!("deliver-{run}") };
        for v in [&r, &seats_file(&t, run)] {
            assert_eq!((v["evidence_seat"].as_str(), v["evidence_trust"].as_str(), v["campaign_slug"].as_str()), (Some("w2"), Some(trust), Some(slug.as_str())), "{lead}/{verifier}/{others}/{approve}");
        }
    }
}

#[test]
fn an_override_that_changes_the_verifiers_harness_recomputes_the_trust_before_requested() {
    let t = Tmp::new();
    let (app, pid) = app_for(&t, true);
    propose(&app, &pid, json!({"workers": [w("w1", "backend", "claude"), w("w2", "verifier", "claude")]})).unwrap();
    let (id, sha) = open_one(&app);
    let seen = RefCell::new(Vec::new());
    let r = app.start_proposal_inner(&id, &sha, StartOptions { overrides: vec![json!({"seat": "w2", "account": "codex"})], approve_checker: false }, starter(&seen)).unwrap();
    let run = r["run"]["run_id"].as_str().unwrap();
    assert_eq!((r["evidence_trust"].as_str(), r["evidence_seat"].as_str()), (Some("outside_guard"), Some("w2")));
    assert!(r["campaign_slug"].as_str().unwrap().ends_with("-og"));
    let req = t.ledger().into_iter().find(|x| x["state"] == "requested").unwrap();
    assert_eq!(req["seats"]["evidence_trust"], "outside_guard", "recorded on the requested line itself");
    assert!(seats_file(&t, run)["campaign_slug"].as_str().unwrap().ends_with("-og"));
}

#[test]
fn editing_preferences_between_propose_and_start_keeps_the_hash_and_the_start_works() {
    let t = Tmp::new();
    let (app, pid) = app_for(&t, true);
    propose(&app, &pid, json!({"workers": [w("w1", "backend", "claude")]})).unwrap();
    let (id, sha) = open_one(&app);
    let before = lock(&app.control).open_proposals(None).unwrap()[0].clone();
    let patch: Patch = serde_json::from_value(json!({"agents": {"default": {"account": "codex"}, "lead": {"account": "codex"}, "roles": {"verifier": {"account": "codex"}, "reviewer": {"account": "codex"}}, "allowed": [{"account": "claude"}, {"account": "codex"}]}})).unwrap();
    let rev = Prefs::new(t.data()).read(None).unwrap().rev;
    Prefs::new(t.data()).update(None, rev, patch, false).unwrap();
    let after = lock(&app.control).open_proposals(None).unwrap()[0].clone();
    assert_eq!((after.proposal_sha256.as_str(), after.seats.clone()), (before.proposal_sha256.as_str(), before.seats.clone()));
    let seen = RefCell::new(Vec::new());
    let r = app.start_proposal_inner(&id, &sha, StartOptions::default(), starter(&seen)).unwrap();
    assert_eq!(r["state"], "started");
    assert_eq!(r["seats"].as_array().unwrap().iter().find(|s| s["seat"] == "lead").unwrap()["requested"]["harness"], "claude", "the lead stays what was proposed");
}

#[test]
fn a_tampered_seats_line_is_proposal_changed() {
    let t = Tmp::new();
    let (app, pid) = app_for(&t, true);
    propose(&app, &pid, json!({"workers": [w("w1", "backend", "claude")]})).unwrap();
    let (id, sha) = open_one(&app);
    let ledger = t.data().join("control/ledger.jsonl");
    let text = fs::read_to_string(&ledger).unwrap().replace("\"network\":false", "\"network\":true");
    fs::write(&ledger, text).unwrap();
    let e = app.start_proposal_inner(&id, &sha, StartOptions::default(), |_, _, _| panic!("must not start")).unwrap_err();
    assert_eq!(code(&e), "proposal_changed");
    assert!(states(&t).is_empty());
}

#[test]
fn a_v1_proposal_still_starts_as_one_claude_run_and_is_refused_for_a_non_claude_project() {
    let t = Tmp::new();
    let (app, pid) = app_for(&t, true);
    propose(&app, &pid, json!({})).unwrap();
    let (id, sha) = open_one(&app);
    let seen = RefCell::new(Vec::new());
    let r = app.start_proposal_inner(&id, &sha, StartOptions::default(), starter(&seen)).unwrap();
    assert_eq!((r["state"].as_str(), r.get("seats").is_none(), seen.borrow()[0].max_in_flight), (Some("started"), true, crate::desktop::caps::DEFAULT_RUN_WORKERS));
    let t2 = Tmp::new();
    let (app2, pid2) = app_for(&t2, true);
    propose(&app2, &pid2, json!({})).unwrap();
    let (id, sha) = open_one(&app2);
    let rev = Prefs::new(t2.data()).read(None).unwrap().rev;
    let patch: Patch = serde_json::from_value(json!({"agents": {"default": {"account": "codex"}, "roles": {}, "allowed": [{"account": "codex"}]}})).unwrap();
    Prefs::new(t2.data()).update(None, rev, patch, false).unwrap();
    let e = app2.start_proposal_inner(&id, &sha, StartOptions::default(), |_, _, _| panic!("must not start")).unwrap_err();
    assert_eq!(code(&e), "harness_missing");
    assert!(states(&t2).is_empty());
    assert_eq!(lock(&app2.control).open_proposals(None).unwrap().len(), 1);
}

#[test]
fn the_inbox_card_shows_the_seats_the_warnings_and_the_label_the_run_will_get() {
    let t = Tmp::new();
    let (app, pid) = app_for(&t, true);
    propose(&app, &pid, json!({"ask": "rotate the session tokens", "workers": [w("w1", "backend", "codex"), w("w2", "verifier", "codex")]})).unwrap();
    let items = app.inbox().unwrap();
    let it = items["items"].as_array().unwrap().iter().find(|i| i["kind"] == "run_proposal").unwrap();
    let c = &it["seats_card"];
    assert_eq!(c["security_warning"], crate::control::service::security_warning("rotate the session tokens", &lock(&app.control).open_proposals(None).unwrap()[0].seats.clone().unwrap()).unwrap());
    assert_eq!((c["evidence"]["seat"].as_str(), c["evidence"]["trust"].as_str(), c["evidence"]["card"].as_str()), (Some("w2"), Some("outside_guard"), Some(OUTSIDE_GUARD_CARD)));
    assert_eq!(c["evidence"]["label"], "Verified (evidence ran outside the guard)");
    assert_eq!(c["evidence"]["approval_offer"], APPROVAL_OFFER);
    let seats = c["seats"].as_array().unwrap();
    let by = |s: &str| seats.iter().find(|x| x["seat"] == s).unwrap();
    // Owner 2026-11-13: a Codex seat runs with the user's own settings and the card never claims the key is out of reach.
    assert_eq!(by("w1")["disclosures"][0], CODEX_WARNING);
    assert!(c["evidence"]["cause"].is_null(), "a Claude lead: the verifier's own note says why");
    assert_eq!(by("lead")["disclosures"][0], CLAUDE_NOTE);
    assert_eq!((by("w1")["limits"]["message"].as_bool(), by("w1")["limits"]["stop"].as_str()), (Some(false), Some("terminate")));
    assert!(seats.iter().any(|s| s["role"] == "security_reviewer"), "added for the security ask");
    assert!(it["proposal_sha256"].as_str().is_some());
}

/// Owner 2026-11-13 ("keep my extras, no key claim") and bhishma #216 H1: a Codex verifier or a Codex lead is never
/// guarded, and no Codex note says it can't read the key.
#[test]
fn a_codex_verifier_or_lead_is_never_guarded_and_no_codex_note_claims_the_key() {
    let p = |lead: &str| {
        let seats: Seats = serde_json::from_value(json!({
            "lead": {"account": lead, "model": null, "effort": null, "source": "role_default"},
            "workers": [{"slot": "w1", "role": "verifier", "task": "t1", "account": "codex", "model": null, "effort": null, "network": false, "security": false, "why": "x", "source": "role_default"}],
            "max_in_flight": 1
        })).unwrap();
        seats
    };
    assert_eq!(evidence_of(&p("claude"), false, "r")["evidence_trust"], "outside_guard", "a Codex verifier");
    assert_eq!(evidence_of(&p("claude"), true, "r")["evidence_trust"], "guarded_by_approval");
    assert_eq!(evidence_of(&p("codex"), false, "r")["evidence_trust"], "outside_guard", "a Codex lead's run is never guarded");
    assert_eq!(outside_cause(&p("codex")), Some(CODEX_LEAD_CAUSE));
    assert_eq!(outside_cause(&p("claude")), None);
    for note in [CODEX_WARNING.to_string(), codex_lead_note(), OUTSIDE_GUARD_WHY.to_string()] {
        assert!(!note.contains("can't read") && !note.contains("out of reach") && !note.contains("kept from"), "no key claim: {note}");
    }
    assert!(CODEX_WARNING.contains("isn't guarded against reading Vajra's signing key"), "{CODEX_WARNING}");
    assert!(CODEX_WARNING.contains("internet follows this seat's switch"), "{CODEX_WARNING}");
}

#[test]
fn a_network_seat_says_so_on_the_card() {
    let t = Tmp::new();
    let (app, pid) = app_for(&t, true);
    let mut net = w("w1", "backend", "claude");
    net["network"] = json!(true);
    propose(&app, &pid, json!({"workers": [net]})).unwrap();
    let items = app.inbox().unwrap();
    let it = items["items"].as_array().unwrap().iter().find(|i| i["kind"] == "run_proposal").unwrap();
    let w1 = it["seats_card"]["seats"].as_array().unwrap().iter().find(|s| s["seat"] == "w1").unwrap().clone();
    assert!(w1["disclosures"].as_array().unwrap().iter().any(|d| d == NETWORK_NOTE), "{w1}");
}
