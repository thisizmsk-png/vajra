//! ADR-15 4.2 board cards in `project_state` (contract (a)-(e) in `board`). The fixtures are `tests`'s.

use super::*;
use crate::projects::contract::{SessionKind, SessionStatus};

/// The test project, with a run line naming each of `slugs` (in order, so the last one is the newest).
fn project_with(t: &Tmp, app: &App, slugs: &[String]) -> String {
    let pid = test_project(&t.0.join("data"), &t.home(), &t.0.join("repo"));
    for (i, s) in slugs.iter().enumerate() {
        let mut line = super::orchestrator::session_line(crate::projects::contract::fresh_id("s").unwrap(), &pid, SessionKind::Run, None, None, Some(("20261105-1810", s)), "claude", 1, SessionStatus::Running);
        line.started = format!("2026-11-05T18:{:02}:00Z", 10 + i);
        app.projects.append_session(&line).unwrap();
    }
    pid
}

/// Write `tasks.json` for campaign `slug`.
fn campaign(t: &Tmp, slug: &str, tasks: Vec<Value>) {
    let camp = t.home().join(".claude/vajra/campaigns").join(slug);
    fs::create_dir_all(camp.join("evidence")).unwrap();
    fs::write(camp.join("tasks.json"), serde_json::to_vec(&json!({"campaign": slug, "schema": "vajra.tasks/v1", "tasks": tasks})).unwrap()).unwrap();
}

fn board(app: &App, pid: &str) -> Value {
    app.project_state(pid, None).unwrap()["board"].clone()
}

fn card<'a>(b: &'a Value, task: &str) -> &'a Value {
    b["campaigns"].as_array().unwrap().iter().flat_map(|c| c["cards"].as_array().unwrap()).find(|c| c["task_id"] == task).unwrap_or_else(|| panic!("no {task} in {b}"))
}

/// (c), one row each: the lane and the badge.
#[test]
fn every_lane_mapping_row() {
    use super::board::lane;
    for s in ["claimed", "in_progress"] {
        assert_eq!(lane(s, None), ("running", false), "{s}");
    }
    assert_eq!(lane("question", None), ("running", true));
    assert_eq!(lane("done", Some((2, 2))), ("done", false), "every criterion verified passing");
    assert_eq!(lane("done", Some((1, 2))), ("review", false), "done with an unverified criterion is review");
    assert_eq!(lane("done", Some((0, 0))), ("review", false), "done with no criteria is never done");
    assert_eq!(lane("done", None), ("review", false));
    assert_eq!(lane("failed", Some((1, 1))), ("review", false));
    for s in ["pending", "ready", "unknown", ""] {
        assert_eq!(lane(s, None), ("backlog", false), "{s}");
    }
    assert_eq!(lane("blocked", None), ("backlog", true));
}

