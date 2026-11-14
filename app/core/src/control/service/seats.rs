//! Proposal seats (ADR-16 16.1): a lead seat and worker seats, validated, completed with the required checker seats,
//! and hashed (`vajra.run_proposal/2`). A proposal with no `workers` keeps today's v1 hash and path untouched.
//!
//! The core derives each seat's `source` (`role_default` or `dg`) once, at proposal. It is stored and hashed from the
//! stored value, and start never derives it again. A Director-General that sends `source` is refused.

use serde::{Deserialize, Serialize};

use super::*;
use crate::control::lead::sanitise;
use crate::crew::roles::{self, Kind};
use crate::preferences::{Agents, Choice};
use crate::providers::launch::Refused;

pub const MAX_WORKERS: usize = 16;
pub const DEFAULT_MAX_IN_FLIGHT: u32 = 4;
pub const SECURITY_WARNING: &str = "If this run plans security work, it will wait for you to pick a worker for it.";
pub const SOURCE_ROLE_DEFAULT: &str = "role_default";
pub const SOURCE_DG: &str = "dg";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeadSeat {
    pub account: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerSeat {
    pub slot: String,
    pub role: String,
    pub task: String,
    pub account: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub security: bool,
    pub network: bool,
    pub why: String,
    pub source: String,
}

/// What a proposal with worker picks stores and hashes. Field order is the hash order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seats {
    pub max_in_flight: u32,
    pub lead: LeadSeat,
    pub workers: Vec<WorkerSeat>,
}

impl WorkerSeat {
    pub fn choice(&self) -> Choice {
        Choice { account: self.account.clone(), model: self.model.clone(), effort: self.effort.clone() }
    }
    fn kind(&self) -> Kind {
        roles::kind_of(&self.role).unwrap_or(Kind::Writer)
    }
}

/// One seat as the launch checks see it.
#[derive(Clone, Debug)]
pub struct SeatCheck {
    pub seat: String,
    pub kind: Kind,
    pub choice: Choice,
}

/// What the app tells the service about the project's seat defaults and what can launch. Installed by the app; with none
/// installed (the service's own tests) no seat is checked and no role default exists.
pub trait SeatPolicy: Send + Sync {
    /// The project's effective agent choices, or `None` when they cannot be read.
    fn agents(&self, project: &str) -> Option<Agents>;
    /// The H8 checks (16.2) for every seat, plus the engine check once. One entry per failing seat.
    fn check(&self, project: &str, seats: &[SeatCheck]) -> Vec<(String, Refused)>;
    /// Can a proposal for `project` carry seats? The build's core route, or the project's Labs switch (16.13).
    fn seats_on(&self, _project: &str) -> bool {
        false
    }
}

impl Seats {
    pub fn checks(&self) -> Vec<SeatCheck> {
        let lead = SeatCheck { seat: "lead".into(), kind: Kind::Lead, choice: Choice { account: self.lead.account.clone(), model: self.lead.model.clone(), effort: self.lead.effort.clone() } };
        std::iter::once(lead).chain(self.workers.iter().map(|w| SeatCheck { seat: w.slot.clone(), kind: w.kind(), choice: w.choice() })).collect()
    }

    /// The seat with `role` (the first one), for the evidence step and the card.
    pub fn first_with_role(&self, role: &str) -> Option<&WorkerSeat> {
        self.workers.iter().find(|w| w.role == role)
    }
}

/// The v2 hash input: a typed struct, so the order is fixed and nulls are written out.
#[derive(Serialize)]
struct ProposalV2<'a> {
    schema: &'static str,
    project: &'a str,
    ask: &'a str,
    base: &'a str,
    rounds: u32,
    max_in_flight: u32,
    lead: &'a LeadSeat,
    workers: &'a [WorkerSeat],
}

/// `vajra.run_proposal/2`: everything that decides what runs, with the seats.
pub fn proposal_sha_v2(project: &str, ask: &str, base: &str, rounds: u32, seats: &Seats) -> String {
    let v = ProposalV2 { schema: "vajra.run_proposal/2", project, ask, base, rounds, max_in_flight: seats.max_in_flight, lead: &seats.lead, workers: &seats.workers };
    sha256::hex_digest(&serde_json::to_vec(&v).unwrap_or_default())
}

