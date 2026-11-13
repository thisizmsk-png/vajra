//! Consent (ADR-17 rule 4, bhishma condition 7): each install needs its own click, and that click mints a token bound
//! to {harness id, plan hash}. The run takes the token (once, even when it fails), recomputes the plan and needs the
//! same hash. Tokens live only here, in memory: never in a log, the audit line, the bus or a spill.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const TTL: Duration = Duration::from_secs(120);
/// Tokens outstanding at once; minting another drops the oldest.
pub const MAX: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentError {
    /// No valid token for this harness and plan: the user must click Install again.
    Required,
    /// An install of this harness is already running.
    Busy,
}

impl ConsentError {
    pub fn code(self) -> &'static str {
        match self {
            ConsentError::Required => "consent_required",
            ConsentError::Busy => "setup_running",
        }
    }
}

struct Entry {
    id: String,
    hash: String,
    minted: Instant,
}

#[derive(Default)]
struct State {
    tokens: HashMap<String, Entry>,
    running: HashSet<String>,
}

#[derive(Default, Clone)]
pub struct Consents(Arc<Mutex<State>>);

impl std::fmt::Debug for Consents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = crate::lock(&self.0);
        write!(f, "Consents {{ outstanding: {}, running: {} }}", s.tokens.len(), s.running.len())
    }
}

fn random_hex() -> String {
    let mut b = [0u8; 16];
    // SAFETY: fills the buffer; arc4random_buf cannot fail.
    unsafe { libc::arc4random_buf(b.as_mut_ptr().cast(), b.len()) };
    super::plan::hex(&b)
}

impl Consents {
    /// A fresh token for installing `id` with the plan whose hash is `hash`.
    pub fn mint(&self, id: &str, hash: &str, now: Instant) -> String {
        let mut s = crate::lock(&self.0);
        s.tokens.retain(|_, e| now.saturating_duration_since(e.minted) <= TTL);
        while s.tokens.len() >= MAX {
            let oldest = s.tokens.iter().min_by_key(|(_, e)| e.minted).map(|(k, _)| k.clone());
            match oldest {
                Some(k) => s.tokens.remove(&k),
                None => break,
            };
        }
        let t = random_hex();
        s.tokens.insert(t.clone(), Entry { id: id.into(), hash: hash.into(), minted: now });
        t
    }

    /// Spend `token` for `id` and `hash`. The token is gone afterwards whether or not it matched.
    pub fn take(&self, token: &str, id: &str, hash: &str, now: Instant) -> Result<(), ConsentError> {
        let e = crate::lock(&self.0).tokens.remove(token).ok_or(ConsentError::Required)?;
        if e.id != id || e.hash != hash || now.saturating_duration_since(e.minted) > TTL {
            return Err(ConsentError::Required);
        }
        Ok(())
    }

    /// Mark `id` as running until the guard is dropped.
    pub fn begin(&self, id: &str) -> Result<RunGuard, ConsentError> {
        if !crate::lock(&self.0).running.insert(id.to_string()) {
            return Err(ConsentError::Busy);
        }
        Ok(RunGuard { c: self.clone(), id: id.into() })
    }
}

/// While held, no other install of the same harness can start.
pub struct RunGuard {
    c: Consents,
    id: String,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        crate::lock(&self.c.0).running.remove(&self.id);
    }
}

#[cfg(test)]
#[path = "consent_tests.rs"]
mod tests;
