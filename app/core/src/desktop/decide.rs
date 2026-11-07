//! Approve or reject a plan item (ADR-14, S5). The app never runs push, PR or deploy itself.
//!
//! - **Approve** starts the approved action as a new turn of the run's own Claude Code session: the fixed text
//!   `Execute approved plan <run_id>.<index>` (an `ApprovedPlanTurn`, which holds no free text). A live session gets it
//!   through `submit_fixed_turn` (refused while Take over is on); an ended one is resumed with that text as its prompt,
//!   and the human answers its permission prompts through Take over. `vajra-deliver` has no paused state, so this is a
//!   new turn, not a resume of the workflow.
//! - **Reject** signs a `rejected plan <id>: <reason>` progress line through the campaign script (one argv element).
//! - Plan ids carry the entry's own index in the workflow result, and the hash covers the whole entry as written.
//! - Both are bound to what the human saw: the plan comes from an unsigned, worker-writable file, so `approve_plan`
//!   re-reads it and refuses unless it still hashes to the `plan_sha256` the inbox showed.
//! - One terminal decision per plan, behind a per-plan lock. The `requested` ledger line is written first.

use serde_json::{Value, json};

use super::inbox::{parse_plan_id, plan_sha};
use super::{App, Res, ctl, fail, invalid, lock, runindex};
use crate::control::lead::sanitise;
use crate::control::service::{Decision, DecisionRecord, DecisionState};
use crate::crew::redact::Redactor;
use crate::sessions::{ApprovedPlanTurn, SessionError, argv};
use crate::vajra::bridge::Request;

pub(super) const REASON_MAX: usize = 500;

/// Holds a plan id in `deciding` until dropped.
struct Deciding<'a>(&'a App, String);

impl Drop for Deciding<'_> {
    fn drop(&mut self) {
        lock(&self.0.deciding).remove(&self.1);
    }
}

fn already(action_id: &str) -> Value {
    json!({"error": "already_decided", "detail": "this plan was already approved or rejected", "action_id": action_id})
}

