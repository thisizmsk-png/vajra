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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::{fail, invalid, App, Res};
use crate::adapter::evidence_runner::{self as runner, EvidenceRun, Profile, RunOutcome};
use crate::control::{audit, fsx, sha256::hex_digest};
use crate::vajra::bridge::Bridge;

/// `evidence_hold_max_s`: how long an ask waits for its answer, and how long an approval stays usable.
pub const HOLD_MAX: Duration = Duration::from_secs(24 * 60 * 60);
const FILE: &str = "approval.json";
/// At most this many commands, each at most this long (the list is shown whole, so it must fit on screen).
const MAX_COMMANDS: usize = 64;
const MAX_COMMAND: usize = 4096;
/// The longest the check that the worktree is exactly the approved commit may take (its tree is computed with git).
const TREE_TIMEOUT: Duration = Duration::from_secs(60);

/// File read-modify-writes run under this lock, so a consume can't race an answer.
static LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
thread_local! {
    /// Tests only: the next record write on this thread fails (a full disk, a lost mount).
    pub(crate) static FAIL_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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
    /// The runner's network setting for these commands (the verifier seat's, `Profile::network`): shown with the list
    /// and bound into its sha, so an approval given with the network off can't be used by a run with it on (PR 247).
    pub network: bool,
    /// The run's repository: its git common dir, set by the core from the run's own record when it asks (codex P1 on
    /// #249). Compared in memory when the approval is used, and with the run's git common dir before anything runs, so
    /// an approval asked for one repository can't run its commands in another at the same commit. Not in the sha or
    /// the dialog's record: the in-memory hold is what binds it.
    pub repo: PathBuf,
}

/// An ask this app made, and a person's answer to it (held in memory only).
#[derive(Clone, Debug)]
pub(crate) struct Held {
    ask: Ask,
    sha: String,
    asked_at: u64,
    answer: Option<bool>,
    /// Set when this run's evidence ticket used it: kept (not removed) so the proof can require it (Opus on #226).
    consumed: bool,
}

/// The app's approvals, as a handle the evidence ticket can hold on its round thread (6i): the same held answers
/// (`App::approvals`, shared, never copied), the app's data dir for the record and its home for the audit log. The
/// app's own dialog and proof go through `App`, which hands out this handle.
#[derive(Clone)]
pub struct ApprovalDesk {
    data: PathBuf,
    home: PathBuf,
    bridge: Bridge,
    approvals: Arc<Mutex<HashMap<String, Held>>>,
}

/// A character that can make a command show differently from what runs (Opus on #226): any control character but tab
/// (CR, ESC and the other C0 and C1 codes), and every Unicode Default_Ignorable_Code_Point (DerivedCoreProperties.txt,
/// Unicode 16): the bidi controls, zero-width and joiner characters, soft hyphen, the combining grapheme joiner,
/// variation selectors, Hangul fillers, tag characters and the reserved ignorable ranges. A renderer may draw none of
/// these, so none can reach the dialog. Also the line and paragraph separators U+2028 and U+2029 (bhishma on #226):
/// they are neither, but a renderer breaks the line at them, so one command could show as two.
fn hides_text(c: char) -> bool {
    (c.is_control() && c != '\t') || matches!(c, '\u{2028}' | '\u{2029}') || default_ignorable(c) || hides_or_overlays(c)
}

