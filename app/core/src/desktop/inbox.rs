//! The needs-you inbox (ADR-14, S5), read-only. It is derived on every call and stored nowhere.
//!
//! Core B1 lists the one kind whose source is already trustworthy: tasks that `tasks.json` says are `blocked` or
//! `question`. `tasks.json` changes only through `vajra_tasks.py` (the worker guard protects it), and nothing in an
//! item comes from stream or terminal text (AC-5.1.6). Every text field is untrusted: control characters are
//! dropped, and the UI renders it as plain text.
//!
//! `tasks.json` has no per-task time (`vajra_tasks.py` keeps only text updates), and the campaign's `updated` time moves
//! with unrelated progress. So `created` and `age_s` are the time THIS APP first saw the task waiting, kept per item in
//! `<data>/inbox-seen.json` and dropped when the task stops waiting. That is not when the task became blocked (a task
//! that blocked while the app was closed shows the time the app first looked); each item says
//! `since: "first seen by the app"` so the UI never presents it as the engine's time.
//!
//! Core B2 adds `plan` and `question` items from the workflow's own result file (`wfresult.rs`, ADR-14). That file is
//! unsigned and worker-writable, so those items say where they came from, and every plan carries `plan_sha256`, the
//! hash of exactly what was shown: `approve_plan` refuses unless the file still hashes to it.

use std::collections::HashMap;
use std::time::SystemTime;

use serde_json::{Value, json};

use std::path::PathBuf;

use super::{App, Res, fail, lock, project_slug, runindex, wfresult};
use crate::control::lead::sanitise;
use crate::control::sha256;
use crate::crew::redact::Redactor;

const TITLE_MAX: usize = 200;
const BODY_MAX: usize = 500;
const CONTEXT_LINES: usize = 3;
const MAX_ITEMS: usize = 200;
const SEEN_FILE: &str = "inbox-seen.json";
const SINCE: &str = "first seen by the app";

/// Plain text for the UI: controls and invisible characters dropped, one cap.
fn clean(text: &str, cap: usize) -> String {
    sanitise(text, &Redactor::default()).chars().take(cap).collect()
}

/// A graph id the engine could have written: letters, digits, `.`, `_`, `-`, at most 64.
fn valid_task_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The id of a campaign no known folder owns (a terminal-started run, or one from before the app recorded folders).
pub(super) const UNSORTED_PROJECT: &str = "p_unsorted";

/// The project id of a run's folder, from the project store. A folder no project owns (or one that is gone, or refused)
/// is Unsorted (ADR-15): it is never guessed into a project and never creates one. One lookup per folder per read.
fn project_of(app: &App, cache: &mut HashMap<std::path::PathBuf, String>, cwd: &std::path::Path) -> String {
    cache
        .entry(cwd.to_path_buf())
        .or_insert_with(|| app.projects.find(cwd).ok().flatten().map_or_else(|| UNSORTED_PROJECT.to_string(), |p| p.id))
        .clone()
}

/// Shown with every item that comes from the workflow result file.
const FROM_RESULT: &str = "from the workflow result";

/// `plan:<run_id>.<index>` -> (run_id, index).
pub(super) fn parse_plan_id(id: &str) -> Option<(&str, usize)> {
    let (run, idx) = id.strip_prefix("plan:")?.split_once('.')?;
    let index = idx.parse::<usize>().ok().filter(|i| *i <= crate::sessions::ApprovedPlanTurn::MAX_INDEX && idx == i.to_string())?;
    crate::ids::valid_run_id(run).then_some((run, index))
}

/// The bytes that are shown and hashed for one plan item.
pub(super) fn plan_sha(shown: &Value) -> String {
    sha256::hex_digest(&serde_json::to_vec(shown).unwrap_or_default())
}

/// What `Arrivals` tracks for one item: its key (id plus the plan hash or the task status, so a changed plan or a task that
/// left and came back is new), its project and its kind (a task's status, so a question and a block tell apart).
fn arrival_key(v: &Value) -> (String, String, String) {
    let s = |k: &str| v[k].as_str().unwrap_or_default();
    let kind = if s("kind") == "task" { s("status") } else { s("kind") };
    (format!("{}@{}", s("id"), if s("plan_sha256").is_empty() { s("status") } else { s("plan_sha256") }), s("project_id").to_string(), kind.to_string())
}

