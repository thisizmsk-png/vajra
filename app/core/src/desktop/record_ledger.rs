//! The core's own ledger of its record writes (bhishma H1 on #190). The signature proves only that a key-holding
//! script wrote an entry, and any agent can run those scripts against the core's campaign. So the core records every
//! write it made, and compares the campaign and task graph it reads back with that. An entry the core didn't write,
//! or one it wrote that is missing, means the record isn't the core's: not verified.
//!
//! The ledger holds what the scripts must show after the core's writes: the goal, each feature and its pass state,
//! every progress line the scripts appended for those writes, in order, and each task's status, owner, attempts and
//! last report. It is pure: `record_app.rs` persists it and reads the campaign back.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::vajra::bridge::{CloseStatus, Request};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerTask {
    /// `open` (pending or ready, which the script derives), `claimed`, `in_progress`, `blocked`, `question`, `failed`
    /// or `done`.
    pub status: String,
    pub attempts: u32,
    pub claimed_by: Option<String>,
    /// The last report: `(status, summary)`.
    pub report: Option<(String, String)>,
    /// What `add` stored: `title`, `owner_agent`, `deps`, `files`, `acceptance` (the script's own definition keys).
    pub def: Value,
    /// What the core's own close stored (bhishma N2): set only when the script answered `closed`.
    pub closed: Option<Closed>,
    /// The branch the task graph names: `add`'s, then the close's when it gave one (codex on #190).
    #[serde(default)]
    pub branch: String,
    /// The worktree the last claim (or `add`) recorded, if any.
    #[serde(default)]
    pub worktree: Option<String>,
}

/// A task close as the script stores it: the sha256 of the evidence bytes, and `commit`, `branch`, `base_commit`
/// (the script stores each only when it isn't empty).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Closed {
    pub evidence_sha256: String,
    pub commit: String,
    pub branch: String,
    pub base_commit: String,
    /// The changed paths the close stored (the script keeps at most 200).
    #[serde(default)]
    pub changed: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub v: u32,
    pub slug: String,
    pub goal: Option<String>,
    /// `(id, description, passes)`, in order.
    pub features: Vec<(String, String, bool)>,
    /// Every progress line the scripts appended for the core's writes, without the timestamp.
    pub progress: Vec<String>,
    pub checkpoints: u32,
    pub tasks: BTreeMap<String, LedgerTask>,
    /// `active`, `done` or `abandoned`.
    pub status: String,
    /// The sha256 of the report bytes the core sent (bhishma N1): another signed report has another hash.
    pub report: Option<String>,
    /// A write was started and its outcome isn't known (a timeout, or the app stopped mid-write): it can never match.
    pub pending: Option<String>,
    pub uncertain: Option<String>,
    /// A core write the script answered as already done by someone else: the record isn't only the core's.
    pub foreign: Option<String>,
}