/// A done task reaches Done only once its evidence is verified (here: after `evidence_list` ran the check); before
/// that, with a cold cache, it is in review. Card text is the inbox's clean text; the key names campaign, task, attempt.
#[test]
fn cards_come_from_tasks_and_cached_verdicts_and_carry_clean_text() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let mut evil = ev_task(&d, "t6", "pending", &[], "");
    evil["title"] = json!("\u{1b}[31mRed\u{202e} title\u{0}");
    let bad_id = ev_task(&d, "t\u{202e}7\u{1b}", "pending", &[], "");
    let mut bad_owner = ev_task(&d, "t8", "pending", &[], "");
    bad_owner["owner_agent"] = json!("ev\u{202e}il\u{1b}[2J\u{0}");
    let (app, slug) = campaign_with_tasks(&t, vec![
        ev_task(&d, "t1", "done", &["ac-1", "ac-2"], "green"),
        ev_task(&d, "t2", "done", &["ac-3"], "red"),
        ev_task(&d, "t3", "in_progress", &["ac-4"], ""),
        ev_task(&d, "t4", "blocked", &["ac-5"], ""),
        ev_task(&d, "t5", "failed", &["ac-6"], ""),
        evil,
        bad_id,
        bad_owner,
    ]);
    let ev_dir = t.home().join(".claude/vajra/campaigns").join(&slug).join("evidence");
    fs::write(ev_dir.join("green.json"), b"{}").unwrap();
    fs::write(ev_dir.join("red.json"), b"RED").unwrap();
    let pid = project_with(&t, &app, std::slice::from_ref(&slug));
    let b = board(&app, &pid);
    assert_eq!((b["truncated"].as_bool(), b["campaigns"][0]["campaign"].as_str()), (Some(false), Some(slug.as_str())), "{b}");
    let t1 = card(&b, "t1");
    assert_eq!((t1["lane"].as_str(), t1["criteria"]["pass"].as_u64(), t1["criteria"]["total"].as_u64()), (Some("review"), Some(0), Some(2)), "cold cache: {t1}");
    assert_eq!((t1["key"].as_str(), t1["attempt"].as_u64(), t1["project_id"].as_str()), (Some(format!("{slug}/t1/1").as_str()), Some(1), Some(pid.as_str())));
    assert_eq!((t1["owner"].as_str(), t1["kind"].as_str()), (Some("Backend"), Some("task")));
    assert!(app.evidence_cache.lock().unwrap().is_empty(), "project_state started no check");
    app.evidence_list(&slug).unwrap();
    let b = board(&app, &pid);
    assert_eq!(card(&b, "t1")["lane"], "done", "verified: {b}");
    assert_eq!(card(&b, "t2")["lane"], "review", "red evidence is not done");
    assert_eq!((card(&b, "t3")["lane"].as_str(), card(&b, "t3")["needs_you"].as_bool()), (Some("running"), Some(false)));
    assert_eq!((card(&b, "t4")["lane"].as_str(), card(&b, "t4")["needs_you"].as_bool()), (Some("backlog"), Some(true)));
    assert_eq!(card(&b, "t5")["lane"], "review");
    let title = card(&b, "t6")["title"].as_str().unwrap().to_string();
    assert!(!title.chars().any(|c| c.is_control() || c == '\u{202e}'), "{title:?}");
    assert!(title.contains("Red") && title.contains("title"), "{title:?}");
    // A task id the engine could not have written is no card; an owner is clean text.
    let all: Vec<&Value> = b["campaigns"].as_array().unwrap().iter().flat_map(|c| c["cards"].as_array().unwrap()).collect();
    assert!(all.iter().all(|c| c["task_id"].as_str().is_none_or(|id| !id.contains('\u{202e}')) && !c["key"].as_str().unwrap().contains('\u{1b}')), "{b}");
    assert_eq!(all.iter().filter(|c| c["kind"] == "task").count(), 7, "t1..t6 and t8");
    let owner = card(&b, "t8")["owner"].as_str().unwrap().to_string();
    assert!(!owner.chars().any(|c| c.is_control() || c == '\u{202e}') && owner.starts_with("Evil"), "{owner:?}");
    // A non-store id has no board.
    assert!(app.project_state("k_0123", None).unwrap()["board"].is_null());
}

/// (d): five campaigns and two hundred cards with a cold cache start no check (each would sleep 30 s) and come back
/// within the call budget.
#[test]
fn a_full_board_with_a_cold_cache_starts_no_check_and_is_quick() {
    let t = Tmp::new();
    fake_tasks_script(&t);
    let app = t.app();
    let slugs: Vec<String> = (0..5).map(|i| format!("deliver-2026110{i}-1810")).collect();
    for s in &slugs {
        let ev = t.home().join(".claude/vajra/campaigns").join(s).join("evidence").join("slow.json");
        let tasks = (0..40).map(|n| {
            let mut x = ev_task(Path::new("/x"), &format!("t{n}"), "done", &["ac"], "slow");
            x["evidence"]["path"] = json!(ev.to_string_lossy());
            x
        }).collect();
        campaign(&t, s, tasks);
        fs::write(&ev, b"{}").unwrap();
    }
    let pid = project_with(&t, &app, &slugs);
    let start = std::time::Instant::now();
    let b = board(&app, &pid);
    let took = start.elapsed();
    assert!(took < Duration::from_secs(1), "took {took:?}");
    let n: usize = b["campaigns"].as_array().unwrap().iter().map(|c| c["cards"].as_array().unwrap().len()).sum();
    assert_eq!((b["campaigns"].as_array().unwrap().len(), n, b["truncated"].as_bool()), (5, 200, Some(false)));
    assert!(b["campaigns"].as_array().unwrap().iter().all(|c| c["truncated"] == false));
    assert!(app.evidence_cache.lock().unwrap().is_empty() && app.sweeping.lock().unwrap().is_empty(), "no check was started or claimed");
    assert_eq!(b["campaigns"][0]["campaign"], slugs[4].as_str(), "newest campaign first");
}

