//! 6i proof, end to end on the real campaign script under a temp HOME (its own fake key), with the fake evidence check
//! and a fake `gh`. The owner's real `~/.claude` and key are never read.

use super::*;

const RUN: &str = "20261112-1900";

/// A signed campaign `slug`, closed `status`, one criterion per task, with a signed report. `None` when the machine
/// lacks jq, openssl or python3 (the real-script tests skip there, like `read_report`'s).
fn signed_run(slug: &str, goal: &str, status: &str, tasks: Vec<Value>) -> Option<(Tmp, App)> {
    let (t, run) = real_script_home()?;
    assert!(run(&["create", slug, "--goal", goal, "--feature", "ac-1|limits apply"]).status.success());
    let text = "Delivered on branch b";
    let b64 = String::from_utf8(std::process::Command::new("sh").args(["-c", &format!("printf '%s' '{text}' | openssl base64 -A")]).output().unwrap().stdout).unwrap();
    assert!(run(&["report", "--campaign", slug, "--bytes", &text.len().to_string(), "--check", &fnv32(&b64), "--base64", &b64]).status.success());
    if status != "active" {
        let out = run(&["close", slug, "--status", status]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
    fake_tasks_script(&t);
    let camp = t.home().join(".claude/vajra/campaigns").join(slug);
    fs::create_dir_all(camp.join("evidence")).unwrap();
    let tasks: Vec<Value> = tasks
        .into_iter()
        .map(|mut x| {
            if let Some(name) = x["evidence"]["path"].as_str().and_then(|p| Path::new(p).file_name().map(|n| n.to_owned())) {
                let p = camp.canonicalize().unwrap().join("evidence").join(name);
                fs::write(&p, "{}").unwrap();
                x["evidence"]["path"] = json!(p.to_string_lossy());
            }
            x
        })
        .collect();
    fs::write(camp.join("tasks.json"), serde_json::to_vec(&json!({"campaign": slug, "schema": "vajra.tasks/v1", "tasks": tasks})).unwrap()).unwrap();
    let mut app = t.app();
    // As 6h's ledger will answer for a record the core wrote; `with_no_ledger` covers today's app.
    app.proof_ledger = Some(proof::Ledger::Match);
    Some((t, app))
}

fn green(id: &str, criterion: &str) -> Value {
    let mut v = ev_task(Path::new("/x"), id, "done", &[criterion], "green");
    v["evidence"]["head"] = json!("0123456789abcdef0123456789abcdef01234567");
    v
}

/// The guarded `seats.json` a Start would have written for `RUN`, with the given trust and campaign slug.
fn seats(t: &Tmp, trust: &str, slug: &str, consumed: bool) {
    let dir = t.0.join("data/control/lead").join(RUN);
    fs::create_dir_all(&dir).unwrap();
    let doc = json!({
        "schema": "vajra.run_seats/1", "run_id": RUN, "evidence_seat": "verifier", "evidence_trust": trust, "campaign_slug": slug,
        // A field here is never trusted: the proof reads the core's N20 approval record.
        "evidence_approval": {"consumed": true},
        "seats": [{"seat": "verifier", "role": "verifier", "account": "openai", "harness": "codex"}],
    });
    fs::write(dir.join("seats.json"), serde_json::to_vec(&doc).unwrap()).unwrap();
    // `consumed` is the old seats field only; the proof never reads it (`approve_and_consume` runs the real flow).
    let _ = consumed;
}

/// The signed progress line `text`, written through the real script (as the evidence ticket, or an agent, could).
fn signed_line(t: &Tmp, slug: &str, text: &str) {
    let script = t.home().join(".claude/skills/vajra/scripts/vajra-campaign.sh");
    let out = std::process::Command::new("bash").arg(script).args(["progress", text, "--campaign", slug]).env("HOME", t.home()).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

/// The N20 flow in `app` (the proof's own app: the consume is held in its memory): asked, approved by a person,
/// consumed by the evidence ticket, which then writes the signed progress line (`signed`) naming the list.
fn approve_and_consume(t: &Tmp, app: &App, slug: &str, signed: bool) {
    use crate::desktop::evidence_approval::{progress_line, Ask};
    let ask = Ask { run_id: RUN.into(), round: 1, commit: "0".repeat(40), seat: "verifier".into(), account: "openai".into(), commands: vec!["npm test".into()] };
    let now = std::time::SystemTime::now();
    let sha = app.request_evidence_approval(&ask, now).unwrap();
    app.answer_evidence_approval_at(RUN, &sha, true, now).unwrap();
    app.consume_evidence_approval(&ask, now).unwrap();
    if signed {
        signed_line(t, slug, &progress_line(&sha));
    }
}

#[test]
fn a_guarded_run_with_every_criterion_green_is_verified_with_its_record_hash() {
    let slug = format!("deliver-{RUN}");
    let Some((t, app)) = signed_run(&slug, "Add rate limits", "done", vec![green("t1", "limits apply")]) else { return };
    seats(&t, "guarded", &slug, false);
    let p = app.run_proof(&slug).unwrap();
    assert_eq!(p["label"], "verified", "{p}");
    assert_eq!(p["label_text"], "Verified");
    assert_eq!(p["integrity"], "ok");
    assert_eq!(p["record_sha256"].as_str().map(str::len), Some(64), "{p}");
    let md = p["markdown"].as_str().unwrap();
    assert!(md.contains(proof::record_note(proof::Label::Verified)) && md.contains(p["record_sha256"].as_str().unwrap()), "{md}");
    assert!(md.contains("codex (openai)"), "the harness that ran the check is named: {md}");
}

#[test]
fn with_no_ledger_no_run_reads_verified_and_a_copied_record_is_caught() {
    let slug = format!("deliver-{RUN}");
    let Some((t, mut app)) = signed_run(&slug, "g", "done", vec![green("t1", "c")]) else { return };
    seats(&t, "guarded", &slug, false);
    app.proof_ledger = None;
    assert_eq!(app.run_proof(&slug).unwrap()["label"], "not_verified", "today's app has no write ledger yet");
    app.proof_ledger = Some(proof::Ledger::Mismatch);
    assert_eq!(app.run_proof(&slug).unwrap()["label"], "not_verified");
    // A signed campaign copied under another folder name: its signature holds, its slug field doesn't match.
    app.proof_ledger = Some(proof::Ledger::Match);
    let camps = t.home().join(".claude/vajra/campaigns");
    let other = format!("deliver-{RUN}-copy");
    let out = std::process::Command::new("cp").arg("-R").arg(camps.join(&slug)).arg(camps.join(&other)).output().unwrap();
    assert!(out.status.success());
    let p = app.run_proof(&other).unwrap();
    assert_eq!(p["label"], "not_verified", "{p}");
}

/// `campaign.json` with the goal marker dropped (the swap a worker would want: outside the guard reads as plain).
fn unmarked(path: &Path) -> Vec<u8> {
    let mut c: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let goal = c["goal"].as_str().unwrap().replace(proof::MARKER, "").trim().to_string();
    c["goal"] = json!(goal);
    serde_json::to_vec(&c).unwrap()
}

#[test]
fn the_label_follows_the_verified_campaign_bytes_never_a_later_read() {
    let slug = format!("deliver-{RUN}");
    let goal = format!("{} Add rate limits", proof::MARKER);
    let Some((t, app)) = signed_run(&slug, &goal, "done", vec![green("t1", "limits apply")]) else { return };
    seats(&t, "guarded", &slug, false);
    let file = t.home().join(".claude/vajra/campaigns").join(&slug).join("campaign.json");
    let genuine = fs::read(&file).unwrap();
    // Swapped after the held read: the check and the label both stay on the signed bytes.
    let (f, swap) = (file.clone(), unmarked(&file));
    let p = app.gather_proof_with(&slug, &move || fs::write(&f, &swap).unwrap()).unwrap();
    assert_eq!((p.label, p.integrity.as_str(), p.goal.as_str()), (proof::Label::EvidenceUnguarded, "ok", "Add rate limits"));
    // Tampered before the read, the genuine copy put back while the check runs: the record fails, so does the label.
    fs::write(&file, unmarked(&file)).unwrap();
    let (f, g) = (file.clone(), genuine.clone());
    let p = app.gather_proof_with(&slug, &move || fs::write(&f, &g).unwrap()).unwrap();
    assert_eq!((p.label, p.integrity.as_str()), (proof::Label::NotVerified, "integrity_failed"));
    // The verified bytes never reach the UI's own record read.
    fs::write(&file, &genuine).unwrap();
    assert!(app.read_report(&slug).unwrap().get(crate::desktop::VERIFIED_KEY).is_none());
}

#[test]
fn an_outside_guard_run_is_labelled_from_its_signed_slug_even_when_seats_say_guarded() {
    let slug = format!("deliver-{RUN}-og");
    let goal = format!("{} Add rate limits", proof::MARKER);
    let Some((t, app)) = signed_run(&slug, &goal, "done", vec![green("t1", "limits apply")]) else { return };
    seats(&t, "guarded", &slug, false);
    let p = app.run_proof(&slug).unwrap();
    assert_eq!(p["label"], "evidence_unguarded", "{p}");
    assert_eq!(p["label_text"], "Verified (evidence ran outside the guard)");
    assert!(p["why"].as_str().unwrap().contains("outside the Vajra guard"));
    assert_eq!(p["goal"], "Add rate limits", "the marker is the label's job, not repeated in the title");
    let md = p["markdown"].as_str().unwrap();
    assert!(!md.contains("**Verified**") && md.contains("**Verified (evidence ran outside the guard)**"), "{md}");
    let page = app.export_proof_page(&slug).unwrap();
    assert_eq!(page["label"], "evidence_unguarded");
    let html = fs::read_to_string(page["path"].as_str().unwrap()).unwrap();
    assert!(html.contains("Verified (evidence ran outside the guard)") && html.contains("isn&#39;t tamper-evident"));
    assert!(!html.contains("Tamper-evident on your Mac"), "an outside-the-guard record is never called tamper-evident");
    assert!(Path::new(page["path"].as_str().unwrap()).starts_with(t.0.join("data/exports")));
}

#[test]
fn a_core_trust_record_of_outside_guard_wins_over_an_unmarked_record() {
    let slug = format!("deliver-{RUN}");
    let Some((t, app)) = signed_run(&slug, "g", "done", vec![green("t1", "c")]) else { return };
    seats(&t, "outside_guard", &slug, false);
    assert_eq!(app.run_proof(&slug).unwrap()["label"], "evidence_unguarded");
    // A record that names another campaign, or can't be read, fails closed.
    seats(&t, "guarded", "deliver-other", false);
    assert_eq!(app.run_proof(&slug).unwrap()["label"], "not_verified");
    fs::write(t.0.join("data/control/lead").join(RUN).join("seats.json"), "{not json").unwrap();
    assert_eq!(app.run_proof(&slug).unwrap()["label"], "not_verified");
}

#[test]
fn an_unconsumed_approval_is_not_verified_and_a_consumed_one_is() {
    let slug = format!("deliver-{RUN}");
    let Some((t, app)) = signed_run(&slug, "g", "done", vec![green("t1", "c")]) else { return };
    seats(&t, "guarded_by_approval", &slug, false);
    assert_eq!(app.run_proof(&slug).unwrap()["label"], "not_verified");
    approve_and_consume(&t, &app, &slug, true);
    assert_eq!(app.run_proof(&slug).unwrap()["label"], "verified_approved");
    // After a restart nothing is held: the same files and signed line read not verified.
    let mut restarted = t.app();
    restarted.proof_ledger = Some(proof::Ledger::Match);
    assert_eq!(restarted.run_proof(&slug).unwrap()["label"], "not_verified");
}

#[test]
fn a_forged_consumed_record_and_signed_line_with_no_approval_in_this_app_is_not_verified() {
    // Opus on #226: an agent writes approval.json as consumed with a sha it picks, and adds the matching signed line
    // through the script. No ask or answer happened in this app.
    use crate::desktop::evidence_approval::{progress_line, Ask};
    let slug = format!("deliver-{RUN}");
    let Some((t, app)) = signed_run(&slug, "g", "done", vec![green("t1", "c")]) else { return };
    seats(&t, "guarded_by_approval", &slug, false);
    let ask = Ask { run_id: RUN.into(), round: 1, commit: "0".repeat(40), seat: "verifier".into(), account: "openai".into(), commands: vec!["npm test".into()] };
    let sha = ask.sha256();
    let rec = json!({"state": "consumed", "ask": {"run_id": RUN, "round": 1, "commit": "0".repeat(40), "seat": "verifier", "account": "openai", "commands": ["npm test"]}, "sha256": sha, "asked_at": 1, "consumed_at": 2});
    fs::write(t.0.join("data/control/lead").join(RUN).join("approval.json"), rec.to_string()).unwrap();
    signed_line(&t, &slug, &progress_line(&sha));
    assert_ne!(app.run_proof(&slug).unwrap()["label"], "verified_approved");
    assert_eq!(app.run_proof(&slug).unwrap()["label"], "not_verified");
    // A signed line that only ends with the approval text (a prefix the agent chose) doesn't count either, even with a
    // real consume held.
    let Some((t, app)) = signed_run(&slug, "g", "active", vec![green("t1", "c")]) else { return };
    seats(&t, "guarded_by_approval", &slug, false);
    approve_and_consume(&t, &app, &slug, false);
    let sha = ask.sha256();
    signed_line(&t, &slug, &format!("note: {}", progress_line(&sha)));
    let out = std::process::Command::new("bash").arg(t.home().join(".claude/skills/vajra/scripts/vajra-campaign.sh")).args(["close", &slug, "--status", "done"]).env("HOME", t.home()).output().unwrap();
    assert!(out.status.success());
    assert_eq!(app.run_proof(&slug).unwrap()["label"], "not_verified");
}

#[test]
fn a_consumed_approval_without_its_signed_progress_line_is_not_verified() {
    let slug = format!("deliver-{RUN}");
    let Some((t, app)) = signed_run(&slug, "g", "active", vec![green("t1", "c")]) else { return };
    seats(&t, "guarded_by_approval", &slug, false);
    approve_and_consume(&t, &app, &slug, false);
    let out = std::process::Command::new("bash").arg(t.home().join(".claude/skills/vajra/scripts/vajra-campaign.sh")).args(["close", &slug, "--status", "done"]).env("HOME", t.home()).output().unwrap();
    assert!(out.status.success());
    // The record says consumed, but no signed line names the list: the file alone never reads approved.
    assert_eq!(app.run_proof(&slug).unwrap()["label"], "not_verified");
}

#[test]
fn an_unverified_or_abandoned_or_tampered_run_exports_as_such() {
    let slug = format!("deliver-{RUN}");
    let red = ev_task(Path::new("/x"), "t2", "done", &["429 body"], "red");
    let Some((t, app)) = signed_run(&slug, "g", "done", vec![green("t1", "c"), red]) else { return };
    let p = app.run_proof(&slug).unwrap();
    assert_eq!(p["label"], "not_verified", "{p}");
    let md = p["markdown"].as_str().unwrap();
    assert!(md.contains("**Not verified**") && md.contains("unverified evidence") && !md.contains("**Verified"), "{md}");
    // An edit after signing: the record fails its check, and nothing reads as a pass.
    let camp = t.home().join(".claude/vajra/campaigns").join(&slug).join("campaign.json");
    let mut c: Value = serde_json::from_slice(&fs::read(&camp).unwrap()).unwrap();
    c["goal"] = json!("changed after signing");
    fs::write(&camp, serde_json::to_vec(&c).unwrap()).unwrap();
    let p = app.run_proof(&slug).unwrap();
    assert_eq!((p["label"].as_str(), p["integrity"].as_str()), (Some("not_verified"), Some("integrity_failed")), "{p}");
    assert!(p["markdown"].as_str().unwrap().contains("failed its check on this Mac"));

    let slug2 = format!("deliver-{RUN}");
    let Some((_t2, app2)) = signed_run(&slug2, "g", "abandoned", vec![green("t1", "c")]) else { return };
    let p = app2.run_proof(&slug2).unwrap();
    assert_eq!(p["label"], "abandoned", "{p}");
    assert!(!p["markdown"].as_str().unwrap().contains("| pass |"), "an abandoned run never shows a pass");
}

#[test]
fn a_nonce_or_key_shaped_value_in_the_evidence_never_reaches_the_export() {
    let slug = format!("deliver-{RUN}");
    let nonce = "f00dfacecafe1234beef5678";
    let key = format!("sk-ant-{}", "Q".repeat(40));
    let mut t1 = green("t1", &format!("token={key} and {nonce} and /secret/x"));
    t1["owner_agent"] = json!(format!("bhima {key}"));
    let Some((t, app)) = signed_run(&slug, &format!("goal with {key}"), "done", vec![t1]) else { return };
    {
        let mut runs = lock(&app.runs);
        let mut nonces = crate::crew::redact::Redactor::default();
        nonces.add(Some(nonce));
        runs.insert("s".into(), Run { session_id: "s".into(), run_id: RUN.into(), campaign: slug.clone(), cwd: t.0.join("repo"), nonces });
    }
    let p = app.run_proof(&slug).unwrap();
    let page = fs::read_to_string(app.export_proof_page(&slug).unwrap()["path"].as_str().unwrap()).unwrap();
    for out in [p.to_string(), page] {
        assert!(!out.contains(nonce), "nonce leaked: {out}");
        assert!(!out.contains("QQQQQQQQQQ"), "key-shaped value leaked: {out}");
    }
}

#[test]
fn a_nonce_in_a_commit_field_never_reaches_the_export_even_on_an_unverified_row() {
    let slug = format!("deliver-{RUN}");
    let nonce = "0badc0de".repeat(4); // 32 hex: shaped exactly like a commit
    let mut t1 = green("t1", "limits apply");
    t1["evidence"]["head"] = json!(nonce);
    // An unverified row (its evidence file was never written), its head the nonce padded to a 40-hex commit.
    let mut t2 = ev_task(Path::new("/x"), "t2", "done", &["429 body"], "never-written");
    t2["evidence"]["head"] = json!(format!("{nonce}01234567"));
    let Some((t, app)) = signed_run(&slug, "g", "done", vec![t1, t2]) else { return };
    {
        let mut runs = lock(&app.runs);
        let mut nonces = crate::crew::redact::Redactor::default();
        nonces.add(Some(&nonce));
        runs.insert("s".into(), Run { session_id: "s".into(), run_id: RUN.into(), campaign: slug.clone(), cwd: t.0.join("repo"), nonces });
    }
    let p = app.run_proof(&slug).unwrap();
    let page = fs::read_to_string(app.export_proof_page(&slug).unwrap()["path"].as_str().unwrap()).unwrap();
    for out in [p.to_string(), page] {
        assert!(!out.contains(&nonce[..12]), "nonce leaked: {out}");
    }
}

#[test]
fn an_exports_symlink_is_refused_and_its_target_is_untouched() {
    use std::os::unix::fs::PermissionsExt;
    let slug = format!("deliver-{RUN}");
    let Some((t, app)) = signed_run(&slug, "g", "done", vec![green("t1", "limits apply")]) else { return };
    let elsewhere = t.0.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o755)).unwrap();
    let _ = fs::remove_dir_all(t.0.join("data/exports"));
    std::os::unix::fs::symlink(&elsewhere, t.0.join("data/exports")).unwrap();
    assert!(app.export_proof_page(&slug).is_err());
    assert_eq!(fs::metadata(&elsewhere).unwrap().permissions().mode() & 0o777, 0o755, "the target was not chmodded");
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0, "nothing was written through the link");
}

