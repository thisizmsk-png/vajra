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
    // Only the harness's own (first) thread.started counts (#185 bhishma L1): an invalid first one is no thread, and a
    // later one printed by a process the session ran can't point the lookup at another session's file.
    let first_bad = format!("{{\"type\":\"thread.started\",\"thread_id\":\"../x\"}}\n{{\"type\":\"thread.started\",\"thread_id\":\"{THREAD}\"}}");
    assert_eq!(codex_thread_id(&first_bad), None);
    let later = format!("{EVENTS}\n{{\"type\":\"thread.started\",\"thread_id\":\"00000000-0000-7000-8000-0000000ffff1\"}}");
    assert_eq!(codex_thread_id(&later).as_deref(), Some(THREAD));
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
    // One line appended to a.txt and one line in the new b.txt.
    assert_eq!(d, Diff { files: vec!["a.txt".into(), "b.txt".into()], files_total: 2, added: 2, removed: 0, partial: false, uncounted: 0 });
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
    // keep.txt gains a line, the four new files hold one line each, and the rename itself adds and removes none.
    assert_eq!((d.added, d.removed), (5, 0));
    for f in &d.files {
        assert!(repo.join(f).is_file(), "{f:?} is a real path");
    }
}

#[test]
fn a_run_that_only_created_files_counts_their_lines_as_added() {
    let repo = tmp();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("keep.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    // Only new files: no tracked edit anywhere, so every line count comes from the untracked ones.
    std::fs::write(repo.join("new one.txt"), "a\nb\nc\n").unwrap();
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    std::fs::write(repo.join("sub/two.txt"), "x\ny\n").unwrap();
    std::fs::write(repo.join("empty.txt"), "").unwrap();
    std::fs::write(repo.join("blob.bin"), b"\0\x01\x02\0").unwrap();
    let bridge = Bridge::new(PathBuf::from("/nonexistent"));
    let d = diff_from_git(&bridge, &repo, "HEAD").expect("git answered");
    let mut files = d.files.clone();
    files.sort();
    assert_eq!(files, ["blob.bin", "empty.txt", "new one.txt", "sub/two.txt"]);
    assert_eq!((d.added, d.removed), (5, 0), "3 + 2 lines; the empty and binary files add none");
}

#[test]
fn counting_new_files_stops_at_a_time_budget_keeps_listing_and_says_the_totals_are_partial() {
    let repo = tmp();
    init_repo(&repo);
    for i in 0..20 {
        std::fs::write(repo.join(format!("n{i:02}.txt")), "a\nb\n").unwrap();
    }
    let bridge = Bridge::new(PathBuf::from("/nonexistent"));
    // No time at all: every new file is listed and none is counted.
    let d = diff_from_git_within(&bridge, &repo, "HEAD", Duration::ZERO).expect("git answered");
    assert_eq!((d.files_total, d.files.len(), d.added, d.partial, d.uncounted), (20, 20, 0, true, 20));
    // A roomy budget counts every line, and nothing is partial.
    let full = diff_from_git_within(&bridge, &repo, "HEAD", Duration::from_secs(30)).unwrap();
    assert_eq!((full.added, full.partial, full.uncounted), (40, false, 0));
}

/// Timing of the new-file count: `cargo test --release --lib untracked_count_timing -- --ignored --nocapture`.
#[test]
#[ignore]
fn untracked_count_timing() {
    for n in [50usize, 500] {
        let repo = tmp();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.invalid"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("keep.txt"), "one\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        for i in 0..n {
            std::fs::write(repo.join(format!("n{i:03}.txt")), "a\nb\nc\n").unwrap();
        }
        let started = std::time::Instant::now();
        let d = diff_from_git(&Bridge::new(PathBuf::from("/nonexistent")), &repo, "HEAD").unwrap();
        println!("TIMING {n} new files: {:?} (added {}, partial {})", started.elapsed(), d.added, d.partial);
    }
}

fn init_repo(repo: &Path) {
    git(repo, &["init", "-q", "-b", "main"]);
    git(repo, &["config", "user.email", "t@example.invalid"]);
    git(repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("keep.txt"), "one\n").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "base"]);
}

