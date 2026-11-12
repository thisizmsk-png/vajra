//! Start or dismiss a run a Director-General proposed (ADR-15 2.4, owner decision D4 option A).
//!
//! The proposal is a `propose` line the app wrote to its own guarded ledger (`control::service::proposals`). The inbox
//! shows its exact ask, base and rounds with `proposal_sha256`. **Start** re-reads the proposal, refuses unless both
//! its stored hash and the hash recomputed from its fields equal the one the human was shown, writes the `requested`
//! line first, and then starts the run through the normal `start_run` path in the project's folder. The caps (D5) and
//! base checks apply exactly as for a run started from the form. **Dismiss** closes the proposal; nothing runs.
//!
//! One decision per proposal, behind the same per-item lock as plan decisions. A failed start (held by a cap, a bad
//! base, an archived project) leaves the proposal open, so it can be started later.
//!
//! FG-02c: a proposal names a store project (`p_`), and starts in that project's recorded root. One from before the
//! store (a `k_` stand-in) is listed **dismiss-only** (under its carried-over project if there is one) and can never be
//! started: its hash binds an id the store does not know.

use serde_json::{Value, json};

use super::{App, Res, caps, ctl, fail, invalid, lock};
pub use super::seat_start::StartOptions;
use super::seat_start::{card, receipt_seats};
use crate::control::service::{SeatPolicy, Decision, DecisionState, Proposal, ProposalDecision, proposal_sha, proposal_sha_v2, valid_proposal_item};
use crate::ids;
use crate::sessions::argv::RunInput;

