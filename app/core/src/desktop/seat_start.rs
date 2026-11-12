//! Starting a proposal that carries worker picks (ADR-16 16.3): overrides, the launch checks again, the evidence trust,
//! `seats.json`, and the card. Nothing here spawns anything: with the core route off (6a), every such start ends at the
//! launch checks with `worker_unavailable`, before the `requested` line.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use super::{App, fail};
use crate::adapter::Engine;
use crate::adapter::profiles::CodexLock;
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

/// A Codex seat's card note when this Mac's launch-time check passed: the sandbox held and none of the user's Codex tools
/// would run outside it (bhishma #216 H2, L1).
pub const CODEX_WARNING: &str = "Runs outside the Vajra guard, in Codex's own sandbox. It can't read Vajra's signing key or records, and it can write only in its worktree. It can read your other files.";
/// Otherwise: the same sandbox, no key claim, and the cause in the user's words after it.
pub const CODEX_WARNING_OPEN: &str = "Runs outside the Vajra guard, in Codex's own sandbox. It can write only in its worktree, and it can read your other files.";

/// The lock answer per Codex account (`SeatEnv::codex_lock` or its hint).
pub type LockOf<'a> = &'a dyn Fn(&str) -> CodexLock;

fn codex_note(lock: &CodexLock) -> String {
    match lock {
        Ok(()) => CODEX_WARNING.to_string(),
        Err(why) => format!("{CODEX_WARNING_OPEN} {why}"),
    }
}
pub const CLAUDE_NOTE: &str = "Runs with the Vajra guard. Your own Claude Code MCP servers run with it.";
pub const NETWORK_NOTE: &str = "Internet on for this worker";
pub const OUTSIDE_GUARD_CARD: &str = "Final check runs outside the Vajra guard";
pub const OUTSIDE_GUARD_WHY: &str = "A helper outside the Vajra guard wrote the final check. Vajra ran it with the signing key locked away, but couldn't confirm the helper itself was kept from the key on this Mac. If it read the key, it could have used it to fake Vajra records and approvals. A key already copied stays valid.";
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
/// 6k: a Codex verifier counts as guarded only when `lock` passes for its account: the OS keeps the session from the
/// Vajra state dir, and none of the user's Codex tools (MCP servers, notify) would run outside its sandbox.
pub fn evidence_of(seats: &Seats, approve_checker: bool, run_id: &str, lock: LockOf<'_>) -> Value {
    let verifier = seats.first_with_role(roles::VERIFIER);
    // bhishma #216 H1: a Codex Lead is the user's own interactive session (approvals and escalation stay theirs), so
    // its run is never counted guarded, whatever the verifier.
    let codex_lead = harness(&seats.lead.account) == Some(Engine::Codex);
    let locked = !codex_lead && verifier.is_some_and(|v| match harness(&v.account) {
        Some(Engine::Claude) => true,
        Some(Engine::Codex) => lock(&v.account).is_ok(),
        None => false,
    });
    let trust = match (locked, approve_checker) {
        (true, _) => "guarded",
        (false, true) => "guarded_by_approval",
        (false, false) => "outside_guard",
    };
    let slug = if trust == "outside_guard" { format!("deliver-{run_id}-og") } else { format!("deliver-{run_id}") };
    json!({"evidence_seat": verifier.map(|v| v.slot.as_str()), "evidence_trust": trust, "campaign_slug": slug})
}

pub const CODEX_LEAD_CAUSE: &str = "The lead runs on Codex, in your own interactive Codex session, so Vajra doesn't count its run as guarded.";

