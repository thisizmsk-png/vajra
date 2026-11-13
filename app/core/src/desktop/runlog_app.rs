//! The run log on the app's side (W2): where a core run's log lives (`<data>/runlog/<run>/log.jsonl`, the app's own,
//! private), its `run_log` notices, the UI's "entries after N" read, and verification against the signed record.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};

use super::App;
use super::record::LedgerStore;
use super::record_app::record_bridge;
use super::runlog;
use super::runlog_store::{self, RunLog};

/// The named event the UI hears: `{run_id, seq}`, the written head. It then asks `run_log_since`.
pub const EVENT: &str = "run_log";

/// A log as a read sees it: the live one this app holds, or a read-only snapshot of the file.
enum Read {
    Live(Arc<RunLog>),
    Snapshot(runlog_store::Snapshot),
}

impl Read {
    fn since(&self, after: u64, limit: usize) -> (Vec<runlog::Entry>, u64) {
        match self {
            Read::Live(l) => l.since(after, limit),
            Read::Snapshot(s) => s.since(after, limit),
        }
    }
    fn entries(&self) -> Vec<runlog::Entry> {
        match self {
            Read::Live(l) => l.entries(),
            Read::Snapshot(s) => s.entries(),
        }
    }
    fn card(&self) -> runlog_store::LogCard {
        match self {
            Read::Live(l) => l.card(),
            Read::Snapshot(s) => s.card(),
        }
    }
}

impl App {
    fn run_log_dir(&self, run_id: &str) -> Result<PathBuf, String> {
        if !crate::ids::valid_run_id(run_id) {
            return Err("not a run id".into());
        }
        Ok(self.data.join("runlog").join(run_id))
    }

    /// A core run's log: the one this app already holds, else opened from disk (resuming after its last good entry).
    /// Its notices go to the UI as `run_log`.
    #[allow(dead_code)] // 6d2c's Start attaches it to a core run's protocol (`RunProtocol::with_log`)
    pub(super) fn run_log(&self, run_id: &str) -> Result<Arc<RunLog>, String> {
        let mut held = super::lock(&self.live_logs);
        held.retain(|_, w| w.strong_count() > 0);
        if let Some(l) = held.get(run_id).and_then(std::sync::Weak::upgrade) {
            return Ok(l);
        }
        let dir = self.run_log_dir(run_id)?;
        if let Some(parent) = dir.parent() {
            crate::control::fsx::private_dir(parent).map_err(|e| format!("the run log folder can't be made: {e}"))?;
        }
        // Reopened in this app's life: the file must still hold the last anchor this session sent (F4), checked before
        // any repair or append, so a log that fails it is left as found.
        let last = super::lock(&self.log_anchors).get(run_id).and_then(|v| v.last().cloned());
        let log = Arc::new(RunLog::open_requiring(&dir, run_id, last.as_ref())?);
        let events = Arc::clone(&self.events);
        log.set_notify(Box::new(move |run, seq| events.emit(EVENT, json!({"run_id": run, "seq": seq}))));
        let sent = Arc::clone(&self.log_anchors);
        log.set_on_anchor(Box::new(move |run, a| super::lock(&sent).entry(run.to_string()).or_default().push(a.clone())));
        held.insert(run_id.to_string(), Arc::downgrade(&log));
        Ok(log)
    }

    /// The held live log, else a read-only snapshot of the file (a finished run). A read never opens a writer, never
    /// repairs or appends, and never spawns a thread, so reads can't race each other or change a finished run's file.
    fn run_log_read(&self, run_id: &str) -> Result<Read, Value> {
        let bad = |d: String| json!({"error": "invalid", "detail": d});
        let dir = self.run_log_dir(run_id).map_err(bad)?;
        if let Some(l) = super::lock(&self.live_logs).get(run_id).and_then(std::sync::Weak::upgrade) {
            return Ok(Read::Live(l));
        }
        if std::fs::symlink_metadata(dir.join(runlog_store::FILE)).is_err() {
            return Err(json!({"error": "not_found", "detail": "that run has no log"}));
        }
        let mut snap = runlog_store::Snapshot::read(&dir, run_id).map_err(|d| json!({"error": "io", "detail": d}))?;
        // A log reread in this app's life must still hold the last anchor this session sent (F4).
        if let Some(last) = super::lock(&self.log_anchors).get(run_id).and_then(|v| v.last().cloned()) {
            snap.require(&last);
        }
        Ok(Read::Snapshot(snap))
    }