fn bridge() -> Bridge {
    Bridge::new(PathBuf::from("/nonexistent"))
}

/// What git itself counts for one new file: `git diff --numstat --no-index /dev/null <file>` (exit 1 when it differs).
fn git_count(repo: &Path, rel: &std::ffi::OsStr) -> u64 {
    let out = Command::new("/usr/bin/git").current_dir(repo).args(["diff", "--numstat", "-z", "--no-index", "--", "/dev/null"]).arg(rel).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let first = text.split('\0').next().unwrap_or("");
    first.split('\t').next().and_then(|n| n.parse().ok()).unwrap_or(0)
}

fn root_of(repo: &Path) -> File {
    open_dir_nofollow(repo).unwrap()
}

/// A deadline far enough away that it never fires in a test.
fn far() -> Instant {
    Instant::now() + Duration::from_secs(3600)
}

fn count_at(repo: &Path, rel: &[u8]) -> Option<u64> {
    count_new_file_at(&root_of(repo), rel, &mut TOTAL_READ_CAP.clone(), far())
}

#[test]
fn the_git_version_the_counter_was_checked_against() {
    let v = Command::new("/usr/bin/git").arg("--version").output().unwrap();
    println!("S6 git --version: {}", String::from_utf8_lossy(&v.stdout).trim());
    assert!(v.status.success());
}

#[test]
fn every_fixture_is_counted_the_way_git_counts_it() {
    use std::os::unix::ffi::OsStrExt;
    let repo = tmp();
    init_repo(&repo);
    std::fs::create_dir_all(repo.join("a/b/c")).unwrap();
    let mut crlf = Vec::new();
    crlf.extend_from_slice(b"one\r\ntwo\r\nthree\r\n");
    let mut late_nul = vec![b'x'; 9000];
    late_nul.push(0);
    let fixtures: Vec<(&[u8], Vec<u8>)> = vec![
        (b"text.txt", b"a\nb\nc\n".to_vec()),
        (b"no-final-newline.txt", b"a\nb\nc".to_vec()),
        (b"crlf.txt", crlf),
        (b"empty.txt", Vec::new()),
        (b"one-byte.txt", b"x".to_vec()),
        (b"blank-lines.txt", b"\n\n\n".to_vec()),
        (b"binary.bin", b"\0\x01text\nmore\n".to_vec()),
        (b"nul-after-the-probe.txt", late_nul),
        (b"a/b/c/deep.txt", b"deep\nfile\n".to_vec()),
        (b"line\nbreak.txt", b"x\ny\n".to_vec()),
        (b"bad-\xff\xfe-name.txt", b"p\nq\nr\n".to_vec()),
    ];
    let mut skipped = 0;
    let mut fixtures = fixtures;
    fixtures.retain(|(name, body)| match std::fs::write(repo.join(std::ffi::OsStr::from_bytes(name)), body) {
        Ok(()) => true,
        // APFS refuses a name that is not valid UTF-8 (EILSEQ); the fixture runs on filesystems that allow one.
        Err(e) if e.raw_os_error() == Some(libc::EILSEQ) => {
            skipped += 1;
            println!("skipped a non-UTF-8 name: this filesystem refuses it");
            false
        }
        Err(e) => panic!("{e}"),
    });
    std::os::unix::fs::symlink("text.txt", repo.join("link-to-text")).unwrap();
    std::os::unix::fs::symlink("/nonexistent/target/that/is/long", repo.join("dangling")).unwrap();
    // A target is arbitrary text: with newlines it is more than one line, as git counts the link's blob.
    std::os::unix::fs::symlink("two\nlines", repo.join("link-newline")).unwrap();
    std::os::unix::fs::symlink("ends\nwith\nnewline\n", repo.join("link-trailing-newline")).unwrap();
    let mut names: Vec<&[u8]> = fixtures.iter().map(|(n, _)| *n).collect();
    names.extend([b"link-to-text".as_slice(), b"dangling".as_slice(), b"link-newline".as_slice(), b"link-trailing-newline".as_slice()]);
    for name in names {
        let want = git_count(&repo, std::ffi::OsStr::from_bytes(name));
        assert_eq!(count_at(&repo, name), Some(want), "{}", String::from_utf8_lossy(name));
    }
    assert_eq!(count_at(&repo, b"link-to-text"), Some(1), "a link counts one line and is not followed");
    assert_eq!((count_at(&repo, b"link-newline"), count_at(&repo, b"link-trailing-newline")), (Some(2), Some(3)));
    // The whole diff agrees with the sum of git's own counts (3+3+3+0+1+3+0+1+2+2, plus four links: 1, 1, 2, 3, so 18 + 7), and nothing is partial.
    let d = diff_from_git(&bridge(), &repo, "HEAD").unwrap();
    let non_utf8 = 1 - skipped; // `bad-\xff\xfe-name.txt`: 3 lines when the filesystem takes it
    assert_eq!((d.files_total, d.added, d.removed, d.partial, d.uncounted), (14 + non_utf8 as u64, 25 + 3 * non_utf8 as u64, 0, false, 0));
}