/// bhishma L2 on PR 247: what a renderer may draw as nothing, as a private glyph, or over its neighbour (Unicode
/// 16.0). Every format character (Cf) that isn't default-ignorable; all private use (Co); the noncharacters; and the
/// combining marks that overlay or enclose the character before them (canonical combining class 1, and Me). And every
/// code point unassigned in the Unicode data the core is built with (Cn, bhishma L1 on the wiring): one assigned later
/// might draw as nothing.
fn hides_or_overlays(c: char) -> bool {
    use icu_properties::{props::GeneralCategory, CodePointMapData};
    let u = c as u32;
    CodePointMapData::<GeneralCategory>::new().get(c) == GeneralCategory::Unassigned
        || (u & 0xFFFE) == 0xFFFE
        || matches!(
            c,
            // Cf, not default-ignorable
            '\u{0600}'..='\u{0605}' | '\u{06DD}' | '\u{070F}' | '\u{0890}'..='\u{0891}' | '\u{08E2}'
                | '\u{FFF9}'..='\u{FFFB}' | '\u{110BD}' | '\u{110CD}' | '\u{13430}'..='\u{1343F}'
                // Co
                | '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}'
                // noncharacters (and U+nFFFE, U+nFFFF above)
                | '\u{FDD0}'..='\u{FDEF}'
                // overlay marks (ccc 1)
                | '\u{0334}'..='\u{0338}' | '\u{1CD4}' | '\u{1CE2}'..='\u{1CE8}' | '\u{20D2}'..='\u{20D3}'
                | '\u{20D8}'..='\u{20DA}' | '\u{20E5}'..='\u{20E6}' | '\u{20EA}'..='\u{20EB}' | '\u{10A39}'
                | '\u{16AF0}'..='\u{16AF4}' | '\u{1BC9E}' | '\u{1D167}'..='\u{1D169}'
                // enclosing marks (Me)
                | '\u{0488}'..='\u{0489}' | '\u{1ABE}' | '\u{20DD}'..='\u{20E0}' | '\u{20E2}'..='\u{20E4}' | '\u{A670}'..='\u{A672}'
        )
}

/// Unicode's Default_Ignorable_Code_Point property, in full (Unicode 16.0 DerivedCoreProperties.txt).
fn default_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

