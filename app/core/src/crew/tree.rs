//! Crew tree derivation (design.md 4.2):
//! Lead > Run > [<Role> · <tid> > round N], Verification rN > passes,
//! Write-back, Run steps, Unassigned. Task status comes from `tasks.json`;
//! with no signal a node is "unknown", never an invented idle.

use std::collections::HashSet;

use serde_json::Value;

use super::names::{title_case, ui_name};
use super::record::{JournalEntry, Meta};
use super::tailer::Transcript;
use crate::providers::effective::ProviderTag;
use crate::vajra::state::Reader;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Ready,
    Claimed,
    InProgress,
    Done,
    Blocked,
    Question,
    Failed,
    Unknown,
}

impl Status {
    pub fn parse(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "ready" => Self::Ready,
            "claimed" => Self::Claimed,
            "in_progress" => Self::InProgress,
            // `closed` is what the task graph writes once verification passed.
            "done" | "closed" => Self::Done,
            "blocked" => Self::Blocked,
            "question" => Self::Question,
            "failed" => Self::Failed,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Claimed => "claimed",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Question => "question",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Lead,
    Run,
    Task,
    Round,
    Verification,
    WriteBack,
    RunSteps,
    Unassigned,
    Worker,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub title: String,
    /// Engine id, for the inspector's "engine id" field only.
    pub engine_id: Option<String>,
    pub status: Status,
    pub overlays: Vec<&'static str>,
    /// What the worker ran on, once it reported it (`None` while not reported).
    pub provider: Option<ProviderTag>,
    pub children: Vec<Node>,
}

impl Node {
    fn group(id: String, kind: NodeKind, title: String) -> Self {
        Self { id, kind, title, engine_id: None, status: Status::Unknown, overlays: Vec::new(), provider: None, children: Vec::new() }
    }
}

/// A subagent seen on disk.
#[derive(Clone, Debug)]
pub struct Worker {
    pub agent_id: String,
    pub engine_id: String,
    /// Workflow label (`task.t1.r1`, `verify.r1`, `kb.search`), or empty.
    pub label: String,
    pub prompt_first_line: Option<String>,
    pub branch: Option<String>,
    pub status: Status,
    /// From the transcript's own model field; `None` until the worker reports one.
    pub provider: Option<ProviderTag>,
}

/// Overlay signals keyed by agent id.
#[derive(Clone, Debug, Default)]
pub struct Signals {
    pub stopped: HashSet<String>,
    pub message_pending: HashSet<String>,
    pub activity: HashSet<String>,
}

pub struct CrewInput<'a> {
    pub run_id: &'a str,
    /// Lead chat session status, `None` when it is not open.
    pub lead: Option<Status>,
    pub run_status: Status,
    pub tasks: Option<&'a Value>,
    pub workers: &'a [Worker],
    pub signals: &'a Signals,
}

/// Build a Worker from its meta file, its journal entry and its transcript.
/// Status: journal result -> done, started -> in_progress, else unknown.
pub fn worker_from(agent_id: &str, meta: &Meta, journal: Option<&JournalEntry>, tr: Option<&Transcript>) -> Worker {
    Worker {
        agent_id: agent_id.to_string(),
        engine_id: meta.agent_type.clone(),
        label: journal.map_or_else(|| meta.description.clone(), |j| j.label.clone()),
        prompt_first_line: tr.and_then(|t| t.prompt_first_line()).map(str::to_string),
        branch: tr.and_then(|t| t.git_branch()).map(str::to_string),
        status: match journal {
            Some(j) if j.finished => Status::Done,
            Some(_) => Status::InProgress,
            None => Status::Unknown,
        },
        provider: tr.and_then(|t| t.model()).map(|model| ProviderTag {
            account: "claude".into(),
            family: "claude".into(),
            model: model.into(),
            billing: "subscription".into(),
        }),
    }
}

enum Place {
    Task(String, u32),
    Verify(u32, String),
    WriteBack,
    Step,
    Unassigned,
}

fn round(s: &str) -> Option<u32> {
    s.strip_prefix('r')?.parse().ok()
}

fn place(w: &Worker, run_id: &str) -> Place {
    let parts: Vec<&str> = w.label.split('.').collect();
    match parts.as_slice() {
        ["task", tid, r] if round(r).is_some() => return Place::Task((*tid).to_string(), round(r).unwrap_or(1)),
        ["verify", r] | ["verify", r, _] if round(r).is_some() => {
            return Place::Verify(round(r).unwrap_or(1), ui_name(&w.engine_id));
        }
        ["review", agent, r] if round(r).is_some() => {
            return Place::Verify(round(r).unwrap_or(1), format!("{} review", ui_name(agent)));
        }
        ["writeback"] => return Place::WriteBack,
        _ if !w.label.is_empty() => return Place::Step,
        _ => {}
    }
    let marker = format!("task graph id {run_id}.");
    if let Some(tid) = w.prompt_first_line.as_deref().and_then(|p| p.split_once(&marker)).and_then(|(_, rest)| {
        let tid: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        (!tid.is_empty()).then_some(tid)
    }) {
        return Place::Task(tid, 1);
    }
    let prefix = format!("vajra/{run_id}/");
    if let Some(rest) = w.branch.as_deref().and_then(|b| b.strip_prefix(&prefix)).filter(|r| !r.is_empty()) {
        return match rest.rsplit_once('-') {
            Some((tid, r)) if round(r).is_some() => Place::Task(tid.to_string(), round(r).unwrap_or(1)),
            _ => Place::Task(rest.to_string(), 1),
        };
    }
    Place::Unassigned
}

