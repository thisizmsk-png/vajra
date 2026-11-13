//! A core run's event log: append-only, sequenced and hash-chained (W2). Pure: `runlog_store.rs` keeps it on disk and
//! hands deltas to the UI.
//!
//! - Every entry has `seq` (1, 2, 3, … with no gap), `prev` (the previous entry's `hash`) and its own `hash`, the
//!   sha256 of the entry's canonical JSON without `hash`. Entry 1's `prev` is bound to the run id, so one run's log
//!   can't be spliced into another's.
//! - Kinds are a closed, typed set. Nothing parses agent or terminal text into an entry; a check result is accepted
//!   only from the verifier channel (`Source::Verifier`), and the verifier can append nothing else (bhishma H1).
//! - The log has no key and no signature of its own. Its trust comes from **anchors**: the chain head written into the
//!   run's signed record (#190) as a progress line, `log head <run> <seq> <hash>`. `verify` reports how far the chain
//!   holds, and `signed_to` how far the signed record vouches for it. The rest is "hash-chained, not yet signed".

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::control::sha256;
use crate::vajra::bridge_record::one_line;

/// The log format; it is inside every hash, so a change of format can't pass for the old one.
pub const FORMAT: u32 = 1;
/// The longest text field an entry keeps (one line).
pub const TEXT_MAX: usize = 500;
/// The prefix of an anchor line in the signed record's progress.
pub const ANCHOR_PREFIX: &str = "log head ";

/// Who appended an entry. A worker is named by its ticket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Core,
    /// The round gate's own channel: the only source of check results.
    Verifier,
    Lead,
    Worker(String),
    Human,
}

/// Tokens by category, as W1's receipt counts them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub fresh: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    /// Calls the run made beside its turns (titles, summaries).
    pub side: u64,
}

/// Memory at its peak during the run, in KiB: the app's own footprint, and the largest agent's resident size.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Weight {
    pub app_footprint_peak_kib: u64,
    pub agent_rss_peak_kib: u64,
}

/// What happened. Each kind's body is fixed; text is one line and capped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
pub enum Kind {
    RunStarted { resumed_from: Option<String> },
    PlanAccepted { round: u32, tasks: Vec<String> },
    Dispatched { task: String, ticket: String, seat: String },
    TicketEnded { task: String, ticket: String, outcome: String },
    Integrated { round: u32, ok: bool, head: Option<String>, why: Option<String> },
    ChecksStarted { round: u32 },
    /// Verifier only.
    CheckResult { round: u32, criterion: String, passed: bool },
    /// Verifier only.
    RoundVerdict { round: u32, verdict: String },
    Question { id: String, text: String },
    Answer { id: String, text: String },
    StatusSet { status: String, reason: String },
    LeadTurn { turns: u32, tokens: u64 },
    /// W1's receipts: `scope` is `run` or `task:<id>`. `usd_micros` is at API prices.
    Receipt { scope: String, model: String, tokens: Tokens, usd_micros: u64, weight: Option<Weight> },
    /// The chain head at `seq` (with `hash`) was sent to the signed record.
    Anchored { seq: u64, hash: String },
    /// The app restarted and cut a torn last line (a crash mid-append): nothing else is ever repaired.
    Recovered { dropped_bytes: u64, at_seq: u64 },
    /// A late or stale event that changed nothing (a "running" after "done"), kept so the log shows it arrived.
    Noop { what: String, why: String },
}

impl Kind {
    /// Must the UI hear of it at once (approval, question, result, crash, end), or may it wait for the next batch?
    pub fn critical(&self) -> bool {
        match self {
            Kind::Question { .. } | Kind::CheckResult { .. } | Kind::RoundVerdict { .. } | Kind::StatusSet { .. } | Kind::Recovered { .. } => true,
            Kind::TicketEnded { outcome, .. } => outcome != "done",
            _ => false,
        }
    }

    fn verifier_only(&self) -> bool {
        matches!(self, Kind::CheckResult { .. } | Kind::RoundVerdict { .. })
    }

    /// Every text field as one capped line. Applied by `Log::append` before hashing, so a stored entry rehashes the same.
    fn capped(self) -> Kind {
        self.map_text(&|s| one_line(&s, TEXT_MAX))
    }