#[test]
fn a_nested_repository_is_listed_and_makes_the_result_partial() {
    let repo = tmp();
    init_repo(&repo);
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    init_repo(&repo.join("sub"));
    std::fs::write(repo.join("plain.txt"), "a\n").unwrap();
    let d = diff_from_git(&bridge(), &repo, "HEAD").unwrap();
    assert!(d.files.iter().any(|f| f == "sub/"), "{:?}", d.files);
    assert_eq!((d.added, d.partial, d.uncounted), (1, true, 1));
}

#[test]
fn five_hundred_new_files_are_counted_quickly_and_exactly() {
    let repo = tmp();
    init_repo(&repo);
    for i in 0..500 {
        std::fs::write(repo.join(format!("n{i:03}.txt")), "a\nb\nc\n").unwrap();
    }
    let started = Instant::now();
    let d = diff_from_git(&bridge(), &repo, "HEAD").unwrap();
    let took = started.elapsed();
    assert_eq!((d.files_total, d.added, d.partial, d.uncounted), (500, 1500, false, 0));
    assert_eq!(d.files.len(), MAX_LISTED, "only the first {MAX_LISTED} are listed");
    assert!(took < Duration::from_millis(300) || cfg!(debug_assertions) && took < Duration::from_secs(2), "{took:?}");
}

#[test]
fn six_thousand_files_are_partial_and_the_rest_are_uncounted() {
    let repo = tmp();
    init_repo(&repo);
    for i in 0..6000 {
        std::fs::write(repo.join(format!("n{i:04}.txt")), "a\n").unwrap();
    }
    let d = diff_from_git(&bridge(), &repo, "HEAD").unwrap();
    assert_eq!(d.files_total, 6000);
    assert!(d.partial && d.uncounted > 0, "{d:?}");
    assert_eq!(d.added + d.uncounted, 6000, "each new file is counted or uncounted, never lost");
}

#[test]
fn a_file_over_the_per_file_cap_or_past_the_total_read_cap_is_uncounted() {
    let repo = tmp();
    init_repo(&repo);
    let f = File::create(repo.join("huge.txt")).unwrap();
    f.set_len(PER_FILE_CAP + 1).unwrap();
    std::fs::write(repo.join("small.txt"), "a\nb\n").unwrap();
    let d = diff_from_git(&bridge(), &repo, "HEAD").unwrap();
    assert_eq!((d.added, d.partial, d.uncounted), (2, true, 1));
    // The total cap: a read budget of 5 bytes covers the first small file only.
    let root = root_of(&repo);
    let mut left = 5u64;
    assert_eq!(count_new_file_at(&root, b"small.txt", &mut left, far()), Some(2));
    assert_eq!(left, 1);
    assert_eq!(count_new_file_at(&root, b"small.txt", &mut left, far()), None);
}