/// (a), (3): a sixth campaign is cut (top-level `truncated`); a two-hundred-and-first card is cut (that campaign's).
#[test]
fn both_levels_say_when_they_were_cut() {
    let t = Tmp::new();
    let app = t.app();
    let slugs: Vec<String> = (0..6).map(|i| format!("deliver-2026110{i}-1810")).collect();
    for s in &slugs {
        campaign(&t, s, vec![ev_task(Path::new("/x"), "t1", "pending", &[], "")]);
    }
    let pid = project_with(&t, &app, &slugs);
    let b = board(&app, &pid);
    assert_eq!((b["campaigns"].as_array().unwrap().len(), b["truncated"].as_bool()), (5, Some(true)), "{b}");
    assert!(!b["campaigns"].as_array().unwrap().iter().any(|c| c["campaign"] == slugs[0].as_str()), "the oldest is the one cut");

    let t = Tmp::new();
    let app = t.app();
    let slug = "deliver-20261107-1810".to_string();
    campaign(&t, &slug, (0..201).map(|n| ev_task(Path::new("/x"), &format!("t{n}"), "pending", &[], "")).collect());
    let pid = project_with(&t, &app, &[slug]);
    let b = board(&app, &pid);
    let c = &b["campaigns"][0];
    assert_eq!((c["cards"].as_array().unwrap().len(), c["truncated"].as_bool(), b["truncated"].as_bool()), (200, Some(true), Some(false)), "{c}");
}

/// #107 review: a verdict `evidence_list` caches moves the project revision, so a client polling with `since_rev` sees
/// the card reach Done (it never keeps a stale review card); a read with nothing changed is still `unchanged`.
#[test]
fn a_newly_cached_verdict_moves_the_revision_so_the_board_is_never_stale() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t1", "done", &["ac-1"], "green")]);
    fs::write(t.home().join(".claude/vajra/campaigns").join(&slug).join("evidence/green.json"), b"{}").unwrap();
    let pid = project_with(&t, &app, std::slice::from_ref(&slug));
    let first = app.project_state(&pid, None).unwrap();
    assert_eq!(card(&first["board"], "t1")["lane"], "review", "cold cache");
    let rev0 = first["rev"].as_u64().unwrap();
    assert_eq!(app.project_state(&pid, Some(rev0)).unwrap()["unchanged"], true, "nothing changed yet");
    app.evidence_list(&slug).unwrap();
    let after = app.project_state(&pid, Some(rev0)).unwrap();
    assert_eq!(after["unchanged"], false, "{after}");
    assert!(after["rev"].as_u64().unwrap() > rev0);
    assert_eq!(card(&after["board"], "t1")["lane"], "done", "{after}");
    assert_eq!(app.project_state(&pid, after["rev"].as_u64()).unwrap()["unchanged"], true, "steady again");
}

fn builds(app: &App) -> u64 {
    app.board_builds.load(std::sync::atomic::Ordering::SeqCst)
}

/// #107 review (cost): with nothing the board reads moving, steady polls answer from the cached board (no rebuild) and
/// stay `unchanged`.
#[test]
fn a_steady_poll_does_not_rebuild_the_board() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t1", "done", &["ac-1"], "green"), ev_task(&d, "t2", "pending", &["ac-2"], "")]);
    fs::write(t.home().join(".claude/vajra/campaigns").join(&slug).join("evidence/green.json"), b"{}").unwrap();
    let pid = project_with(&t, &app, std::slice::from_ref(&slug));
    app.evidence_list(&slug).unwrap();
    let mut rev = app.project_state(&pid, None).unwrap()["rev"].as_u64();
    for _ in 0..3 {
        rev = app.project_state(&pid, rev).unwrap()["rev"].as_u64().or(rev);
    }
    let before = builds(&app);
    for _ in 0..20 {
        assert_eq!(app.project_state(&pid, rev).unwrap()["unchanged"], true);
    }
    assert_eq!(builds(&app), before, "a steady poll rebuilt the board");
    assert_eq!(card(&app.project_state(&pid, None).unwrap()["board"], "t1")["lane"], "done", "the cached board is the right one");
}

/// #107 review (cost): an input that moved (the engine rewrote tasks.json) rebuilds the board, moves the revision and
/// shows the new lane; a cached verdict written by evidence_list does the same.
#[test]
fn a_moved_board_input_rebuilds_and_moves_the_revision() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t1", "pending", &["ac-1"], "")]);
    let pid = project_with(&t, &app, std::slice::from_ref(&slug));
    let mut rev = app.project_state(&pid, None).unwrap()["rev"].as_u64();
    for _ in 0..3 {
        rev = app.project_state(&pid, rev).unwrap()["rev"].as_u64().or(rev);
    }
    let before = builds(&app);
    assert_eq!(app.project_state(&pid, rev).unwrap()["unchanged"], true);
    campaign(&t, &slug, vec![ev_task(&d, "t1", "in_progress", &["ac-1"], "")]);
    let after = app.project_state(&pid, rev).unwrap();
    assert_eq!(after["unchanged"], false, "{after}");
    assert!(builds(&app) > before, "the board was not rebuilt");
    assert_eq!(card(&after["board"], "t1")["lane"], "running", "{after}");
    // A write to the verdict cache is an input too.
    let rev2 = after["rev"].as_u64();
    let rev2 = app.project_state(&pid, rev2).unwrap()["rev"].as_u64().or(rev2);
    let mid = builds(&app);
    app.board_inputs_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    app.project_state(&pid, rev2).unwrap();
    assert!(builds(&app) > mid, "a cache write did not rebuild the board");
}

