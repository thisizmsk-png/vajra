//! Run proposals (ADR-15 2.4, owner decision D4 option A): a project's Director-General proposes a run with
//! `propose_run`, and **nothing runs until a human starts it** in the app.
//!
//! - Only a project-bound call can propose (`call_tool_for_project`). An ADR-13 run-bound or unbound call gets
//!   `not_permitted`.
//! - The proposal is one `propose` ledger line under the guarded control dir, with a sha256 over exactly what will run
//!   (project, ask, base, rounds). Starting it requires that sha back, so what the human approved is what runs.
//! - The ask must be plain text: a control or invisible character is refused, so what the inbox shows is byte for
//!   byte what `start_run` gets. The nonce is redacted, as for every Lead text.
//! - Start and dismiss are `approve` / `reject` lines on the item `proposal:<action_id>`, so `decided` closes it the
//!   same way it closes a plan: a `requested` line with no outcome counts as decided (never started twice).

use std::collections::{HashMap, HashSet};

use super::seats::SeatArgs;
use super::*;
use crate::control::lead::sanitise;

/// Open proposals per project. A Director-General that keeps proposing is refused, not queued without bound.
pub const MAX_OPEN_PROPOSALS: usize = 20;
pub const DEFAULT_PROPOSAL_ROUNDS: u64 = 2;
pub const DEFAULT_PROPOSAL_BASE: &str = "HEAD";
const ITEM_PREFIX: &str = "proposal:";

/// One proposal no decision has closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// The inbox item id, `proposal:<action_id>`.
    pub item: String,
    pub action_id: String,
    pub project: String,
    pub generation: u32,
    pub ask: String,
    pub base: String,
    pub rounds: u32,
    pub proposal_sha256: String,
    /// The seats of a proposal with worker picks (`vajra.run_proposal/2`); `None` for a v1 proposal.
    pub seats: Option<Seats>,
    /// The warning the card shows when the ask or a seat's task names security work and no seat is marked.
    pub security_warning: Option<String>,
    /// The ledger time (ISO, ms).
    pub ts: String,
}

/// What a start or a dismiss of a proposal writes.
pub struct ProposalDecision<'a> {
    pub decision: Decision,
    pub action_id: Option<&'a str>,
    pub item: &'a str,
    pub proposal_sha256: &'a str,
    pub state: DecisionState,
    /// The run a start launched (final line only).
    pub started_run: Option<&'a str>,
    pub reason: Option<&'a str>,
    /// The seats a start was checked and recorded with (`seats.json`) and the sha256 of that file; on the `requested` line.
    pub seats: Option<(&'a Value, &'a str)>,
}

/// The hash a start must name: everything that decides what runs, and where.
pub fn proposal_sha(project: &str, ask: &str, base: &str, rounds: u32) -> String {
    let v = json!({"schema": "vajra.run_proposal/1", "project": project, "ask": ask, "base": base, "rounds": rounds});
    sha256::hex_digest(&serde_json::to_vec(&v).unwrap_or_default())
}

