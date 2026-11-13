//! Control service types: actors, errors, the action model (kind and state), and the ask and decision records.

use super::*;

/// How long a queued message waits for the worker's next tool call.
pub const DEFAULT_TTL: Duration = Duration::from_secs(30 * 60);
/// The most ended core workers' nonces the redactor keeps carrying (oldest out first).
pub const RETIRED_NONCES: usize = 4096;
/// An inflight claim with no ack after this is "delivery unknown" (5.4 step 6).
/// Cap for files a worker can write in the control dir (acks, stops).
pub const MAX_ACK_BYTES: u64 = 16 * 1024;
pub const INFLIGHT_UNKNOWN: Duration = Duration::from_secs(60);
/// A queued action whose settlement check fails this many ticks in a row, over at least `CHECK_FAIL_AFTER`, is
/// settled as `delivery_unknown` (`scan_failed`) instead of staying pending, silently, for as long as the failure lasts.
pub const CHECK_FAIL_TICKS: u32 = 5;
pub const CHECK_FAIL_AFTER: Duration = Duration::from_secs(30);
pub const MAX_TEXT: usize = 4000;
pub const MAX_REASON: usize = 500;
pub const MAX_WORKER: usize = 128;
pub const MAX_LINES: u64 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    /// The human in the app UI.
    Human,
    /// The Lead chat session, through MCP and the socket.
    Lead,
}

impl Actor {
    /// The only place a source is chosen.
    pub fn source(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Lead => "lead-via-desktop",
        }
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Lead => "lead",
        }
    }
    pub(super) fn via(self) -> &'static str {
        match self {
            Self::Human => "ui",
            Self::Lead => "mcp",
        }
    }
}

#[derive(Debug)]
pub enum ControlError {
    WorkerNotFound,
    WorkerFinished,
    WorkerAmbiguous,
    ControlDisabled,
    NotPermitted,
    NotStopped,
    /// A Lead bound to a run called a tool while that run is not the current control session.
    RunNotCurrent,
    /// A Director-General's tool call carried an older conversation generation than the project's current one: the
    /// process belongs to a conversation that was replaced, so it may not see or steer anything. Same wire code as a
    /// stale worker attempt (`StaleGeneration`): the caller refreshes either way.
    StaleLeadGeneration,
    ActionNotFound,
    /// A stop that named the attempt it saw (`expect_generation`) found a different one at the moment of the stop.
    StaleGeneration(u64),
    /// The Director-General's project is archived, or gone from the store (FG-02c): it may not see or steer anything.
    ProjectArchived,
    ProjectMissing,
    /// The app's projects folder was refused at open (#106 review), so no project can be checked. The detail says what is
    /// wrong; the fix and path stay with the app's own UI (a tool result never carries a path).
    ProjectsUnavailable(String),
    UnknownTool,
    InvalidArgs(String),
    /// Seats the launch checks refused: one `{seat, code, detail}` for each, all in one `invalid_args` answer.
    Refusals(Vec<Value>),
    /// Message or Redirect to a core worker whose harness has no channel for it (16.7): why, from its profile.
    NotSupportedByHarness(&'static str),
    Io(io::Error),
}

/// A worker the core runs itself (6c) is named `core:<ticket>`.
pub const CORE_PREFIX: &str = "core:";

/// The app's worker runtime, answering for `core:<ticket>` workers (16.7). Their controls come from the harness profile.
pub trait CoreWorkers: Send + Sync {
    /// The ticket's run and round while it can still be stopped: `WorkerNotFound`, or `WorkerFinished` once it ended.
    fn live(&self, ticket: &str) -> Result<(String, u32)>;
    /// Where a Message or Redirect for the ticket goes: its harness session id, keyed by which the control hook delivers
    /// to a top-level session (Claude, design 16.7). Otherwise why its harness takes none (Codex), or none yet.
    fn message_to(&self, ticket: &str) -> std::result::Result<String, &'static str>;
    /// The profile's stop. True when it ended the ticket or asked it to end (its end is then reported through
    /// `Service::core_ticket_ended`); false when there was nothing left to stop.
    fn stop(&self, ticket: &str) -> bool;
}

impl ControlError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::WorkerNotFound => "worker_not_found",
            Self::WorkerFinished => "worker_finished",
            Self::WorkerAmbiguous => "worker_ambiguous",
            Self::ControlDisabled => "control_disabled",
            Self::NotPermitted => "not_permitted",
            Self::NotStopped => "not_stopped",
            Self::RunNotCurrent => "run_not_current",
            Self::StaleLeadGeneration => "stale_generation",
            Self::ActionNotFound => "action_not_found",
            Self::StaleGeneration(_) => "stale_generation",
            Self::ProjectArchived => "project_archived",
            Self::ProjectMissing => "project_missing",
            Self::ProjectsUnavailable(_) => "projects_unavailable",
            Self::UnknownTool => "unknown_tool",
            Self::InvalidArgs(_) | Self::Refusals(_) => "invalid_args",
            Self::NotSupportedByHarness(_) => "not_supported_by_harness",
            Self::Io(_) => "io_error",
        }
    }
    /// The error as a tool result. I/O detail stays out (it can carry paths).
    pub fn to_json(&self) -> Value {
        match self {
            Self::InvalidArgs(m) | Self::ProjectsUnavailable(m) => json!({"error": self.code(), "detail": m}),
            Self::Refusals(r) => json!({"error": self.code(), "detail": "some seats cannot be launched", "refusals": r}),
            Self::StaleGeneration(g) => json!({"error": self.code(), "detail": "the worker changed attempt since you looked; refresh and try again", "current_generation": g}),
            Self::NotSupportedByHarness(why) => json!({"error": self.code(), "detail": why}),
            _ => json!({"error": self.code()}),
        }
    }
}

