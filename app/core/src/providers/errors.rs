//! Provider errors as typed events (H9). A worker's failure becomes one `provider_error` event for the run lead and the
//! Director-General, with a retry policy that lives here, in the core, and never in the model.
//!
//! - `classify` reads a CLI's free error text (Codex's `turn.failed` and `error` events, Claude's stream errors) into a
//!   `Kind`, an HTTP status and a `Retry-After`. Codex does not classify anything itself.
//! - `decide` is the H9 policy: timeout and upstream get two retries (30 s, then 2 min); a 429 waits for its window if
//!   that is 15 minutes or less; auth, quota and bad request never retry. After that the action is `asking_user`.
//! - `run_with_policy` drives an attempt through that policy on an injected `Clock`.
//! - **Never an automatic switch (H7).** Nothing in this module can change the account or the provider: the identity of
//!   a worker (`Ident`) is borrowed read-only, and every retry reuses it. Moving to another provider is a new pick the
//!   user confirms, made elsewhere.
//! - **Redaction (R19).** Every message is cleaned by `crew::redact::redact_provider_text` when the event is built, so no
//!   field of an event, a log line or the UI can carry a `user_id`, a `cf-ray`, a key or a URL query.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use crate::crew::redact::redact_provider_text;

/// Backoff before retry 1 and retry 2 of a timeout or an upstream error.
pub const BACKOFF: [Duration; 2] = [Duration::from_secs(30), Duration::from_secs(120)];
/// A 429 is waited out only if its window is at most this long.
pub const MAX_RATE_WAIT: Duration = Duration::from_secs(15 * 60);
/// A 429 is waited out at most this many times in a row; the next one asks the user.
pub const MAX_RATE_WAITS: u32 = 2;
/// The longest message an event carries.
const MAX_MESSAGE_CHARS: usize = 240;
/// Lines of a dead process's output that an `unexplained_exit` carries.
pub const TAIL_LINES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Auth,
    Quota,
    RateLimit,
    Timeout,
    Upstream,
    BadRequest,
    UnexplainedExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// The core will run the same worker again on the same account after the wait.
    Retrying,
    /// The task goes back to the lead as blocked and the Director-General asks the user.
    AskingUser,
}

/// What the text said, before the policy decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub kind: Kind,
    pub http_status: Option<u16>,
    pub retry_after_s: Option<u64>,
}

/// Who failed. Borrowed read-only by everything here: no function in this module takes it mutably or builds a different
/// one, so a retry can only ever run on the same account (H7).
#[derive(Debug, Clone, Copy)]
pub struct Ident<'a> {
    pub slot: &'a str,
    pub session_id: &'a str,
    pub account: &'a str,
}

/// The `provider_error` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub slot: String,
    pub session_id: String,
    pub account: String,
    pub kind: Kind,
    pub http_status: Option<u16>,
    pub retry_after_s: Option<u64>,
    /// 1 for the first failure.
    pub attempt: u32,
    /// Already redacted and bounded.
    pub message: String,
    pub action: Action,
}

impl ProviderError {
    pub fn new(who: &Ident, c: &Classified, attempt: u32, raw: &str, action: Action) -> Self {
        let message: String = redact_provider_text(raw).chars().take(MAX_MESSAGE_CHARS).collect();
        Self {
            slot: who.slot.into(),
            session_id: who.session_id.into(),
            account: who.account.into(),
            kind: c.kind,
            http_status: c.http_status,
            retry_after_s: c.retry_after_s,
            attempt,
            message,
            action,
        }
    }

    /// The wire form, `{"type":"provider_error", ...}`.
    pub fn to_json(&self) -> Value {
        json!({
            "type": "provider_error",
            "slot": self.slot, "session_id": self.session_id, "account": self.account,
            "kind": self.kind, "http_status": self.http_status, "retry_after_s": self.retry_after_s,
            "attempt": self.attempt, "message": self.message, "action": self.action,
        })
    }
}

// ---------------------------------------------------------------------------------------------- classifier

/// Text that introduces an HTTP status, lower case. The first one followed by a plausible status wins.
const STATUS_LEADS: [&str; 8] = ["last status: ", "unexpected status ", "\"code\":", "api error: ", "http ", "status code ", "status: ", "status "];

fn status_in(lc: &str) -> Option<u16> {
    for lead in STATUS_LEADS {
        for (at, _) in lc.match_indices(lead) {
            let rest = lc[at + lead.len()..].trim_start();
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let (3, Ok(n @ 100..=599)) = (digits.len(), digits.parse::<u16>()) {
                return Some(n);
            }
        }
    }
    None
}

fn retry_after_in(lc: &str) -> Option<u64> {
    for lead in ["retry-after:", "retry-after\":", "retry after ", "retry_after_s\":"] {
        if let Some(at) = lc.find(lead) {
            let rest = lc[at + lead.len()..].trim_start_matches([' ', '"']);
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = digits.parse::<u64>() {
                return Some(n);
            }
        }
    }
    None
}

