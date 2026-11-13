//! Starting a proposal that carries worker picks (ADR-16 16.3): overrides, the launch checks again, the evidence trust,
//! `seats.json`, and the card. Nothing here spawns anything: with the core route off (6a), every such start ends at the
//! launch checks with `worker_unavailable`, before the `requested` line.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use super::{App, fail};
use crate::adapter::Engine;
use crate::control::service::{Proposal, Seats, security_warning};
use crate::crew::roles::{self, Kind};
use crate::providers::launch;

/// What a start carries besides the hash: the user's overrides, and the N20 choice.
#[derive(Default)]
pub struct StartOptions {
    /// `[{seat: "lead"|"w<n>", account, model, effort, network}]`; each field optional. Outside `proposal_sha256`.
    pub overrides: Vec<Value>,
    /// With a non-Claude checker: approve its commands when they're ready, to get plain Verified (`guarded_by_approval`).
    pub approve_checker: bool,
}

/// Every Codex seat's card note (owner 2026-11-13, "keep my extras, no key claim"): Codex runs with the user's own
/// settings and tools, and Vajra makes no claim that it is kept from the signing key.
pub const CODEX_WARNING: &str = "Runs outside the Vajra guard, with your own Codex settings. Your Codex tools (MCP servers, hooks, plugins, notify) run with it, outside its sandbox. Its internet follows this seat's switch. It isn't guarded against reading Vajra's signing key, and with that key it could fake Vajra's records and approvals, not only test results.";

/// bhishma #216 M-Lead: the Codex Lead row also names why its run is never counted guarded.
fn codex_lead_note() -> String {
    format!("{CODEX_WARNING} {CODEX_LEAD_CAUSE}")
}
pub const CLAUDE_NOTE: &str = "Runs with the Vajra guard. Your own Claude Code MCP servers run with it.";
pub const NETWORK_NOTE: &str = "Internet on for this worker";
pub const OUTSIDE_GUARD_CARD: &str = "Final check runs outside the Vajra guard";
pub const OUTSIDE_GUARD_WHY: &str = "A helper outside the Vajra guard wrote the final check. Vajra ran it with the signing key locked away, but the helper itself isn't guarded against reading the key, so it could have copied it and used it to fake Vajra records and approvals. A key already copied stays valid.";
pub const APPROVAL_OFFER: &str = "Approve the checker's commands when they're ready, to get plain Verified.";

fn override_invalid(detail: impl Into<String>) -> Value {
    json!({"error": "override_invalid", "detail": detail.into()})
}

fn harness(account: &str) -> Option<Engine> {
    launch::harness_of(account)
}

/// Apply the user's overrides to a proposal's seats. A missing field keeps the proposal's value; `model` and `effort`
/// may be sent as `null` (the CLI's own default). Returns the new seats and the seats the user changed.
pub(super) fn apply_overrides(seats: &Seats, overrides: &[Value]) -> Result<(Seats, BTreeSet<String>), Value> {
    let mut out = seats.clone();
    let mut changed = BTreeSet::new();
    for (i, o) in overrides.iter().enumerate() {
        let Value::Object(o) = o else { return Err(override_invalid(format!("overrides[{i}] must be an object"))) };
        if let Some(k) = o.keys().find(|k| !["seat", "account", "model", "effort", "network"].contains(&k.as_str())) {
            return Err(override_invalid(format!("an override can change only account, model, effort and network, not {k}")));
        }
        let Some(seat) = o.get("seat").and_then(Value::as_str) else { return Err(override_invalid(format!("overrides[{i}].seat is required"))) };
        if !changed.insert(seat.to_string()) {
            return Err(override_invalid(format!("{seat} is overridden twice")));
        }
        if seat == "lead" && o.contains_key("network") {
            return Err(override_invalid("network can't be turned on for the lead: a lead is always read-only with no network"));
        }
        let account = match o.get("account") {
            None => None,
            Some(Value::String(a)) if crate::projects::contract::valid_provider(a) => Some(a.clone()),
            _ => return Err(override_invalid(format!("{seat}.account is not an account id"))),
        };
        let opt = |k: &str, ok: &dyn Fn(&str) -> bool| -> Result<Option<Option<String>>, Value> {
            match o.get(k) {
                None => Ok(None),
                Some(Value::Null) => Ok(Some(None)),
                Some(Value::String(s)) if ok(s) => Ok(Some(Some(s.clone()))),
                _ => Err(override_invalid(format!("{seat}.{k} is not valid"))),
            }
        };
        let model = opt("model", &crate::providers::contract::valid_model_id)?;
        let effort = opt("effort", &|e| !e.is_empty() && e.len() <= 16 && e.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))?;
        let network = match o.get("network") {
            None => None,
            Some(Value::Bool(b)) => Some(*b),
            _ => return Err(override_invalid(format!("{seat}.network must be true or false"))),
        };
        if seat == "lead" {
            let l = &mut out.lead;
            l.account = account.unwrap_or_else(|| l.account.clone());
            l.model = model.unwrap_or_else(|| l.model.clone());
            l.effort = effort.unwrap_or_else(|| l.effort.clone());
            l.source = "user".into();
        } else {
            let Some(w) = out.workers.iter_mut().find(|w| w.slot == seat) else { return Err(override_invalid(format!("{seat} is not a seat of this proposal"))) };
            w.account = account.unwrap_or_else(|| w.account.clone());
            w.model = model.unwrap_or_else(|| w.model.clone());
            w.effort = effort.unwrap_or_else(|| w.effort.clone());
            w.network = network.unwrap_or(w.network);
            w.source = "user".into();
        }
    }
    Ok((out, changed))
}

