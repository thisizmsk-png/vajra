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
//! - (e) Revision: cards ride the existing `rev` bumps, and `project_state` also folds a fingerprint of the cards into
//!   its quiet-field check (with the worker list), so a card whose inputs changed without a bump (a verdict `evidence_list` cached, a task file
//!   the engine rewrote) moves the revision before `since_rev` is answered (#107 review).
//!   The board is rebuilt only when something it reads moved (`board_for_state`'s stamp); otherwise the last one is reused.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

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

/// The files and folders one board build read, beyond the fixed ones `board_stamp` always looks at.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Inputs {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
}

/// The last board of a project and the stamp of everything it was built from. `stamp: None` keeps the inputs (so the next
/// build can be stamped before it starts) but never answers from the cache.
pub(super) struct Cached {
    stamp: Option<u64>,
    inputs: Inputs,
    board: Value,
    /// A hash of `board`, made once per build (`project_state` folds it into its quiet fingerprint).
    fp: u64,
    /// The earliest moment a cached verdict or repo state it may have read stops counting.
    expires: Option<Instant>,
}

/// A folder with more entries than this is not stamped entry by entry: the board is rebuilt.
const STAMP_DIR_MAX: usize = 512;

/// Hash what identifies the file's content without reading it: device, inode, size, mtime and ctime (nanoseconds). A
/// missing file hashes as missing; any other error is doubt (`false`), so the board is rebuilt.
fn stat_into(h: &mut impl Hasher, p: &Path) -> bool {
    match std::fs::metadata(p) {
        Ok(m) => {
            (1u8, m.dev(), m.ino(), m.len(), m.mtime(), m.mtime_nsec(), m.ctime(), m.ctime_nsec()).hash(h);
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            0u8.hash(h);
            true
        }
        Err(_) => false,
    }
}

