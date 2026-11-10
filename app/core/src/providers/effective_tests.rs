use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::control::receipt::LaunchReceipt;

// Real event and session-file shapes, with every id replaced.
const EVENTS: &str = include_str!("../../tests/fixtures/launch/codex-events.jsonl");
const ROLLOUT: &str = include_str!("../../tests/fixtures/launch/codex-rollout.jsonl");
const CLAUDE: &str = include_str!("../../tests/fixtures/launch/claude-events.jsonl");
const THREAD: &str = "00000000-0000-7000-8000-00000000a001";

fn tmp() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("vajra-effective-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A `CODEX_HOME` holding the rollout fixture where Codex puts it.
fn codex_home(rollout: &str) -> PathBuf {
    let home = tmp();
    let dir = home.join("sessions/2026/01/01");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("rollout-2026-01-01T00-00-00-{THREAD}.jsonl")), rollout).unwrap();
    home
}

fn requested(model: &str, effort: Option<&str>) -> Requested {
    Requested { account: "openrouter".into(), harness: "codex".into(), model: model.into(), effort: effort.map(String::from) }
}

fn receipt(requested: Requested, effective: Effective) -> LaunchReceipt {
    LaunchReceipt { slot: "w3".into(), session_id: "s_test".into(), source: "role_default".into(), requested, effective }
}

#[test]
fn a_matched_receipt_reads_the_session_file_and_raises_nothing() {
    let eff = read_codex(EVENTS, &codex_home(ROLLOUT));
    assert_eq!(
        (eff.model.as_str(), eff.effort.as_deref(), eff.sandbox.as_str(), eff.account.as_str(), eff.billing.as_str(), eff.read_from.as_deref()),
        ("vendor/model-a", Some("high"), "workspace-write", "openrouter", "gateway", Some("harness_session_file"))
    );
    let r = receipt(requested("vendor/model-a", Some("high")), eff);
    assert!(r.mismatches().is_empty());
    assert!(r.mismatch_event().is_none());
    let p = r.provider().expect("a read receipt has a provider");
    assert_eq!((p.account.as_str(), p.label.as_str(), p.family.as_str(), p.model.as_str(), p.billing.as_str()), ("openrouter", "OpenRouter", "vendor", "vendor/model-a", "gateway"));
    assert_eq!(p.requested.map(|q| (q.model, q.label)), Some((Some("vendor/model-a".into()), "OpenRouter".into())), "no mismatch: requested equals effective");
    assert_eq!((p.model_name, p.error), (None, None));
}

/// The same bytes `app/ui/tests/launchProviderGolden.test.ts` feeds to `parseWorkerProvider`: the two sides cannot drift apart.
#[test]
fn the_provider_object_is_the_golden_json_the_ui_parser_reads() {
    let eff = read_codex(EVENTS, &codex_home(ROLLOUT));
    let p = receipt(requested("vendor/model-a", Some("high")), eff).provider().unwrap();
    let golden: Value = serde_json::from_str(include_str!("../../tests/fixtures/launch/worker-provider.json")).unwrap();
    assert_eq!(serde_json::to_value(&p).unwrap(), golden);
}

#[test]
fn with_no_session_file_everything_is_unknown_and_nothing_is_copied_from_the_request() {
    let empty = tmp();
    for (events, home) in [(EVENTS, empty.as_path()), ("", empty.as_path()), ("not json\n{\"type\":\"thread.started\"}\n", empty.as_path())] {
        let eff = read_codex(events, home);
        assert_eq!((eff.model.as_str(), eff.account.as_str(), eff.billing.as_str(), eff.sandbox.as_str()), ("unknown", "unknown", "unknown", "unknown"));
        assert_eq!((eff.effort.as_deref(), eff.read_from.as_deref()), (Some("unknown"), None));
        assert!(eff.tag(None).is_none(), "nothing read: no chip data");
        let r = receipt(requested("vendor/model-a", Some("high")), eff);
        assert_ne!(r.effective.model, r.requested.model);
        assert!(r.mismatches().is_empty() && r.mismatch_event().is_none(), "an unreported value is not a mismatch");
    }
}

#[test]
fn a_different_model_in_the_session_file_is_a_mismatch_and_emits_launch_mismatch() {
    let eff = read_codex(EVENTS, &codex_home(&ROLLOUT.replace("vendor/model-a", "vendor/model-b")));
    let r = receipt(requested("vendor/model-a", Some("high")), eff);
    assert_eq!(r.mismatches(), ["model"]);
    let ev = r.mismatch_event().expect("event");
    assert_eq!((ev["type"].as_str(), ev["slot"].as_str(), ev["session_id"].as_str()), (Some("launch_mismatch"), Some("w3"), Some("s_test")));
    assert_eq!(ev["fields"], serde_json::json!(["model"]));
    assert_eq!((ev["requested"]["model"].as_str(), ev["effective"]["model"].as_str()), (Some("vendor/model-a"), Some("vendor/model-b")));
}

