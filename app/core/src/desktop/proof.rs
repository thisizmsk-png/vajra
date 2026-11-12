//! A run's proof (ADR-16 16.5a, 6i; owner D6): the run label, the signed record's check, and the export.
//!
//! The label is computed here, from the signed record and the core's own trust record, and every surface shows the same
//! one: the Run view, the criterion rows, the exported page and the PR comment. A run whose evidence ran outside the
//! guard is never labelled plain Verified, and nothing exported can look more trusted than the app shows.
//!
//! The record is HMAC-signed with a key on this Mac, so only this Mac can check it, and never claims that someone else
//! can. What the record note says about it lives in `record_note` alone (owner decision, key A): until agents can't
//! read the signing key, it says "Checked by Vajra"; `TAMPER_EVIDENT` flips it, and an outside-the-guard record never
//! gets the tamper-evident words either way.

use serde_json::{json, Value};

/// The marker an `outside_guard` run's signed goal starts with (16.5a).
pub const MARKER: &str = "[evidence ran outside the Vajra guard]";

/// Each label the run card can show. Matches on it are exhaustive, so a new one can't fall through to plain Verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    Verified,
    /// Verified, with the user's one-time approval of a non-Claude checker's commands (N20).
    VerifiedApproved,
    /// Every check passed, but the evidence ran outside the guard (N17): never shown, counted or filtered as Verified.
    EvidenceUnguarded,
    NotVerified,
    Rejected,
    Abandoned,
    InProgress,
}

impl Label {
    pub fn id(self) -> &'static str {
        match self {
            Label::Verified => "verified",
            Label::VerifiedApproved => "verified_approved",
            Label::EvidenceUnguarded => "evidence_unguarded",
            Label::NotVerified => "not_verified",
            Label::Rejected => "rejected",
            Label::Abandoned => "abandoned",
            Label::InProgress => "in_progress",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Label::Verified => "Verified",
            Label::VerifiedApproved => "Verified (check commands approved by you)",
            Label::EvidenceUnguarded => "Verified (evidence ran outside the guard)",
            Label::NotVerified => "Not verified",
            Label::Rejected => "Rejected by review",
            Label::Abandoned => "Abandoned",
            Label::InProgress => "In progress",
        }
    }

    /// The word a passing criterion row shows under this label.
    pub fn pass_text(self) -> &'static str {
        match self {
            Label::EvidenceUnguarded => "verified (evidence ran outside the guard)",
            Label::Verified | Label::VerifiedApproved => "pass",
            Label::NotVerified | Label::Rejected | Label::Abandoned | Label::InProgress => "pass, not verified",
        }
    }
}

/// How far the evidence can be trusted, the worse of every source wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Trust {
    Guarded,
    GuardedByApproval,
    OutsideGuard,
}

impl Trust {
    pub fn parse(s: &str) -> Option<Trust> {
        match s {
            "guarded" => Some(Trust::Guarded),
            "guarded_by_approval" => Some(Trust::GuardedByApproval),
            "outside_guard" => Some(Trust::OutsideGuard),
            _ => None,
        }
    }
}

/// What the label is computed from. Every field comes from the core: the signed campaign, its check, the evidence rows
/// and the guarded `seats.json`.
#[derive(Clone, Debug, Default)]
pub struct Input<'a> {
    pub slug: &'a str,
    pub goal: &'a str,
    /// The signed campaign's own status: `active`, `done`, `stopped`, `failed` or `abandoned`.
    pub status: &'a str,
    /// The record check: `ok`, `no_report`, `integrity_failed`, `not_text` or `unknown`.
    pub integrity: &'a str,
    /// The evidence row statuses (`pass`, `fail`, `stale`, ...), one per criterion.
    pub rows: Vec<&'a str>,
    /// The core's trust record: `seats.json`'s `evidence_trust`. `None` for a run started outside this app (today's
    /// workflow, whose check runs in a Claude session) and `Some("?")` for a record that can't be read.
    pub trust: Option<&'a str>,
    /// A `guarded_by_approval` run's approval was consumed by its evidence ticket (N20).
    pub approval_consumed: bool,
    /// A review rejected the approach.
    pub rejected: bool,
    /// The signed campaign's own `slug` field. The signature doesn't bind the folder name, so it must equal `slug`.
    pub slug_field: &'a str,
    /// The read-back campaign and task graph against the core's own write ledger (6h).
    pub ledger: Ledger,
}

/// The read-back record against the core's write ledger: any extra or changed entry is a mismatch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ledger {
    Match,
    Mismatch,
    /// No ledger: a record the core didn't write (today's workflow), or one from before the ledger existed.
    #[default]
    Missing,
}

/// The trust the signed record itself carries: the `-og` slug or the goal marker. When unsure, the label.
pub fn signed_trust(slug: &str, goal: &str) -> Trust {
    if slug.ends_with("-og") || goal.trim_start().starts_with(MARKER) {
        Trust::OutsideGuard
    } else {
        Trust::Guarded
    }
}