impl App {
    /// The board `project_state` shows (ADR-15 4.2, #107 review): rebuilt only when something it reads moved. The stamp
    /// covers the project's session log, the run index, the control ledger, every task file, progress file, evidence file
    /// and workflow folder the last build read, the live runs, the verdict and repo-state caches (a write counter and the
    /// earliest expiry) and the project revision. The stamp is taken before and after a build; the board is answered from
    /// the cache only when both agree and the inputs did not change, so a change during a build is never cached.
    /// Returns the board and a hash of it.
    pub(super) fn board_for_state(&self, project_id: &str, rev: Option<u64>) -> (Value, u64) {
        if !pc::valid_project_id(project_id) {
            return (Value::Null, 0);
        }
        let old = {
            let g = lock(&self.board_cache);
            match g.get(project_id) {
                Some(c) => {
                    let live = c.expires.is_none_or(|e| Instant::now() < e);
                    if live && c.stamp.is_some() && self.board_stamp(project_id, rev, &c.inputs) == c.stamp {
                        return (c.board.clone(), c.fp);
                    }
                    c.inputs.clone()
                }
                None => Inputs::default(),
            }
        };
        // Deadlines are taken from before the build: one that passes while the board is built is kept (and the build is not
        // cached), so a verdict read as live during the build can never be served after it expired (#107 review).
        let since = Instant::now();
        let pre = self.board_stamp(project_id, rev, &old);
        let mut inputs = Inputs::default();
        #[cfg(test)]
        std::thread::sleep(*lock(&self.board_build_pause));
        let board = self.board_build(project_id, &mut inputs);
        self.board_builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let expires = self.next_input_expiry(since);
        let post = self.board_stamp(project_id, rev, &inputs);
        let crossed = expires.is_some_and(|e| Instant::now() >= e);
        let stamp = post.filter(|_| pre.is_some() && inputs == old && pre == post && !crossed);
        let fp = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            board.to_string().hash(&mut h);
            h.finish()
        };
        lock(&self.board_cache).insert(project_id.to_string(), Cached { stamp, inputs, board: board.clone(), fp, expires });
        (board, fp)
    }

    fn board_stamp(&self, project_id: &str, rev: Option<u64>, inputs: &Inputs) -> Option<u64> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (rev, self.board_inputs_gen.load(std::sync::atomic::Ordering::SeqCst)).hash(&mut h);
        let mut runs: Vec<(String, PathBuf)> = lock(&self.runs).values().map(|r| (r.campaign.clone(), r.cwd.clone())).collect();
        runs.sort();
        runs.hash(&mut h);
        let fixed = [self.data.join("projects").join(project_id).join("sessions.jsonl"), self.data.join("runs.jsonl"), self.data.join("control/ledger.jsonl")];
        for p in fixed.iter().chain(&inputs.files) {
            if !stat_into(&mut h, p) {
                return None;
            }
        }
        for d in &inputs.dirs {
            if !stat_into(&mut h, d) {
                return None;
            }
            let mut names: Vec<std::ffi::OsString> = match std::fs::read_dir(d) {
                Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.file_name())).take(STAMP_DIR_MAX + 1).collect(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(_) => return None,
            };
            if names.len() > STAMP_DIR_MAX {
                return None;
            }
            names.sort();
            for n in names {
                n.hash(&mut h);
                if !stat_into(&mut h, &d.join(&n)) {
                    return None;
                }
            }
        }
        Some(h.finish())
    }

    /// The earliest moment after `since` that a cached verdict or repo state stops counting. One that expired before
    /// `since` was already a miss when the build read it; one that expired after it may have been read as live.
    pub(super) fn next_input_expiry(&self, since: Instant) -> Option<Instant> {
        let ev = self.evidence_cache.lock().unwrap_or_else(|e| e.into_inner()).values().filter_map(|(_, until)| *until).filter(|u| *u > since).min();
        let wt = lock(&self.worktree_state)
            .values()
            .map(|(at, st)| *at + if st.is_some() { super::WORKTREE_STATE_TTL } else { super::WORKTREE_STATE_UNKNOWN_TTL })
            .filter(|u| *u > since)
            .min();
        [ev, wt].into_iter().flatten().min()
    }

    /// A fresh board, no cache (tests).
    #[cfg(test)]
    pub(super) fn board(&self, project_id: &str) -> Value {
        if !pc::valid_project_id(project_id) {
            return Value::Null;
        }
        self.board_build(project_id, &mut Inputs::default())
    }

    /// Build the board of store project `project_id`, recording in `inputs` what it read.
    fn board_build(&self, project_id: &str, inputs: &mut Inputs) -> Value {
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
        let campaigns: Vec<Value> = slugs.iter().map(|slug| self.campaign_board(project_id, slug, origins.get(slug), &mut left, inputs)).collect();
        json!({"campaigns": campaigns, "truncated": truncated})
    }

    fn campaign_board(&self, project_id: &str, slug: &str, origin: Option<&super::runindex::Entry>, left: &mut usize, inputs: &mut Inputs) -> Value {
        let camp = self.claude.join("vajra/campaigns").join(slug);
        inputs.files.extend([camp.join("tasks.json"), camp.join("progress.md")]);
        if let Some(e) = origin {
            inputs.dirs.push(self.workflows_dir(e));
        }
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
        // Every evidence file a verdict is keyed on (its bytes): one rewritten moves the stamp.
        inputs.files.extend(tasks.into_iter().flatten().filter_map(|t| t.get("evidence")?.get("path")?.as_str().map(PathBuf::from)));
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
            // An id the engine could not have written is no card (it would ride in the key and `task_id` raw).
            let Some(id) = t.get("id").and_then(Value::as_str).filter(|id| super::inbox::valid_task_id(id)) else { continue };
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
                "owner": t.get("owner_agent").and_then(Value::as_str).map(|o| super::inbox::clean(&crate::crew::names::ui_name(o), TITLE_MAX)),
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