/// Standard base64 (the alphabet the bridge allows), decoded. `None`: not base64.
fn unbase64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let body = s.trim_end_matches('=');
    if s.len() - body.len() > 2 {
        return None;
    }
    let (mut out, mut acc, mut bits) = (Vec::with_capacity(body.len() * 3 / 4), 0u32, 0u32);
    for c in body.bytes() {
        acc = (acc << 6) | u32::from(val(c)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// What the scripts show of a campaign, read back: `campaign.json`, `features.json`, `progress.md` (held bytes, after
/// the signature check) and the task graph (`vajra_tasks.py list --json`).
#[derive(Clone, Debug)]
pub struct ReadBack {
    pub campaign: Value,
    pub features: Value,
    pub progress: String,
    pub tasks: Value,
}

fn open(s: &str) -> bool {
    matches!(s, "pending" | "ready" | "open")
}

impl Ledger {
    pub fn new(slug: &str) -> Self {
        Ledger { v: 1, slug: slug.into(), status: "active".into(), ..Ledger::default() }
    }

    fn task(&mut self, id: &str) -> &mut LedgerTask {
        self.tasks.entry(id.to_string()).or_insert_with(|| LedgerTask { status: "open".into(), ..LedgerTask::default() })
    }

    /// The core's write `req` succeeded (exit 0) with no output to read: `applied` with none.
    pub fn apply(&mut self, req: &Request) {
        self.applied(req, "", None)
    }

    /// The core's write `req` succeeded (exit 0), the script printing `stdout`: what it changed. A read changes
    /// nothing. `evidence_sha256`: for a task close, the sha256 of the evidence bytes the core handed the script.
    pub fn applied(&mut self, req: &Request, stdout: &str, evidence_sha256: Option<&str>) {
        match req {
            Request::CampaignCreate { goal, features, .. } => {
                // `create` succeeds only on a campaign that didn't exist: whatever a saved ledger said is gone with it.
                *self = Ledger::new(&self.slug);
                self.goal = Some(goal.clone());
                self.features = features.iter().map(|f| (f.id.clone(), f.desc.clone(), false)).collect();
                self.progress.push(format!("created with {} feature(s)", features.len()));
            }
            Request::CampaignProgress { text, .. } => self.progress.push(text.clone()),
            Request::CampaignCheckpoint { label, .. } => {
                self.checkpoints += 1;
                self.progress.push(format!("checkpoint {:04}: {label}", self.checkpoints));
            }
            Request::CampaignPass { feature, evidence, .. } => {
                if let Some(f) = self.features.iter_mut().find(|f| &f.0 == feature) {
                    f.2 = true;
                }
                self.progress.push(format!("pass: {feature} (evidence: {evidence})"));
            }
            Request::CampaignClose { status, .. } => {
                let st = if *status == CloseStatus::Done { "done" } else { "abandoned" };
                self.status = st.into();
                self.progress.push(format!("closed: {st}"));
            }
            Request::CampaignReport { base64, .. } => {
                // The bridge refused anything but base64 before the script ran, so this decodes; if not, never matches.
                self.report = Some(unbase64(base64).map_or_else(|| "undecodable".into(), |b| crate::control::sha256::hex_digest(&b)));
            }
            Request::TasksAdd { id, title, owner, deps, files, acceptance, branch, .. } => {
                // The bridge refused anything but non-blank idents and one-line text within the script's caps, so the
                // script stores these as sent. A done task keeps its definition ("exists").
                if self.tasks.get(id).is_some_and(|t| t.status == "done") {
                    return;
                }
                let t = self.task(id);
                t.def = serde_json::json!({"title": title, "owner_agent": owner, "deps": deps, "files": files, "acceptance": acceptance});
                // A new task stores the branch (or ""); an existing one takes it only when one is given.
                if !branch.is_empty() {
                    t.branch = branch.clone();
                }
            }
            Request::TasksClaim { id, worker, worktree, .. } => {
                let t = self.task(id);
                // Both of the script's claim paths ("claimed" and "already claimed") record a given worktree.
                if let Some(w) = worktree {
                    t.worktree = Some(w.to_string_lossy().into_owned());
                }
                // The script's "already claimed": the same worker on a claimed or in-progress task changes nothing.
                if !(matches!(t.status.as_str(), "claimed" | "in_progress") && t.claimed_by.as_deref() == Some(worker)) {
                    t.status = "claimed".into();
                    t.claimed_by = Some(worker.clone());
                    t.attempts += 1;
                }
            }
            Request::TasksReport { id, status, summary, .. } => {
                let t = self.task(id);
                let s = status.as_str();
                let new = if matches!(s, "done" | "progress" | "in_progress") { "in_progress" } else { s };
                // A ready task reported in progress without a claim starts an attempt (the script's rule).
                if new == "in_progress" && open(&t.status) {
                    t.attempts += 1;
                }
                t.status = new.into();
                t.report = Some((s.into(), summary.clone().unwrap_or_default()));
            }
            Request::TasksClose { id, commit, branch, base_commit, changed, .. } => {
                // `already done` (exit 0 too) means someone else closed it, with evidence the core never handed over.
                let result = serde_json::from_str::<Value>(stdout).ok().and_then(|v| v["result"].as_str().map(str::to_string));
                match (result.as_deref(), evidence_sha256) {
                    (Some("closed"), Some(sha)) => {
                        let t = self.task(id);
                        t.status = "done".into();
                        if !branch.is_empty() {
                            t.branch = branch.clone();
                        }
                        // The bridge sent one-line paths within the script's cap; it keeps the first 200, blanks dropped.
                        let changed: Vec<String> = changed.iter().filter(|c| !c.trim().is_empty()).take(200).cloned().collect();
                        t.closed = Some(Closed { evidence_sha256: sha.into(), commit: commit.clone(), branch: branch.clone(), base_commit: base_commit.clone(), changed });
                    }
                    (r, _) => self.foreign = Some(format!("task {id} close answered {}", r.unwrap_or("nothing"))),
                }
            }
            _ => {}
        }
    }

    /// The campaign read back is exactly what the core's writes made. `Err`: the first difference.
    pub fn check(&self, rb: &ReadBack) -> Result<(), String> {
        if let Some(w) = self.uncertain.as_ref().or(self.pending.as_ref()) {
            return Err(format!("a record write's outcome is unknown ({w})"));
        }
        if let Some(w) = &self.foreign {
            return Err(format!("the record holds a write that isn't the core's ({w})"));
        }
        let c = &rb.campaign;
        if c["slug"] != self.slug.as_str() {
            return Err("the campaign's slug isn't the run's".into());
        }
        if self.goal.as_deref().is_none_or(|g| c["goal"] != g) {
            return Err("the campaign's goal isn't the one the core wrote".into());
        }
        if c["status"] != self.status.as_str() {
            return Err(format!("the campaign is {} but the core left it {}", c["status"].as_str().unwrap_or("?"), self.status));
        }
        if c.get("reportSha256").and_then(Value::as_str) != self.report.as_deref() {
            return Err("the campaign's report isn't the one the core wrote".into());
        }
        let feats = rb.features["features"].as_array().cloned().unwrap_or_default();
        let same = feats.len() == self.features.len()
            && feats.iter().zip(&self.features).all(|(x, (id, d, p))| x["id"] == id.as_str() && x["description"] == d.as_str() && x["passes"] == *p);
        if !same {
            return Err("the criteria or their pass states aren't the ones the core wrote".into());
        }
        let lines: Vec<&str> = rb
            .progress
            .lines()
            .filter_map(|l| l.strip_prefix("- "))
            .map(|l| l.split_once(' ').map_or("", |(_, rest)| rest))
            .collect();
        if lines != self.progress.iter().map(String::as_str).collect::<Vec<_>>() {
            let at = lines.iter().zip(&self.progress).position(|(a, b)| a != b).unwrap_or(lines.len().min(self.progress.len()));
            return Err(format!("progress line {} isn't one the core wrote ({} lines, the core wrote {})", at + 1, lines.len(), self.progress.len()));
        }
        let tasks = rb.tasks["tasks"].as_array().cloned().unwrap_or_default();
        if tasks.len() != self.tasks.len() {
            return Err(format!("the task graph has {} tasks, the core added {}", tasks.len(), self.tasks.len()));
        }
        // Each task the core added, exactly once: a duplicate standing in for a missing task never matches.
        let mut seen = std::collections::BTreeSet::new();
        for t in &tasks {
            let id = t["id"].as_str().unwrap_or_default();
            if !seen.insert(id) {
                return Err(format!("task {id} is in the task graph more than once"));
            }
            let Some(want) = self.tasks.get(id) else { return Err(format!("task {id} isn't one the core added")) };
            let st = t["status"].as_str().unwrap_or_default();
            let status_ok = if want.status == "open" { open(st) } else { st == want.status };
            let report = t.get("report").filter(|r| !r.is_null()).map(|r| (r["status"].as_str().unwrap_or_default().to_string(), r["summary"].as_str().unwrap_or_default().to_string()));
            let by_ok = want.claimed_by.as_ref().is_none_or(|w| t["claimed_by"] == w.as_str()) && (want.claimed_by.is_some() || t.get("claimed_by").is_none_or(Value::is_null));
            // Done: exactly the core's close (evidence hash, commit, branch, base). Not done: no close at all.
            let stored = |k: &str, want: &str| if want.is_empty() { t.get(k).is_none_or(Value::is_null) } else { t[k] == want };
            let none = |k: &str| t.get(k).is_none_or(Value::is_null);
            let evidence_ok = match &want.closed {
                Some(c) => {
                    want.status == "done"
                        && t["evidence"]["sha256"] == c.evidence_sha256.as_str()
                        && stored("commit", &c.commit)
                        && stored("base_commit", &c.base_commit)
                        && if c.changed.is_empty() { none("changed") } else { t["changed"] == serde_json::json!(c.changed) }
                }
                None => want.status != "done" && none("evidence") && none("commit") && none("base_commit") && none("changed"),
            };
            // The branch and worktree the core's add, claims and close left (codex on #190); the script keeps "" for none.
            let text = |k: &str| t.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let place_ok = text("branch") == want.branch && text("worktree") == want.worktree.clone().unwrap_or_default();
            let def_ok = ["title", "owner_agent", "deps", "files", "acceptance"].iter().all(|k| t[*k] == want.def[*k]);
            if !status_ok || t["attempts"] != want.attempts || report != want.report || !by_ok || !evidence_ok || !def_ok || !place_ok {
                return Err(format!("task {id} isn't in the state the core's writes left it"));
            }
        }
        if let Some(id) = self.tasks.keys().find(|k| !seen.contains(k.as_str())) {
            return Err(format!("task {id} is missing from the task graph"));
        }
        Ok(())
    }
}