/// Who wrote the final check, and so how far its evidence can be trusted (ADR-16 16.5a). Always the verifier seat: no
/// agent session runs the evidence, so it never moves to another seat. Fixed at Start, from the harness in effect.
/// A Codex verifier is never guarded: it runs with the user's own settings (owner 2026-11-13).
pub fn evidence_of(seats: &Seats, approve_checker: bool, run_id: &str) -> Value {
    let verifier = seats.first_with_role(roles::VERIFIER);
    // bhishma #216 H1: a Codex Lead is the user's own interactive session (approvals and escalation stay theirs), so
    // its run is never counted guarded, whatever the verifier.
    let codex_lead = harness(&seats.lead.account) == Some(Engine::Codex);
    let claude = !codex_lead && verifier.is_some_and(|v| harness(&v.account) == Some(Engine::Claude));
    let trust = match (claude, approve_checker) {
        (true, _) => "guarded",
        (false, true) => "guarded_by_approval",
        (false, false) => "outside_guard",
    };
    let slug = if trust == "outside_guard" { format!("deliver-{run_id}-og") } else { format!("deliver-{run_id}") };
    json!({"evidence_seat": verifier.map(|v| v.slot.as_str()), "evidence_trust": trust, "campaign_slug": slug})
}

pub const CODEX_LEAD_CAUSE: &str = "The lead runs on Codex, in your own interactive Codex session, so Vajra doesn't count its run as guarded.";

/// Why the evidence is outside the guard when the verifier alone wouldn't say: a Codex lead (bhishma #216 H1).
pub fn outside_cause(seats: &Seats) -> Option<&'static str> {
    (harness(&seats.lead.account) == Some(Engine::Codex)).then_some(CODEX_LEAD_CAUSE)
}

fn protection(h: Option<Engine>, kind: Kind) -> &'static str {
    match (h, kind) {
        (Some(Engine::Claude), _) => "guard",
        (Some(Engine::Codex), Kind::Lead) => "sandbox_read_only",
        (Some(Engine::Codex), _) => "sandbox_worktree",
        (None, _) => "none",
    }
}

/// What the card says about one seat: its harness, protection, per-worker limits (16.7) and disclosures (16.4, 16.8).
fn seat_entry(seat: &str, role: &str, kind: Kind, account: &str, extra: Map<String, Value>, network: bool) -> Value {
    let h = harness(account);
    let (limits, mut disclosures) = match h {
        Some(Engine::Claude) => (json!({"message": true, "redirect": true, "stop": "hook_block"}), vec![CLAUDE_NOTE.to_string()]),
        Some(Engine::Codex) => {
            let note = if kind == Kind::Lead { codex_lead_note() } else { CODEX_WARNING.to_string() };
            (json!({"message": false, "redirect": false, "stop": "terminate", "note": "This worker can't take a message mid-run."}), vec![note])
        }
        None => (json!({}), vec![]),
    };
    if network {
        disclosures.push(NETWORK_NOTE.to_string());
    }
    let mut v = Map::new();
    v.insert("seat".into(), json!(seat));
    v.insert("role".into(), json!(role));
    v.insert("account".into(), json!(account));
    v.insert("harness".into(), json!(h.map(|e| e.id())));
    v.insert("network".into(), json!(network));
    v.insert("protection".into(), json!(protection(h, kind)));
    v.insert("limits".into(), limits);
    v.insert("disclosures".into(), json!(disclosures));
    v.extend(extra);
    Value::Object(v)
}