#[test]
fn the_budget_is_enforced_inside_a_file_read() {
    let repo = tmp();
    init_repo(&repo);
    // Several chunks, so the loop's own check has to fire; and a one-chunk file, so the check before accepting does.
    std::fs::write(repo.join("big.txt"), "x\n".repeat(200_000)).unwrap();
    std::fs::write(repo.join("one.txt"), "a\nb\n").unwrap();
    let root = root_of(&repo);
    let passed = Instant::now();
    let mut left = TOTAL_READ_CAP;
    assert_eq!(count_new_file_at(&root, b"big.txt", &mut left, passed), None);
    assert_eq!(count_new_file_at(&root, b"one.txt", &mut left, passed), None);
    assert_eq!(left, TOTAL_READ_CAP, "nothing was read once the deadline had passed, or only what was read is charged");
    assert_eq!(count_new_file_at(&root, b"one.txt", &mut left, far()), Some(2));
}

#[test]
fn a_symlinked_directory_partway_down_is_not_followed_and_a_fifo_is_never_opened() {
    let repo = tmp();
    init_repo(&repo);
    let outside = tmp();
    std::fs::write(outside.join("secret.txt"), "s1\ns2\ns3\n").unwrap();
    std::os::unix::fs::symlink(&outside, repo.join("linkdir")).unwrap();
    assert_eq!(count_at(&repo, b"linkdir/secret.txt"), None, "the walk refuses the link in the path");
    // A directory swapped for a link after git listed its files is not entered either.
    std::fs::create_dir_all(repo.join("real")).unwrap();
    std::fs::write(repo.join("real/in.txt"), "x\n").unwrap();
    assert_eq!(count_at(&repo, b"real/in.txt"), Some(1));
    std::fs::remove_dir_all(repo.join("real")).unwrap();
    std::os::unix::fs::symlink(&outside, repo.join("real")).unwrap();
    assert_eq!(count_at(&repo, b"real/secret.txt"), None);
    // A FIFO with no writer would block an open forever; it is never opened and the diff says so.
    let fifo = repo.join("pipe");
    let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert_eq!(count_at(&repo, b"pipe"), None);
    let started = Instant::now();
    let d = diff_from_git(&bridge(), &repo, "HEAD").unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    // Git does not list a FIFO at all; the two links count one line each and nothing blocks.
    assert_eq!((d.added, d.partial), (2, false), "{d:?}");
}

#[test]
fn refused_paths_are_not_counted() {
    let repo = tmp();
    init_repo(&repo);
    std::fs::write(repo.join("ok.txt"), "a\n").unwrap();
    for bad in [&b""[..], b"ok.txt/", b"./ok.txt", b"../x", b"a//b", b"a/./b", b"a/../ok.txt", b"ok\0.txt", b"/ok.txt"] {
        assert_eq!(count_at(&repo, bad), None, "{:?}", String::from_utf8_lossy(bad));
    }
    assert_eq!(count_at(&repo, &vec![b'a'; libc::PATH_MAX as usize]), None, "a path longer than PATH_MAX");
    assert_eq!(count_at(&repo, b"ok.txt"), Some(1));
}

#[test]
fn a_git_that_cannot_answer_is_none_and_garbage_numstat_is_partial() {
    let repo = tmp();
    init_repo(&repo);
    std::fs::write(repo.join("new.txt"), "a\nb\n").unwrap();
    let with = |tag: &str, on: &str, then: &str| {
        let bin = tmp().join(format!("git-{tag}"));
        std::fs::write(&bin, format!("#!/bin/sh\ncase \"$*\" in *'{on}'*) {then};; esac\nexec /usr/bin/git \"$@\"\n")).unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let mut b = Bridge::new(PathBuf::from("/nonexistent"));
        b.git = bin;
        diff_from_git(&b, &repo, "HEAD")
    };
    assert_eq!(with("fails", "ls-files", "exit 2"), None);
    let d = with("garbage", "--numstat -z", "printf 'x\\ty\\tkeep.txt\\0'; exit 0").unwrap();
    assert!(d.partial && d.files.iter().any(|f| f == "keep.txt") && d.files.iter().any(|f| f == "new.txt"), "{d:?}");
    let ok = diff_from_git(&bridge(), &repo, "HEAD").unwrap();
    assert_eq!((ok.added, ok.partial), (2, false));
}