/// The warning the card carries when no seat is marked `security: true` and the ask or a seat's task names security work.
pub fn security_warning(ask: &str, seats: &Seats) -> Option<&'static str> {
    let named = crate::crew::security::matches(ask) || seats.workers.iter().any(|w| crate::crew::security::matches(&w.task));
    (named && !seats.workers.iter().any(|w| w.security)).then_some(SECURITY_WARNING)
}

fn bad<T>(m: impl Into<String>) -> Result<T> {
    Err(ControlError::InvalidArgs(m.into()))
}

fn valid_effort(e: &str) -> bool {
    !e.is_empty() && e.len() <= 16 && e.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_slot(s: &str) -> bool {
    s.strip_prefix('w').and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| (1..=MAX_WORKERS).contains(&n) && s == format!("w{n}"))
}

impl Service {
    /// Plain text: the same `sanitise` check as `ask`, a byte bound, and the nonce redacted.
    fn seat_text(&self, v: Option<&Value>, name: &str, min: usize, max: usize) -> Result<String> {
        let s = match v {
            None if min == 0 => return Ok(String::new()),
            Some(Value::String(s)) => s.as_str(),
            _ => return bad(format!("{name} must be a string")),
        };
        if s.len() < min || s.len() > max {
            return bad(format!("{name} must be {min} to {max} bytes"));
        }
        let s = self.redactor().redact(s.to_string());
        if sanitise(&s, &Redactor::default()) != s {
            return bad(format!("{name} must be plain text (no control or invisible characters)"));
        }
        Ok(s)
    }