fn seat_entries(seats: &Seats, user: &BTreeSet<String>) -> Vec<Value> {
    let src = |seat: &str, s: &str| if user.contains(seat) { "user".to_string() } else { s.to_string() };
    let mut lead = Map::new();
    lead.insert("model".into(), json!(seats.lead.model));
    lead.insert("effort".into(), json!(seats.lead.effort));
    lead.insert("source".into(), json!(src("lead", &seats.lead.source)));
    let mut out = vec![seat_entry("lead", "lead", Kind::Lead, &seats.lead.account, lead, false)];
    for w in &seats.workers {
        let mut m = Map::new();
        m.insert("slot".into(), json!(w.slot));
        m.insert("task".into(), json!(w.task));
        m.insert("model".into(), json!(w.model));
        m.insert("effort".into(), json!(w.effort));
        m.insert("security".into(), json!(w.security));
        m.insert("why".into(), json!(w.why));
        m.insert("source".into(), json!(src(&w.slot, &w.source)));
        out.push(seat_entry(&w.slot, &w.role, roles::kind_of(&w.role).unwrap_or(Kind::Writer), &w.account, m, w.network));
    }
    out
}

/// The guarded `seats.json` document: the seats in effect after overrides, and the evidence seat, trust and campaign slug.
pub(super) fn seats_doc(run_id: &str, proposal_sha: &str, seats: &Seats, user: &BTreeSet<String>, approve_checker: bool) -> Value {
    let mut doc = json!({"schema": "vajra.run_seats/1", "run_id": run_id, "proposal_sha256": proposal_sha, "max_in_flight": seats.max_in_flight, "seats": seat_entries(seats, user)});
    if let (Value::Object(d), Value::Object(e)) = (&mut doc, evidence_of(seats, approve_checker, run_id)) {
        d.extend(e);
    }
    doc
}

/// The start receipt's one entry per seat: `{seat, role, source, requested{...}, protection}`.
pub(super) fn receipt_seats(doc: &Value) -> Vec<Value> {
    doc["seats"].as_array().into_iter().flatten().map(|s| json!({"seat": s["seat"], "role": s["role"], "source": s["source"], "requested": {"account": s["account"], "harness": s["harness"], "model": s["model"], "effort": s["effort"], "network": s["network"]}, "protection": s["protection"]})).collect()
}

/// The proposal card's seat section: the seats as proposed, the warnings, and what the run's final check will be called.
pub(super) fn card(p: &Proposal) -> Option<Value> {
    let seats = p.seats.as_ref()?;
    let ev = evidence_of(seats, false, "<run>");
    let outside = ev["evidence_trust"] == "outside_guard";
    let mut v = json!({
        "max_in_flight": seats.max_in_flight,
        "seats": seat_entries(seats, &BTreeSet::new()),
        "evidence": {"seat": ev["evidence_seat"], "trust": ev["evidence_trust"]},
    });
    if outside {
        v["evidence"]["card"] = json!(OUTSIDE_GUARD_CARD);
        v["evidence"]["label"] = json!("Verified (evidence ran outside the guard)");
        v["evidence"]["why"] = json!(OUTSIDE_GUARD_WHY);
        v["evidence"]["approval_offer"] = json!(APPROVAL_OFFER);
        if let Some(c) = outside_cause(seats) {
            v["evidence"]["cause"] = json!(c);
        }
    }
    if let Some(w) = security_warning(&p.ask, seats) {
        v["security_warning"] = json!(w);
    }
    Some(v)
}

impl App {
    /// A refusal list as the answer of a start.
    pub(super) fn worker_unavailable(refusals: Vec<(String, launch::Refused)>) -> Value {
        let list: Vec<Value> = refusals.into_iter().map(|(seat, r)| json!({"seat": seat, "code": r.code, "detail": r.detail})).collect();
        json!({"error": "worker_unavailable", "detail": "some seats cannot be launched", "refusals": list})
    }

    /// Everything a seat start needs before the `requested` line: overrides applied, every seat checked again with them,
    /// the run id taken, and `seats.json` written. Returns the run id, the id's minute, the document and the file's sha256.
    pub(super) fn prepare_seats(&self, p: &Proposal, opts: &StartOptions) -> Result<(String, u64, Value, String), Value> {
        let Some(seats) = &p.seats else { return Err(fail("not_found", "this proposal has no seats")) };
        let (seats, user) = apply_overrides(seats, &opts.overrides)?;
        let refusals = self.seat_env.refusals(&p.project, &seats.checks());
        if !refusals.is_empty() {
            return Err(Self::worker_unavailable(refusals));
        }
        let (now, run_id) = self.fresh_run_id(crate::ids::unix_now())?;
        let doc = seats_doc(&run_id, &p.proposal_sha256, &seats, &user, opts.approve_checker);
        let sha = self.record_run_seats(&run_id, &doc).map_err(|_| fail("io", "could not record the run's seats"))?;
        Ok((run_id, now, doc, sha))
    }
}

#[cfg(test)]
#[path = "seat_start_tests.rs"]
mod tests;
