//! 6i part B: the proof's write-ledger check on a campaign the core really recorded (6h's `CoreRecord`), through the
//! engine's own scripts in a temp HOME with its own test key. The owner's `~/.claude` and key are never read.
//! bhishma's rule: Verified needs both the campaign's signature check and the core's ledger; failing either is never
//! Verified.

use super::*;
use crate::desktop::record::{Event, PlanRecord, TaskRecord};
use crate::vajra::bridge::{Feature, Request};

const RUN: &str = "20261113-0500";

fn slug() -> String {
    format!("deliver-{RUN}")
}

/// A campaign the core recorded (plan, a claim and a progress line) and, once `before_close` ran, closed as
/// abandoned through the core's own ledger (an open run reads In progress whatever its ledger, so the check is skipped).
/// `None` where the engine's tools are missing.
fn recorded() -> Option<(Tmp, App)> {
    recorded_then(|_| {})
}

fn recorded_then(before_close: impl FnOnce(&App)) -> Option<(Tmp, App)> {
    use std::os::unix::fs::PermissionsExt;
    if ["/usr/bin/jq", "/usr/bin/openssl", "/usr/bin/python3"].iter().any(|p| !Path::new(p).is_file()) {
        eprintln!("skipped: the engine needs /usr/bin/jq, /usr/bin/openssl and /usr/bin/python3");
        return None;
    }
    let t = Tmp::new();
    let home = t.home();
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts").canonicalize().unwrap();
    let scripts = home.join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&scripts).unwrap();
    for f in fs::read_dir(&real).unwrap().flatten() {
        if f.path().is_file() {
            fs::copy(f.path(), scripts.join(f.file_name())).unwrap();
        }
    }
    let key = home.join(".claude/vajra/.hmac-key");
    fs::write(&key, "00".repeat(32)).unwrap();
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
    fs::create_dir_all(home.join(".claude/agents")).unwrap();
    fs::write(home.join(".claude/agents/bhima.md"), "---\nname: x\n---\n").unwrap();
    let app = t.app();
    app.record_run_seats(RUN, &json!({"campaign_slug": slug(), "evidence_trust": "guarded"})).unwrap();
    let rec = app.run_record(RUN, false).unwrap();
    let task = format!("{RUN}.t1");
    rec.plan(&PlanRecord {
        goal: "Add rate limits".into(),
        features: vec![Feature { id: "ac-1".into(), desc: "limits apply".into() }],
        tasks: vec![TaskRecord {
            id: task.clone(),
            title: "do t1".into(),
            agent: "bhima".into(),
            deps: vec![],
            files: vec!["src/".into()],
            criteria: vec!["ac-1".into()],
            branch: format!("vajra/{RUN}/task-t1-r1"),
        }],
    })
    .unwrap();
    rec.send(Event::Claim { task, agent: "bhima".into(), worktree: None });
    rec.send(Event::Progress(format!("run {RUN} round 1: 0/1 criteria passing")));
    rec.flush();
    assert!(rec.card().notes.is_empty(), "{:?}", rec.card().notes);
    rec.end();
    drop(rec);
    before_close(&app);
    assert_eq!(app.record_close_orphan(RUN, &[]).unwrap()["status"], "abandoned");
    Some((t, app))
}

/// The ledger answer for what the record check verified just now.
fn ledger(app: &App) -> proof::Ledger {
    let mut report = app.read_report_with(&slug(), &|_| {}, &|| {}).unwrap();
    let verified = report.as_object_mut().and_then(|m| m.remove(crate::desktop::VERIFIED_KEY)).unwrap_or(Value::Null);
    app.ledger_of(&slug(), &verified)
}

#[test]
fn the_cores_own_record_matches_its_ledger() {
    let Some((_t, app)) = recorded() else { return };
    assert_eq!(app.read_report(&slug()).unwrap()["integrity"], "no_report");
    assert_eq!(ledger(&app), proof::Ledger::Match);
}

