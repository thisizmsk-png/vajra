//! The Director-General's actions through action receipts (FG-02c, decision Q2; #74's `act`).
//!
//! `act_ask_lead`, `act_new_conversation`, `act_cancel_ask`, `act_start_proposal` and `act_dismiss_proposal` take the
//! `action_id` / `project_id` / `expect_generation` envelope and always answer with a `vajra.receipt/1`:
//! - a retried `action_id` returns the first receipt and runs nothing again;
//! - `expect_generation` is the Director-General's conversation the click was aimed at. An older one is
//!   `stale_generation` (checked when the receipt is begun, and again under the ask lock, so a click racing a "New
//!   conversation" never lands in the new one);
//! - the envelope's `project_id` must be the target's project, or nothing is stored (`project_mismatch`), so a receipt
//!   is never filed under another project. A target that cannot be resolved to a project (a run that is not a
//!   project's, an ask that is not running, a proposal not open) stores nothing either: the refusal comes back with
//!   `receipt: null`. Only an `action_id` that already has a receipt goes through as is, so a retry still replays.
//!
//! A proposal has no conversation generation: `proposal_sha256` stays the binding for what starts. Known limit (#74 P2):
//! an `action_id` is matched on (project, action, target), not on the full request (the ask text).

use serde_json::{Value, json};

use super::App;
use super::state::Envelope;
use crate::control::receipt::Caller;
use crate::ids;

/// Refused before any receipt is stored: the envelope names a different project than the target's.
fn mismatch() -> Value {
    json!({"receipt": null, "error": {"error": "project_mismatch", "detail": "this action names a different project than its target; refresh and try again"}})
}

/// Refused before any receipt is stored: the target could not be bound to a project.
fn unbound(error: Value) -> Value {
    json!({"receipt": null, "error": error})
}

/// A receipt generation as a Director-General generation (one that does not fit is never current).
fn expect_of(env: &Envelope) -> Option<u32> {
    env.expect_generation.map(|g| u32::try_from(g).unwrap_or(u32::MAX))
}

impl App {
    /// The project and current conversation of a run's Director-General, or the refusal that says why it has none.
    fn run_orchestrator(&self, run_id: &str) -> Result<(String, u32), Value> {
        if !ids::valid_run_id(run_id) {
            return Err(super::invalid("run_id", "must be YYYYMMDD-HHMM"));
        }
        let t = self.lead_target(run_id).map_err(|e| e.to_json())?;
        let o = self.orchestrator_for_run(&t.run_id, &t.cwd).map_err(|e| e.to_json())?;
        Ok((o.id, o.generation))
    }

    /// Has this `action_id` a receipt already? Then `act` answers it (a replay, or `action_id_reuse`), bound or not.
    fn has_receipt(&self, env: &Envelope) -> bool {
        super::lock(&self.acts).store.get(&env.action_id).is_some()
    }

    /// The project the target is bound to must be the envelope's; `None` = refused, with what to return.
    fn bound(&self, env: &Envelope, target: Result<String, Value>) -> Option<Value> {
        if self.has_receipt(env) {
            return None;
        }
        match target {
            Ok(p) if p == env.project_id => None,
            Ok(_) => Some(mismatch()),
            Err(e) => Some(unbound(e)),
        }
    }

    pub fn act_ask_lead(&self, env: &Envelope, run_id: &str, text: &str) -> Value {
        let found = self.run_orchestrator(run_id);
        if let Some(refused) = self.bound(env, found.clone().map(|(p, _)| p)) {
            return refused;
        }
        let cur = found.ok().map(|(_, g)| u64::from(g));
        self.act(env, Caller::Ui, true, "ask_lead", run_id, cur, || self.ask_lead_expecting(run_id, text, expect_of(env)).map_err(|e| e.to_json()))
    }

    pub fn act_new_conversation(&self, env: &Envelope, run_id: &str) -> Value {
        let found = self.run_orchestrator(run_id);
        if let Some(refused) = self.bound(env, found.clone().map(|(p, _)| p)) {
            return refused;
        }
        let cur = found.ok().map(|(_, g)| u64::from(g));
        self.act(env, Caller::Ui, true, "new_conversation", run_id, cur, || self.new_conversation_expecting(run_id, expect_of(env)).map_err(|e| e.to_json()))
    }

    pub fn act_cancel_ask(&self, env: &Envelope, ask_id: &str) -> Value {
        let owner = self.ask_owner(ask_id).ok_or_else(|| json!({"error": "ask_not_found"}));
        if let Some(refused) = self.bound(env, owner) {
            return refused;
        }
        self.act(env, Caller::Ui, true, "cancel_ask", ask_id, None, || self.cancel_ask(ask_id))
    }

    pub fn act_start_proposal(&self, env: &Envelope, id: &str, proposal_sha256: &str) -> Value {
        if let Some(refused) = self.bound(env, self.proposal_project(id)) {
            return refused;
        }
        self.act(env, Caller::Ui, true, "start_proposal", id, None, || self.start_proposal(id, proposal_sha256))
    }

    pub fn act_dismiss_proposal(&self, env: &Envelope, id: &str) -> Value {
        if let Some(refused) = self.bound(env, self.proposal_project(id)) {
            return refused;
        }
        self.act(env, Caller::Ui, true, "dismiss_proposal", id, None, || self.dismiss_proposal(id))
    }
}