/// A fake `gh` that records its argv and stdin, and exits `code`.
fn fake_gh(t: &Tmp, code: i32) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let dir = t.0.join("ghbin");
    fs::create_dir_all(&dir).unwrap();
    let log = t.0.join("gh.log");
    let body = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{l}.argv'\ncat > '{l}.stdin'\n[ {code} = 0 ] && echo 'https://github.com/acme/web/pull/42#issuecomment-1' || echo 'GraphQL: token=ghp_abcdefghijklmnop1234 denied' >&2\nexit {code}\n",
        l = log.display()
    );
    fs::write(dir.join("gh"), body).unwrap();
    fs::set_permissions(dir.join("gh"), fs::Permissions::from_mode(0o755)).unwrap();
    (dir, log)
}

#[test]
fn posting_the_comment_is_a_human_action_with_a_fixed_argv_and_the_app_s_own_text() {
    let slug = format!("deliver-{RUN}-og");
    let Some((t, mut app)) = signed_run(&slug, &format!("{} g", proof::MARKER), "done", vec![green("t1", "c")]) else { return };
    let (bin, log) = fake_gh(&t, 0);
    app.provider_dirs = Some(vec![bin]);
    let pid = test_project(&t.0.join("data"), &t.home(), &t.0.join("repo"));
    let env = |id: &str| Envelope { action_id: id.into(), project_id: pid.clone(), expect_generation: None };
    // A link that isn't a plain GitHub PR is refused before anything runs.
    let bad = app.act_post_proof_comment(&env("post-a1-bad"), &slug, "https://github.com/acme/web/pull/42; rm -rf /");
    assert_eq!(bad["receipt"]["state"], "rejected", "{bad}");
    assert!(!Path::new(&format!("{}.argv", log.display())).exists(), "nothing was spawned");
    let ok = app.act_post_proof_comment(&env("post-a2-good"), &slug, "https://github.com/acme/web/pull/42");
    assert_eq!(ok["receipt"]["state"], "done", "{ok}");
    let argv = fs::read_to_string(format!("{}.argv", log.display())).unwrap();
    assert_eq!(argv.lines().collect::<Vec<_>>(), vec!["pr", "comment", "42", "-R", "acme/web", "--body-file", "-"]);
    let sent = fs::read_to_string(format!("{}.stdin", log.display())).unwrap();
    assert_eq!(sent, app.run_proof(&slug).unwrap()["markdown"].as_str().unwrap(), "the posted text is the shown text");
    assert!(sent.contains("Verified (evidence ran outside the guard)"));
}