#[test]
fn a_campaign_the_core_never_recorded_has_no_ledger() {
    let Some((t, app)) = recorded() else { return };
    fs::remove_file(t.0.join("data/control/record-ledgers").join(format!("{}.json", slug()))).unwrap();
    assert_eq!(ledger(&app), proof::Ledger::Missing);
}

#[test]
fn a_validly_signed_write_the_core_didnt_make_passes_the_campaign_check_but_never_the_ledger() {
    // An agent runs the same signed script against the core's campaign: the signature is still valid.
    let Some((_t, app)) = recorded_then(|app| {
        let out = app.bridge.run(&Request::CampaignProgress { slug: slug(), text: "all criteria pass".into() }).unwrap();
        assert_eq!(out.exit(), Some(0), "{}", out.stderr);
    }) else {
        return;
    };
    assert_eq!(app.read_report(&slug()).unwrap()["integrity"], "no_report", "the campaign check still passes");
    assert_eq!(ledger(&app), proof::Ledger::Mismatch);
    assert!(!verified(&app), "never Verified");
    // A task report written outside the core is caught the same way.
    let Some((_t, app)) = recorded_then(|app| {
        let done = crate::vajra::bridge::TaskStatus::Done;
        let out = app.bridge.run(&Request::TasksReport { slug: slug(), id: format!("{RUN}.t1"), status: done, summary: Some("done".into()) }).unwrap();
        assert_eq!(out.exit(), Some(0), "{}", out.stderr);
    }) else {
        return;
    };
    assert_eq!(app.read_report(&slug()).unwrap()["integrity"], "no_report");
    assert_eq!(ledger(&app), proof::Ledger::Mismatch);
    assert!(!verified(&app), "never Verified");
}

#[test]
fn an_open_run_skips_the_ledger_read() {
    let Some((_t, app)) = recorded() else { return };
    // An active campaign answers at once, without a task-graph read (the label reads In progress first anyway).
    let verified = json!({"campaign": {"status": "active"}, "features": {}, "progress": ""});
    assert_eq!(app.ledger_of(&slug(), &verified), proof::Ledger::Missing);
}

#[test]
fn a_record_that_fails_the_campaign_check_is_never_verified_whatever_the_ledger() {
    let Some((t, app)) = recorded() else { return };
    let file = t.home().join(".claude/vajra/campaigns").join(slug()).join("campaign.json");
    let mut c: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    c["goal"] = json!("Add rate limits, and ship it");
    fs::write(&file, serde_json::to_vec(&c).unwrap()).unwrap();
    let mut report = app.read_report_with(&slug(), &|_| {}, &|| {}).unwrap();
    assert_eq!(report["integrity"], "integrity_failed");
    let verified = report.as_object_mut().and_then(|m| m.remove(crate::desktop::VERIFIED_KEY));
    assert!(verified.is_none(), "a failed check hands the ledger no bytes");
    assert_eq!(app.ledger_of(&slug(), &Value::Null), proof::Ledger::Mismatch);
    let p = app.gather_proof_with(&slug(), &|| {}).unwrap();
    assert_eq!(p.label, proof::Label::NotVerified, "{}", p.reason);
}

/// The label the app shows now is Verified or Verified-approved. (These runs close as abandoned; on a done run,
/// `Ledger::Mismatch` reads Not verified, which `proof_tests` covers.)
fn verified(app: &App) -> bool {
    let p = app.gather_proof_with(&slug(), &|| {}).unwrap();
    matches!(p.label, proof::Label::Verified | proof::Label::VerifiedApproved)
}

#[test]
fn a_ledger_saved_under_another_campaigns_name_is_never_a_match() {
    let Some((t, app)) = recorded() else { return };
    let file = t.0.join("data/control/record-ledgers").join(format!("{}.json", slug()));
    let mut l: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    l["slug"] = json!("deliver-20261113-0400");
    fs::write(&file, serde_json::to_vec(&l).unwrap()).unwrap();
    assert_eq!(ledger(&app), proof::Ledger::Mismatch);
}