impl Ask {
    pub(crate) fn check(&self) -> Result<(), String> {
        if !crate::ids::valid_run_id(&self.run_id) {
            return Err("not a run id".into());
        }
        if !(self.commit.len() == 40 && self.commit.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())) {
            return Err("the commit must be a full sha".into());
        }
        if self.seat.is_empty() || self.account.is_empty() || [&self.seat, &self.account].iter().any(|s| s.chars().any(hides_text)) {
            return Err("the evidence seat and account are required, as plain text".into());
        }
        if !self.repo.is_absolute() {
            return Err("the run's repository is required, as an absolute path".into());
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
        json!({"run_id": self.run_id, "round": self.round, "commit": self.commit, "seat": self.seat, "account": self.account, "commands": self.commands, "network": self.network})
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

/// The file agrees with what this app holds: its whole ask equals the held ask, and its sha is the held sha (codex on
/// #226: an agent can edit the file's commands and keep its sha). The file is never the source of what is shown or
/// approved; this only decides whether it has been tampered with.
fn file_matches(rec: &Value, held: &Held) -> bool {
    rec["ask"] == held.ask.to_json() && rec["sha256"].as_str() == Some(held.sha.as_str()) && held.ask.sha256() == held.sha
}

/// Why an audited change didn't happen: the opening audit line couldn't be written, or the change itself failed.
enum AuditedErr {
    Audit(String),
    Change(String),
}

impl AuditedErr {
    fn text(self) -> String {
        match self {
            AuditedErr::Audit(e) | AuditedErr::Change(e) => e,
        }
    }
}

fn expired(asked_at: u64, now: SystemTime) -> bool {
    secs(now).saturating_sub(asked_at) > HOLD_MAX.as_secs()
}

impl App {
    /// The handle the evidence ticket uses: it shares this app's held answers.
    pub fn approval_desk(&self) -> ApprovalDesk {
        ApprovalDesk { data: self.data.clone(), home: self.home.clone(), bridge: self.bridge.clone(), approvals: Arc::clone(&self.approvals) }
    }

    /// The run's approval, for the UI (`ApprovalDesk::evidence_approval`).
    pub fn evidence_approval(&self, run_id: &str) -> Res {
        self.approval_desk().evidence_approval(run_id)
    }

    /// Approve or decline: a person's click in the app only.
    pub fn act_answer_evidence_approval(&self, env: &super::Envelope, run_id: &str, sha: &str, approve: bool) -> Value {
        let action = if approve { "approve_evidence" } else { "decline_evidence" };
        let target = format!("evidence:{run_id}");
        let desk = self.approval_desk();
        self.act(env, crate::control::receipt::Caller::Ui, true, action, &target, None::<u64>, || desk.answer_evidence_approval(run_id, sha, approve, SystemTime::now()))
    }

    pub(crate) fn evidence_approval_consumed(&self, run_id: &str) -> Option<String> {
        self.approval_desk().evidence_approval_consumed(run_id)
    }

    pub(crate) fn evidence_approval_unheld(&self, run_id: &str) -> bool {
        self.approval_desk().evidence_approval_unheld(run_id)
    }

    #[cfg(test)]
    pub(crate) fn request_evidence_approval(&self, ask: &Ask, now: SystemTime) -> Result<String, String> {
        self.approval_desk().request_evidence_approval(ask, now)
    }

    #[cfg(test)]
    pub(crate) fn void_evidence_approval(&self, run_id: &str, why: &str) -> Result<(), String> {
        self.approval_desk().void_evidence_approval(run_id, why)
    }

    #[cfg(test)]
    pub(crate) fn consume_evidence_approval(&self, ask: &Ask, now: SystemTime) -> Result<String, String> {
        self.approval_desk().consume_evidence_approval(ask, now)
    }

    #[cfg(test)]
    pub(crate) fn run_approved_evidence_with<F>(&self, ask: Ask, profile: &Profile, run: &EvidenceRun, now: SystemTime, exec: F) -> Result<(String, RunOutcome), String>
    where
        F: FnOnce(&Profile, &EvidenceRun) -> RunOutcome,
    {
        self.approval_desk().run_approved_evidence_with(ask, profile, run, now, exec)
    }

    #[cfg(test)]
    pub(crate) fn answer_evidence_approval_at(&self, run_id: &str, sha: &str, approve: bool, now: SystemTime) -> Res {
        self.approval_desk().answer_evidence_approval(run_id, sha, approve, now)
    }

    #[cfg(test)]
    pub(crate) fn evidence_approval_read_at(&self, run_id: &str, now: SystemTime) -> Res {
        self.approval_desk().evidence_approval_at(run_id, now)
    }
}

impl ApprovalDesk {
    /// The evidence ticket's ask and wait (6i): asks for exactly `ask`, then waits for the person's answer while `live`
    /// says the run goes on. `Ok` only once they approved this list. A decline, no answer within the hold, a void or
    /// another list is `Err` with the reason; so is the run's end, which voids the ask first.
    pub(crate) fn await_answer(&self, ask: &Ask, live: &dyn Fn() -> bool, poll: Duration) -> Result<(), String> {
        let asked = self.request_evidence_approval(ask, SystemTime::now())?;
        loop {
            if !live() {
                let _ = self.void_evidence_approval(&ask.run_id, "the run ended");
                return Err("the run ended before you answered".into());
            }
            // Under the record lock: an answer writes the file before it sets the held answer, and a read between the
            // two would see an approved file with no held answer, which reads as expired.
            let state = {
                let _g = crate::lock(&LOCK);
                self.evidence_approval_at(&ask.run_id, SystemTime::now())
            }
            .map_err(|e| e["reason"].as_str().unwrap_or("the approval can't be read").to_string())?;
            if state["sha256"].as_str() != Some(asked.as_str()) {
                return Err("the approval is for another list".into());
            }
            match state["state"].as_str() {
                Some("approved") => return Ok(()),
                Some("pending") => std::thread::sleep(poll),
                Some("declined") => return Err("you declined the check commands".into()),
                Some("expired") => return Err(format!("no answer came within {} hours", HOLD_MAX.as_secs() / 3600)),
                _ => return Err("the approval was voided".into()),
            }
        }
    }

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
        #[cfg(test)]
        if FAIL_WRITE.with(|f| f.replace(false)) {
            return Err("injected write failure".into());
        }
        let dir = self.data.join("control/lead").join(run_id);
        fsx::private_dir(&self.data.join("control")).and_then(|_| fsx::private_dir(&self.data.join("control/lead"))).and_then(|_| fsx::private_dir(&dir)).map_err(|e| e.to_string())?;
        fsx::write_atomic(&dir, FILE, v.to_string().as_bytes()).map(drop).map_err(|e| e.to_string())
    }

    /// The audit line, before the state it records changes: no line, no change (fail closed).
    fn audit_approval(&self, verdict: &str, what: &str, run_id: &str, sha: &str, now: SystemTime) -> Result<(), String> {
        let line = format!("evidence-approval {what} run={run_id} sha={sha}");
        audit::append_line(&self.home.join(".claude/vajra/audit.log"), verdict, &line, now).map_err(|_| "Vajra couldn't write this to its audit log, so nothing was changed. Check that the Vajra folder is writable, then try again.".to_string())
    }

    /// One state change, logged truthfully (codex on #226): an `<what>-attempt` line first (no line, no change), then
    /// the change, then `<what>` once it is durable, or `<what>-failed` when it isn't. The log never says a change
    /// happened that didn't. If the closing line can't be written, the attempt line still stands for it.
    fn audited<T>(&self, verdict: &str, what: &str, run_id: &str, sha: &str, now: SystemTime, change: impl FnOnce() -> Result<T, String>) -> Result<T, AuditedErr> {
        self.audit_approval(verdict, &format!("{what}-attempt"), run_id, sha, now).map_err(AuditedErr::Audit)?;
        match change() {
            Ok(v) => {
                let _ = self.audit_approval(verdict, what, run_id, sha, now);
                Ok(v)
            }
            Err(e) => {
                let _ = self.audit_approval("DENY", &format!("{what}-failed"), run_id, sha, now);
                Err(AuditedErr::Change(e))
            }
        }
    }

    /// The core asks (after the gate passes). Any earlier ask of this run, answered or not, is void: a redo or a new
    /// round asks again, and only the newest list can be approved. Returns the list's sha256.
    pub(crate) fn request_evidence_approval(&self, ask: &Ask, now: SystemTime) -> Result<String, String> {
        ask.check()?;
        let _g = crate::lock(&LOCK);
        let sha = ask.sha256();
        let held = Held { ask: ask.clone(), sha: sha.clone(), asked_at: secs(now), answer: None, consumed: false };
        let rec = json!({"state": "pending", "ask": ask.to_json(), "sha256": sha, "asked_at": secs(now)});
        // The file first, then memory: a failed write leaves the earlier ask (if any) as it was.
        self.audited("ALLOW", "asked", &ask.run_id, &sha, now, || self.write_approval(&ask.run_id, &rec)).map_err(AuditedErr::text)?;
        crate::lock(&self.approvals).insert(ask.run_id.clone(), held);
        Ok(sha)
    }

    /// Void the run's approval (a redo, a new round, the run ended): it can never be consumed.
    pub(crate) fn void_evidence_approval(&self, run_id: &str, why: &str) -> Result<(), String> {
        let _g = crate::lock(&LOCK);
        // A consumed approval stays consumed (in memory too, as in the file); anything else is dropped.
        let mut held = crate::lock(&self.approvals);
        if held.get(run_id).is_some_and(|h| !h.consumed) {
            held.remove(run_id);
        }
        drop(held);
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
    pub(crate) fn answer_evidence_approval(&self, run_id: &str, sha: &str, approve: bool, now: SystemTime) -> Res {
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
        if held.sha != sha {
            return Err(fail("list_changed", "the command list changed; review the new list"));
        }
        if !file_matches(&rec, &held) {
            crate::lock(&self.approvals).remove(run_id);
            rec["state"] = json!("void");
            rec["why"] = json!("the approval record was changed outside Vajra");
            self.write_approval(run_id, &rec).map_err(|e| fail("io_error", e))?;
            return Err(fail("record_changed", "This approval's record was changed outside Vajra, so it can't be trusted. Vajra will ask again."));
        }
        if expired(held.asked_at, now) {
            crate::lock(&self.approvals).remove(run_id);
            rec["state"] = json!("expired");
            self.write_approval(run_id, &rec).map_err(|e| fail("io_error", e))?;
            return Err(fail("expired", "the approval wait ran out; the run ends not verified"));
        }
        let what = if approve { "approved" } else { "declined" };
        rec["state"] = json!(what);
        rec["answered_at"] = json!(secs(now));
        // The file first, memory only once it's written: a failed write leaves both pending, so the person can retry.
        self.audited(if approve { "ALLOW" } else { "DENY" }, what, run_id, sha, now, || self.write_approval(run_id, &rec)).map_err(|e| match e {
            AuditedErr::Audit(e) => fail("audit_failed", e),
            AuditedErr::Change(e) => fail("io_error", e),
        })?;
        if let Some(h) = crate::lock(&self.approvals).get_mut(run_id) {
            h.answer = Some(approve);
        }
        Ok(json!({"run_id": run_id, "state": what, "sha256": sha}))
    }

    /// The evidence ticket uses the approval: only an answer a person gave in this app's lifetime, for exactly `ask`,
    /// within the hold, with the file agreeing, once. Anything else is refused (and voids it).
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
        if held.consumed {
            return Err("the approval was already used".into());
        }
        if held.answer != Some(true) {
            return Err(format!("the approval is {}", if held.answer == Some(false) { "declined" } else { "not answered" }));
        }
        if held.ask != *ask || held.sha != sha {
            return refuse("the commands, commit, round, seat or repository changed after approval");
        }
        if expired(held.asked_at, now) {
            return refuse("the approval is older than the hold");
        }
        let Some(mut rec) = rec.filter(|r| r["state"] == "approved" && file_matches(r, &held)) else {
            return refuse("the approval record doesn't match the answer given");
        };
        rec["state"] = json!("consumed");
        rec["consumed_at"] = json!(secs(now));
        // The file first, memory only once it's written: a failed write leaves both approved, so the ticket can retry;
        // the log then reads consumed-attempt, consumed-failed, never consumed.
        self.audited("ALLOW", "consumed", &ask.run_id, &sha, now, || self.write_approval(&ask.run_id, &rec)).map_err(AuditedErr::text)?;
        if let Some(h) = crate::lock(&self.approvals).get_mut(&ask.run_id) {
            h.consumed = true;
        }
        Ok(sha)
    }

    /// Runs a `guarded_by_approval` run's check commands, only as approved (bhishma L1 on PR 247). The ask's network
    /// is set from `profile`, the one the runner really launches with, so an approval given with the network off can't
    /// be used by a run with it on. The approval is then used up for that exact ask, and the commands that run are
    /// `ask.commands`, the very strings the approval's sha covers: `run.commands` is ignored, and nothing is re-read
    /// from a file or an agent between the check and the run. Any refusal runs nothing. Returns the approval's sha
    /// (for the signed progress line) and the run's outcome. Its caller is the evidence ticket (`delivery`, 6i).
    pub(crate) fn run_approved_evidence(&self, ask: Ask, profile: &Profile, run: &EvidenceRun, now: SystemTime) -> Result<(String, RunOutcome), String> {
        self.run_approved_evidence_with(ask, profile, run, now, runner::run)
    }

    /// `run_approved_evidence` with the runner passed in, so tests see exactly what would run.
    pub(crate) fn run_approved_evidence_with<F>(&self, mut ask: Ask, profile: &Profile, run: &EvidenceRun, now: SystemTime, exec: F) -> Result<(String, RunOutcome), String>
    where
        F: FnOnce(&Profile, &EvidenceRun) -> RunOutcome,
    {
        ask.network = profile.network;
        // Where the commands run must be what was approved (bhishma M1 on the wiring), checked before the approval is
        // used, so a mismatch runs nothing and leaves it unused: the slug is this run's; the worktree is exactly
        // ask.commit, its HEAD and every file (`at_approved_commit`).
        if super::proof_app::run_of(run.slug) != Some(ask.run_id.as_str()) {
            return Err("the evidence folder belongs to another run".into());
        }
        // The repository is the one the approval was asked for (codex P1 on #249): the held ask's `repo` must equal
        // `ask.repo` (`consume_evidence_approval`), and that must be the run's git common dir, which the worktree is
        // then proved to belong to (`at_approved_commit`). A clone at the same commit is a different folder.
        let same = matches!((std::fs::canonicalize(&ask.repo), std::fs::canonicalize(run.git_common)), (Ok(a), Ok(b)) if a == b);
        if !same {
            return Err("the run's repository isn't the one the approval was asked for".into());
        }
        let at_commit = || self.at_approved_commit(&profile.worktree, run.git_common, run.base, run.slug, &ask.commit, &ask.commands).map(Some);
        at_commit()?;
        let sha = self.consume_evidence_approval(&ask, now)?;
        // Checked again by the runner immediately before it spawns (bhishma L1 on 07a0ee4): a change made after the check
        // above, by anything outside the sandbox, runs nothing. That check's account of the ignored files goes into the
        // signed bundle (bhishma M2 on 7c09e27).
        let approved = EvidenceRun { commands: &ask.commands, at_spawn: Some(runner::AtSpawn(&at_commit)), ..run.clone() };
        Ok((sha, exec(profile, &approved)))
    }

    /// The worktree's files are the approved `commit`'s (bhishma M1 on the wiring, L1 on 07a0ee4, M1 to M3 on
    /// 7c09e27). What is bound, and what isn't:
    /// - **HEAD** is `commit`, read through the pinned walk in the run's git common dir `common` (which proves that dir
    ///   is its own).
    /// - **Every tracked and untracked file** matches: the worktree's tree equals `commit`'s. The worktree's tree is
    ///   the one `vajra-verify-work.sh` records as `git.worktreeTree` (the bridge's `tree_id`: the repository's config
    ///   read once, no filter, hook or fsmonitor ever run). `commit`'s tree is read from `common` alone, with replace
    ///   objects off, from a commit object whose own hash is `commit` (`Bridge::commit_tree`), so neither a planted
    ///   `.git` nor `refs/replace` can choose it. A dirty worktree is refused rather than run and labelled: the person
    ///   approved the commands for that commit.
    /// - **The walkthrough folder** `base` is left out of the tree, as verify-work does, so inside the worktree it must
    ///   hold nothing but this run's empty `slug` folder (`runner::base_is_clear`); a commit with files there is refused.
    /// - **Ignored files are not bound.** The ignore rules keep installed dependencies, build output and `.env` files
    ///   out of the tree. Their listing's count and hash are returned for the signed bundle, and an ignored file a test
    ///   runner would collect (`bridge::TEST_LIKE`) is refused.
    ///
    /// Returns the bundle's `core` field: the bound tree and the ignored listing.
    #[allow(clippy::too_many_arguments)]
    fn at_approved_commit(&self, worktree: &Path, common: &Path, base: &Path, slug: &str, commit: &str, commands: &[String]) -> Result<Value, String> {
        let head = runner::worktree_head(worktree, common).map_err(String::from)?;
        if head != commit {
            return Err("the worktree isn't at the approved commit".into());
        }
        runner::base_is_clear(base, worktree, slug).map_err(String::from)?;
        let mut bridge = self.bridge.clone();
        bridge.timeout = TREE_TIMEOUT;
        let want = bridge.commit_tree(common, commit).ok_or("the approved commit's tree can't be read from the repository's own git folder")?;
        // The tree records a submodule as its commit alone, and neither the tree nor the ignored listing looks inside
        // its folder, so a file planted there would run unseen (owner review on #256). Refused, not labelled.
        let links = bridge.gitlinks(common, &want).ok_or("the approved commit's submodules can't be listed from the repository's own git folder")?;
        if let Some(first) = links.first() {
            let more = if links.len() > 1 { format!(" and {} more", links.len() - 1) } else { String::new() };
            return Err(format!(
                "the approved commit has a submodule ({}{more}); files inside a submodule's folder can't be compared with the approved commit, so the checks don't run in a repository with submodules",
                super::plan::one_line(first, 200)
            ));
        }
        if let Ok(rel) = base.strip_prefix(worktree) {
            let rel = rel.to_str().ok_or("the walkthrough folder's name isn't plain text")?;
            if !rel.is_empty() && bridge.tree_has(common, &want, rel) != Some(false) {
                return Err("the approved commit has files in the walkthrough folder, so its tree can't be compared".into());
            }
        }
        let base_str = base.to_string_lossy();
        let snap = bridge.worktree_snapshot(worktree, &[base_str.as_ref()], commands).ok_or("the worktree's files couldn't be read as one tree (the repository's config defines a filter, or git took too long)")?;
        if snap.tree != want {
            return Err("the worktree has changes that aren't in the approved commit".into());
        }
        if let Some(t) = snap.test_like.first() {
            let more = if snap.test_like.len() > 1 { format!(" and {} more", snap.test_like.len() - 1) } else { String::new() };
            return Err(format!("an ignored file a test runner would collect is in the worktree ({}{more}), and it isn't part of the approved commit", super::plan::one_line(t, 200)));
        }
        Ok(json!({
            "approvedTree": want,
            "bound": "tracked and untracked files; ignored files are not bound",
            "ignored": {"count": snap.ignored.len(), "sha256": snap.ignored_sha256, "listing": "git ls-files --others --ignored --exclude-standard --directory"},
        }))
    }

    /// The run's approval, for the UI: the state, and while pending the exact list to show. A pending or approved
    /// record this app isn't holding (written by hand, or kept over a restart) reads `void`; past the hold, `expired`.
    pub fn evidence_approval(&self, run_id: &str) -> Res {
        self.evidence_approval_at(run_id, SystemTime::now())
    }

    pub(crate) fn evidence_approval_at(&self, run_id: &str, now: SystemTime) -> Res {
        if !crate::ids::valid_run_id(run_id) {
            return Err(invalid("run_id", "not a run id"));
        }
        let rec = self.read_approval(run_id).map_err(|e| fail("io_error", e))?;
        let held = crate::lock(&self.approvals).get(run_id).cloned();
        // What is shown always comes from memory (codex on #226): the file says only whether it still agrees. A record
        // with no held ask (hand-written, or kept over a restart) or one that differs from it shows no list at all.
        let Some(h) = held else {
            return Ok(match rec {
                None => json!({"run_id": run_id, "state": "none"}),
                Some(r) => {
                    let s = r["state"].as_str().unwrap_or("void");
                    json!({"run_id": run_id, "state": if matches!(s, "declined" | "expired" | "void") { s } else { "void" }, "hold_max_s": HOLD_MAX.as_secs()})
                }
            });
        };
        let file_state = rec.as_ref().filter(|r| file_matches(r, &h)).and_then(|r| r["state"].as_str().map(str::to_string));
        let state = match (file_state.as_deref(), h.answer, h.consumed) {
            (Some("consumed"), Some(true), true) => "consumed",
            (Some("approved"), Some(true), false) if !expired(h.asked_at, now) => "approved",
            (Some("pending"), None, false) if !expired(h.asked_at, now) => "pending",
            (Some("pending" | "approved"), _, false) => "expired",
            (Some("declined"), Some(false), _) => "declined",
            _ => "void",
        };
        Ok(json!({"run_id": run_id, "state": state, "sha256": h.sha, "ask": h.ask.to_json(), "asked_at": h.asked_at, "hold_max_s": HOLD_MAX.as_secs()}))
    }

    /// The sha of the list this run's evidence ticket consumed: only when this app made the ask, a person approved it
    /// and the ticket consumed it in this app's lifetime (held in memory), and the core's record agrees. The proof also
    /// needs the signed progress line naming it (`progress_line`). The file and the line can both be written by an agent
    /// before key A, so neither counts without the held entry; after a restart nothing is held, and a run reads not
    /// verified (Opus on #226).
    pub(crate) fn evidence_approval_consumed(&self, run_id: &str) -> Option<String> {
        let held = crate::lock(&self.approvals).get(run_id).cloned()?;
        if !(held.consumed && held.answer == Some(true) && held.ask.run_id == run_id) {
            return None;
        }
        let r = self.read_approval(run_id).ok()??;
        (r["state"] == "consumed" && file_matches(&r, &held)).then_some(held.sha)
    }

    /// Whether this run's record says its approval was used, but this app holds no such answer: what a restart leaves
    /// behind. It only picks the reason shown under a Not verified label (the label is the same either way), so a
    /// record written by hand can change that sentence and nothing else.
    pub(crate) fn evidence_approval_unheld(&self, run_id: &str) -> bool {
        if crate::lock(&self.approvals).get(run_id).is_some_and(|h| h.consumed) {
            return false;
        }
        self.read_approval(run_id).ok().flatten().is_some_and(|r| r["state"] == "consumed")
    }
}

#[cfg(test)]
#[path = "evidence_approval_tests.rs"]
mod tests;