    /// Every text field through `f` (the protocol's redactor, before the entry is hashed; bhishma F7).
    pub fn map_text(self, f: &dyn Fn(String) -> String) -> Kind {
        let c = |s: String| f(s);
        let co = |s: Option<String>| s.map(c);
        match self {
            Kind::RunStarted { resumed_from } => Kind::RunStarted { resumed_from: co(resumed_from) },
            Kind::PlanAccepted { round, tasks } => Kind::PlanAccepted { round, tasks: tasks.into_iter().take(200).map(c).collect() },
            Kind::Dispatched { task, ticket, seat } => Kind::Dispatched { task: c(task), ticket: c(ticket), seat: c(seat) },
            Kind::TicketEnded { task, ticket, outcome } => Kind::TicketEnded { task: c(task), ticket: c(ticket), outcome: c(outcome) },
            Kind::Integrated { round, ok, head, why } => Kind::Integrated { round, ok, head: co(head), why: co(why) },
            Kind::CheckResult { round, criterion, passed } => Kind::CheckResult { round, criterion: c(criterion), passed },
            Kind::RoundVerdict { round, verdict } => Kind::RoundVerdict { round, verdict: c(verdict) },
            Kind::Question { id, text } => Kind::Question { id: c(id), text: c(text) },
            Kind::Answer { id, text } => Kind::Answer { id: c(id), text: c(text) },
            Kind::StatusSet { status, reason } => Kind::StatusSet { status: c(status), reason: c(reason) },
            Kind::Receipt { scope, model, tokens, usd_micros, weight } => Kind::Receipt { scope: c(scope), model: c(model), tokens, usd_micros, weight },
            Kind::Anchored { seq, hash } => Kind::Anchored { seq, hash: c(hash) },
            Kind::Noop { what, why } => Kind::Noop { what: c(what), why: c(why) },
            k @ (Kind::ChecksStarted { .. } | Kind::LeadTurn { .. } | Kind::Recovered { .. }) => k,
        }
    }
}

/// One log entry, as it is stored (one JSON line) and sent to the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub v: u32,
    pub seq: u64,
    pub prev: String,
    pub at_ms: u64,
    pub source: Source,
    #[serde(flatten)]
    pub kind: Kind,
    pub hash: String,
}

/// JSON with every object's keys sorted and no whitespace: the bytes an entry's hash covers, whatever map order
/// serde_json was built with.
fn canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

impl Entry {
    /// The bytes an entry's hash covers: its JSON without `hash`, keys sorted at every level, no whitespace, UTF-8.
    /// No field is a float. A golden test pins these bytes, so a serde change can't move every hash unnoticed (F6).
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Value::Object(m) = &mut v {
            m.remove("hash");
        }
        let mut s = String::new();
        canonical(&v, &mut s);
        s.into_bytes()
    }

    /// The sha256 of `canonical_bytes`.
    pub fn compute_hash(&self) -> String {
        sha256::hex_digest(&self.canonical_bytes())
    }
}

/// Entry 1's `prev`: bound to the run, so another run's log never verifies as this one's.
pub fn genesis(run_id: &str) -> String {
    sha256::hex_digest(format!("vajra-runlog-v{FORMAT}\n{run_id}").as_bytes())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A check result or verdict not from the verifier, or the verifier appending anything else.
    WrongSource,
    /// The log is broken: nothing more is appended to it until the human decides.
    Broken(u64),
}

/// The log in memory: the run, its entries in order, and where it broke if it did.
#[derive(Clone, Debug)]
pub struct Log {
    run_id: String,
    entries: Vec<Entry>,
    broken: Option<Break>,
}

impl Log {
    pub fn new(run_id: &str) -> Self {
        Log { run_id: run_id.to_string(), entries: Vec::new(), broken: None }
    }

