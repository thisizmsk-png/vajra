//! N20 (owner answer 2026-11-10): the single-use approval of a `guarded_by_approval` run's check commands.
//!
//! - The core asks once the gate passes: the exact ordered command list, the integration commit, and the evidence
//!   seat and account. The human approves or declines the whole list.
//! - An approval is bound to the run id, round, commit, seat, account and the sha256 of the list, and is used once.
//!   A redo, a new round, a resume (a new run id) or any changed command makes it void.
//! - No allow rule, setting or preference is written, and there is no "always".
//! - Only a human click in the app approves (`act` with `human_only`): never MCP, a socket or an agent.
//! - Decline, or no answer within `HOLD_MAX`, ends `not_verified`. Approve, once consumed by this run's evidence ticket,
//!   reads plain Verified with the note "check commands approved by you".
//! - The ask, each answer and the consume are `audit.log` lines, and fail closed: an answer whose line can't be written
//!   is refused, and the person sees why (bhishma L2 on #226). The evidence ticket writes the signed `progress` line
//!   (`progress_line`).
//!
//! bhishma M1 on #226: until agents can't write the app's data folder (key A), the file can't be trusted on its own.
//! The asks and a person's answers are held in memory (`App::approvals`) for the app's lifetime; a consume needs the
//! held answer and the file to agree. After a restart nothing is held, so a pending or approved record is void and the
//! core asks again. The proof's `approval_consumed` needs the signed progress line as well (`proof_app`).
//!
//! The record is `<data>/control/lead/<run>/approval.json`, the core's own (private dir, atomic writes); it is what the
//! UI shows.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::{fail, invalid, App, Res};
use crate::control::{audit, fsx, sha256::hex_digest};

/// `evidence_hold_max_s`: how long an ask waits for its answer, and how long an approval stays usable.
pub const HOLD_MAX: Duration = Duration::from_secs(24 * 60 * 60);
const FILE: &str = "approval.json";
/// At most this many commands, each at most this long (the list is shown whole, so it must fit on screen).
const MAX_COMMANDS: usize = 64;
const MAX_COMMAND: usize = 4096;

/// File read-modify-writes run under this lock, so a consume can't race an answer.
static LOCK: Mutex<()> = Mutex::new(());

/// What the human is asked to approve: everything the approval is bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub run_id: String,
    pub round: u32,
    /// The integration commit (a full 40-hex sha).
    pub commit: String,
    pub seat: String,
    pub account: String,
    /// Every setup, check and suite command, in order.
    pub commands: Vec<String>,
}

/// An ask this app made, and a person's answer to it (held in memory only).
#[derive(Clone, Debug)]
pub(crate) struct Held {
    ask: Ask,
    sha: String,
    asked_at: u64,
    answer: Option<bool>,
}

/// A character that can make a command show differently from what runs: any control character but tab (CR, ESC and
/// the other C0 and C1 codes), and the Unicode bidi and invisible format characters (Opus on #226).
fn hides_text(c: char) -> bool {
    (c.is_control() && c != '\t')
        || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{061C}' | '\u{FEFF}')
}

impl Ask {
    fn check(&self) -> Result<(), String> {
        if !crate::ids::valid_run_id(&self.run_id) {
            return Err("not a run id".into());
        }
        if !(self.commit.len() == 40 && self.commit.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())) {
            return Err("the commit must be a full sha".into());
        }
        if self.seat.is_empty() || self.account.is_empty() || [&self.seat, &self.account].iter().any(|s| s.chars().any(hides_text)) {
            return Err("the evidence seat and account are required, as plain text".into());
        }
        if self.commands.is_empty() || self.commands.len() > MAX_COMMANDS {
            return Err(format!("1 to {MAX_COMMANDS} commands"));
        }
        if self.commands.iter().any(|c| c.trim().is_empty() || c.len() > MAX_COMMAND) {
            return Err("a command is empty or too long".into());
        }
        if self.commands.iter().any(|c| c.chars().any(hides_text)) {
            return Err("a command has a control or invisible character, so it can't be shown exactly as it runs".into());
        }
        Ok(())
    }

    fn to_json(&self) -> Value {
        json!({"run_id": self.run_id, "round": self.round, "commit": self.commit, "seat": self.seat, "account": self.account, "commands": self.commands})
    }

    /// sha256 of the canonical list (compact JSON, fixed key order): what the human approved, and what the audit and
    /// progress lines name.
    pub fn sha256(&self) -> String {
        hex_digest(self.to_json().to_string().as_bytes())
    }
}

