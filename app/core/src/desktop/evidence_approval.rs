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
//! - Each answer is an `audit.log` line; the evidence ticket writes the signed `progress` line (`progress_line`).
//!
//! The record is `<data>/control/lead/<run>/approval.json`, the core's own (private dir, atomic writes).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::{fail, invalid, App, Res};
use crate::control::{audit, fsx, sha256::hex_digest};

/// `evidence_hold_max_s`: how long an ask waits for its answer.
pub const HOLD_MAX: Duration = Duration::from_secs(24 * 60 * 60);
const FILE: &str = "approval.json";
/// At most this many commands, each at most this long (the list is shown whole, so it must fit on screen).
const MAX_COMMANDS: usize = 64;
const MAX_COMMAND: usize = 4096;

/// Read-modify-write of every approval record runs under this lock, so a consume can't race an answer.
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

impl Ask {
    fn check(&self) -> Result<(), String> {
        if !crate::ids::valid_run_id(&self.run_id) {
            return Err("not a run id".into());
        }
        if !(self.commit.len() == 40 && self.commit.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())) {
            return Err("the commit must be a full sha".into());
        }
        if self.seat.is_empty() || self.account.is_empty() {
            return Err("the evidence seat and account are required".into());
        }
        if self.commands.is_empty() || self.commands.len() > MAX_COMMANDS {
            return Err(format!("1 to {MAX_COMMANDS} commands"));
        }
        if self.commands.iter().any(|c| c.trim().is_empty() || c.len() > MAX_COMMAND || c.contains('\0')) {
            return Err("a command is empty, too long or has a NUL".into());
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

    fn from_json(v: &Value) -> Option<Self> {
        Some(Ask {
            run_id: v["run_id"].as_str()?.into(),
            round: u32::try_from(v["round"].as_u64()?).ok()?,
            commit: v["commit"].as_str()?.into(),
            seat: v["seat"].as_str()?.into(),
            account: v["account"].as_str()?.into(),
            commands: v["commands"].as_array()?.iter().map(|c| c.as_str().map(str::to_string)).collect::<Option<_>>()?,
        })
    }
}

/// The signed `progress` line the evidence ticket writes once it consumed the approval.
#[allow(dead_code)] // the evidence ticket (runproto, after the run-protocol stack lands) is its first caller
pub fn progress_line(sha: &str) -> String {
    format!("check commands approved by the user (sha {})", &sha[..sha.len().min(12)])
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
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

    fn audit_approval(&self, verdict: &str, what: &str, run_id: &str, sha: &str, now: SystemTime) {
        let line = format!("evidence-approval {what} run={run_id} sha={sha}");
        // The answer stands even if the log can't be appended (the record and the signed progress line carry it).
        let _ = audit::append_line(&self.home.join(".claude/vajra/audit.log"), verdict, &line, now);
    }

    /// The core asks (after the gate passes). Any earlier ask of this run, answered or not, is void: a redo or a new
    /// round asks again, and only the newest list can be approved. Returns the list's sha256.
    #[allow(dead_code)] // the evidence ticket (runproto, after the run-protocol stack lands) is its first caller
    pub(crate) fn request_evidence_approval(&self, ask: &Ask, now: SystemTime) -> Result<String, String> {
        ask.check()?;
        let _g = crate::lock(&LOCK);
        let sha = ask.sha256();
        self.write_approval(&ask.run_id, &json!({"state": "pending", "ask": ask.to_json(), "sha256": sha, "asked_at": secs(now)}))?;
        Ok(sha)
    }

    /// Void the run's approval (a redo, a new round, the run ended): it can never be consumed.
    #[allow(dead_code)] // the evidence ticket (runproto, after the run-protocol stack lands) is its first caller
    pub(crate) fn void_evidence_approval(&self, run_id: &str, why: &str) -> Result<(), String> {
        let _g = crate::lock(&LOCK);
        let Some(mut rec) = self.read_approval(run_id)? else { return Ok(()) };
        if rec["state"] != "consumed" {
            rec["state"] = json!("void");
            rec["why"] = json!(why);
            self.write_approval(run_id, &rec)?;
        }
        Ok(())
    }

    /// The human's answer. `sha` is the list the dialog showed: an answer to an older list is refused.
    fn answer_evidence_approval(&self, run_id: &str, sha: &str, approve: bool, now: SystemTime) -> Res {
        if !crate::ids::valid_run_id(run_id) {
            return Err(invalid("run_id", "not a run id"));
        }
        let _g = crate::lock(&LOCK);
        let mut rec = self.read_approval(run_id).map_err(|e| fail("io_error", e))?.ok_or_else(|| fail("not_found", "this run isn't waiting for an approval"))?;
        if rec["state"] != "pending" {
            return Err(fail("not_pending", "this approval was already answered or is void"));
        }
        if rec["sha256"].as_str() != Some(sha) {
            return Err(fail("list_changed", "the command list changed; review the new list"));
        }
        let asked = rec["asked_at"].as_u64().unwrap_or(0);
        if secs(now).saturating_sub(asked) > HOLD_MAX.as_secs() {
            rec["state"] = json!("expired");
            self.write_approval(run_id, &rec).map_err(|e| fail("io_error", e))?;
            return Err(fail("expired", "the approval wait ran out; the run ends not verified"));
        }
        rec["state"] = json!(if approve { "approved" } else { "declined" });
        rec["answered_at"] = json!(secs(now));
        self.write_approval(run_id, &rec).map_err(|e| fail("io_error", e))?;
        self.audit_approval(if approve { "ALLOW" } else { "DENY" }, if approve { "approved" } else { "declined" }, run_id, sha, now);
        Ok(json!({"run_id": run_id, "state": rec["state"], "sha256": sha}))
    }

    /// The evidence ticket uses the approval: only an approved record whose binding equals `ask` exactly, once. Any
    /// other state, or a different run, round, commit, seat, account or command, is refused (and voids it).
    #[allow(dead_code)] // the evidence ticket (runproto, after the run-protocol stack lands) is its first caller
    pub(crate) fn consume_evidence_approval(&self, ask: &Ask, now: SystemTime) -> Result<String, String> {
        ask.check()?;
        let _g = crate::lock(&LOCK);
        let mut rec = self.read_approval(&ask.run_id)?.ok_or("no approval for this run")?;
        let state = rec["state"].as_str().unwrap_or("").to_string();
        if state != "approved" {
            return Err(format!("the approval is {}", if state.is_empty() { "unreadable" } else { &state }));
        }
        let sha = ask.sha256();
        let bound = Ask::from_json(&rec["ask"]);
        if bound.as_ref() != Some(ask) || rec["sha256"].as_str() != Some(sha.as_str()) {
            rec["state"] = json!("void");
            rec["why"] = json!("the commands, commit, round or seat changed after approval");
            self.write_approval(&ask.run_id, &rec)?;
            return Err("the approval was for a different list".into());
        }
        rec["state"] = json!("consumed");
        rec["consumed_at"] = json!(secs(now));
        self.write_approval(&ask.run_id, &rec)?;
        self.audit_approval("ALLOW", "consumed", &ask.run_id, &sha, now);
        Ok(sha)
    }

    /// The run's approval, for the UI: the state, and while pending the exact list to show.
    pub fn evidence_approval(&self, run_id: &str) -> Res {
        if !crate::ids::valid_run_id(run_id) {
            return Err(invalid("run_id", "not a run id"));
        }
        let rec = self.read_approval(run_id).map_err(|e| fail("io_error", e))?;
        Ok(match rec {
            None => json!({"run_id": run_id, "state": "none"}),
            Some(r) => json!({"run_id": run_id, "state": r["state"], "sha256": r["sha256"], "ask": r["ask"], "asked_at": r["asked_at"], "hold_max_s": HOLD_MAX.as_secs()}),
        })
    }

    /// Whether this run's evidence ticket consumed its approval (the proof's `approval_consumed`).
    pub(crate) fn evidence_approval_consumed(&self, run_id: &str) -> bool {
        matches!(self.read_approval(run_id), Ok(Some(r)) if r["state"] == "consumed" && r["ask"]["run_id"] == run_id)
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
}

#[cfg(test)]
#[path = "evidence_approval_tests.rs"]
mod tests;