    /// `account`, `model` and `effort` of a seat: a known account id, and `null` or a valid model and effort.
    fn seat_choice(&self, o: &Map<String, Value>, what: &str) -> Result<(String, Option<String>, Option<String>)> {
        let account = match o.get("account") {
            Some(Value::String(a)) if crate::projects::contract::valid_provider(a) => a.clone(),
            _ => return bad(format!("{what}.account must be an account id")),
        };
        let opt = |k: &str, ok: &dyn Fn(&str) -> bool| match o.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) if ok(s) => Ok(Some(s.clone())),
            _ => bad(format!("{what}.{k} is not valid")),
        };
        Ok((account, opt("model", &crate::providers::contract::valid_model_id)?, opt("effort", &valid_effort)?))
    }

    fn check_keys(o: &Map<String, Value>, allowed: &[&str], what: &str) -> Result<()> {
        if o.contains_key("source") {
            return bad(format!("{what}.source is set by the core, not sent"));
        }
        match o.keys().find(|k| !allowed.contains(&k.as_str())) {
            Some(k) => bad(format!("unknown property {what}.{k}")),
            None => Ok(()),
        }
    }

    /// Validate the seat arguments of `propose_run`, add the required seats from the role defaults, and run the launch
    /// checks. Every failing seat is listed in one refusal; nothing is written here.
    pub(super) fn build_seats(&self, project: &str, ask: &str, args: &SeatArgs<'_>) -> Result<Seats> {
        let max_in_flight = match args.max_in_flight {
            None => DEFAULT_MAX_IN_FLIGHT,
            Some(v) => match v.as_u64().and_then(|n| u32::try_from(n).ok()) {
                Some(n) if (1..=MAX_WORKERS as u32).contains(&n) => n,
                _ => return bad("max_in_flight must be 1 to 16"),
            },
        };
        let Some(Value::Array(list)) = args.workers else { return bad("workers must be a list") };
        if list.is_empty() {
            return bad("workers must not be empty");
        }
        if list.len() > MAX_WORKERS {
            return bad("workers must have at most 16 seats");
        }
        let agents = self.seat_policy.as_ref().and_then(|p| p.agents(project));
        let mut workers: Vec<WorkerSeat> = Vec::new();
        for (i, w) in list.iter().enumerate() {
            let what = format!("workers[{i}]");
            let Value::Object(o) = w else { return bad(format!("{what} must be an object")) };
            Self::check_keys(o, &["slot", "role", "task", "account", "model", "effort", "security", "network", "why"], &what)?;
            let slot = match o.get("slot") {
                Some(Value::String(s)) if valid_slot(s) => s.clone(),
                _ => return bad(format!("{what}.slot must be w1 to w16")),
            };
            if workers.iter().any(|x| x.slot == slot) {
                return bad(format!("{what}.slot {slot} is used twice"));
            }
            let role = match o.get("role") {
                Some(Value::String(r)) if roles::is_role(r) => r.clone(),
                _ => return bad(format!("{what}.role is not a known crew role")),
            };
            let flag = |k: &str| match o.get(k) {
                None => Ok(false),
                Some(Value::Bool(b)) => Ok(*b),
                _ => bad(format!("{what}.{k} must be true or false")),
            };
            let (account, model, effort) = self.seat_choice(o, &what)?;
            workers.push(WorkerSeat {
                slot,
                role,
                task: self.seat_text(o.get("task"), &format!("{what}.task"), 1, 200)?,
                account,
                model,
                effort,
                security: flag("security")?,
                network: flag("network")?,
                why: self.seat_text(o.get("why"), &format!("{what}.why"), 0, 300)?,
                source: SOURCE_DG.into(),
            });
        }
        let lead = match args.lead {
            None | Some(Value::Null) => {
                let Some(a) = &agents else { return bad("the project's lead default is unavailable; send a lead seat") };
                let c = a.lead();
                LeadSeat { account: c.account.clone(), model: c.model.clone(), effort: c.effort.clone(), source: SOURCE_ROLE_DEFAULT.into() }
            }
            Some(Value::Object(o)) => {
                if o.contains_key("network") {
                    return bad("lead.network is not allowed: a lead is always read-only with no network");
                }
                Self::check_keys(o, &["account", "model", "effort"], "lead")?;
                let (account, model, effort) = self.seat_choice(o, "lead")?;
                LeadSeat { account, model, effort, source: SOURCE_DG.into() }
            }
            Some(_) => return bad("lead must be an object"),
        };
        // Required seats (N4, N15): a verifier, a reviewer, and a security reviewer when the ask names security work.
        let mut required = vec![roles::VERIFIER, roles::REVIEWER];
        if crate::crew::security::matches(ask) {
            required.push(roles::SECURITY_REVIEWER);
        }
        for role in required {
            if workers.iter().any(|w| w.role == role) {
                continue;
            }
            let (Some(a), Some(slot)) = (&agents, (1..=MAX_WORKERS).map(|n| format!("w{n}")).find(|s| workers.iter().all(|w| &w.slot != s))) else {
                return bad(format!("this proposal needs a {role} seat and none can be added: there is no {role} default or no free slot"));
            };
            let c = a.choice_for(Some(role));
            let task = match role {
                roles::VERIFIER => "Verify the integrated work",
                roles::REVIEWER => "Review the design of the integrated work",
                _ => "Review the integrated work for security issues",
            };
            workers.push(WorkerSeat { slot, role: role.into(), task: task.into(), account: c.account.clone(), model: c.model.clone(), effort: c.effort.clone(), security: false, network: false, why: "Added from the role default".into(), source: SOURCE_ROLE_DEFAULT.into() });
        }
        let seats = Seats { max_in_flight, lead, workers };
        if let Some(p) = &self.seat_policy {
            let refusals: Vec<Value> = p.check(project, &seats.checks()).into_iter().map(|(seat, r)| json!({"seat": seat, "code": r.code, "detail": r.detail})).collect();
            if !refusals.is_empty() {
                return Err(ControlError::Refusals(refusals));
            }
        }
        Ok(seats)
    }
}

/// The seat arguments of one `propose_run` call, as sent.
#[derive(Default)]
pub(super) struct SeatArgs<'a> {
    pub lead: Option<&'a Value>,
    pub workers: Option<&'a Value>,
    pub max_in_flight: Option<&'a Value>,
}

impl SeatArgs<'_> {
    pub fn any(&self) -> bool {
        self.lead.is_some() || self.workers.is_some() || self.max_in_flight.is_some()
    }
}

#[cfg(test)]
#[path = "seats_tests.rs"]
mod tests;
