//! Start or dismiss a run a Director-General proposed (ADR-15 2.4, owner decision D4 option A).
//!
//! The proposal is a `propose` line the app wrote to its own guarded ledger (`control::service::proposals`). The inbox
//! shows its exact ask, base and rounds with `proposal_sha256`. **Start** re-reads the proposal, refuses unless both
//! its stored hash and the hash recomputed from its fields equal the one the human was shown, writes the `requested`
//! line first, and then starts the run through the normal `start_run` path in the project's folder. The caps (D5) and
//! base checks apply exactly as for a run started from the form. **Dismiss** closes the proposal; nothing runs.
//!
//! One decision per proposal, behind the same per-item lock as plan decisions. A failed start (held by a cap, a bad
//! base) leaves the proposal open, so it can be started later.

use serde_json::{Value, json};

use super::{App, Res, caps, ctl, fail, invalid, lock};
use crate::control::service::{Decision, DecisionState, Proposal, ProposalDecision, proposal_sha, valid_proposal_item};
use crate::sessions::argv::RunInput;

/// What the inbox shows next to a proposal: who wrote it.
pub(super) const FROM_DIRECTOR_GENERAL: &str = "proposed by the Director-General";

impl App {
    /// The open proposal `id`, or why it cannot be decided now.
    fn open_proposal(&self, id: &str) -> Result<Proposal, Value> {
        if !valid_proposal_item(id) {
            return Err(invalid("id", "not a proposal id"));
        }
        match lock(&self.control).decided(id) {
            Ok(None) => {}
            Ok(Some(a)) => return Err(json!({"error": "already_decided", "detail": "this proposal was already started or dismissed", "action_id": a})),
            Err(e) => return Err(fail("io_error", e)),
        }
        let open = lock(&self.control).open_proposals(None).map_err(|e| fail("io_error", e))?;
        open.into_iter().find(|p| p.item == id).ok_or_else(|| fail("not_found", "no such proposal"))
    }

    fn proposal_line(&self, d: Decision, action: Option<&str>, p: &Proposal, state: DecisionState, run: Option<&str>, reason: Option<&str>) -> Result<String, Value> {
        let r = ProposalDecision { decision: d, action_id: action, item: &p.item, proposal_sha256: &p.proposal_sha256, state, started_run: run, reason };
        ctl(lock(&self.control).record_proposal_decision(&r))
    }

    /// Start proposal `id` as the human saw it (`proposal_sha256` is the hash the inbox returned).
    pub fn start_proposal(&self, id: &str, proposal_sha256: &str) -> Res {
        self.start_proposal_with(id, proposal_sha256, |input, cwd| self.start_run(input, cwd))
    }

    /// `start_proposal` with the run starter given, so a test can see exactly what would start without a real CLI.
    pub(super) fn start_proposal_with(&self, id: &str, proposal_sha256: &str, start: impl FnOnce(RunInput, &str) -> Res) -> Res {
        let _one = self.begin_decision(id)?;
        let p = self.open_proposal(id)?;
        let changed = || json!({"error": "proposal_changed", "detail": "this proposal is not what was shown; review it again"});
        if proposal_sha(&p.project, &p.ask, &p.base, p.rounds) != p.proposal_sha256 || p.proposal_sha256 != proposal_sha256 {
            return Err(changed());
        }
        let root = self.orchestrator_root(&p.project).filter(|r| r.is_dir()).ok_or_else(|| fail("not_found", "the project's folder is gone"))?;
        let root = root.to_str().ok_or_else(|| fail("not_found", "the project's folder is not a plain path"))?.to_string();
        // Written before anything starts: a crash after this line leaves the proposal decided, never started twice.
        let action = self.proposal_line(Decision::Approve, None, &p, DecisionState::Requested, None, None)?;
        let input = RunInput { ask: p.ask.clone(), max_rounds: p.rounds, base: p.base.clone(), max_in_flight: caps::DEFAULT_RUN_WORKERS, campaign: None };
        match start(input, &root) {
            Ok(run) => {
                self.proposal_line(Decision::Approve, Some(&action), &p, DecisionState::Final, run["run_id"].as_str(), None)?;
                self.note("proposal_started", &[]);
                Ok(json!({"action_id": action, "state": "started", "run": run}))
            }
            Err(err) => {
                let why = err["error"].as_str().unwrap_or("failed").to_string();
                let _ = self.proposal_line(Decision::Approve, Some(&action), &p, DecisionState::Failed, None, Some(&why));
                Err(err)
            }
        }
    }

    /// Dismiss proposal `id`: it leaves the inbox and nothing runs.
    pub fn dismiss_proposal(&self, id: &str) -> Res {
        let _one = self.begin_decision(id)?;
        let p = self.open_proposal(id)?;
        let action = self.proposal_line(Decision::Reject, None, &p, DecisionState::Final, None, Some("dismissed"))?;
        self.note("proposal_dismissed", &[]);
        Ok(json!({"action_id": action, "state": "dismissed"}))
    }

    /// The inbox items for open proposals: the exact ask that would run, plain text by construction.
    pub(super) fn proposal_items(&self, now: i64) -> Vec<(i64, String, Value)> {
        let open = match lock(&self.control).open_proposals(None) {
            Ok(o) => o,
            Err(_) => {
                // Fail closed: an unreadable ledger shows no proposal (none can be started either).
                self.note("inbox_proposals_failed", &[]);
                return Vec::new();
            }
        };
        open.into_iter()
            .map(|p| {
                let first = crate::control::time::parse_iso(&p.ts).unwrap_or(now);
                let folder = self.orchestrator_root(&p.project).and_then(|r| r.file_name().map(|n| n.to_string_lossy().into_owned()));
                let created = crate::control::time::iso_secs(std::time::UNIX_EPOCH + std::time::Duration::from_secs(first.max(0) as u64));
                let v = json!({
                    "id": p.item, "kind": "run_proposal", "origin": "app", "source": FROM_DIRECTOR_GENERAL,
                    "created": created, "age_s": (now - first).max(0),
                    "title": "Start a run?", "body": p.ask, "ask": p.ask, "base": p.base, "rounds": p.rounds,
                    "project_id": p.project, "project": folder, "generation": p.generation,
                    "proposal_sha256": p.proposal_sha256, "actions": ["start", "dismiss"],
                });
                (first, p.item, v)
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "proposals_tests.rs"]
mod tests;
