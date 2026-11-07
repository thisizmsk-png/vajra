//! The needs-you inbox (ADR-14, S5), read-only. It is derived on every call and stored nowhere.
//!
//! Core B1 lists the one kind whose source is already trustworthy: tasks that `tasks.json` says are `blocked` or
//! `question`. `tasks.json` changes only through `vajra_tasks.py` (the worker guard protects it), and nothing in an
//! item comes from stream or terminal text (AC-5.1.6). Every text field is untrusted: control characters are
//! dropped, and the UI renders it as plain text.
//!
//! `plan` and `question` items need the workflow's final result, whose location is still open (the `Workflow`
//! tool_result in the run transcript is only the launch acknowledgement), so they are not listed here.

use std::time::SystemTime;

use serde_json::{Value, json};

use super::{App, Res, fail, runindex};
use crate::control::lead::sanitise;
use crate::control::time::parse_iso;
use crate::crew::redact::Redactor;

const TITLE_MAX: usize = 200;
const BODY_MAX: usize = 500;
const CONTEXT_LINES: usize = 3;
const MAX_ITEMS: usize = 200;

/// Plain text for the UI: controls and invisible characters dropped, one cap.
fn clean(text: &str, cap: usize) -> String {
    sanitise(text, &Redactor::default()).chars().take(cap).collect()
}

/// A graph id the engine could have written: letters, digits, `.`, `_`, `-`, at most 64.
fn valid_task_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl App {
    /// Everything waiting on the human, oldest first, at most 200 items with the true count.
    pub fn inbox(&self) -> Res {
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let launched = runindex::latest(&self.data.join("runs.jsonl"));
        let mut items: Vec<(i64, String, Value)> = Vec::new();
        self.with_reader(|r| {
            for slug in r.campaigns().map_err(|e| fail("io_error", e))? {
                let updated = r.campaign_json(&slug).value.and_then(|c| c.get("updated").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
                let created = parse_iso(&updated).unwrap_or(now);
                let origin = if launched.contains_key(&slug) { "app" } else { "terminal" };
                let tasks = r.tasks(&slug).value;
                for t in tasks.as_ref().and_then(|v| v.get("tasks")).and_then(Value::as_array).into_iter().flatten() {
                    let (Some(id), Some(status)) = (t.get("id").and_then(Value::as_str), t.get("status").and_then(Value::as_str)) else { continue };
                    if !matches!(status, "blocked" | "question") || !valid_task_id(id) {
                        continue;
                    }
                    let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or_default().to_string();
                    let updates: Vec<&str> = t.get("updates").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                    let context: Vec<String> = updates.iter().rev().take(CONTEXT_LINES).rev().map(|l| clean(l, BODY_MAX)).collect();
                    let item_id = format!("task:{slug}.{id}");
                    items.push((
                        created,
                        item_id.clone(),
                        json!({
                            "id": item_id, "kind": "task", "campaign": slug, "origin": origin,
                            "status": status,
                            "created": updated, "age_s": (now - created).max(0),
                            "title": clean(&text(t.get("title")), TITLE_MAX),
                            "body": clean(&text(t.get("report").and_then(|r| r.get("summary"))), BODY_MAX),
                            "context": context,
                            "actions": ["open"],
                        }),
                    ));
                }
            }
            Ok(())
        })?;
        items.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        let count = items.len();
        let list: Vec<Value> = items.into_iter().take(MAX_ITEMS).map(|(_, _, v)| v).collect();
        Ok(json!({"count": count, "items": list}))
    }
}

#[cfg(test)]
#[path = "inbox_tests.rs"]
mod tests;
