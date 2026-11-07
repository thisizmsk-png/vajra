//! "Ask the Lead" (ADR-13, option C): one headless `claude -p --resume <lead_sid>` turn per question, the framed
//! question on stdin, the reply read from stream-json.
//!
//! This file holds everything that does not need the app: the ask frame and its sanitiser, the ULID, the Lead's
//! generated `--settings` (guard hooks plus `//abs` deny rules), the `system/init` scan, and the process driver.
//! `desktop/lead.rs` wires it to the ledger, the bus and the three commands.

use std::fmt;

use super::{nonce, time};
use crate::crew::redact::Redactor;
use crate::ids;

pub use super::lead_settings::{deny_rules, settings, settings_no_hooks};

/// Ask text length, in chars (ADR-13 Decision 3). Same bound as a worker message.
pub const MAX_TEXT: usize = super::service::MAX_TEXT;

/// Why an ask did not run. `code` is what the UI sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeadError {
    /// A field failed validation; the text says which.
    Invalid(String),
    /// The run is not one this app launched or resumed (S3 shows the input disabled).
    ReadOnly(&'static str),
    Busy,
    /// No Lead is spawned without the Vajra guard (ADR-13 2a). The text says which precondition failed.
    GuardMissing(String),
    ClaudeNotFound,
    NoInit,
    ToolsMismatch,
    ModeMismatch,
    Timeout,
    Cancelled,
    Io(String),
}

impl LeadError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_args",
            Self::ReadOnly(_) => "read_only",
            Self::Busy => "lead_busy",
            Self::GuardMissing(_) => "lead_guard_missing",
            Self::ClaudeNotFound => "claude_not_found",
            Self::NoInit => "lead_no_init",
            Self::ToolsMismatch => "lead_tools_mismatch",
            Self::ModeMismatch => "lead_mode_mismatch",
            Self::Timeout => "lead_timeout",
            Self::Cancelled => "cancelled",
            Self::Io(_) => "io_error",
        }
    }

    /// The detail shown next to the code. Never a path: an I/O error text can carry one.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Invalid(d) | Self::GuardMissing(d) => Some(d.clone()),
            Self::ReadOnly(d) => Some((*d).to_string()),
            _ => None,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self.detail() {
            Some(d) => serde_json::json!({"error": self.code(), "detail": d}),
            None => serde_json::json!({"error": self.code()}),
        }
    }
}

impl fmt::Display for LeadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())?;
        if let Some(d) = self.detail() {
            write!(f, ": {d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LeadError {}

impl From<std::io::Error> for LeadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.kind().to_string())
    }
}

// ------------------------------------------------------------ ask id

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// `^[0-9A-HJKMNP-TV-Z]{26}$`: the only shape an ask id may have in the frame header.
pub fn valid_ulid(s: &str) -> bool {
    s.len() == 26 && s.bytes().all(|b| CROCKFORD.contains(&b))
}

/// A ULID: 48 bits of milliseconds, 80 random bits, Crockford base32 (time-ordered like an action id).
pub fn new_ulid(now: std::time::SystemTime) -> std::io::Result<String> {
    ulid_from(time::unix_ms(now), &nonce::random_hex(10)?)
}

fn ulid_from(ms: u64, rand_hex: &str) -> std::io::Result<String> {
    let mut v: u128 = (u128::from(ms) & 0xFFFF_FFFF_FFFF) << 80;
    for (i, c) in rand_hex.bytes().take(20).enumerate() {
        let d = (c as char).to_digit(16).ok_or_else(|| std::io::Error::other("hex"))?;
        v |= u128::from(d) << (76 - 4 * i);
    }
    // 26 base32 digits hold 130 bits; the top digit carries only 3.
    Ok((0..26).rev().map(|i| CROCKFORD[((v >> (5 * i)) & 31) as usize] as char).collect())
}

// ------------------------------------------------------------ framing

/// Text after the header is dropped from: C0 and C1 controls except `\n` and `\t` (so ESC, DEL, `\r` and NUL go),
/// bidi overrides and isolates, the line and paragraph separators, and zero-width and byte-order-mark characters.
fn drop_char(c: char) -> bool {
    if c == '\n' || c == '\t' {
        return false;
    }
    c.is_control()
        || matches!(c,
            '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// The ask text as it will be framed, ledgered and shown: controls gone, the steering nonce redacted.
///
/// The nonce is redacted twice. First on the raw text, where the redactor matches through escape sequences and
/// invisible characters (an ESC sequence inside the nonce must not survive as visible `[0m`). Then again after the
/// controls are dropped, because dropping a plain control byte can join the halves of a nonce it had split.
pub fn sanitise(text: &str, red: &Redactor) -> String {
    let kept: String = red.redact(text.to_string()).chars().filter(|c| !drop_char(*c)).collect();
    red.redact(kept)
}

const HEADER_START: &str = "[Vajra Desktop: question from the human, ask ";
const HEADER_MID: &str = ", run ";
const HEADER_END: &str = ". Answer with the read and control tools only.]\n";

/// The exact stdin body of one Lead turn: a fixed header line, then the sanitised text. The field is private and
/// the spawn function takes only this type, so no code path can write a raw `&str` to the Lead's stdin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FramedAsk {
    body: String,
    header_len: usize,
}

impl FramedAsk {
    pub fn as_bytes(&self) -> &[u8] {
        self.body.as_bytes()
    }

    /// The fixed first line, newline included.
    pub fn header(&self) -> &str {
        &self.body[..self.header_len]
    }

    /// The sanitised question after the header.
    pub fn text(&self) -> &str {
        &self.body[self.header_len..]
    }
}

/// Frame one question. Both header fields are checked before formatting, so neither can hold a newline or `/`, and
/// the first line is always the fixed header. The header is never trimmed and the text is never moved up to line 1:
/// the spike showed a stdin body whose first byte is `/` runs as a CLI slash command, so this frame is the defence.
pub fn frame(ask_id: &str, run_id: &str, text: &str, red: &Redactor) -> Result<FramedAsk, LeadError> {
    if !valid_ulid(ask_id) {
        return Err(LeadError::Invalid("ask_id: must be a ULID".into()));
    }
    if !ids::valid_run_id(run_id) {
        return Err(LeadError::Invalid("run_id: must be YYYYMMDD-HHMM".into()));
    }
    let n = text.chars().count();
    if n == 0 || n > MAX_TEXT {
        return Err(LeadError::Invalid(format!("text: must be 1..={MAX_TEXT} chars")));
    }
    let clean = sanitise(text, red);
    if clean.trim().is_empty() {
        return Err(LeadError::Invalid("text: must not be empty".into()));
    }
    let header = format!("{HEADER_START}{ask_id}{HEADER_MID}{run_id}{HEADER_END}");
    let header_len = header.len();
    Ok(FramedAsk { body: header + &clean, header_len })
}

#[cfg(test)]
#[path = "lead_tests.rs"]
mod tests;