#[test]
fn a_failed_post_says_why_with_secrets_redacted_and_a_missing_gh_says_so() {
    let slug = format!("deliver-{RUN}");
    let Some((t, mut app)) = signed_run(&slug, "g", "done", vec![green("t1", "c")]) else { return };
    let pid = test_project(&t.0.join("data"), &t.home(), &t.0.join("repo"));
    let env = |id: &str| Envelope { action_id: id.into(), project_id: pid.clone(), expect_generation: None };
    app.provider_dirs = Some(vec![t.0.join("empty")]);
    let r = app.act_post_proof_comment(&env("post-b1-nogh"), &slug, "https://github.com/acme/web/pull/42");
    assert!(r.to_string().contains("gh_missing"), "{r}");
    let (bin, _) = fake_gh(&t, 1);
    app.provider_dirs = Some(vec![bin]);
    let r = app.act_post_proof_comment(&env("post-b2-fail"), &slug, "https://github.com/acme/web/pull/42");
    assert_eq!(r["receipt"]["state"], "rejected", "{r}");
    assert!(r.to_string().contains("post_failed") && !r.to_string().contains("ghp_abcdefghijklmnop1234"), "{r}");
}

#[test]
fn an_unfinished_run_has_nothing_to_post() {
    let slug = format!("deliver-{RUN}");
    let Some((t, mut app)) = signed_run(&slug, "g", "active", vec![green("t1", "c")]) else { return };
    let (bin, log) = fake_gh(&t, 0);
    app.provider_dirs = Some(vec![bin]);
    assert_eq!(app.run_proof(&slug).unwrap()["label"], "in_progress");
    let pid = test_project(&t.0.join("data"), &t.home(), &t.0.join("repo"));
    let r = app.act_post_proof_comment(&Envelope { action_id: "post-c1-active".into(), project_id: pid, expect_generation: None }, &slug, "https://github.com/acme/web/pull/42");
    assert!(r.to_string().contains("run_not_finished"), "{r}");
    assert!(!Path::new(&format!("{}.argv", log.display())).exists());
}
