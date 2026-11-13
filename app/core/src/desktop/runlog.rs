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

    /// Every text field as one capped line.
    fn capped(self) -> Kind {
        let c = |s: String| one_line(&s, TEXT_MAX);
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
    /// The sha256 of this entry's canonical JSON without `hash`.
    pub fn compute_hash(&self) -> String {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Value::Object(m) = &mut v {
            m.remove("hash");
        }
        let mut s = String::new();
        canonical(&v, &mut s);
        sha256::hex_digest(s.as_bytes())
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

    /// The anchor line for the current head: `log head <run> <seq> <hash>`.
    pub fn anchor_line(&self) -> Option<String> {
        self.head().map(|h| format!("{ANCHOR_PREFIX}{} {} {}", self.run_id, h.seq, h.hash))
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

/// This run's anchors in the signed record's progress text (`- <time> log head <run> <seq> <hash>` lines), in order.
/// Other runs' anchors (a resumed run shares the campaign) and every other line are skipped.
pub fn anchors(run_id: &str, progress: &str) -> Vec<Anchor> {
    progress
        .lines()
        .filter_map(|l| {
            // `- <time> <text>`: the text itself must be the anchor, not merely contain one.
            let text = l.strip_prefix("- ")?.split_once(' ')?.1;
            let mut it = text.strip_prefix(ANCHOR_PREFIX)?.split(' ');
            let (run, seq, hash) = (it.next()?, it.next()?, it.next()?);
            (run == run_id && it.next().is_none() && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| seq.parse().ok().map(|seq| Anchor { seq, hash: hash.to_string() }))
                .flatten()
        })
        .collect()
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

/// Compare the log's valid prefix with every anchor. Any anchor it doesn't match is a mismatch, even an old one: a log
/// rewritten after it was signed, or cut short, fails here even when its own chain is consistent.
pub fn signed(run_id: &str, entries: &[Entry], anchors: &[Anchor]) -> Signed {
    let ok = verify(run_id, entries).ok_to;
    let at = |seq: u64| (seq >= 1 && seq <= ok).then(|| &entries[(seq - 1) as usize]);
    let mismatch = anchors.iter().find(|a| at(a.seq).is_none_or(|e| e.hash != a.hash)).cloned();
    let signed_to = if mismatch.is_some() { 0 } else { anchors.iter().map(|a| a.seq).max().unwrap_or(0) };
    Signed { signed_to, unsigned_tail: (entries.len() as u64).saturating_sub(signed_to), mismatch }
}