/// The signed `progress` line the evidence ticket writes once it consumed the approval.
pub fn progress_line(sha: &str) -> String {
    format!("check commands approved by the user (sha {})", &sha[..sha.len().min(12)])
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn expired(asked_at: u64, now: SystemTime) -> bool {
    secs(now).saturating_sub(asked_at) > HOLD_MAX.as_secs()
}

impl App {
    fn approval_path(&self, run_id: &str) -> PathBuf {
        self.data.join("control/lead").join(run_id).join(FILE)
    }

    fn read_approval(&self, run_id: &str) -> Result<Option<Value>, String> {
        match fsx::read_nofollow(&self.approval_path(run_id)) {
            Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|_| "the approval record is unreadable".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("the approval record is unreadable".into()),
        }
    }

    fn write_approval(&self, run_id: &str, v: &Value) -> Result<(), String> {
        let dir = self.data.join("control/lead").join(run_id);
        fsx::private_dir(&self.data.join("control")).and_then(|_| fsx::private_dir(&self.data.join("control/lead"))).and_then(|_| fsx::private_dir(&dir)).map_err(|e| e.to_string())?;
        fsx::write_atomic(&dir, FILE, v.to_string().as_bytes()).map(drop).map_err(|e| e.to_string())
    }

    /// The audit line, before the state it records changes: no line, no change (fail closed).
    fn audit_approval(&self, verdict: &str, what: &str, run_id: &str, sha: &str, now: SystemTime) -> Result<(), String> {
        let line = format!("evidence-approval {what} run={run_id} sha={sha}");
        audit::append_line(&self.home.join(".claude/vajra/audit.log"), verdict, &line, now).map_err(|_| "Vajra couldn't write this to its audit log, so nothing was changed. Check that the Vajra folder is writable, then try again.".to_string())
    }

    /// The core asks (after the gate passes). Any earlier ask of this run, answered or not, is void: a redo or a new
    /// round asks again, and only the newest list can be approved. Returns the list's sha256.
    #[allow(dead_code)] // the evidence ticket (runproto, after the run-protocol stack lands) is its first caller
    pub(crate) fn request_evidence_approval(&self, ask: &Ask, now: SystemTime) -> Result<String, String> {
        ask.check()?;
        let _g = crate::lock(&LOCK);
        let sha = ask.sha256();
        self.audit_approval("ALLOW", "asked", &ask.run_id, &sha, now)?;
        crate::lock(&self.approvals).insert(ask.run_id.clone(), Held { ask: ask.clone(), sha: sha.clone(), asked_at: secs(now), answer: None });
        self.write_approval(&ask.run_id, &json!({"state": "pending", "ask": ask.to_json(), "sha256": sha, "asked_at": secs(now)}))?;
        Ok(sha)
    }

    /// Void the run's approval (a redo, a new round, the run ended): it can never be consumed.
    #[allow(dead_code)] // the evidence ticket (runproto, after the run-protocol stack lands) is its first caller
    pub(crate) fn void_evidence_approval(&self, run_id: &str, why: &str) -> Result<(), String> {
        let _g = crate::lock(&LOCK);
        crate::lock(&self.approvals).remove(run_id);
        let Some(mut rec) = self.read_approval(run_id)? else { return Ok(()) };
        if rec["state"] != "consumed" {
            rec["state"] = json!("void");
            rec["why"] = json!(why);
            self.write_approval(run_id, &rec)?;
        }
        Ok(())
    }

    /// The human's answer. `sha` is the list the dialog showed: an answer to an older list is refused. Only an ask this
    /// app made and still holds can be answered.
    fn answer_evidence_approval(&self, run_id: &str, sha: &str, approve: bool, now: SystemTime) -> Res {
        if !crate::ids::valid_run_id(run_id) {
            return Err(invalid("run_id", "not a run id"));
        }
        let _g = crate::lock(&LOCK);
        let held = crate::lock(&self.approvals).get(run_id).cloned();
        let rec = self.read_approval(run_id).map_err(|e| fail("io_error", e))?;
        let (Some(held), Some(mut rec)) = (held, rec) else { return Err(fail("not_found", "this run isn't waiting for an approval")) };
        if held.answer.is_some() || rec["state"] != "pending" {
            return Err(fail("not_pending", "this approval was already answered or is void"));
        }
        if held.sha != sha || rec["sha256"].as_str() != Some(sha) {
            return Err(fail("list_changed", "the command list changed; review the new list"));
        }
        if expired(held.asked_at, now) {
            crate::lock(&self.approvals).remove(run_id);
            rec["state"] = json!("expired");
            self.write_approval(run_id, &rec).map_err(|e| fail("io_error", e))?;
            return Err(fail("expired", "the approval wait ran out; the run ends not verified"));
        }
        let what = if approve { "approved" } else { "declined" };
        self.audit_approval(if approve { "ALLOW" } else { "DENY" }, what, run_id, sha, now).map_err(|e| fail("audit_failed", e))?;
        if let Some(h) = crate::lock(&self.approvals).get_mut(run_id) {
            h.answer = Some(approve);
        }
        rec["state"] = json!(what);
        rec["answered_at"] = json!(secs(now));
        self.write_approval(run_id, &rec).map_err(|e| fail("io_error", e))?;
        Ok(json!({"run_id": run_id, "state": what, "sha256": sha}))
    }

    /// The evidence ticket uses the approval: only an answer a person gave in this app's lifetime, for exactly `ask`,
    /// within the hold, with the file agreeing, once. Anything else is refused (and voids it).
    #[allow(dead_code)] // the evidence ticket (runproto, after the run-protocol stack lands) is its first caller
    pub(crate) fn consume_evidence_approval(&self, ask: &Ask, now: SystemTime) -> Result<String, String> {
        ask.check()?;
        let _g = crate::lock(&LOCK);
        let sha = ask.sha256();
        let held = crate::lock(&self.approvals).get(&ask.run_id).cloned();
        let rec = self.read_approval(&ask.run_id)?;
        let refuse = |why: &str| -> Result<String, String> {
            crate::lock(&self.approvals).remove(&ask.run_id);
            if let Ok(Some(mut r)) = self.read_approval(&ask.run_id) {
                if r["state"] != "consumed" {
                    r["state"] = json!("void");
                    r["why"] = json!(why);
                    let _ = self.write_approval(&ask.run_id, &r);
                }
            }
            Err(why.to_string())
        };
        let Some(held) = held else { return refuse("no approval this app asked for and a person gave") };
        if held.answer != Some(true) {
            return Err(format!("the approval is {}", if held.answer == Some(false) { "declined" } else { "not answered" }));
        }
        if held.ask != *ask || held.sha != sha {
            return refuse("the commands, commit, round or seat changed after approval");
        }
        if expired(held.asked_at, now) {
            return refuse("the approval is older than the hold");
        }
        let Some(mut rec) = rec.filter(|r| r["state"] == "approved" && r["sha256"].as_str() == Some(sha.as_str())) else {
            return refuse("the approval record doesn't match the answer given");
        };
        self.audit_approval("ALLOW", "consumed", &ask.run_id, &sha, now)?;
        crate::lock(&self.approvals).remove(&ask.run_id);
        rec["state"] = json!("consumed");
        rec["consumed_at"] = json!(secs(now));
        self.write_approval(&ask.run_id, &rec)?;
        Ok(sha)
    }

    /// The run's approval, for the UI: the state, and while pending the exact list to show. A pending or approved
    /// record this app isn't holding (written by hand, or kept over a restart) reads `void`; past the hold, `expired`.
    pub fn evidence_approval(&self, run_id: &str) -> Res {
        self.evidence_approval_at(run_id, SystemTime::now())
    }

    fn evidence_approval_at(&self, run_id: &str, now: SystemTime) -> Res {
        if !crate::ids::valid_run_id(run_id) {
            return Err(invalid("run_id", "not a run id"));
        }
        let rec = self.read_approval(run_id).map_err(|e| fail("io_error", e))?;
        let Some(r) = rec else { return Ok(json!({"run_id": run_id, "state": "none"})) };
        let mut state = r["state"].as_str().unwrap_or("void").to_string();
        if state == "pending" || state == "approved" {
            let held = crate::lock(&self.approvals).get(run_id).cloned();
            match held {
                Some(h) if r["sha256"].as_str() == Some(h.sha.as_str()) => {
                    if expired(h.asked_at, now) {
                        state = "expired".into();
                    }
                }
                _ => state = "void".into(),
            }
        }
        Ok(json!({"run_id": run_id, "state": state, "sha256": r["sha256"], "ask": r["ask"], "asked_at": r["asked_at"], "hold_max_s": HOLD_MAX.as_secs()}))
    }

    /// The sha of the list this run's evidence ticket consumed, from the core's record. The proof also needs the
    /// signed progress line naming it (`progress_line`), so the file alone never makes a run read approved.
    pub(crate) fn evidence_approval_consumed(&self, run_id: &str) -> Option<String> {
        let r = self.read_approval(run_id).ok()??;
        (r["state"] == "consumed" && r["ask"]["run_id"] == run_id).then(|| r["sha256"].as_str().map(str::to_string)).flatten()
    }

    /// Approve or decline: a person's click in the app only.
    pub fn act_answer_evidence_approval(&self, env: &super::Envelope, run_id: &str, sha: &str, approve: bool) -> Value {
        let action = if approve { "approve_evidence" } else { "decline_evidence" };
        let target = format!("evidence:{run_id}");
        self.act(env, crate::control::receipt::Caller::Ui, true, action, &target, None::<u64>, || self.answer_evidence_approval(run_id, sha, approve, SystemTime::now()))
    }

    #[cfg(test)]
    pub(crate) fn answer_evidence_approval_at(&self, run_id: &str, sha: &str, approve: bool, now: SystemTime) -> Res {
        self.answer_evidence_approval(run_id, sha, approve, now)
    }

    #[cfg(test)]
    pub(crate) fn evidence_approval_read_at(&self, run_id: &str, now: SystemTime) -> Res {
        self.evidence_approval_at(run_id, now)
    }
}

#[cfg(test)]
#[path = "evidence_approval_tests.rs"]
mod tests;