impl From<io::Error> for ControlError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

pub(super) type Result<T> = std::result::Result<T, ControlError>;

/// What the app knows about one worker: the crew record plus identity the
/// hook can match on. `title` and `lines` are agent text (untrusted).
#[derive(Clone, Debug)]
pub struct WorkerInfo {
    pub worker: Worker,
    pub task_id: Option<String>,
    pub title: Option<String>,
    pub round: Option<u32>,
    pub live: bool,
    pub last_activity: Option<String>,
    /// The subagent transcript cwd (its worktree).
    pub cwd: Option<String>,
    /// Recent transcript lines, newest last.
    pub lines: Vec<String>,
}

impl WorkerInfo {
    /// The app key: the agent id, else the cwd.
    pub fn key(&self) -> &str {
        if self.worker.agent_id.is_empty() { self.cwd.as_deref().unwrap_or("") } else { &self.worker.agent_id }
    }
    pub(super) fn finished(&self) -> bool {
        !self.live || matches!(self.worker.status, Status::Done | Status::Failed)
    }
}

/// `task.<id>.r<n>`, the only steerable step label.
pub(super) fn is_task_label(label: &str) -> bool {
    let Some(rest) = label.strip_prefix("task.") else { return false };
    let Some((id, round)) = rest.rsplit_once(".r") else { return false };
    !id.is_empty() && !id.contains('.') && !round.is_empty() && round.bytes().all(|b| b.is_ascii_digit())
}

/// `{untrusted:true, source:'agent-output', text}`
pub fn untrusted(text: &str) -> Value {
    json!({"untrusted": true, "source": "agent-output", "text": text})
}

#[derive(Clone, Debug)]
pub(super) struct Action {
    pub(super) id: String,
    pub(super) kind: Kind,
    pub(super) state: State,
    pub(super) actor: Actor,
    pub(super) session_id: String,
    pub(super) run_id: String,
    pub(super) worker: String,
    pub(super) engine_id: String,
    pub(super) task_id: Option<String>,
    pub(super) created: SystemTime,
    pub(super) reason: Option<String>,
    pub(super) stop_match: Option<Value>,
    pub(super) inflight_seen: Option<SystemTime>,
    /// The run of consecutive failed settlement checks, if one is going on.
    pub(super) check_failed: Option<CheckFailure>,
}

/// Consecutive failures of `check` for one action: when the run began, how long it is, and the last error's code.
#[derive(Clone, Debug)]
pub(super) struct CheckFailure {
    pub(super) first: SystemTime,
    pub(super) count: u32,
    pub(super) errno: i64,
}

/// What an action does. `as_str` is the ledger and JSON name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    List,
    Inspect,
    Message,
    Redirect,
    Stop,
    Resume,
    /// ADR-13: the human asked the Lead a question. Always actor human, via ui; it reaches no worker.
    Ask,
    /// ADR-14: the human approved or rejected a plan item. Always actor human, via ui.
    Approve,
    Reject,
    /// ADR-15 2.4: a Director-General proposed a run. Always actor Lead, via mcp; a human starts or dismisses it.
    Propose,
}

/// Where an action is. `as_str` is the ledger and JSON name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Requested,
    AwaitingHumanConfirm,
    Queued,
    Delivered,
    NotDelivered,
    DeliveryUnknown,
    Refused,
    Done,
    /// An ask that did not complete (the `reason` says why).
    Failed,
}