/// One fingerprint per project of the keys waiting in it, order-independent.
fn project_fingerprints(keys: impl Iterator<Item = (String, String, String)>) -> HashMap<String, u64> {
    use std::hash::{Hash, Hasher};
    let mut by: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (key, project, _) in keys {
        by.entry(project).or_default().push(key);
    }
    by.into_iter()
        .map(|(p, mut ks)| {
            ks.sort();
            let mut h = std::collections::hash_map::DefaultHasher::new();
            ks.hash(&mut h);
            (p, h.finish() | 1)
        })
        .collect()
}

impl App {
    /// The runs whose plans the app will show or act on, one per campaign: the campaign's newest app-launched run in
    /// its newest session, narrowed to the newest run that STARTED A WORKFLOW (a resume with `vajra-deliver`). The
    /// approval-only resume (ADR-14) starts no workflow, so it keeps naming the workflow that produced the plan; a
    /// resume that runs a new workflow gets its own id, so its plans are new items even when their text repeats an
    /// earlier, already decided one. `runs.jsonl` is worker-writable, so an index line counts only if its directory,
    /// session id and campaign all equal what the app recorded under its guarded control dir at launch.
    pub(super) fn plan_origins(&self) -> HashMap<String, runindex::Entry> {
        let trusted = |e: &runindex::Entry| {
            crate::ids::valid_session_id(&e.session_id)
                && crate::ids::valid_run_id(&e.run_id)
                && self.read_run_record(&e.run_id).map(|(cwd, sid, camp, wf)| (cwd == e.cwd && sid == e.session_id && camp == e.campaign, wf)).is_some_and(|(same, _)| same)
        };
        let workflow = |e: &runindex::Entry| self.read_run_record(&e.run_id).is_some_and(|r| r.3);
        let all: Vec<runindex::Entry> = runindex::entries(&self.data.join("runs.jsonl")).into_iter().filter(|e| trusted(e)).collect();
        let mut latest: HashMap<String, &runindex::Entry> = HashMap::new();
        for e in &all {
            latest.insert(e.campaign.clone(), e);
        }
        latest
            .into_iter()
            .filter_map(|(camp, l)| {
                let wf = all.iter().filter(|e| e.campaign == camp && e.session_id == l.session_id && workflow(e)).max_by(|a, b| a.run_id.cmp(&b.run_id))?;
                Some((camp, wf.clone()))
            })
            .collect()
    }

    /// The `workflows/` directory of a run's session (the run is one `plan_origins` returned).
    fn workflows_dir(&self, e: &runindex::Entry) -> PathBuf {
        self.claude.join("projects").join(project_slug(&e.cwd)).join(&e.session_id).join("workflows")
    }

    /// What the newest `vajra-deliver` result of this app run asks of the human.
    pub(super) fn waiting(&self, e: &runindex::Entry) -> Option<wfresult::Waiting> {
        wfresult::waiting_for(&self.allow().ok()?, &self.workflows_dir(e), &e.campaign, self.read_run_started(&e.run_id)?).ok().flatten()
    }