/// The label, with the reason shown under it.
pub fn label(i: &Input) -> (Label, String) {
    if i.integrity == "integrity_failed" {
        return (Label::NotVerified, "The signed record failed its check on this Mac.".into());
    }
    if i.slug_field != i.slug {
        return (Label::NotVerified, "The signed record names a different campaign than this run.".into());
    }
    if i.status == "abandoned" {
        return (Label::Abandoned, "The run was closed as abandoned.".into());
    }
    if i.rejected {
        return (Label::Rejected, "A review rejected the approach.".into());
    }
    if i.status == "active" || i.status.is_empty() {
        return (Label::InProgress, "The run has not finished.".into());
    }
    if i.status != "done" {
        return (Label::NotVerified, format!("The run ended {}.", i.status));
    }
    if i.integrity == "no_report" {
        return (Label::NotVerified, "No delivery report was recorded.".into());
    }
    if i.integrity != "ok" {
        return (Label::NotVerified, "The signed record could not be checked. Try again.".into());
    }
    if i.rows.is_empty() {
        return (Label::NotVerified, "The run has no acceptance criteria.".into());
    }
    let failing = i.rows.iter().filter(|s| **s != "pass").count();
    if failing > 0 {
        return (Label::NotVerified, format!("{failing} of {} criteria have no current passing evidence.", i.rows.len()));
    }
    match i.ledger {
        Ledger::Match => {}
        Ledger::Mismatch => return (Label::NotVerified, "The record has entries Vajra didn't write.".into()),
        Ledger::Missing => return (Label::NotVerified, "Vajra has no write ledger for this record, so it can't confirm it.".into()),
    }
    let own = match i.trust {
        None => Trust::Guarded,
        Some(t) => match Trust::parse(t) {
            Some(t) => t,
            None => return (Label::NotVerified, "The run's trust record can't be read.".into()),
        },
    };
    match own.max(signed_trust(i.slug, i.goal)) {
        Trust::Guarded => (Label::Verified, "Every criterion passed on the delivered commit.".into()),
        Trust::GuardedByApproval if i.approval_consumed => (Label::VerifiedApproved, "Every criterion passed. You approved the check commands for this run.".into()),
        Trust::GuardedByApproval => (Label::NotVerified, "The check commands were not approved for this run.".into()),
        Trust::OutsideGuard => (Label::EvidenceUnguarded, "Every criterion passed, but the evidence ran outside the guard.".into()),
    }
}

/// The one capability check for the export (D4: the signed shareable record is the planned paid feature). Proof on
/// screen stays free and never calls this.
pub fn export_allowed() -> bool {
    true
}

// ---------------------------------------------------------------- export

/// Owner decision, key A: on only once agents can't read the signing key (6k part 2 and the Claude worker sandbox).
/// This and `record_note` are the one place the record's wording lives.
pub const TAMPER_EVIDENT: bool = false;

/// What the record note says, for this label. An outside-the-guard record is never called tamper-evident.
pub fn record_note(label: Label) -> &'static str {
    match (label, TAMPER_EVIDENT) {
        (Label::EvidenceUnguarded, _) => "Checked by Vajra on this Mac. The evidence ran outside the guard, so this record isn't tamper-evident.",
        (_, true) => "Tamper-evident on your Mac: only the Mac that signed this record can check it.",
        (_, false) => "Checked by Vajra on this Mac. The signing key is guarded, not yet sandboxed.",
    }
}
pub const WHY_OUTSIDE: &str = crate::desktop::seat_start::OUTSIDE_GUARD_WHY;
pub const PLAIN_HINT: &str = "To get plain Verified, pick a Claude helper as the checker.";

/// One criterion as exported.
pub struct Row {
    pub criterion: String,
    pub task: String,
    pub status: String,
    pub commit: Option<String>,
    pub ran_by: Option<String>,
}

pub struct Proof {
    pub slug: String,
    pub goal: String,
    pub label: Label,
    pub reason: String,
    pub integrity: String,
    pub record_sha256: Option<String>,
    pub rows: Vec<Row>,
    pub report: Option<String>,
}

fn row_word(label: Label, status: &str) -> String {
    match status {
        "pass" => label.pass_text().into(),
        "fail" => "fail".into(),
        "stale" => "stale evidence".into(),
        "unverified" => "unverified evidence".into(),
        "checking" => "not checked yet".into(),
        _ => "not run".into(),
    }
}

fn record_line(p: &Proof) -> String {
    let check = match p.integrity.as_str() {
        "ok" | "no_report" | "not_text" => "the signature checked on this Mac",
        "integrity_failed" => "the signature failed its check on this Mac",
        _ => "the signature could not be checked",
    };
    match &p.record_sha256 {
        Some(h) => format!("Signed record: {check}. Record hash (sha256): {h}"),
        None => format!("Signed record: {check}."),
    }
}