impl App {
    fn begin_decision(&self, id: &str) -> Result<Deciding<'_>, Value> {
        if !lock(&self.deciding).insert(id.to_string()) {
            return Err(fail("busy", "this plan is being decided"));
        }
        Ok(Deciding(self, id.to_string()))
    }

    /// Refuse if this exact plan (id and content hash) was already approved or rejected. An unreadable ledger refuses too.
    fn undecided(&self, id: &str, sha: &str) -> Result<(), Value> {
        match lock(&self.control).decided(id, sha) {
            Ok(None) => Ok(()),
            Ok(Some(a)) => Err(already(&a)),
            Err(e) => Err(fail("io_error", e)),
        }
    }

    /// The run a plan id names: the app's own origin run for that campaign (see `plan_origins`).
    fn plan_run(&self, run_id: &str) -> Result<runindex::Entry, Value> {
        self.plan_origins().into_values().find(|e| e.run_id == run_id).ok_or_else(|| fail("not_found", "no such run"))
    }

    #[allow(clippy::too_many_arguments)]
    fn decision_line(&self, d: Decision, action: Option<&str>, item: &str, e: &runindex::Entry, sha: Option<&str>, state: DecisionState, reason: Option<&str>) -> Result<String, Value> {
        let r = DecisionRecord { decision: d, action_id: action, item, run_id: &e.run_id, session_id: &e.session_id, campaign: &e.campaign, plan_sha256: sha, state, reason };
        ctl(lock(&self.control).record_decision(&r))
    }

    /// Approve plan `id` as the human saw it (`plan_sha256` is the hash the inbox returned).
    pub fn approve_plan(&self, id: &str, plan_sha256: &str) -> Res {
        let (run_id, index) = parse_plan_id(id).ok_or_else(|| invalid("id", "not a plan id"))?;
        let _one = self.begin_decision(id)?;
        let e = self.plan_run(run_id)?;
        let changed = || json!({"error": "plan_changed", "detail": "plan changed since you viewed it; review it again"});
        if lock(&self.shown).get(id).map(String::as_str) != Some(plan_sha256) {
            return Err(changed());
        }
        let w = self.waiting(&e).ok_or_else(changed)?;
        let plan = w.plans.iter().find(|p| p.index == index).ok_or_else(changed)?;
        if plan_sha(&plan.raw) != plan_sha256 {
            return Err(changed());
        }
        if let Some(why) = plan.invalid {
            return Err(fail("plan_invalid", format!("this plan cannot be approved: {why}")));
        }
        self.undecided(id, plan_sha256)?;
        let turn = ApprovedPlanTurn::new(run_id, index).ok_or_else(|| invalid("id", "not a plan id"))?;
        let stream = format!("run-{}", e.session_id);
        let live = self.sup.is_running(&stream).unwrap_or(false);
        if live && self.sup.take_over(&stream).unwrap_or(false) {
            return Err(fail("take_over_on", SessionError::TakeOverActive));
        }
        // Written before anything is delivered: a crash after this line leaves the plan decided, never run twice.
        let action = self.decision_line(Decision::Approve, None, id, &e, Some(plan_sha256), DecisionState::Requested, None)?;
        let sent = if live {
            self.sup.submit_fixed_turn(&stream, &turn).map_err(|err| fail(if err == SessionError::TakeOverActive { "take_over_on" } else { "send_failed" }, err))
        } else {
            self.resume_with_turn(&e, &turn)
        };
        match sent {
            Ok(()) => {
                self.decision_line(Decision::Approve, Some(&action), id, &e, Some(plan_sha256), DecisionState::Final, None)?;
                self.note("plan_approved", &[]);
                Ok(json!({"action_id": action, "state": "delivered", "via": if live { "session" } else { "resume" }}))
            }
            Err(err) => {
                let note = err["error"].as_str().unwrap_or("failed").to_string();
                let _ = self.decision_line(Decision::Approve, Some(&action), id, &e, Some(plan_sha256), DecisionState::Failed, Some(&note));
                Err(err)
            }
        }
    }

    /// An ended session continues from the approval: `claude --resume <sid> "Execute approved plan <id>"` in a new PTY.
    fn resume_with_turn(&self, e: &runindex::Entry, turn: &ApprovedPlanTurn) -> Result<(), Value> {
        let (_, run_id) = self.fresh_run_id(crate::ids::unix_now());
        self.launch(&e.session_id, &run_id, &e.campaign, &e.cwd, |_| argv::build_approve_resume_argv(&e.session_id, turn).map_err(super::invalid_input), true).map(|_| ())
    }

    /// Reject plan `id` with a reason (1 to 500 characters, controls dropped). The reason goes into the signed
    /// campaign progress log as one argument; if that fails the plan stays in the inbox.
    pub fn reject_plan(&self, id: &str, reason: &str) -> Res {
        let (run_id, index) = parse_plan_id(id).ok_or_else(|| invalid("id", "not a plan id"))?;
        // One line: the progress log is line-oriented, so every whitespace run (newlines included) becomes one space.
        let reason = sanitise(reason, &Redactor::default()).split_whitespace().collect::<Vec<_>>().join(" ");
        if reason.is_empty() || reason.chars().count() > REASON_MAX {
            return Err(invalid("reason", "must be 1 to 500 characters"));
        }
        let _one = self.begin_decision(id)?;
        let e = self.plan_run(run_id)?;
        let sha = self.waiting(&e).and_then(|w| w.plans.iter().find(|p| p.index == index).map(|p| plan_sha(&p.raw))).ok_or_else(|| fail("not_found", "no such plan"))?;
        self.undecided(id, &sha)?;
        let sha = Some(sha.as_str());
        let action = self.decision_line(Decision::Reject, None, id, &e, sha, DecisionState::Requested, Some(&reason))?;
        let text = format!("rejected plan {id}: {reason}");
        match self.bridge.run(&Request::CampaignProgress { slug: e.campaign.clone(), text }) {
            Ok(o) if o.code == Some(0) => {
                self.decision_line(Decision::Reject, Some(&action), id, &e, sha, DecisionState::Final, Some(&reason))?;
                self.note("plan_rejected", &[]);
                Ok(json!({"action_id": action, "state": "rejected"}))
            }
            other => {
                let why = match other {
                    Ok(o) => format!("progress exited {:?}", o.code),
                    Err(err) => err.to_string(),
                };
                let _ = self.decision_line(Decision::Reject, Some(&action), id, &e, sha, DecisionState::Failed, Some("progress_failed"));
                Err(fail("reject_failed", why))
            }
        }
    }
}

#[cfg(test)]
#[path = "decide_tests.rs"]
mod tests;