#[test]
fn the_profile_read_back_reads_the_same_session_file() {
    let home = codex_home(ROLLOUT);
    assert_eq!(codex_rollout(EVENTS, &home).as_deref(), Some(ROLLOUT));
    assert_eq!(codex_rollout("{}\n", &home), None, "no thread id, no file");
}

/// A reader that takes its time, like a stalled network volume.
struct SlowRead<'a>(&'a [u8], Duration);

impl std::io::Read for SlowRead<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        std::thread::sleep(self.1);
        let n = self.0.len().min(out.len());
        out[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        Ok(n)
    }
}

#[test]
fn a_binary_read_that_outlasts_the_budget_is_uncounted() {
    let data = b"\0binary\n";
    let slow = Duration::from_millis(80);
    // The deadline is still ahead when the read starts and gone when it returns, and the chunk holds a NUL.
    let mut left = TOTAL_READ_CAP;
    let deadline = Instant::now() + Duration::from_millis(30);
    assert_eq!(count_lines(&mut SlowRead(data, slow), data.len() as u64, &mut left, deadline), None);
    // The same read with room to spare is binary: it counts 0.
    let mut left = TOTAL_READ_CAP;
    assert_eq!(count_lines(&mut SlowRead(data, Duration::ZERO), data.len() as u64, &mut left, far()), Some(0));
    // And a slow text read that ends past the deadline is uncounted too.
    let mut left = TOTAL_READ_CAP;
    let deadline = Instant::now() + Duration::from_millis(30);
    assert_eq!(count_lines(&mut SlowRead(b"a\nb\n", slow), 4, &mut left, deadline), None);
}

#[test]
fn a_file_that_shrinks_while_it_is_read_is_uncounted() {
    let mut left = TOTAL_READ_CAP;
    // `fstat` said 100 bytes, the file then held 4: a prefix is not the file.
    assert_eq!(count_lines(&mut &b"a\nb\n"[..], 100, &mut left, far()), None);
    // The same bytes with the size they really have count normally.
    assert_eq!(count_lines(&mut &b"a\nb\n"[..], 4, &mut left, far()), Some(2));
    // End of file at once (truncated to empty) is the same.
    assert_eq!(count_lines(&mut &b""[..], 10, &mut left, far()), None);
    // An empty file really is empty: size 0, nothing to read, 0 lines.
    assert_eq!(count_lines(&mut &b""[..], 0, &mut left, far()), Some(0));
}

/// Codex review #241: a Lead read-back pass picks the newest `MAX_LEAD_FILES` by metadata and reads only those, however
/// many recent session files the day folders hold. Files from before `since` are never read.
#[test]
fn a_lead_read_back_pass_reads_at_most_its_file_limit_newest_first() {
    let home = tmp();
    let day = home.join("sessions/2026/11/14");
    std::fs::create_dir_all(&day).unwrap();
    let old = home.join("sessions/2026/11/13");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("rollout-2026-11-13T00-00-00-old.jsonl"), "old\n").unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    let since = std::time::SystemTime::now();
    // Created after `since`, oldest first, with a pause so their creation times are ordered.
    let total = MAX_LEAD_FILES + 9;
    for i in 0..total {
        std::fs::write(day.join(format!("rollout-2026-11-14T00-00-{i:02}-f{i:02}.jsonl")), format!("file {i:02}\n")).unwrap();
        std::thread::sleep(Duration::from_millis(15));
    }
    LEAD_READS.with(|c| c.set(0));
    let got = codex_rollouts_since(&home, since);
    assert_eq!(LEAD_READS.with(|c| c.get()), MAX_LEAD_FILES, "only the picked files are read");
    let want: Vec<String> = (total - MAX_LEAD_FILES..total).rev().map(|i| format!("file {i:02}\n")).collect();
    assert_eq!(got, want, "the newest {MAX_LEAD_FILES}, newest first");
    assert!(!got.iter().any(|t| t.starts_with("old")));
}
