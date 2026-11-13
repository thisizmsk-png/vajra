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
        let log = Arc::new(RunLog::open(&self.run_log_dir(run_id)?, run_id)?);
        let events = Arc::clone(&self.events);
        log.set_notify(Box::new(move |run, seq| events.emit(EVENT, json!({"run_id": run, "seq": seq}))));
        held.insert(run_id.to_string(), Arc::downgrade(&log));
        Ok(log)
    }

    /// The held log, or the one on disk read once (a finished run): never a second writer for a live one.
    fn run_log_read(&self, run_id: &str) -> Result<Arc<RunLog>, Value> {
        let bad = |d: String| json!({"error": "invalid", "detail": d});
        let dir = self.run_log_dir(run_id).map_err(bad)?;
        if let Some(l) = super::lock(&self.live_logs).get(run_id).and_then(std::sync::Weak::upgrade) {
            return Ok(l);
        }
        if !dir.join(runlog_store::FILE).exists() {
            return Err(json!({"error": "not_found", "detail": "that run has no log"}));
        }
        self.run_log(run_id).map_err(|d| json!({"error": "io", "detail": d}))
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

    /// How far the log holds, and how far the signed record vouches for it. Verified only when the chain holds to its
    /// head, the campaign reads back with a valid signature and exactly the core's writes (#190's ledger), and every
    /// anchor in it matches the log. Entries after the last anchor are "hash-chained, not yet signed".
    pub fn run_log_verify(&self, run_id: &str) -> Value {
        let log = match self.run_log_read(run_id) {
            Ok(l) => l,
            Err(e) => return e,
        };
        let entries = log.entries();
        let card = log.card();
        let chain = runlog::verify(run_id, &entries);
        let (signed, record_error) = match self.run_log_anchors(run_id) {
            Ok(anchors) => (Some(runlog::signed(run_id, &entries, &anchors)), None),
            Err(why) => (None, Some(why)),
        };
        let verified = card.broken.is_none() && chain.broken.is_none() && signed.as_ref().is_some_and(runlog::Signed::verified);
        json!({
            "run_id": run_id,
            "head_seq": card.head_seq,
            "chain": chain,
            "broken": card.broken,
            "signed": signed,
            "record_error": record_error,
            "verified": verified,
        })
    }

    /// This run's anchors, read back from its signed campaign, after checking the campaign holds only the core's writes.
    fn run_log_anchors(&self, run_id: &str) -> Result<Vec<runlog::Anchor>, String> {
        let seats = self.read_run_seats(run_id).map_err(|e| format!("the run's seats can't be read: {e}"))?.ok_or("the run has no seats")?;
        let slug = seats["campaign_slug"].as_str().ok_or("the run's seats name no campaign")?.to_string();
        let bridge = record_bridge(&self.bridge, &self.data).map_err(|e| format!("the record's folder can't be made: {e}"))?;
        let store = self.ledger_store(&slug, &bridge)?;
        let rb = store.read_back(&slug)?;
        let ledger = store.load()?.ok_or("the run record has no ledger")?;
        ledger.check(&rb).map_err(|why| format!("the run record isn't only the core's: {why}"))?;
        Ok(runlog::anchors(run_id, &rb.progress))
    }
}
