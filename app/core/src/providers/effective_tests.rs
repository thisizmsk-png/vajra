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
    assert_eq!(r.effective.tag().map(|t| (t.account, t.family, t.model, t.billing)), Some(("openrouter".into(), "codex".into(), "vendor/model-a".into(), "gateway".into())));
}

#[test]
fn with_no_session_file_everything_is_unknown_and_nothing_is_copied_from_the_request() {
    let empty = tmp();
    for (events, home) in [(EVENTS, empty.as_path()), ("", empty.as_path()), ("not json\n{\"type\":\"thread.started\"}\n", empty.as_path())] {
        let eff = read_codex(events, home);
        assert_eq!((eff.model.as_str(), eff.account.as_str(), eff.billing.as_str(), eff.sandbox.as_str()), ("unknown", "unknown", "unknown", "unknown"));
        assert_eq!((eff.effort.as_deref(), eff.read_from.as_deref()), (Some("unknown"), None));
        assert!(eff.tag().is_none(), "Not reported: no chip data");
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
fn a_claude_worker_is_read_from_its_init_event() {
    let eff = read_claude(CLAUDE);
    assert_eq!((eff.model.as_str(), eff.effort.as_deref(), eff.read_from.as_deref(), eff.account.as_str()), ("claude-model-x", None, Some("harness_event"), "claude"));
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
