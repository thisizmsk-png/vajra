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