/// Why a Codex part of the run keeps the evidence outside the guard, named for the card (bhishma #216 H2): the Codex
/// lead, or the verifier's launch-time answer. `None` when the verifier simply isn't on a guarded harness.
pub fn outside_cause(seats: &Seats, lock: LockOf<'_>) -> Option<String> {
    if harness(&seats.lead.account) == Some(Engine::Codex) {
        return Some(CODEX_LEAD_CAUSE.into());
    }
    let v = seats.first_with_role(roles::VERIFIER)?;
    if harness(&v.account) != Some(Engine::Codex) {
        return None;
    }
    lock(&v.account).err()
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
fn seat_entry(seat: &str, role: &str, kind: Kind, account: &str, extra: Map<String, Value>, network: bool, lock: LockOf<'_>) -> Value {
    let h = harness(account);
    let (limits, mut disclosures) = match h {
        Some(Engine::Claude) => (json!({"message": true, "redirect": true, "stop": "hook_block"}), vec![CLAUDE_NOTE.to_string()]),
        Some(Engine::Codex) => (json!({"message": false, "redirect": false, "stop": "terminate", "note": "This worker can't take a message mid-run."}), vec![codex_note(&lock(account))]),
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

fn seat_entries(seats: &Seats, user: &BTreeSet<String>, lock: LockOf<'_>) -> Vec<Value> {
    let src = |seat: &str, s: &str| if user.contains(seat) { "user".to_string() } else { s.to_string() };
    let mut lead = Map::new();
    lead.insert("model".into(), json!(seats.lead.model));
    lead.insert("effort".into(), json!(seats.lead.effort));
    lead.insert("source".into(), json!(src("lead", &seats.lead.source)));
    let mut out = vec![seat_entry("lead", "lead", Kind::Lead, &seats.lead.account, lead, false, lock)];
    for w in &seats.workers {
        let mut m = Map::new();
        m.insert("slot".into(), json!(w.slot));
        m.insert("task".into(), json!(w.task));
        m.insert("model".into(), json!(w.model));
        m.insert("effort".into(), json!(w.effort));
        m.insert("security".into(), json!(w.security));
        m.insert("why".into(), json!(w.why));
        m.insert("source".into(), json!(src(&w.slot, &w.source)));
        out.push(seat_entry(&w.slot, &w.role, roles::kind_of(&w.role).unwrap_or(Kind::Writer), &w.account, m, w.network, lock));
    }
    out
}

/// The guarded `seats.json` document: the seats in effect after overrides, and the evidence seat, trust and campaign slug.
pub(super) fn seats_doc(run_id: &str, proposal_sha: &str, seats: &Seats, user: &BTreeSet<String>, approve_checker: bool, lock: LockOf<'_>) -> Value {
    let mut doc = json!({"schema": "vajra.run_seats/1", "run_id": run_id, "proposal_sha256": proposal_sha, "max_in_flight": seats.max_in_flight, "seats": seat_entries(seats, user, lock)});
    if let (Value::Object(d), Value::Object(e)) = (&mut doc, evidence_of(seats, approve_checker, run_id, lock)) {
        d.extend(e);
    }
    doc
}

/// The start receipt's one entry per seat: `{seat, role, source, requested{...}, protection}`.
pub(super) fn receipt_seats(doc: &Value) -> Vec<Value> {
    doc["seats"].as_array().into_iter().flatten().map(|s| json!({"seat": s["seat"], "role": s["role"], "source": s["source"], "requested": {"account": s["account"], "harness": s["harness"], "model": s["model"], "effort": s["effort"], "network": s["network"]}, "protection": s["protection"]})).collect()
}

/// The proposal card's seat section: the seats as proposed, the warnings, and what the run's final check will be called.
/// `lock` gives the last launch-time check's answer per Codex account (`SeatEnv::codex_lock_hint`); Start checks again.
pub(super) fn card(p: &Proposal, lock: LockOf<'_>) -> Option<Value> {
    let seats = p.seats.as_ref()?;
    let ev = evidence_of(seats, false, "<run>", lock);
    let outside = ev["evidence_trust"] == "outside_guard";
    let mut v = json!({
        "max_in_flight": seats.max_in_flight,
        "seats": seat_entries(seats, &BTreeSet::new(), lock),
        "evidence": {"seat": ev["evidence_seat"], "trust": ev["evidence_trust"]},
    });
    if outside {
        v["evidence"]["card"] = json!(OUTSIDE_GUARD_CARD);
        v["evidence"]["label"] = json!("Verified (evidence ran outside the guard)");
        v["evidence"]["why"] = json!(OUTSIDE_GUARD_WHY);
        v["evidence"]["approval_offer"] = json!(APPROVAL_OFFER);
        if let Some(c) = outside_cause(seats, lock) {
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
        // 6k: the launch-time check, run fresh at every Start, once per Codex account on the seats. The verifier's
        // answer decides the trust; every Codex seat's card note follows its own.
        let mut answers: std::collections::HashMap<String, CodexLock> = std::collections::HashMap::new();
        for a in std::iter::once(&seats.lead.account).chain(seats.workers.iter().map(|w| &w.account)) {
            if harness(a) == Some(Engine::Codex) && !answers.contains_key(a.as_str()) {
                answers.insert(a.clone(), self.seat_env.codex_lock(a));
            }
        }
        let lock = |a: &str| answers.get(a).cloned().unwrap_or_else(|| Err(crate::adapter::profiles::CODEX_NOT_PROVEN.into()));
        let (now, run_id) = self.fresh_run_id(crate::ids::unix_now())?;
        let doc = seats_doc(&run_id, &p.proposal_sha256, &seats, &user, opts.approve_checker, &lock);
        let sha = self.record_run_seats(&run_id, &doc).map_err(|_| fail("io", "could not record the run's seats"))?;
        Ok((run_id, now, doc, sha))
    }
}

#[cfg(test)]
#[path = "seat_start_tests.rs"]
mod tests;