/// #107 review: a cached verdict or repo state that expires while a board is being built is not dropped from the
/// cache's deadline, and that build is not served afterwards: the next steady poll rebuilds.
#[test]
fn a_verdict_expiring_during_a_rebuild_is_never_served_after_it() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t1", "pending", &["ac-1"], "")]);
    let pid = project_with(&t, &app, std::slice::from_ref(&slug));
    let mut rev = app.project_state(&pid, None).unwrap()["rev"].as_u64();
    for _ in 0..3 {
        rev = app.project_state(&pid, rev).unwrap()["rev"].as_u64().or(rev);
    }
    // A live verdict that stops counting 60 ms from now, written (so the next poll rebuilds) just before a 150 ms build.
    let until = std::time::Instant::now() + Duration::from_millis(60);
    app.evidence_cache.lock().unwrap().insert((t.0.join("ev.json"), b"{}".to_vec(), None, None, None), (true, Some(until)));
    app.board_inputs_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *lock(&app.board_build_pause) = Duration::from_millis(150);
    let since = std::time::Instant::now();
    app.project_state(&pid, rev).unwrap();
    *lock(&app.board_build_pause) = Duration::ZERO;
    // The deadline that passed during the build is still reported from the build's start.
    assert_eq!(app.next_input_expiry(since), Some(until));
    let before = builds(&app);
    app.project_state(&pid, rev).unwrap();
    assert!(builds(&app) > before, "the board built across the deadline was served from the cache");
}

/// #107 review: in the cache-only path the board reads, a cached failed git read is unknown, not a known state: a green
/// live verdict reads `checking` and its card stays in review. A good cached state still lets it reach Done.
#[test]
fn a_cached_failed_git_read_keeps_a_live_pass_out_of_done() {
    let t = Tmp::new();
    let (app, slug, repo, _head) = live_evidence_repo(&t, true, false);
    let pid = project_with(&t, &app, std::slice::from_ref(&slug));
    assert_eq!(app.evidence_list(&slug).unwrap()["rows"][0]["status"], "pass", "a full check caches a green verdict and the repo state");
    assert_eq!(card(&app.board(&pid), "t1")["lane"], "done", "with a known, matching repo state");
    // The repo state read failed and that failure is cached (as `worktree_state` does for 30 s).
    lock(&app.worktree_state).insert(repo.clone(), (std::time::Instant::now(), None));
    assert_eq!(app.evidence_list_with(&slug, false).unwrap()["rows"][0]["status"], "checking");
    let c = card(&app.board(&pid), "t1").clone();
    assert_eq!((c["lane"].as_str(), c["criteria"]["pass"].as_u64()), (Some("review"), Some(0)), "{c}");
}

/// #107 review sweep: a tasks.json that cannot be read now answers with its last good snapshot; the campaign says
/// `stale` and no card reaches Done from it.
#[test]
fn an_unreadable_task_file_never_shows_a_card_as_done() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t1", "done", &["ac-1"], "green")]);
    let camp = t.home().join(".claude/vajra/campaigns").join(&slug);
    fs::write(camp.join("evidence/green.json"), b"{}").unwrap();
    let pid = project_with(&t, &app, std::slice::from_ref(&slug));
    app.evidence_list(&slug).unwrap();
    let b = app.board(&pid);
    assert_eq!((card(&b, "t1")["lane"].as_str(), b["campaigns"][0]["stale"].as_bool()), (Some("done"), Some(false)), "{b}");
    fs::write(camp.join("tasks.json"), b"{ not json").unwrap();
    let b = app.board(&pid);
    assert_eq!(b["campaigns"][0]["stale"], true, "{b}");
    assert_eq!(card(&b, "t1")["lane"], "review", "a stale snapshot reached Done: {b}");
}

/// #107 review sweep: on the board's cache-only path, live evidence with no head cannot be held against the repo, so a
/// green verdict for it reads `checking`, not a pass.
#[test]
fn a_live_verdict_with_no_head_is_not_a_pass_on_the_board() {
    let t = Tmp::new();
    let (app, slug, _repo, _head) = live_evidence_repo(&t, true, false);
    let path = t.home().join(".claude/vajra/campaigns").join(&slug).join("tasks.json");
    let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    v["tasks"][0]["evidence"].as_object_mut().unwrap().remove("head");
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let full = app.evidence_list(&slug).unwrap();
    assert_eq!(full["rows"][0]["status"], "pass", "a full check: the verdict stands (its rule is unchanged): {full}");
    assert_eq!(app.evidence_list_with(&slug, false).unwrap()["rows"][0]["status"], "checking");
}
