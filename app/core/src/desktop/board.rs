//! Board cards in `project_state` (ADR-15 4.2, FG-06): the core computes the cards the UI adapter used to.
//!
//! Contract (agreed with vajra-1, recorded in the PR so the wave-3 board switch has one source):
//! - (a) Scope: `project_state(p_).board = {campaigns: [{campaign, run_id, cards, truncated}], truncated}`. The project's
//!   campaigns come from its `sessions.jsonl` run lines (`kind: run`, `campaign`), newest first, at most
//!   `MAX_CAMPAIGNS`; at most `MAX_CARDS` cards across them. `truncated` says a level was cut.
//! - (b) Card: `{key, kind: task|plan|question, local, title, owner, status, lane, needs_you, criteria: {pass, total} |
//!   null, project_id, campaign, task_id, attempt}`. A task's key is `<campaign>/<task_id>/<attempt>`, an inbox card's
//!   `inbox:<id>` (its `task_id` and `attempt` are null). Text from tasks and the inbox goes through the inbox's clean().
//! - (c) Lanes: claimed, in_progress and question are running; done is done only when it has criteria and every one is a
//!   verified pass, otherwise review; failed is review; pending, ready, blocked (and anything unknown) are backlog.
//!   `needs_you` = question or blocked. A waiting plan or question is a review card with `needs_you`. Proposals are not
//!   cards. No status ever moves a card to done by itself.
//! - (d) Cost: cards read the evidence verdicts already cached (the ones `evidence_list` fills) and never start a check;
//!   a criterion not checked yet reads unverified, so its card stays in review.
//! - (e) Revision: cards ride the existing `rev` bumps; there is no new revision source.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::{App, lock, runview};
use crate::projects::contract::{self as pc, SessionKind};

pub(super) const MAX_CAMPAIGNS: usize = 5;
pub(super) const MAX_CARDS: usize = 200;
const TITLE_MAX: usize = 200;

/// The engine's task statuses the board knows; anything else reads `unknown` (and sits in backlog).
const STATUSES: [&str; 8] = ["pending", "ready", "claimed", "in_progress", "done", "blocked", "question", "failed"];

/// (c): the lane and the needs-you badge of a task card.
pub(super) fn lane(status: &str, criteria: Option<(usize, usize)>) -> (&'static str, bool) {
    let lane = match status {
        "claimed" | "in_progress" | "question" => "running",
        "done" if criteria.is_some_and(|(pass, total)| total > 0 && pass == total) => "done",
        "done" | "failed" => "review",
        _ => "backlog",
    };
    (lane, matches!(status, "question" | "blocked"))
}

impl App {
    /// The board of store project `project_id`, or `null` for any other id.
    pub(super) fn board(&self, project_id: &str) -> Value {
        if !pc::valid_project_id(project_id) {
            return Value::Null;
        }
        let log = match self.projects.sessions(project_id) {
            Ok(l) => l,
            Err(_) => return json!({"campaigns": [], "truncated": false, "error": "sessions_unreadable"}),
        };
        let mut runs: Vec<(String, String)> = log
            .sessions
            .into_iter()
            .filter(|s| s.kind == SessionKind::Run)
            .filter_map(|s| Some((s.started, s.campaign?)))
            .filter(|(_, c)| crate::bus::valid_stream_id(c))
            .collect();
        runs.sort_by(|a, b| b.0.cmp(&a.0));
        let mut slugs: Vec<String> = Vec::new();
        for (_, c) in runs {
            if !slugs.contains(&c) {
                slugs.push(c);
            }
        }
        // Cut at the limit, or a session log read only in part (older campaigns may be missing).
        let truncated = slugs.len() > MAX_CAMPAIGNS || log.truncated;
        slugs.truncate(MAX_CAMPAIGNS);
        let origins = if slugs.is_empty() { HashMap::new() } else { self.plan_origins() };
        let mut left = MAX_CARDS;
        let campaigns: Vec<Value> = slugs.iter().map(|slug| self.campaign_board(project_id, slug, origins.get(slug), &mut left)).collect();
        json!({"campaigns": campaigns, "truncated": truncated})
    }