#[test]
fn the_effort_is_recorded_and_a_different_one_is_a_mismatch() {
    let eff = read_codex(EVENTS, &codex_home(ROLLOUT));
    assert_eq!(eff.effort.as_deref(), Some("high"));
    assert_eq!(receipt(requested("vendor/model-a", Some("low")), eff.clone()).mismatches(), ["effort"]);
    assert!(receipt(requested("vendor/model-a", None), eff).mismatches().is_empty(), "no effort asked: the harness default is fine");
    // The file names no effort: unknown, not the request.
    let no_effort = ROLLOUT.replace(",\"effort\":\"high\"", "");
    let eff = read_codex(EVENTS, &codex_home(&no_effort));
    assert_eq!(eff.effort.as_deref(), Some("unknown"));
    assert!(receipt(requested("vendor/model-a", Some("high")), eff).mismatches().is_empty());
}

#[test]
fn a_provider_id_vajra_did_not_write_is_an_unknown_account() {
    let eff = read_codex(EVENTS, &codex_home(&ROLLOUT.replace("vajra-openrouter", "someone-elses")));
    assert_eq!((eff.account.as_str(), eff.billing.as_str(), eff.model.as_str()), ("unknown", "unknown", "vendor/model-a"));
}

#[test]
fn a_codex_worker_on_the_openai_provider_does_not_get_a_billing_kind_from_its_provider_id() {
    // `openai` is used both signed in with a plan and with an API key, so the session file cannot tell which.
    let eff = read_codex(EVENTS, &codex_home(&ROLLOUT.replace("vajra-openrouter", "openai")));
    assert_eq!((eff.account.as_str(), eff.billing.as_str(), eff.model.as_str()), ("codex", "unknown", "vendor/model-a"));
    // The id Vajra wrote for its own gateway account is the one thing that does name a route.
    assert_eq!(read_codex(EVENTS, &codex_home(ROLLOUT)).billing, "gateway");
}

#[test]
fn a_claude_worker_is_read_from_its_init_event() {
    let eff = read_claude(CLAUDE);
    assert_eq!((eff.model.as_str(), eff.effort.as_deref(), eff.read_from.as_deref(), eff.account.as_str()), ("claude-model-x", None, Some("harness_event"), "claude"));
    // An init event naming a model is not proof of how the CLI was signed in: billing stays unknown, key or no key.
    assert_eq!(eff.billing, "unknown");
    for init in [r#"{"type":"system","subtype":"init","model":"m","apiKeySource":"none"}"#, r#"{"type":"system","subtype":"init","model":"m","apiKeySource":"ANTHROPIC_API_KEY"}"#] {
        assert_eq!(read_claude(init).billing, "unknown", "{init}");
    }
    let none = read_claude("{\"type\":\"result\"}\n");
    assert_eq!((none.model.as_str(), none.read_from.as_deref(), none.effort.as_deref()), ("unknown", None, None));
    let r = receipt(Requested { account: "claude".into(), harness: "claude".into(), model: "claude-model-y".into(), effort: None }, eff);
    assert_eq!(r.mismatches(), ["model"]);
}