    /// Everything waiting on the human, oldest first, at most 200 items with the true count.
    pub fn inbox(&self) -> Res {
        self.inbox_at(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64))
    }

    fn inbox_at(&self, now: i64) -> Res {
        let all = self.inbox_all_at(now)?;
        let count = all.len();
        Ok(json!({"count": count, "items": all.into_iter().take(MAX_ITEMS).collect::<Vec<_>>()}))
    }

    /// Every waiting item, oldest first, with no cut: a caller that counts or picks (a project's projection, the item an
    /// answer targets) does it here, and cuts only what it sends out.
    pub(super) fn inbox_all(&self) -> Result<Vec<Value>, Value> {
        self.inbox_all_at(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64))
    }

    fn inbox_all_at(&self, now: i64) -> Result<Vec<Value>, Value> {
        let seen_path = self.data.join(SEEN_FILE);
        let mut seen: std::collections::BTreeMap<String, i64> = crate::control::fsx::read_nofollow_capped(&seen_path, 1 << 20)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let before = seen.clone();
        let mut waiting: std::collections::BTreeSet<String> = Default::default();
        let launched = runindex::latest(&self.data.join("runs.jsonl"));
        let mut folders: HashMap<std::path::PathBuf, String> = HashMap::new();
        let mut items: Vec<(i64, String, Value)> = Vec::new();
        self.with_reader(|r| {
            for slug in r.campaigns().map_err(|e| fail("io_error", e))? {
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
                    // Keyed with the status: a task that leaves and re-enters the waiting state starts again.
                    let key = format!("{item_id}@{status}");
                    let created = *seen.entry(key.clone()).or_insert(now);
                    waiting.insert(key);
                    items.push((
                        created,
                        item_id.clone(),
                        json!({
                            "id": item_id, "kind": "task", "campaign": slug, "origin": origin,
                            "project_id": launched.get(&slug).map_or_else(|| UNSORTED_PROJECT.to_string(), |e| project_of(self, &mut folders, &e.cwd)),
                            "status": status,
                            "since": SINCE, "created": crate::control::time::iso_secs(std::time::UNIX_EPOCH + std::time::Duration::from_secs(created.max(0) as u64)), "age_s": (now - created).max(0),
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
        for (slug, e) in &self.plan_origins() {
            let Some(w) = self.waiting(e) else { continue };
            // Same rule as tasks: the time is when the app first saw this exact item waiting (the result file's own
            // mtime is worker-writable and says nothing about when it started waiting).
            let mut base = |id: &str, kind: &str, key: String| {
                let first = *seen.entry(key.clone()).or_insert(now);
                waiting.insert(key);
                let created = crate::control::time::iso_secs(std::time::UNIX_EPOCH + std::time::Duration::from_secs(first.max(0) as u64));
                (first, json!({"id": id, "kind": kind, "campaign": slug, "project_id": project_of(self, &mut folders, &e.cwd), "origin": "app", "source": FROM_RESULT, "since": SINCE, "created": created, "age_s": (now - first).max(0)}))
            };
            if let Some(q) = &w.question {
                let id = format!("question:{}", e.run_id);
                let (first, mut v) = base(&id, "question", format!("{id}@{}", plan_sha(&json!(q))));
                v["title"] = json!("The run needs an answer");
                v["body"] = json!(q);
                v["actions"] = json!(["open"]);
                items.push((first, id, v));
            }
            for p in &w.plans {
                let id = format!("plan:{}.{}", e.run_id, p.index);
                let sha = plan_sha(&p.raw);
                // A decided plan id is not offered again, even with a different entry at its index: the approval turn
                // names only the id. A ledger that cannot be read fails closed (the plan is hidden).
                if !matches!(lock(&self.control).decided(&id), Ok(None)) {
                    continue;
                }
                lock(&self.shown).insert(id.clone(), sha.clone());
                let (first, mut v) = base(&id, "plan", format!("{id}@{sha}"));
                v["title"] = json!(format!("Approve: {}", p.action.replace('_', " ")));
                v["body"] = p.shown.get("why").cloned().unwrap_or(Value::Null);
                // The whole entry, as written (unsafe characters made visible), in the order the workflow wrote it.
                v["entry"] = p.shown.clone();
                v["plan"] = p.shown.get("plan").cloned().unwrap_or(Value::Null);
                v["action"] = json!(p.shown.get("action").cloned().unwrap_or(Value::Null));
                v["plan_sha256"] = json!(sha);
                v["invalid"] = json!(p.invalid);
                v["actions"] = if p.invalid.is_some() { json!(["reject"]) } else { json!(["approve", "reject"]) };
                items.push((first, id, v));
            }
        }
        seen.retain(|k, _| waiting.contains(k));
        if seen != before {
            let saved = crate::control::fsx::private_dir(&self.data).and_then(|()| crate::control::fsx::write_atomic(&self.data, SEEN_FILE, &serde_json::to_vec(&seen).unwrap_or_default()));
            if saved.is_err() {
                self.note("inbox_seen_failed", &[]);
            }
        }
        items.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        self.events.observe(items.iter().map(|(_, _, v)| arrival_key(v)));
        self.inbox_read(project_fingerprints(items.iter().map(|(_, _, v)| arrival_key(v))));
        Ok(items.into_iter().map(|(_, _, v)| v).collect())
    }
}

#[cfg(test)]
#[path = "inbox_tests.rs"]
mod tests;