    fn campaign_board(&self, project_id: &str, slug: &str, origin: Option<&super::runindex::Entry>, left: &mut usize) -> Value {
        let mut cards: Vec<Value> = Vec::new();
        let mut cut = false;
        let mut push = |cards: &mut Vec<Value>, c: Value, left: &mut usize| {
            if *left == 0 {
                cut = true;
            } else {
                *left -= 1;
                cards.push(c);
            }
        };
        let read = self.with_reader(|r| Ok((r.tasks(slug), r.progress(slug).unwrap_or_default()))).ok();
        let (value, progress) = read.map(|(s, p)| (s.value, p)).unwrap_or((None, String::new()));
        let tasks: Option<&[Value]> = value.as_ref().and_then(|v| v.get("tasks")).and_then(Value::as_array).map(Vec::as_slice);
        let (run_id, scoped) = runview::scope(&progress, tasks);
        // Per task: (verified passes, criteria), from the cached verdicts only.
        let mut crit: HashMap<String, (usize, usize)> = HashMap::new();
        if tasks.is_some() {
            if let Ok(ev) = self.evidence_list_with(slug, false) {
                for r in ev["rows"].as_array().into_iter().flatten() {
                    let e = crit.entry(r["task"].as_str().unwrap_or("").to_string()).or_insert((0, 0));
                    e.1 += 1;
                    if r["status"] == "pass" {
                        e.0 += 1;
                    }
                }
            }
        }
        let prefix = format!("{run_id}.");
        for t in scoped.iter().flatten() {
            let Some(id) = t.get("id").and_then(Value::as_str) else { continue };
            let raw = t.get("status").and_then(Value::as_str).unwrap_or("");
            let status = if STATUSES.contains(&raw) { raw } else { "unknown" };
            let criteria = crit.get(id).copied().filter(|(_, total)| *total > 0);
            let (lane, needs_you) = lane(status, criteria);
            let attempt = t.get("attempts").and_then(Value::as_u64).unwrap_or(0);
            let local = id.strip_prefix(&prefix).unwrap_or(id);
            let title = t.get("title").and_then(Value::as_str).unwrap_or(local);
            let card = json!({
                "key": format!("{slug}/{id}/{attempt}"), "kind": "task", "local": super::inbox::clean(local, TITLE_MAX),
                "title": super::inbox::clean(title, TITLE_MAX),
                "owner": t.get("owner_agent").and_then(Value::as_str).map(crate::crew::names::ui_name),
                "status": status, "lane": lane, "needs_you": needs_you,
                "criteria": criteria.map(|(pass, total)| json!({"pass": pass, "total": total})),
                "project_id": project_id, "campaign": slug, "task_id": id, "attempt": attempt,
            });
            push(&mut cards, card, left);
        }
        // Waiting plans and questions: review cards with the badge. Read-only: nothing is marked shown or seen here.
        if let Some(w) = origin.and_then(|e| self.waiting(e).map(|w| (e, w))) {
            let (e, w) = w;
            let inbox_card = |id: String, kind: &str, title: String| {
                json!({"key": format!("inbox:{id}"), "kind": kind, "local": id, "title": super::inbox::clean(&title, TITLE_MAX), "owner": null,
                    "status": "needs_approval", "lane": "review", "needs_you": true, "criteria": null,
                    "project_id": project_id, "campaign": slug, "task_id": null, "attempt": null})
            };
            if w.question.is_some() {
                push(&mut cards, inbox_card(format!("question:{}", e.run_id), "question", "The run needs an answer".into()), left);
            }
            for p in &w.plans {
                let id = format!("plan:{}.{}", e.run_id, p.index);
                if !matches!(lock(&self.control).decided(&id), Ok(None)) {
                    continue;
                }
                push(&mut cards, inbox_card(id, "plan", format!("Approve: {}", p.action.replace('_', " "))), left);
            }
        }
        json!({"campaign": slug, "run_id": run_id, "cards": cards, "truncated": cut})
    }
}