/// What the inbox shows next to a proposal: who wrote it.
pub(super) const FROM_DIRECTOR_GENERAL: &str = "proposed by the Director-General";
/// Next to a proposal from before projects: it can only be dismissed.
pub(super) const PRE_STORE: &str = "from before projects: dismiss only";

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
        self.proposal_line_with(d, action, p, state, run, reason, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn proposal_line_with(&self, d: Decision, action: Option<&str>, p: &Proposal, state: DecisionState, run: Option<&str>, reason: Option<&str>, seats: Option<(&Value, &str)>) -> Result<String, Value> {
        let r = ProposalDecision { decision: d, action_id: action, item: &p.item, proposal_sha256: &p.proposal_sha256, state, started_run: run, reason, seats };
        ctl(lock(&self.control).record_proposal_decision(&r))
    }

    /// The project an open proposal is listed under in the inbox (the same rule as the listing): its own, or for one from
    /// before projects the project its conversation was carried over to, else Unsorted. The receipt's `project_id` must
    /// name it, and a `k_` stand-in id is never one the window can send.
    pub(super) fn proposal_project(&self, id: &str) -> Result<String, Value> {
        let p = self.open_proposal(id)?;
        Ok(if ids::valid_standin_ref(&p.project) {
            self.standin_mapped_to(&p.project).unwrap_or_else(|| super::inbox::UNSORTED_PROJECT.to_string())
        } else {
            p.project
        })
    }

    /// Start proposal `id` as the human saw it (`proposal_sha256` is the hash the inbox returned).
    pub fn start_proposal(&self, id: &str, proposal_sha256: &str) -> Res {
        self.start_proposal_opts(id, proposal_sha256, StartOptions::default())
    }

    /// `start_proposal` with the user's overrides and start choice (ADR-16 16.3).
    pub fn start_proposal_opts(&self, id: &str, proposal_sha256: &str, opts: StartOptions) -> Res {
        // A proposal with seats is a core run with its own lead (6d2c); one without starts exactly as before.
        self.start_proposal_core(id, proposal_sha256, opts, |input, cwd, taken, project| match taken {
            Some((_, run)) => self.start_lead_run(project, input, cwd, &run),
            None => self.start_run_as(input, cwd, None),
        })
    }

    /// `start_proposal` with the run starter given, so a test can see exactly what would start without a real CLI.
    #[cfg(test)]
    pub(super) fn start_proposal_with(&self, id: &str, proposal_sha256: &str, start: impl FnOnce(RunInput, &str) -> Res) -> Res {
        self.start_proposal_inner(id, proposal_sha256, StartOptions::default(), |input, cwd, _| start(input, cwd))
    }

    #[cfg(test)]
    pub(super) fn start_proposal_inner(&self, id: &str, proposal_sha256: &str, opts: StartOptions, start: impl FnOnce(RunInput, &str, Option<(u64, String)>) -> Res) -> Res {
        self.start_proposal_core(id, proposal_sha256, opts, |input, cwd, taken, _| start(input, cwd, taken))
    }

    /// `start_proposal_inner`, with the proposal's (verified) project given to the starter.
    fn start_proposal_core(&self, id: &str, proposal_sha256: &str, opts: StartOptions, start: impl FnOnce(RunInput, &str, Option<(u64, String)>, &str) -> Res) -> Res {
        let _one = self.begin_decision(id)?;
        let p = self.open_proposal(id)?;
        let changed = || json!({"error": "proposal_changed", "detail": "this proposal is not what was shown; review it again"});
        let recomputed = match &p.seats {
            Some(s) => proposal_sha_v2(&p.project, &p.ask, &p.base, p.rounds, s),
            None => proposal_sha(&p.project, &p.ask, &p.base, p.rounds),
        };
        if recomputed != p.proposal_sha256 || p.proposal_sha256 != proposal_sha256 || !crate::projects::contract::valid_project_id(&p.project) {
            return Err(changed());
        }
        // Before anything is written: an archived or missing project starts nothing and the proposal stays open.
        let project = self.active_project(&p.project).map_err(|e| e.to_json())?;
        let root = std::path::PathBuf::from(&project.root);
        if !root.is_dir() {
            return Err(fail("not_found", "the project's folder is gone"));
        }
        let root = root.to_str().ok_or_else(|| fail("not_found", "the project's folder is not a plain path"))?.to_string();
        // A run with no worker picks is one Claude run session. A project whose default is another harness can't run it.
        if p.seats.is_none() && !opts.overrides.is_empty() {
            return Err(json!({"error": "override_invalid", "detail": "this proposal has no seats to override"}));
        }
        if p.seats.is_none() {
            let default = self.seat_env.agents(&p.project).map(|a| a.default.account);
            if default.is_some_and(|a| crate::providers::launch::harness_of(&a) != Some(crate::adapter::Engine::Claude)) {
                return Err(fail("harness_missing", "this project's default is not Claude Code, so a run without worker picks cannot start; ask for one with worker picks"));
            }
        }
        // Seats: overrides applied, every seat checked again, and `seats.json` written, all before the `requested` line.
        let prepared = if p.seats.is_some() { Some(self.prepare_seats(&p, &opts)?) } else { None };
        // Written before anything starts: a crash after this line leaves the proposal decided, never started twice.
        let action = self.proposal_line_with(Decision::Approve, None, &p, DecisionState::Requested, None, None, prepared.as_ref().map(|(_, _, doc, sha)| (doc, sha.as_str())))?;
        let max_in_flight = p.seats.as_ref().map_or(caps::DEFAULT_RUN_WORKERS, |s| s.max_in_flight);
        let input = RunInput { ask: p.ask.clone(), max_rounds: p.rounds, base: p.base.clone(), max_in_flight, campaign: None, provider: None };
        let (taken, doc) = match prepared {
            Some((run, now, doc, _)) => (Some((now, run)), Some(doc)),
            None => (None, None),
        };
        match start(input, &root, taken, &p.project) {
            Ok(run) => {
                self.proposal_line(Decision::Approve, Some(&action), &p, DecisionState::Final, run["run_id"].as_str(), None)?;
                self.note("proposal_started", &[]);
                let mut out = json!({"action_id": action, "state": "started", "run": run});
                if let Some(doc) = doc {
                    out["seats"] = json!(receipt_seats(&doc));
                    for k in ["evidence_seat", "evidence_trust", "campaign_slug"] {
                        out[k] = doc[k].clone();
                    }
                }
                Ok(out)
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
                // `project` is the project's full recorded root, the key the UI matches projects on (#90 review);
                // `project_name` is its name in the store. Neither is a fresh read of the repo. A stand-in proposal is
                // shown under the project its conversation went to, if any (Unsorted if none), and can only be dismissed.
                // `project_id` is always a store id (what the UI and `inbox_new` key on); the Director-General's own id
                // travels in `orchestrator_id` (#83 review).
                let standin = ids::valid_standin_ref(&p.project);
                let project_id = if standin {
                    self.standin_mapped_to(&p.project).unwrap_or_else(|| super::inbox::UNSORTED_PROJECT.to_string())
                } else {
                    p.project.clone()
                };
                let (root, name) = match self.projects.get(&project_id) {
                    Ok(sp) => (Some(sp.root), Some(sp.name)),
                    Err(_) => {
                        let r = self.orchestrator_root(&p.project);
                        (r.as_ref().map(|r| r.to_string_lossy().into_owned()), r.and_then(|r| r.file_name().map(|n| n.to_string_lossy().into_owned())))
                    }
                };
                let actions = if standin { json!(["dismiss"]) } else { json!(["start", "dismiss"]) };
                let created = crate::control::time::iso_secs(std::time::UNIX_EPOCH + std::time::Duration::from_secs(first.max(0) as u64));
                let v = json!({
                    "id": p.item, "kind": "run_proposal", "origin": "app", "source": FROM_DIRECTOR_GENERAL,
                    "created": created, "age_s": (now - first).max(0),
                    "title": "Start a run?", "body": p.ask, "ask": p.ask, "base": p.base, "rounds": p.rounds,
                    "project_id": project_id, "orchestrator_id": p.project, "project": root, "project_name": name, "generation": p.generation,
                    "proposal_sha256": p.proposal_sha256, "actions": actions,
                });
                let mut v = v;
                if let Some(c) = card(&p) {
                    v["seats_card"] = c;
                }
                if standin {
                    v["pre_store"] = json!(true);
                    v["note"] = json!(PRE_STORE);
                }
                (first, p.item, v)
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "proposals_tests.rs"]
mod tests;