impl Kind {
    pub(super) const ALL: [Kind; 10] = [Kind::List, Kind::Inspect, Kind::Message, Kind::Redirect, Kind::Stop, Kind::Resume, Kind::Ask, Kind::Approve, Kind::Reject, Kind::Propose];
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Kind::List => "list",
            Kind::Inspect => "inspect",
            Kind::Message => "message",
            Kind::Redirect => "redirect",
            Kind::Stop => "stop",
            Kind::Resume => "resume",
            Kind::Ask => "ask",
            Kind::Approve => "approve",
            Kind::Reject => "reject",
            Kind::Propose => "propose",
        }
    }
    pub(super) fn parse(k: &str) -> Option<Kind> {
        Self::ALL.into_iter().find(|x| x.as_str() == k)
    }
    /// Message and redirect go through the queue and can expire.
    pub(super) fn queued_text(self) -> bool {
        matches!(self, Kind::Message | Kind::Redirect)
    }
}

impl State {
    pub(super) const ALL: [State; 9] = [State::Requested, State::AwaitingHumanConfirm, State::Queued, State::Delivered, State::NotDelivered, State::DeliveryUnknown, State::Refused, State::Done, State::Failed];
    pub(super) fn as_str(self) -> &'static str {
        match self {
            State::Requested => "requested",
            State::AwaitingHumanConfirm => "awaiting_human_confirm",
            State::Queued => "queued",
            State::Delivered => "delivered",
            State::NotDelivered => "not_delivered",
            State::DeliveryUnknown => "delivery_unknown",
            State::Refused => "refused",
            State::Done => "done",
            State::Failed => "failed",
        }
    }
    /// An unknown ledger state reads as final.
    pub(super) fn parse(s: &str) -> State {
        Self::ALL.into_iter().find(|x| x.as_str() == s).unwrap_or(State::Done)
    }
}


/// Where an ask is (ADR-13 Decision 6). `queued`: stdin written; `delivered`: the `init` check passed; `done`: the
/// `result` event arrived; `failed`: anything else, with a reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskState {
    Queued,
    Delivered,
    Done,
    Failed,
}

impl AskState {
    pub(super) fn state(self) -> State {
        match self {
            Self::Queued => State::Queued,
            Self::Delivered => State::Delivered,
            Self::Done => State::Done,
            Self::Failed => State::Failed,
        }
    }
}

/// One ledger line for an ask. `text` (already framed-sanitised, nonce redacted) goes on the `queued` line only.
#[derive(Clone, Copy, Debug)]
pub struct AskRecord<'a> {
    pub ask_id: &'a str,
    pub run_id: &'a str,
    /// The run session the question is about (the Lead itself has `lead_session_id`).
    pub session_id: &'a str,
    pub lead_session_id: &'a str,
    pub state: AskState,
    pub text: Option<&'a str>,
    pub reason: Option<&'a str>,
    /// The Director-General's project and conversation the ask went to (FG-02c); `None` only in the service's tests.
    pub project: Option<(&'a str, u32)>,
    /// The ended turn's token receipt (`vajra.token_receipt/1`), written on the `done` line, or on the `failed` line of a turn that ran and errored.
    pub tokens: Option<&'a serde_json::Value>,
}

/// A human decision on a plan item (ADR-14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Approve,
    Reject,
}

/// Where a decision is. `Requested` is written before anything is delivered; `Final` is `delivered` for an approve
/// (the fixed turn was typed or the resumed session was started) and `done` for a reject (the progress line was signed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionState {
    Requested,
    Final,
    Failed,
}

/// One ledger line for a decision. `reason` is the reject reason (cleaned, bounded) or a failure note.
#[derive(Clone, Copy, Debug)]
pub struct DecisionRecord<'a> {
    pub decision: Decision,
    /// Names the action across its lines; `None` on the first line, which mints it.
    pub action_id: Option<&'a str>,
    pub item: &'a str,
    pub run_id: &'a str,
    pub session_id: &'a str,
    pub campaign: &'a str,
    pub plan_sha256: Option<&'a str>,
    pub state: DecisionState,
    pub reason: Option<&'a str>,
}

pub(super) fn str_field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

pub(super) fn new_action_id(now: SystemTime) -> io::Result<String> {
    // Time-ordered like a ULID: 12 hex of ms, then 64 random bits.
    Ok(format!("a{:012x}{}", time::unix_ms(now), nonce::random_hex(8)?))
}