    /// A log loaded from disk: the longest valid prefix, and the break after it if there was one. `unreadable`: the
    /// position of the first whole line that wasn't an entry (the entries given are the ones before it).
    pub fn loaded(run_id: &str, entries: Vec<Entry>, unreadable: Option<u64>) -> Self {
        let chain = verify(run_id, &entries);
        let ok = usize::try_from(chain.ok_to).unwrap_or(usize::MAX).min(entries.len());
        let mut entries = entries;
        entries.truncate(ok);
        let broken = chain.broken.or_else(|| unreadable.map(|seq| Break { seq, why: Why::Edited }));
        Log { run_id: run_id.to_string(), entries, broken }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn head(&self) -> Option<&Entry> {
        self.entries.last()
    }
    pub fn head_seq(&self) -> u64 {
        self.head().map_or(0, |e| e.seq)
    }
    pub fn broken(&self) -> Option<&Break> {
        self.broken.as_ref()
    }

    /// Append one event. The verifier alone appends check results and verdicts, and nothing else.
    pub fn append(&mut self, source: Source, kind: Kind, at_ms: u64) -> Result<&Entry, Refused> {
        if let Some(b) = &self.broken {
            return Err(Refused::Broken(b.seq));
        }
        if kind.verifier_only() != (source == Source::Verifier) {
            return Err(Refused::WrongSource);
        }
        let prev = self.head().map_or_else(|| genesis(&self.run_id), |e| e.hash.clone());
        let mut e = Entry { v: FORMAT, seq: self.head_seq() + 1, prev, at_ms, source, kind: kind.capped(), hash: String::new() };
        e.hash = e.compute_hash();
        self.entries.push(e);
        Ok(self.entries.last().expect("just pushed"))
    }

    /// The entries after `after` (by seq), at most `limit`.
    pub fn since(&self, after: u64, limit: usize) -> &[Entry] {
        let from = usize::try_from(after).unwrap_or(usize::MAX).min(self.entries.len());
        let to = from.saturating_add(limit).min(self.entries.len());
        &self.entries[from..to]
    }

    /// The anchor for the current head.
    pub fn head_anchor(&self) -> Option<Anchor> {
        self.head().map(|h| Anchor { seq: h.seq, hash: h.hash.clone() })
    }

    /// Require the chain to hold `a` (an anchor this app session sent; F4). If it doesn't, the log is broken at that
    /// seq: nothing more is appended and the run can't be verified on it. `true` when it holds.
    pub fn require(&mut self, a: &Anchor) -> bool {
        let ok = usize::try_from(a.seq).ok().and_then(|s| s.checked_sub(1)).and_then(|i| self.entries.get(i)).is_some_and(|e| e.hash == a.hash);
        if !ok && self.broken.as_ref().is_none_or(|b| b.seq > a.seq) {
            self.broken = Some(Break { seq: a.seq, why: Why::Unanchored });
        }
        ok
    }
}

/// Why the chain stops holding at `seq`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// An entry's content doesn't match its own hash.
    Edited,
    /// A sequence number is missing.
    Dropped,
    /// Entries are out of order, or one repeats.
    Reordered,
    /// An entry's `prev` isn't the hash before it (its neighbour was removed, replaced or rehashed).
    Unlinked,
    /// Entry 1 isn't bound to this run.
    WrongRun,
    /// An entry of another format.
    Format,
    /// The chain at an anchor this app session sent doesn't hold that anchor's hash (the log changed under it; F4).
    Unanchored,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Break {
    /// The position (1-based) of the first entry that doesn't hold.
    pub seq: u64,
    pub why: Why,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Chain {
    /// Entries 1..=ok_to hold.
    pub ok_to: u64,
    pub broken: Option<Break>,
}

/// Check the whole chain. A valid log gives `ok_to == entries.len()` and no break.
pub fn verify(run_id: &str, entries: &[Entry]) -> Chain {
    let mut prev = genesis(run_id);
    for (i, e) in entries.iter().enumerate() {
        let want = i as u64 + 1;
        let stop = |why| Chain { ok_to: want - 1, broken: Some(Break { seq: want, why }) };
        if e.v != FORMAT {
            return stop(Why::Format);
        }
        if e.compute_hash() != e.hash {
            return stop(Why::Edited);
        }
        if e.seq != want {
            // A later number with the wanted one further on is a swap; a later number with it nowhere is a gap.
            let later = entries[i..].iter().any(|x| x.seq == want);
            return stop(if e.seq < want || later { Why::Reordered } else { Why::Dropped });
        }
        if e.prev != prev {
            return stop(if want == 1 { Why::WrongRun } else { Why::Unlinked });
        }
        prev = e.hash.clone();
    }
    Chain { ok_to: entries.len() as u64, broken: None }
}

/// One anchor read from the signed record's progress: this run's chain head at `seq` was `hash`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Anchor {
    pub seq: u64,
    pub hash: String,
}

/// The one anchor format (bhishma F1): `log head <run id> <seq> <64 lowercase hex>`, single spaces, nothing else.
/// The run id is in it because a resumed run shares its campaign, so several runs' anchors sit in one progress file.
/// With a run id of at most 64 characters it is at most 9 + 64 + 1 + 20 + 1 + 64 = 159 characters, under the record's
/// progress cap of 300 after `one_line`, so the hash is never cut (a test pins it).
pub fn anchor_text(run_id: &str, a: &Anchor) -> String {
    format!("{ANCHOR_PREFIX}{run_id} {} {}", a.seq, a.hash)
}

/// Parse one progress text (no timestamp) as an anchor: `(run id, anchor)`, or `None` if it isn't exactly the format.
pub fn parse_anchor(text: &str) -> Option<(String, Anchor)> {
    let mut it = text.strip_prefix(ANCHOR_PREFIX)?.split(' ');
    let (run, seq, hash) = (it.next()?, it.next()?, it.next()?);
    let hex = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let seq: u64 = seq.parse().ok().filter(|s: &u64| *s >= 1)?;
    let a = Anchor { seq, hash: hash.to_string() };
    // Canonical only: "007" or a trailing space doesn't parse back to the same text.
    let run_ok = !run.is_empty() && run.len() <= 64 && run.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
    (it.next().is_none() && hex && run_ok && anchor_text(run, &a) == text).then(|| (run.to_string(), a))
}

/// Does this progress text claim to be an anchor (well-formed or not)?
fn claims_anchor(text: &str) -> bool {
    text.trim_start().starts_with(ANCHOR_PREFIX.trim_end())
}

/// The text of each `- <time> <text>` line of a progress file.
fn progress_texts(progress: &str) -> impl Iterator<Item = &str> {
    progress.lines().filter_map(|l| l.strip_prefix("- ")?.split_once(' ').map(|x| x.1))
}

/// This run's anchors in a progress file, in order (for tests and display; verification uses `held_anchors`).
pub fn anchors(run_id: &str, progress: &str) -> Vec<Anchor> {
    progress_texts(progress).filter_map(parse_anchor).filter(|(r, _)| r == run_id).map(|x| x.1).collect()
}

/// This run's anchors, counted only from progress writes the core's own ledger recorded (bhishma F3). `held`: the
/// ledger's progress texts, in order; `shown`: the signed progress file read back. A line in `shown` that claims to be
/// an anchor and isn't, in order, one the core wrote is an error: the result is "not verified", never a skipped line.
pub fn held_anchors(run_id: &str, held: &[String], shown: &str) -> Result<Vec<Anchor>, String> {
    let shown_claims: Vec<&str> = progress_texts(shown).filter(|t| claims_anchor(t)).collect();
    let held_claims: Vec<&str> = held.iter().map(String::as_str).filter(|t| claims_anchor(t)).collect();
    if let Some(i) = (0..shown_claims.len().max(held_claims.len())).find(|&i| shown_claims.get(i) != held_claims.get(i)) {
        return Err(match shown_claims.get(i) {
            Some(t) => format!("a log head line the core didn't write: {}", one_line(t, 120)),
            None => "a log head line the core wrote is missing from the record".into(),
        });
    }
    let mut out = Vec::new();
    for t in held_claims {
        let (run, a) = parse_anchor(t).ok_or_else(|| format!("a log head line isn't in the anchor format: {}", one_line(t, 120)))?;
        if run == run_id {
            out.push(a);
        }
    }
    Ok(out)
}

/// How far the signed record vouches for the log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Signed {
    /// The highest anchored seq that the valid chain matches (0: none).
    pub signed_to: u64,
    /// Entries after it: hash-chained, not yet signed.
    pub unsigned_tail: u64,
    /// An anchor the log doesn't match (missing entry, other hash): the log isn't the one that was signed.
    pub mismatch: Option<Anchor>,
}