/// Read one failure text into a kind, a status and a `Retry-After`. Anything that says nothing typed is
/// `unexplained_exit`: the caller reports it with the exit code, and the worker never just disappears.
pub fn classify(text: &str) -> Classified {
    let lc = text.to_ascii_lowercase();
    let http_status = status_in(&lc);
    let has = |needles: &[&str]| needles.iter().any(|n| lc.contains(n));
    let kind = if has(&["idle timeout", "stream disconnected", "timed out"]) {
        Kind::Timeout
    } else if let Some(s) = http_status {
        match s {
            401 | 403 => Kind::Auth,
            402 => Kind::Quota,
            429 => Kind::RateLimit,
            408 | 504 => Kind::Timeout,
            500..=599 => Kind::Upstream,
            400 | 404 | 413 | 422 => Kind::BadRequest,
            _ => Kind::UnexplainedExit,
        }
    } else if has(&["authentication_error", "invalid api key", "not logged in", "please run /login", "signed out", "unauthorized"]) {
        Kind::Auth
    } else if has(&["insufficient credit", "credit balance", "payment required"]) {
        Kind::Quota
    } else if has(&["rate_limit_error", "rate limit", "usage limit", "too many requests"]) {
        Kind::RateLimit
    } else if has(&["overloaded_error", "overloaded", "bad gateway", "service unavailable"]) {
        Kind::Upstream
    } else if has(&["invalid_request_error", "not a valid model"]) {
        Kind::BadRequest
    } else {
        Kind::UnexplainedExit
    };
    let retry_after_s = if kind == Kind::RateLimit { retry_after_in(&lc) } else { None };
    Classified { kind, http_status, retry_after_s }
}

/// The event for a process that ended badly: its own typed error if its last lines held one, else `unexplained_exit`
/// with the exit code and the (redacted) last lines.
pub fn classify_exit(code: Option<u32>, signal: Option<&str>, tail: &[String]) -> (Classified, String) {
    let joined = tail.join("\n");
    let typed = classify(&joined);
    if typed.kind != Kind::UnexplainedExit {
        return (typed, joined);
    }
    let how = match (signal, code) {
        (Some(s), _) => format!("ended by signal {s}"),
        (None, Some(c)) => format!("exit code {c}"),
        (None, None) => "ended".to_string(),
    };
    (typed, format!("{how}: {joined}"))
}

// ---------------------------------------------------------------------------------------------- policy

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub action: Action,
    /// Present exactly when `action` is `Retrying`.
    pub wait: Option<Duration>,
}

const ASK: Decision = Decision { action: Action::AskingUser, wait: None };

/// The H9 policy. `failures` counts this failure (1 for the first). Timeout and upstream: two retries, 30 s then 2 min.
/// 429: wait for `retry_after_s` if it is known and at most 15 minutes, up to `MAX_RATE_WAITS` times. Everything else
/// asks the user at once.
pub fn decide(kind: Kind, failures: u32, retry_after_s: Option<u64>) -> Decision {
    match kind {
        Kind::Timeout | Kind::Upstream => match BACKOFF.get(failures.saturating_sub(1) as usize) {
            Some(&wait) if failures >= 1 => Decision { action: Action::Retrying, wait: Some(wait) },
            _ => ASK,
        },
        Kind::RateLimit => match retry_after_s.map(Duration::from_secs) {
            Some(wait) if wait <= MAX_RATE_WAIT && failures <= MAX_RATE_WAITS => Decision { action: Action::Retrying, wait: Some(wait) },
            _ => ASK,
        },
        Kind::Auth | Kind::Quota | Kind::BadRequest | Kind::UnexplainedExit => ASK,
    }
}

// ---------------------------------------------------------------------------------------------- driver

/// Where waiting happens, so tests run on a fake one.
pub trait Clock {
    fn sleep(&self, d: Duration);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// A clock that never waits and remembers every wait it was asked for.
#[derive(Default)]
pub struct FakeClock {
    slept: Mutex<Vec<Duration>>,
}

impl FakeClock {
    pub fn slept(&self) -> Vec<Duration> {
        self.slept.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Clock for FakeClock {
    fn sleep(&self, d: Duration) {
        self.slept.lock().unwrap_or_else(|e| e.into_inner()).push(d);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// The policy gave up: the task goes back to the lead as blocked, and the Director-General asks the user. The last
    /// event is carried so the caller can hand it on.
    Blocked(ProviderError),
}

/// Run `attempt` until it succeeds or the policy says to ask the user. `Err` carries the CLI's failure text. Every
/// failure is reported to `emit` before any wait. The same `who` is used throughout: a retry never changes account.
pub fn run_with_policy(clock: &dyn Clock, who: &Ident, emit: &mut dyn FnMut(&ProviderError), mut attempt: impl FnMut() -> Result<(), String>) -> Outcome {
    let mut failures = 0u32;
    loop {
        let Err(text) = attempt() else { return Outcome::Done };
        failures += 1;
        let c = classify(&text);
        let d = decide(c.kind, failures, c.retry_after_s);
        let ev = ProviderError::new(who, &c, failures, &text, d.action);
        emit(&ev);
        match d.wait {
            Some(wait) => clock.sleep(wait),
            None => return Outcome::Blocked(ev),
        }
    }
}

#[cfg(test)]
#[path = "errors_tests.rs"]
mod tests;