#[test]
fn a_hostile_session_file_or_thread_id_is_not_followed_or_trusted() {
    // A thread id is matched against file names, so only a plain id is taken.
    assert_eq!(codex_thread_id("{\"type\":\"thread.started\",\"thread_id\":\"../../etc/passwd\"}"), None);
    assert_eq!(codex_thread_id("{\"type\":\"thread.started\",\"thread_id\":\"abc\"}"), None);
    assert_eq!(codex_thread_id(EVENTS).as_deref(), Some(THREAD));
    // A link at the rollout name is not read.
    let home = tmp();
    let outside = tmp().join("outside.jsonl");
    std::fs::write(&outside, ROLLOUT).unwrap();
    let dir = home.join("sessions/2026/01/01");
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join(format!("rollout-2026-01-01T00-00-00-{THREAD}.jsonl"))).unwrap();
    assert_eq!(read_codex(EVENTS, &home).model, "unknown");
    // A link at `sessions`, at a date folder, or at the home itself is not walked, even with a matching rollout behind it.
    let name = format!("rollout-2026-01-01T00-00-00-{THREAD}.jsonl");
    let outside_tree = tmp();
    std::fs::create_dir_all(outside_tree.join("2026/01/01")).unwrap();
    std::fs::write(outside_tree.join("2026/01/01").join(&name), ROLLOUT).unwrap();
    assert_eq!(read_codex(EVENTS, &codex_home(ROLLOUT)).model, "vendor/model-a", "the plain layout still reads");
    let linked_sessions = tmp();
    std::os::unix::fs::symlink(&outside_tree, linked_sessions.join("sessions")).unwrap();
    assert_eq!(read_codex(EVENTS, &linked_sessions).model, "unknown");
    let linked_year = tmp();
    std::fs::create_dir_all(linked_year.join("sessions")).unwrap();
    std::os::unix::fs::symlink(outside_tree.join("2026"), linked_year.join("sessions/2026")).unwrap();
    assert_eq!(read_codex(EVENTS, &linked_year).model, "unknown");
    let linked_day = tmp();
    std::fs::create_dir_all(linked_day.join("sessions/2026/01")).unwrap();
    std::os::unix::fs::symlink(outside_tree.join("2026/01/01"), linked_day.join("sessions/2026/01/01")).unwrap();
    assert_eq!(read_codex(EVENTS, &linked_day).model, "unknown");
    let real_home = codex_home(ROLLOUT);
    let linked_home = tmp().join("home-link");
    std::os::unix::fs::symlink(&real_home, &linked_home).unwrap();
    assert_eq!(read_codex(EVENTS, &linked_home).model, "unknown");
    // A model id with control characters or a huge one is dropped, not shown.
    for bad in ["vendor/model\\u001b[31m-a", &"m".repeat(300)] {
        let rollout = ROLLOUT.replace("vendor/model-a", bad);
        assert_eq!(read_codex(EVENTS, &codex_home(&rollout)).model, "unknown", "{bad:?}");
    }
}

fn git(cwd: &Path, args: &[&str]) {
    let st = Command::new("/usr/bin/git").args(args).current_dir(cwd).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_SYSTEM", "/dev/null").status().unwrap();
    assert!(st.success(), "git {args:?}");
}

#[test]
fn the_diff_comes_from_git_for_a_run_that_only_edited_through_a_shell() {
    let repo = tmp();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    // The worker ran only shell commands: no edit events exist anywhere. Its change is what git shows.
    let sh = |cmd: &str| assert!(Command::new("/bin/sh").args(["-c", cmd]).current_dir(&repo).status().unwrap().success());
    sh("echo three >> a.txt; echo new > b.txt");

    let bridge = Bridge::new(PathBuf::from("/nonexistent"));
    let d = diff_from_git(&bridge, &repo, "HEAD").expect("git answered");
    assert_eq!(d, Diff { files: vec!["a.txt".into(), "b.txt".into()], added: 1, removed: 0 });
    // A clean tree is an empty diff, not a failure; an unknown base is a failure (None), not an empty diff.
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "w"]);
    assert_eq!(diff_from_git(&bridge, &repo, "HEAD"), Some(Diff::default()));
    assert_eq!(diff_from_git(&bridge, &repo, "no-such-ref"), None);
}

#[test]
fn git_paths_are_real_paths_even_with_odd_characters_or_a_staged_rename() {
    let repo = tmp();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "t"]);
    let body = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";
    for f in ["old name.txt", "keep.txt"] {
        std::fs::write(repo.join(f), body).unwrap();
    }
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    // A staged rename (with a space), one edit, and untracked files whose names git would quote without -z.
    git(&repo, &["mv", "old name.txt", "new \"quoted\" name.txt"]);
    std::fs::write(repo.join("keep.txt"), format!("{body}nine\n")).unwrap();
    let odd = ["caf\u{e9} \u{2603}.txt", "tab\there.txt", "line\nbreak.txt", "it's \\ here.txt"];
    for f in odd {
        std::fs::write(repo.join(f), "x\n").unwrap();
    }
    let bridge = Bridge::new(PathBuf::from("/nonexistent"));
    let d = diff_from_git(&bridge, &repo, "HEAD").expect("git answered");
    let mut want: Vec<String> = vec!["keep.txt".into(), "new \"quoted\" name.txt".into()];
    want.extend(odd.iter().map(|s| s.to_string()));
    let mut got = d.files.clone();
    got.sort();
    want.sort();
    assert_eq!(got, want);
    assert_eq!((d.added, d.removed), (1, 0), "the rename itself adds and removes no lines");
    for f in &d.files {
        assert!(repo.join(f).is_file(), "{f:?} is a real path");
    }
}