/// `proposal:a<12 hex><16 hex>`, the shape `new_action_id` makes.
pub fn valid_proposal_item(item: &str) -> bool {
    item.strip_prefix(ITEM_PREFIX).is_some_and(|a| a.len() == 29 && a.starts_with('a') && a.bytes().skip(1).all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// A base the run form would accept: the same rule as `start_run`'s, so a proposal never names one it refuses.
fn valid_base(s: &str) -> bool {
    !s.is_empty() && s.len() <= 255 && !s.starts_with('-') && !s.contains("..") && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
}

impl Service {
    pub(super) fn propose_run(&mut self, ask: &str, base: Option<&str>, rounds: Option<u64>, seat_args: &SeatArgs<'_>) -> Result<Value> {
        let Some((project, generation)) = self.bound_project.clone() else {
            return Err(ControlError::NotPermitted);
        };
        let ask = self.text_arg(ask, MAX_TEXT, "ask", false)?;
        if ask.trim().is_empty() {
            return Err(ControlError::InvalidArgs("ask must not be blank".into()));
        }
        if sanitise(&ask, &Redactor::default()) != ask {
            return Err(ControlError::InvalidArgs("ask must be plain text (no control or invisible characters)".into()));
        }
        let base = base.unwrap_or(DEFAULT_PROPOSAL_BASE);
        if !valid_base(base) {
            return Err(ControlError::InvalidArgs("base must be a branch name or commit".into()));
        }
        let rounds = rounds.unwrap_or(DEFAULT_PROPOSAL_ROUNDS);
        if !(1..=3).contains(&rounds) {
            return Err(ControlError::InvalidArgs("rounds must be 1 to 3".into()));
        }
        let rounds = rounds as u32;
        let seats = if seat_args.any() { Some(self.build_seats(&project, &ask, seat_args)?) } else { None };
        if self.open_proposals(Some(&project))?.len() >= MAX_OPEN_PROPOSALS {
            return Err(ControlError::InvalidArgs(format!("{MAX_OPEN_PROPOSALS} proposals are already waiting for the human; wait for them to be started or dismissed")));
        }
        let now = SystemTime::now();
        let a = self.new_action(Kind::Propose, Actor::Lead, None, now)?;
        let item = format!("{ITEM_PREFIX}{}", a.id);
        let (sha, seats) = match &seats {
            Some(s) => (proposal_sha_v2(&project, &ask, base, rounds, s), Some(s)),
            None => (proposal_sha(&project, &ask, base, rounds), None),
        };
        let mut extra = json!({"item": item, "project": project, "generation": generation, "ask": ask, "base": base, "rounds": rounds, "proposal_sha256": sha});
        if let Some(s) = seats {
            extra["seats"] = serde_json::to_value(s).unwrap_or_default();
        }
        self.record(&a, State::AwaitingHumanConfirm, extra, now)?;
        Ok(json!({"action_id": a.id, "item": item, "state": "awaiting_human", "detail": "Proposed. Nothing runs until the human starts it in the app."}))
    }

    /// The proposals no start or dismiss has closed, oldest first, for one project or all. An unreadable ledger is an
    /// error (the caller fails closed), never "no proposals".
    pub fn open_proposals(&self, project: Option<&str>) -> io::Result<Vec<Proposal>> {
        let records = Ledger::read_all(&self.control_root.join("ledger.jsonl"))?;
        // The newest state of each decision action, by item: an item is closed by any action not `failed`.
        let mut decisions: HashMap<&str, (&str, &str)> = HashMap::new();
        for r in records.iter().filter(|r| matches!(str_field(r, "kind"), "approve" | "reject") && str_field(r, "item").starts_with(ITEM_PREFIX)) {
            decisions.insert(str_field(r, "action_id"), (str_field(r, "item"), str_field(r, "state")));
        }
        let closed: HashSet<&str> = decisions.values().filter(|(_, st)| *st != "failed").map(|(i, _)| *i).collect();
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for r in records.iter().filter(|r| str_field(r, "kind") == "propose") {
            let item = str_field(r, "item");
            if !valid_proposal_item(item) || closed.contains(item) || !seen.insert(item.to_string()) {
                continue;
            }
            if project.is_some_and(|p| p != str_field(r, "project")) {
                continue;
            }
            let num = |k: &str| r.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
            let (Some(generation), Some(rounds)) = (num("generation"), num("rounds")) else { continue };
            // A proposal line with `seats` that cannot be read back is skipped: it can be neither shown nor started.
            let seats = match r.get("seats") {
                None => None,
                Some(v) => match serde_json::from_value::<Seats>(v.clone()) {
                    Ok(s) => Some(s),
                    Err(_) => continue,
                },
            };
            out.push(Proposal {
                item: item.to_string(),
                action_id: str_field(r, "action_id").to_string(),
                project: str_field(r, "project").to_string(),
                generation,
                ask: str_field(r, "ask").to_string(),
                base: str_field(r, "base").to_string(),
                rounds,
                proposal_sha256: str_field(r, "proposal_sha256").to_string(),
                seats: seats.clone(),
                security_warning: seats.as_ref().and_then(|s| security_warning(str_field(r, "ask"), s)).map(str::to_string),
                ts: str_field(r, "ts").to_string(),
            });
        }
        Ok(out)
    }

    /// Append one start or dismiss line for a proposal and return the action id. The final line also appends the
    /// `audit.log` line (`approve id=<action_id> item=<item>` or `reject ...`) that reconcile joins on.
    pub fn record_proposal_decision(&mut self, d: &ProposalDecision<'_>) -> Result<String> {
        if !valid_proposal_item(d.item) {
            return Err(ControlError::InvalidArgs("item".into()));
        }
        let now = SystemTime::now();
        let kind = if d.decision == Decision::Approve { Kind::Approve } else { Kind::Reject };
        let mut a = self.new_action(kind, Actor::Human, None, now)?;
        if let Some(id) = d.action_id {
            a.id = id.to_string();
        }
        a.session_id.clear();
        a.run_id.clear();
        a.reason = d.reason.map(str::to_string);
        let mut extra = json!({"item": d.item, "proposal_sha256": d.proposal_sha256});
        if let Some(run) = d.started_run {
            extra["started_run_id"] = json!(run);
        }
        if let Some((seats, sha)) = d.seats {
            extra["seats"] = seats.clone();
            extra["seats_sha256"] = json!(sha);
        }
        let (state, verb) = match (d.state, d.decision) {
            (DecisionState::Requested, _) => (State::Requested, ""),
            (DecisionState::Failed, _) => (State::Failed, ""),
            (DecisionState::Final, Decision::Approve) => (State::Delivered, "approve"),
            (DecisionState::Final, Decision::Reject) => (State::Done, "reject"),
        };
        self.record(&a, state, extra, now)?;
        if !verb.is_empty() && audit::append_line(&self.audit_log, "ALLOW", &format!("{verb} id={} item={}", a.id, d.item), now).is_err() {
            self.log("control_audit_failed", &[]);
        }
        Ok(a.id)
    }
}