fn worker_node(w: &Worker, title: String, kind: NodeKind, signals: &Signals) -> Node {
    let mut overlays = Vec::new();
    if signals.stopped.contains(&w.agent_id) {
        overlays.push("stopped by the human");
    }
    if signals.message_pending.contains(&w.agent_id) {
        overlays.push("message pending");
    }
    if signals.activity.contains(&w.agent_id) {
        overlays.push("activity");
    }
    Node {
        id: format!("agent:{}", w.agent_id),
        kind,
        title,
        engine_id: Some(w.engine_id.clone()),
        status: w.status,
        overlays,
        provider: w.provider.clone(),
        children: Vec::new(),
    }
}

/// Group status: in progress if any child is, done if all are, else unknown.
fn aggregate(children: &[Node]) -> Status {
    if children.iter().any(|c| c.status == Status::InProgress) {
        Status::InProgress
    } else if !children.is_empty() && children.iter().all(|c| c.status == Status::Done) {
        Status::Done
    } else {
        Status::Unknown
    }
}

pub fn build(input: &CrewInput) -> Node {
    let mut tasks: Vec<(Node, Vec<(u32, Node)>)> = Vec::new();
    for t in input.tasks.and_then(|v| v.get("tasks")).and_then(Value::as_array).into_iter().flatten() {
        let Some(gid) = t.get("id").and_then(Value::as_str) else { continue };
        // Graph ids are `<run_id>.t1`; worker labels and branches carry the run-local `t1`.
        let tid = gid.strip_prefix(input.run_id).and_then(|r| r.strip_prefix('.')).unwrap_or(gid);
        let owner = t.get("owner_agent").and_then(Value::as_str).unwrap_or("");
        let mut n = Node::group(format!("task:{tid}"), NodeKind::Task, format!("{} · {tid}", ui_name(owner)));
        n.engine_id = Some(owner.to_string());
        n.status = t.get("status").and_then(Value::as_str).map_or(Status::Unknown, Status::parse);
        tasks.push((n, Vec::new()));
    }

    let mut verify: Vec<(u32, Node)> = Vec::new();
    let mut writeback = Node::group("writeback".into(), NodeKind::WriteBack, "Write-back".into());
    let mut steps = Node::group("run-steps".into(), NodeKind::RunSteps, "Run steps".into());
    let mut unassigned = Node::group("unassigned".into(), NodeKind::Unassigned, "Unassigned".into());

    for w in input.workers {
        match place(w, input.run_id) {
            Place::Task(tid, r) => {
                let i = match tasks.iter().position(|(n, _)| n.id == format!("task:{tid}")) {
                    Some(i) => i,
                    None => {
                        // Seen only in a label: no task status to show.
                        let mut n = Node::group(
                            format!("task:{tid}"),
                            NodeKind::Task,
                            format!("{} · {tid}", ui_name(&w.engine_id)),
                        );
                        n.engine_id = Some(w.engine_id.clone());
                        tasks.push((n, Vec::new()));
                        tasks.len() - 1
                    }
                };
                tasks[i].1.push((r, worker_node(w, format!("round {r}"), NodeKind::Round, input.signals)));
            }
            Place::Verify(r, title) => {
                let i = match verify.iter().position(|(n, _)| *n == r) {
                    Some(i) => i,
                    None => {
                        let g = Node::group(format!("verification:r{r}"), NodeKind::Verification, format!("Verification r{r}"));
                        verify.push((r, g));
                        verify.len() - 1
                    }
                };
                verify[i].1.children.push(worker_node(w, title, NodeKind::Worker, input.signals));
            }
            Place::WriteBack => {
                writeback.children.push(worker_node(w, ui_name(&w.engine_id), NodeKind::Worker, input.signals));
            }
            Place::Step => steps.children.push(worker_node(w, title_case(&w.label), NodeKind::Worker, input.signals)),
            Place::Unassigned => {
                unassigned.children.push(worker_node(w, ui_name(&w.engine_id), NodeKind::Worker, input.signals));
            }
        }
    }

    let mut run = Node::group(format!("run:{}", input.run_id), NodeKind::Run, format!("Run {}", input.run_id));
    run.status = input.run_status;
    for (mut n, mut rounds) in tasks {
        rounds.sort_by_key(|(r, _)| *r); // stable: same round keeps arrival order
        n.children = rounds.into_iter().map(|(_, n)| n).collect();
        run.children.push(n);
    }
    verify.sort_by_key(|(r, _)| *r);
    for (_, mut v) in verify {
        v.status = aggregate(&v.children);
        run.children.push(v);
    }
    for mut g in [writeback, steps, unassigned] {
        if !g.children.is_empty() {
            g.status = aggregate(&g.children);
            run.children.push(g);
        }
    }

    let mut lead = Node::group("lead".into(), NodeKind::Lead, ui_name("krishna"));
    lead.status = input.lead.unwrap_or(Status::Unknown);
    lead.children.push(run);
    lead
}

/// Re-read `tasks.json` (last good snapshot on a parse error) and rebuild.
/// The UI polls this at 2 Hz, so a tasks.json change shows within 2 s.
pub fn snapshot(
    reader: &mut Reader,
    slug: &str,
    run_id: &str,
    lead: Option<Status>,
    run_status: Status,
    workers: &[Worker],
    signals: &Signals,
) -> Node {
    let snap = reader.tasks(slug);
    build(&CrewInput { run_id, lead, run_status, tasks: snap.value.as_ref(), workers, signals })
}