impl Signed {
    pub fn verified(&self) -> bool {
        self.mismatch.is_none() && self.signed_to > 0
    }
}

/// What the run card may say about a log. Never "tamper-evident": until the worker sandboxes land, an agent on the same
/// Mac could rewrite the log and its anchors together (bhishma F5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// The chain holds, and every anchor in the signed record is one the core's ledger holds and this app session sent.
    Signed,
    /// The chain holds and matches the ledger's anchors, but this app session sent none of them (the app restarted):
    /// "hash-chained, anchors not confirmed by this session".
    Unconfirmed,
    /// The chain breaks, an anchor doesn't match, a log head line isn't the core's, or nothing was anchored.
    NotVerified,
}

pub const CAVEAT: &str = "hash-chained; until the worker sandboxes land, an agent on this Mac could rewrite this log together with its anchors";

/// The standing of a log: `held` are the anchors the core's ledger holds (`held_anchors`), `sent` the ones this app
/// session sent. An anchor the session didn't send, while it sent others, is not confirmed: not verified.
pub fn standing(run_id: &str, entries: &[Entry], held: &[Anchor], sent: &[Anchor]) -> (Standing, Signed) {
    let s = signed(run_id, entries, held);
    let chain_ok = verify(run_id, entries).broken.is_none();
    let st = if !chain_ok || !s.verified() {
        Standing::NotVerified
    } else if held.iter().all(|a| sent.contains(a)) {
        Standing::Signed
    } else if sent.is_empty() {
        Standing::Unconfirmed
    } else {
        Standing::NotVerified
    };
    (st, s)
}

/// Compare the log's valid prefix with every anchor. Any anchor it doesn't match is a mismatch, even an old one: a log
/// rewritten after it was signed, or cut short, fails here even when its own chain is consistent.
pub fn signed(run_id: &str, entries: &[Entry], anchors: &[Anchor]) -> Signed {
    let ok = verify(run_id, entries).ok_to;
    let at = |seq: u64| (seq >= 1 && seq <= ok).then(|| &entries[(seq - 1) as usize]);
    let mismatch = anchors.iter().find(|a| at(a.seq).is_none_or(|e| e.hash != a.hash)).cloned();
    let signed_to = if mismatch.is_some() { 0 } else { anchors.iter().map(|a| a.seq).max().unwrap_or(0) };
    Signed { signed_to, unsigned_tail: (entries.len() as u64).saturating_sub(signed_to), mismatch }
}
