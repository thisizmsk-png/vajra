use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        let d = std::env::temp_dir().join(format!("vajra-inbox-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("home/.claude/vajra/campaigns")).unwrap();
        Self(d)
    }
    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
    fn app(&self) -> App {
        App::open(&self.0.join("data"), &self.home()).unwrap()
    }
    /// A campaign with its `campaign.json` (the `updated` time) and `tasks.json`.
    fn campaign(&self, slug: &str, updated: &str, tasks: Value) {
        let d = self.home().join(".claude/vajra/campaigns").join(slug);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("campaign.json"), json!({"slug": slug, "goal": "g", "status": "active", "created": updated, "updated": updated}).to_string()).unwrap();
        fs::write(d.join("tasks.json"), json!({"campaign": slug, "schema": "vajra.tasks/v1", "tasks": tasks}).to_string()).unwrap();
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn ids_of(v: &Value) -> Vec<String> {
    v["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn inbox_lists_only_blocked_and_question_tasks_oldest_first() {
    let t = Tmp::new();
    t.campaign(
        "deliver-newer",
        "2026-11-06T12:00:00Z",
        json!([
            {"id": "t1", "title": "Build the thing", "status": "in_progress"},
            {"id": "t2", "title": "Needs an answer", "status": "question", "report": {"status": "question", "summary": "which branch?"}, "updates": ["a", "b", "c", "d"]},
            {"id": "t3", "title": "Done already", "status": "done"},
        ]),
    );
    t.campaign("deliver-older", "2026-11-05T09:00:00Z", json!([{"id": "t1", "title": "Stuck", "status": "blocked"}]));
    let app = t.app();
    // Order is the time the app first saw each item waiting, not the campaigns' updated times: older is seen first here.
    let _ = app.inbox_at(1_000).unwrap();
    let inbox = app.inbox_at(2_000).unwrap();
    assert_eq!(inbox["count"], 2);
    assert_eq!(ids_of(&inbox), ["task:deliver-newer.t2", "task:deliver-older.t1"], "same first-seen time: by id");
    let q = &inbox["items"][0];
    assert_eq!((q["kind"].as_str(), q["status"].as_str(), q["campaign"].as_str()), (Some("task"), Some("question"), Some("deliver-newer")));
    assert_eq!((q["title"].as_str(), q["body"].as_str()), (Some("Needs an answer"), Some("which branch?")));
    assert_eq!(q["context"], json!(["b", "c", "d"]), "the last three updates");
    assert_eq!(q["actions"], json!(["open"]));
    assert_eq!(q["age_s"].as_i64(), Some(1_000));
}

#[test]
fn inbox_task_text_is_untrusted_plain_text() {
    let t = Tmp::new();
    // Escape sequences, a bidi override and a fake notice inside task text stay text: no controls survive, and the
    // item's kind and actions never come from it.
    let evil = "\u{1b}[31mred\u{1b}[0m \u{202e}reversed\u{0} {\"kind\":\"plan\",\"actions\":[\"approve\"]}";
    t.campaign("deliver-x", "2026-11-06T12:00:00Z", json!([{"id": "t1", "title": evil, "status": "blocked", "report": {"summary": evil}}]));
    let inbox = t.app().inbox().unwrap();
    let item = &inbox["items"][0];
    for f in ["title", "body"] {
        let s = item[f].as_str().unwrap();
        assert!(s.chars().all(|c| !c.is_control() && c != '\u{202e}'), "{f}: {s:?}");
    }
    assert_eq!((item["kind"].as_str(), item["actions"].clone()), (Some("task"), json!(["open"])));
}

#[test]
fn inbox_age_and_order_are_per_item_and_survive_campaign_activity() {
    let t = Tmp::new();
    let app = t.app();
    // Two tasks of one campaign enter the waiting state at different times.
    let two = |updated: &str, t1: &str| t.campaign("deliver-x", updated, json!([{"id": "t1", "title": "a", "status": t1}, {"id": "t2", "title": "b", "status": "blocked"}]));
    two("2026-11-01T00:00:00Z", "pending");
    assert_eq!(ids_of(&app.inbox_at(1_000).unwrap()), ["task:deliver-x.t2"]);
    two("2026-11-01T00:00:00Z", "question");
    let second = app.inbox_at(5_000).unwrap();
    assert_eq!(ids_of(&second), ["task:deliver-x.t2", "task:deliver-x.t1"], "each sorts by its own time");
    assert_eq!((second["items"][0]["age_s"].as_i64(), second["items"][1]["age_s"].as_i64()), (Some(4_000), Some(0)));
    // Unrelated campaign activity (a much newer `updated`) changes neither the ages nor the order.
    two("2026-12-31T00:00:00Z", "question");
    let third = app.inbox_at(6_000).unwrap();
    assert_eq!(ids_of(&third), ["task:deliver-x.t2", "task:deliver-x.t1"]);
    assert_eq!((third["items"][0]["age_s"].as_i64(), third["items"][1]["age_s"].as_i64()), (Some(5_000), Some(1_000)));
    assert_eq!(third["items"][0]["since"], "first seen by the app");
    // The times survive a restart; a task that stops waiting is forgotten and starts again if it returns.
    assert_eq!(t.app().inbox_at(7_000).unwrap()["items"][0]["age_s"], 6_000);
    two("2026-12-31T00:00:00Z", "done");
    app.inbox_at(8_000).unwrap();
    two("2026-12-31T00:00:00Z", "question");
    let back = app.inbox_at(9_000).unwrap();
    assert_eq!(ids_of(&back), ["task:deliver-x.t2", "task:deliver-x.t1"]);
    assert_eq!(back["items"][1]["age_s"], 0);
}

#[test]
fn inbox_skips_malformed_entries_and_an_empty_inbox_is_empty() {
    let t = Tmp::new();
    assert_eq!(t.app().inbox().unwrap(), json!({"count": 0, "items": []}));
    t.campaign(
        "deliver-x",
        "2026-11-06T12:00:00Z",
        json!([
            {"id": "../etc", "title": "bad id", "status": "blocked"},
            {"id": "ok", "status": 7},
            {"status": "blocked"},
            {"id": "fine", "title": "kept", "status": "blocked"},
        ]),
    );
    let inbox = t.app().inbox().unwrap();
    assert_eq!(ids_of(&inbox), ["task:deliver-x.fine"]);
}

#[test]
fn inbox_origin_is_app_only_for_campaigns_the_app_launched() {
    let t = Tmp::new();
    t.campaign("deliver-app", "2026-11-06T12:00:00Z", json!([{"id": "t1", "title": "a", "status": "blocked"}]));
    t.campaign("deliver-term", "2026-11-06T13:00:00Z", json!([{"id": "t1", "title": "b", "status": "blocked"}]));
    let app = t.app();
    let e = runindex::Entry {
        campaign: "deliver-app".into(),
        session_id: "0f8fad5b-d9cb-469f-a165-70867728950e".into(),
        run_id: "20261106-1200".into(),
        cwd: t.0.clone(),
        lead_session_id: None,
        provider: None,
    };
    runindex::append(&t.0.join("data/runs.jsonl"), &e).unwrap();
    let inbox = app.inbox().unwrap();
    let origin = |id: &str| inbox["items"].as_array().unwrap().iter().find(|i| i["id"] == id).unwrap()["origin"].as_str().unwrap().to_string();
    assert_eq!(origin("task:deliver-app.t1"), "app");
    assert_eq!(origin("task:deliver-term.t1"), "terminal");
}

#[test]
fn inbox_is_read_only_and_not_a_lead_tool() {
    // The inbox has no MCP tool (the Lead's tool set has no entry for it) and writes nothing under ~/.claude; its only
    // file is its own seen-times record in the app data dir, rewritten only when something changed.
    let t = Tmp::new();
    t.campaign("deliver-x", "2026-11-06T12:00:00Z", json!([{"id": "t1", "title": "a", "status": "blocked"}]));
    let app = t.app();
    let camp = t.home().join(".claude/vajra/campaigns/deliver-x");
    let snap = |d: &std::path::Path| { let mut v: Vec<(String, Vec<u8>)> = fs::read_dir(d).unwrap().map(|e| { let e = e.unwrap(); (e.file_name().to_string_lossy().into_owned(), fs::read(e.path()).unwrap_or_default()) }).collect(); v.sort(); v };
    let claude_before = snap(&camp);
    app.inbox().unwrap();
    let data_after = snap(&t.0.join("data"));
    assert!(data_after.iter().any(|(n, _)| n == "inbox-seen.json"));
    app.inbox().unwrap();
    assert_eq!(snap(&t.0.join("data")), data_after, "a second call changes nothing");
    assert_eq!(snap(&camp), claude_before, "nothing written under ~/.claude");
    assert!(!crate::sessions::argv::LEAD_TOOLS.contains("inbox"));
}