/// Markdown table cell: one line, no pipes, no HTML, no `@mention` (a zero-width space after `@`) and no `[text](url)`
/// link, so a posted comment never pings anyone or links anywhere.
fn cell(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
        .replace('|', "\\|")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('@', "@\u{200B}")
}

/// The PR comment. Every value in it has passed `redact` first (see `App::run_proof`).
pub fn markdown(p: &Proof) -> String {
    let mut out = format!("## Vajra proof: {}\n\n**{}**. {}\n\n", cell(&p.goal), p.label.text(), cell(&p.reason));
    if p.label == Label::EvidenceUnguarded {
        out.push_str(&format!("> {WHY_OUTSIDE}\n\n"));
    }
    out.push_str("| Criterion | Task | Status | Commit | Checked by |\n|---|---|---|---|---|\n");
    for r in &p.rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            cell(&r.criterion),
            cell(&r.task),
            row_word(p.label, &r.status),
            r.commit.as_deref().map(|c| format!("`{}`", &c[..c.len().min(12)])).unwrap_or_default(),
            cell(r.ran_by.as_deref().unwrap_or("")),
        ));
    }
    out.push_str(&format!("\nCampaign `{}`. {}\n\n{}\n", cell(&p.slug), record_line(p), record_note(p.label)));
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// The self-contained page: inline styles only, no script, no external link or font.
pub fn page(p: &Proof) -> String {
    let rows: String = p
        .rows
        .iter()
        .map(|r| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
                esc(&r.criterion),
                esc(&r.task),
                esc(&row_word(p.label, &r.status)),
                esc(r.commit.as_deref().unwrap_or("")),
                esc(r.ran_by.as_deref().unwrap_or(""))
            )
        })
        .collect();
    let why = if p.label == Label::EvidenceUnguarded { format!("<p class=\"why\">{}</p>", esc(WHY_OUTSIDE)) } else { String::new() };
    let report = p.report.as_deref().map(|r| format!("<h2>Delivery report</h2><pre>{}</pre>", esc(r))).unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\"><title>Vajra proof: {title}</title><style>\
body{{font:14px/1.5 -apple-system,system-ui,sans-serif;max-width:960px;margin:32px auto;padding:0 16px}}\
table{{border-collapse:collapse;width:100%}}th,td{{text-align:left;padding:6px 8px;border-bottom:1px solid currentColor;vertical-align:top}}\
pre{{white-space:pre-wrap}}.why{{border-left:3px solid currentColor;padding-left:12px}}</style></head><body>\
<h1>{title}</h1><p><strong>{label}</strong>. {reason}</p>{why}\
<table><thead><tr><th>Criterion</th><th>Task</th><th>Status</th><th>Commit</th><th>Checked by</th></tr></thead><tbody>{rows}</tbody></table>\
<p>Campaign <code>{slug}</code>. {record}</p><p>{note}</p>{report}</body></html>\n",
        title = esc(&p.goal),
        label = esc(p.label.text()),
        reason = esc(&p.reason),
        slug = esc(&p.slug),
        record = esc(&record_line(p)),
        note = esc(record_note(p.label)),
    )
}

/// A GitHub pull request named by its URL: `https://github.com/<owner>/<repo>/pull/<n>`, nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrRef {
    pub repo: String,
    pub number: u64,
}

pub fn parse_pr(url: &str) -> Option<PrRef> {
    let rest = url.trim().strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    let [owner, repo, "pull", n] = parts.as_slice() else { return None };
    let name_ok = |s: &str| !s.is_empty() && s.len() <= 100 && !s.starts_with('.') && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    let digits = !n.is_empty() && n.len() <= 9 && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit());
    if !(digits && name_ok(owner) && name_ok(repo)) {
        return None;
    }
    Some(PrRef { repo: format!("{owner}/{repo}"), number: n.parse().ok()? })
}

pub fn proof_json(p: &Proof, markdown_text: &str) -> Value {
    json!({
        "slug": p.slug,
        "goal": p.goal,
        "label": p.label.id(),
        "label_text": p.label.text(),
        "pass_text": p.label.pass_text(),
        "reason": p.reason,
        "why": (p.label == Label::EvidenceUnguarded).then_some(WHY_OUTSIDE),
        "hint": (p.label == Label::EvidenceUnguarded).then_some(PLAIN_HINT),
        "integrity": p.integrity,
        "record_sha256": p.record_sha256,
        "record_note": record_note(p.label),
        "export": export_allowed(),
        "markdown": markdown_text,
        // A row whose check hasn't finished: the UI asks again (bounded), since a finished sweep sends no event.
        "pending": p.rows.iter().any(|r| r.status == "checking"),
    })
}

#[cfg(test)]
#[path = "proof_tests.rs"]
mod tests;
