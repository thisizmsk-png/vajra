//! The run session's launch argv (AC-1.1.1, 1.1.2, 1.1.4; NFR-SEC-4).
//!
//! The result is an argv array for `Supervisor::spawn`; nothing here is ever
//! joined into a shell command line, so shell metacharacters in `ask` are
//! inert bytes inside one element.

use std::fmt;
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

/// Largest accepted ask, in bytes.
pub const MAX_ASK_BYTES: usize = 32 * 1024;

/// What the Start-run form collects.
#[derive(Clone, Debug)]
pub struct RunInput {
    pub ask: String,
    /// 1..=3
    pub max_rounds: u32,
    /// Branch or commit to build on.
    pub base: String,
    /// 1..=16, sent as `maxInFlight`.
    pub max_in_flight: u32,
    /// Campaign slug to resume; `None` means `deliver-<run_id>`.
    pub campaign: Option<String>,
}

/// A rejected field. `field` is the JSON name the form shows the error under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidInput {
    pub field: &'static str,
    pub reason: &'static str,
}

impl fmt::Display for InvalidInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}

impl std::error::Error for InvalidInput {}

fn bad(field: &'static str, reason: &'static str) -> InvalidInput {
    InvalidInput { field, reason }
}

/// `[claude, --session-id, <uuid>, "/vajra-deliver <json>"]` with a fresh v4
/// uuid and the current UTC minute as run_id. `steer_nonce` is the caller's
/// per-session C6 nonce.
pub fn build_run_argv(input: &RunInput, steer_nonce: &str) -> Result<Vec<String>, InvalidInput> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let session_id = new_uuid_v4().map_err(|_| bad("session_id", "no randomness available"))?;
    build_run_argv_at(input, steer_nonce, now, &session_id)
}

/// Deterministic core of `build_run_argv`.
pub fn build_run_argv_at(
    input: &RunInput,
    steer_nonce: &str,
    unix_secs: u64,
    session_id: &str,
) -> Result<Vec<String>, InvalidInput> {
    if input.ask.trim().is_empty() {
        return Err(bad("ask", "must not be empty"));
    }
    if input.ask.len() > MAX_ASK_BYTES {
        return Err(bad("ask", "is longer than 32 KiB"));
    }
    if input.ask.contains('\0') {
        return Err(bad("ask", "must not contain NUL"));
    }
    if !(1..=3).contains(&input.max_rounds) {
        return Err(bad("max_rounds", "must be 1 to 3"));
    }
    if !valid_base(&input.base) {
        return Err(bad("base", "must be a branch name or commit"));
    }
    if !(1..=16).contains(&input.max_in_flight) {
        return Err(bad("maxInFlight", "must be 1 to 16"));
    }
    check_nonce(steer_nonce)?;
    if session_id.len() < 8
        || !session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(bad("session_id", "must match ^[A-Za-z0-9-]{8,}$"));
    }
    let run_id = run_id_at(unix_secs);
    let campaign = match &input.campaign {
        Some(c) if !crate::bus::valid_stream_id(c) => {
            return Err(bad(
                "campaign",
                "must be 1 to 128 of [A-Za-z0-9._-], not starting with a dot",
            ));
        }
        Some(c) => c.clone(),
        None => format!("deliver-{run_id}"),
    };

    let mut json = String::with_capacity(input.ask.len() + 256);
    json.push_str("{\"ask\":");
    push_json_str(&mut json, &input.ask);
    json.push_str(",\"run_id\":");
    push_json_str(&mut json, &run_id);
    json.push_str(",\"session_id\":");
    push_json_str(&mut json, session_id);
    json.push_str(&format!(",\"max_rounds\":{}", input.max_rounds));
    json.push_str(",\"base\":");
    push_json_str(&mut json, &input.base);
    json.push_str(&format!(",\"maxInFlight\":{}", input.max_in_flight));
    json.push_str(",\"campaign\":");
    push_json_str(&mut json, &campaign);
    json.push_str(",\"steer_nonce\":");
    push_json_str(&mut json, steer_nonce);
    json.push('}');

    Ok(vec![
        "claude".to_string(),
        "--session-id".to_string(),
        session_id.to_string(),
        format!("/vajra-deliver {json}"),
    ])
}

/// A git ref or commit: ref-safe chars, no leading `-` (would read as an
/// option), no `..`.
fn valid_base(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with('-')
        && !s.contains("..")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
}

/// Same rule as vajra-deliver.js (E5), which reads `steer_nonce` at the top level.
fn check_nonce(n: &str) -> Result<(), InvalidInput> {
    if (32..=128).contains(&n.len()) && n.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        Ok(())
    } else {
        Err(bad("steer_nonce", "must be 32 to 128 lowercase hex characters"))
    }
}

/// ADR-9 resume: `claude --resume <id>` continuing the campaign through
/// `/vajra-deliver {campaign}` (what `/vajra continue` runs) with the rotated
/// nonce, so new worker briefs carry the nonce the hook now uses.
pub fn build_resume_argv(session_id: &str, campaign: &str, steer_nonce: &str) -> Result<Vec<String>, InvalidInput> {
    if session_id.len() < 8 || !session_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(bad("session_id", "must match ^[A-Za-z0-9-]{8,}$"));
    }
    if !crate::bus::valid_stream_id(campaign) {
        return Err(bad("campaign", "must be 1 to 128 of [A-Za-z0-9._-], not starting with a dot"));
    }
    check_nonce(steer_nonce)?;
    let mut json = String::from("{\"campaign\":");
    push_json_str(&mut json, campaign);
    json.push_str(",\"session_id\":");
    push_json_str(&mut json, session_id);
    json.push_str(",\"steer_nonce\":");
    push_json_str(&mut json, steer_nonce);
    json.push('}');
    Ok(vec!["claude".into(), "--resume".into(), session_id.into(), format!("/vajra-deliver {json}")])
}

fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Controls, plus the JS line separators, as \u escapes.
            c if c.is_control() || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// UTC `YYYYMMDD-HHMM` for a unix time.
pub fn run_id_at(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}", rem / 3600, rem % 3600 / 60)
}

fn new_uuid_v4() -> std::io::Result<String> {
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

#[cfg(test)]
#[path = "argv_tests.rs"]
mod tests;