    /// The UI's delta read: the written entries after `after_seq` (at most `limit`, capped), the written head, and the
    /// log's break if it has one. The UI applies these in order and asks again only on a gap.
    pub fn run_log_since(&self, run_id: &str, after_seq: u64, limit: usize) -> Value {
        let log = match self.run_log_read(run_id) {
            Ok(l) => l,
            Err(e) => return e,
        };
        let (entries, head_seq) = log.since(after_seq, limit);
        let card = log.card();
        json!({"run_id": run_id, "entries": entries, "head_seq": head_seq, "broken": card.broken, "io_error": card.io_error})
    }

    /// How far the log holds, and how far the signed record vouches for it (`standing`):
    /// - `signed`: the chain holds to its head; the campaign reads back with a valid signature and exactly the core's
    ///   writes (#190's ledger); every anchor in it is one the ledger holds (F3) and this app session sent (F4); and the
    ///   log matches each. Entries after the last anchor are "hash-chained, not yet signed".
    /// - `unconfirmed`: all of that except the session part. The ledger is saved in the app's data folder and does
    ///   survive a restart, but an agent of this user could edit that file, so after a restart it confirms nothing.
    /// - `not_verified`: anything else, with the reason.
    pub fn run_log_verify(&self, run_id: &str) -> Value {
        let log = match self.run_log_read(run_id) {
            Ok(l) => l,
            Err(e) => return e,
        };
        let entries = log.entries();
        let card = log.card();
        let chain = runlog::verify(run_id, &entries);
        let sent = super::lock(&self.log_anchors).get(run_id).cloned().unwrap_or_default();
        let (standing, signed, record_error) = match self.run_log_anchors(run_id) {
            Ok(held) => {
                let (st, s) = runlog::standing(run_id, &entries, &held, &sent);
                (st, Some(s), None)
            }
            Err(why) => (runlog::Standing::NotVerified, None, Some(why)),
        };
        let standing = if card.broken.is_some() { runlog::Standing::NotVerified } else { standing };
        json!({
            "run_id": run_id,
            "head_seq": card.head_seq,
            "chain": chain,
            "broken": card.broken,
            "signed": signed,
            "record_error": record_error,
            "standing": standing,
            "verified": standing == runlog::Standing::Signed,
            "caveat": runlog::CAVEAT,
        })
    }

    /// This run's anchors from progress writes the core's ledger recorded, after checking the campaign reads back with
    /// a valid signature and only the core's writes. A log head line the ledger doesn't hold is an error (F3).
    fn run_log_anchors(&self, run_id: &str) -> Result<Vec<runlog::Anchor>, String> {
        let seats = self.read_run_seats(run_id).map_err(|e| format!("the run's seats can't be read: {e}"))?.ok_or("the run has no seats")?;
        let slug = seats["campaign_slug"].as_str().ok_or("the run's seats name no campaign")?.to_string();
        let bridge = record_bridge(&self.bridge, &self.data).map_err(|e| format!("the record's folder can't be made: {e}"))?;
        let store = self.ledger_store(&slug, &bridge)?;
        let rb = store.read_back(&slug)?;
        let ledger = store.load()?.ok_or("the run record has no ledger")?;
        ledger.check(&rb).map_err(|why| format!("the run record isn't only the core's: {why}"))?;
        runlog::held_anchors(run_id, &ledger.progress, &rb.progress)
    }
}
