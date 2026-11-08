//! Action receipts (`vajra.receipt/1`, ADR-15 section 3).
//!
//! Every UI action carries an `action_id`; the answer is a receipt. The same `action_id` always yields the same
//! receipt, so a double click or a retry runs the effect at most once. A receipt is written to the control ledger
//! (`kind:"receipt"`, a kind the control service's `rebuild` skips) BEFORE the effect runs. If the process dies
//! between "accepted" and the outcome, the next start reports `unknown` and never retries on its own: the caller
//! must look at the project state and issue a new action. `accepted` is never presented as `done`.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ledger::Ledger;

pub const SCHEMA: &str = "vajra.receipt/1";
/// Dedupe window: an `action_id` older than this may be reused.
pub const DEDUPE_MS: u64 = 7 * 24 * 3600 * 1000;
/// How much of the ledger tail is read at startup to rebuild the index.
pub const TAIL_BYTES: u64 = 4 * 1024 * 1024;
/// In-memory index bound (oldest receipts fall out first).
pub const MAX_INDEX: usize = 5000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    /// The desktop UI (a Tauri command).
    Ui,
    /// The Lead chat session over the MCP socket.
    Mcp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Accepted,
    Done,
    Rejected,
    Failed,
    Unknown,
}

impl State {
    fn terminal(self) -> bool {
        self != State::Accepted
    }
}

/// What the caller may do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retry {
    /// Nothing to do (done) or wait for the outcome (accepted).
    No,
    /// Same request again is safe with a NEW action_id.
    Safe,
    /// Refresh the project state first, then decide (stale generation, unknown outcome).
    AfterRefresh,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub schema: String,
    pub action_id: String,
    pub project_id: String,
    pub action: String,
    pub target: String,
    /// The generation the actor saw when it acted (None when the action has no target generation).
    pub expect_generation: Option<u64>,
    /// The generation of the target now (set on `stale_generation`).
    pub current_generation: Option<u64>,
    pub state: State,
    pub reason: Option<String>,
    pub detail: Option<String>,
    pub retry: Retry,
    /// Project state revision this receipt is consistent with.
    pub rev: u64,
    pub ts_ms: u64,
}

pub struct Request<'a> {
    pub action_id: &'a str,
    pub project_id: &'a str,
    pub action: &'a str,
    pub target: &'a str,
    pub expect_generation: Option<u64>,
    pub caller: Caller,
    /// Human-only actions (stop, approve plan) refuse the MCP caller.
    pub human_only: bool,
}

pub enum Begin {
    /// Proceed with the effect, then call `finish`. The receipt is `accepted` and already in the ledger.
    Run(Receipt),
    /// Do not run the effect: this is the (replayed or refusing) receipt to return.
    Done(Receipt),
}

pub struct Store {
    ledger: Ledger,
    by_id: HashMap<String, Receipt>,
    order: VecDeque<String>,
}

fn valid_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Store {
    /// Open the ledger and rebuild the index from its bounded tail. A receipt last seen `accepted` has no outcome:
    /// the process died mid-action, so it becomes `unknown` (and is written as such).
    pub fn open(path: &Path, now_ms: u64) -> io::Result<Self> {
        let records = Ledger::read_tail(path, TAIL_BYTES)?;
        let mut ledger = Ledger::open(path)?;
        ledger.seal_torn_tail()?;
        let mut s = Self { ledger, by_id: HashMap::new(), order: VecDeque::new() };
        for r in records {
            if r.get("kind").and_then(Value::as_str) != Some("receipt") {
                continue;
            }
            let Some(rc) = r.get("receipt").and_then(|v| serde_json::from_value::<Receipt>(v.clone()).ok()) else {
                continue;
            };
            if rc.schema != SCHEMA || now_ms.saturating_sub(rc.ts_ms) > DEDUPE_MS {
                continue;
            }
            s.insert(rc);
        }
        let dangling: Vec<Receipt> = s.by_id.values().filter(|r| r.state == State::Accepted).cloned().collect();
        for mut rc in dangling {
            rc.state = State::Unknown;
            rc.reason = Some("interrupted".into());
            rc.detail = Some("the app stopped before this action reported an outcome; check the project before acting again".into());
            rc.retry = Retry::AfterRefresh;
            rc.ts_ms = now_ms;
            s.persist(&rc)?;
            s.insert(rc);
        }
        Ok(s)
    }

    fn insert(&mut self, rc: Receipt) {
        if !self.by_id.contains_key(&rc.action_id) {
            self.order.push_back(rc.action_id.clone());
            while self.order.len() > MAX_INDEX {
                if let Some(old) = self.order.pop_front() {
                    self.by_id.remove(&old);
                }
            }
        }
        self.by_id.insert(rc.action_id.clone(), rc);
    }

    fn persist(&mut self, rc: &Receipt) -> io::Result<()> {
        self.ledger.append(&json!({"v": 1, "kind": "receipt", "action_id": rc.action_id, "receipt": rc}))
    }

    /// The newest receipts of one project, newest first, at most `n`.
    pub fn recent(&self, project_id: &str, n: usize) -> Vec<&Receipt> {
        self.order.iter().rev().filter_map(|id| self.by_id.get(id)).filter(|r| r.project_id == project_id).take(n).collect()
    }

    pub fn get(&self, action_id: &str) -> Option<&Receipt> {
        self.by_id.get(action_id)
    }

    fn make(req: &Request<'_>, state: State, reason: Option<&str>, detail: Option<&str>, retry: Retry, rev: u64, now_ms: u64) -> Receipt {
        Receipt {
            schema: SCHEMA.into(),
            action_id: req.action_id.into(),
            project_id: req.project_id.into(),
            action: req.action.into(),
            target: req.target.into(),
            expect_generation: req.expect_generation,
            current_generation: None,
            state,
            reason: reason.map(Into::into),
            detail: detail.map(Into::into),
            retry,
            rev,
            ts_ms: now_ms,
        }
    }

    /// Decide what to do with a request. `current_generation` is the target's generation now (None when the
    /// action has no generation to check). Order matters: replay first (so a retry is never re-judged), then
    /// refusals, then the receipt-before-effect write.
    pub fn begin(&mut self, req: &Request<'_>, current_generation: Option<u64>, rev: u64, now_ms: u64) -> io::Result<Begin> {
        if !valid_id(req.action_id) {
            // Not stored: there is no usable key. Fabricate nothing persistent.
            let rc = Self::make(req, State::Rejected, Some("invalid_action_id"), Some("action_id must be 8..=64 chars of A-Z a-z 0-9 - _"), Retry::Safe, rev, now_ms);
            return Ok(Begin::Done(rc));
        }
        if let Some(prev) = self.by_id.get(req.action_id) {
            let same = prev.project_id == req.project_id && prev.action == req.action && prev.target == req.target;
            if !same {
                // Reusing an id for a different action must never replay or overwrite the first receipt.
                let rc = Self::make(req, State::Rejected, Some("action_id_reuse"), Some("this action_id was used for a different action"), Retry::Safe, rev, now_ms);
                return Ok(Begin::Done(rc));
            }
            return Ok(Begin::Done(prev.clone()));
        }
        if req.human_only && req.caller != Caller::Ui {
            return self.reject(req, "human_only", "this action can only be taken by a person in the app", Retry::No, None, rev, now_ms);
        }
        if let (Some(want), Some(have)) = (req.expect_generation, current_generation) {
            if want != have {
                return self.reject(req, "stale_generation", "the target changed since you looked; refresh and try again", Retry::AfterRefresh, Some(have), rev, now_ms);
            }
        }
        let rc = Self::make(req, State::Accepted, None, None, Retry::No, rev, now_ms);
        self.persist(&rc)?;
        self.insert(rc.clone());
        Ok(Begin::Run(rc))
    }

    #[allow(clippy::too_many_arguments)]
    fn reject(&mut self, req: &Request<'_>, reason: &str, detail: &str, retry: Retry, current: Option<u64>, rev: u64, now_ms: u64) -> io::Result<Begin> {
        let mut rc = Self::make(req, State::Rejected, Some(reason), Some(detail), retry, rev, now_ms);
        rc.current_generation = current;
        self.persist(&rc)?;
        self.insert(rc.clone());
        Ok(Begin::Done(rc))
    }

    /// Record the outcome of an accepted action. A receipt that is already terminal is returned unchanged
    /// (first outcome wins).
    pub fn finish(&mut self, action_id: &str, state: State, reason: Option<&str>, detail: Option<&str>, rev: u64, now_ms: u64) -> io::Result<Receipt> {
        self.finish_with_generation(action_id, state, reason, detail, None, rev, now_ms)
    }

    /// `finish` that also records the target's generation after the effect (a start run's new project generation),
    /// so a restart can recover it from the receipts.
    #[allow(clippy::too_many_arguments)]
    pub fn finish_with_generation(&mut self, action_id: &str, state: State, reason: Option<&str>, detail: Option<&str>, current_generation: Option<u64>, rev: u64, now_ms: u64) -> io::Result<Receipt> {
        let Some(prev) = self.by_id.get(action_id).cloned() else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such receipt"));
        };
        if prev.state.terminal() {
            return Ok(prev);
        }
        let mut rc = prev;
        rc.state = state;
        rc.reason = reason.map(Into::into);
        rc.detail = detail.map(Into::into);
        rc.retry = match state {
            State::Rejected if reason == Some("stale_generation") => Retry::AfterRefresh,
            State::Done | State::Accepted | State::Rejected => Retry::No,
            State::Failed => Retry::Safe,
            State::Unknown => Retry::AfterRefresh,
        };
        if current_generation.is_some() {
            rc.current_generation = current_generation;
        }
        rc.rev = rev;
        rc.ts_ms = now_ms;
        if let Err(e) = self.persist(&rc) {
            // The effect ran (or may have) but its outcome is not on disk. Say `unknown` in memory too, so a retry
            // of this action_id is told to look first instead of waiting on an `accepted` that will never finish.
            rc.state = State::Unknown;
            rc.reason = Some("outcome_not_recorded".into());
            rc.detail = Some("the action may have happened but its outcome could not be saved; check the project before acting again".into());
            rc.retry = Retry::AfterRefresh;
            self.insert(rc);
            return Err(e);
        }
        self.insert(rc.clone());
        Ok(rc)
    }

    /// Every receipt in the index (recovery of counters at startup).
    pub fn all(&self) -> impl Iterator<Item = &Receipt> {
        self.by_id.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tempdir() -> TempDir {
        let p = std::env::temp_dir().join(format!("vajra-rcpt-{}-{}", std::process::id(), crate::control::nonce::random_hex(4).unwrap()));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    const T0: u64 = 1_800_000_000_000;

    fn path(dir: &TempDir) -> std::path::PathBuf {
        dir.path().join("ledger.jsonl")
    }

    fn req<'a>(id: &'a str, generation: Option<u64>) -> Request<'a> {
        Request { action_id: id, project_id: "proj-1", action: "stop_worker", target: "w1", expect_generation: generation, caller: Caller::Ui, human_only: true }
    }

    fn run(b: Begin) -> Receipt {
        match b {
            Begin::Run(r) => r,
            Begin::Done(r) => panic!("expected Run, got {r:?}"),
        }
    }
    fn done(b: Begin) -> Receipt {
        match b {
            Begin::Done(r) => r,
            Begin::Run(r) => panic!("expected Done, got {r:?}"),
        }
    }

    // Given a first click is accepted and finished, When the same action_id arrives again (double click),
    // Then the stored receipt is returned and the effect is not run twice.
    #[test]
    fn double_click_returns_same_receipt_without_a_second_effect() {
        let d = tempdir();
        let mut s = Store::open(&path(&d), T0).unwrap();
        let a = run(s.begin(&req("act-0001-aaaa", Some(1)), Some(1), 4, T0).unwrap());
        assert_eq!(a.state, State::Accepted);
        let fin = s.finish("act-0001-aaaa", State::Done, None, None, 5, T0 + 1).unwrap();
        assert_eq!((fin.state, fin.rev), (State::Done, 5));
        let again = done(s.begin(&req("act-0001-aaaa", Some(1)), Some(1), 6, T0 + 2).unwrap());
        assert_eq!(again, fin);
        // A double click while still in flight gets the accepted receipt, never "done".
        let b = run(s.begin(&req("act-0002-bbbb", Some(1)), Some(1), 6, T0 + 3).unwrap());
        let second = done(s.begin(&req("act-0002-bbbb", Some(1)), Some(1), 6, T0 + 4).unwrap());
        assert_eq!((second.state, second.retry), (State::Accepted, Retry::No));
        assert_eq!(second, b);
    }

    // Given the app died between accepted and the outcome, When it restarts, Then the receipt is unknown,
    // is not retried by itself, and the same action_id keeps answering unknown.
    #[test]
    fn retry_after_crash_is_unknown_and_never_reruns() {
        let d = tempdir();
        {
            let mut s = Store::open(&path(&d), T0).unwrap();
            run(s.begin(&req("act-0003-cccc", Some(2)), Some(2), 1, T0).unwrap());
        } // crash: no finish
        let mut s = Store::open(&path(&d), T0 + 10).unwrap();
        let r = done(s.begin(&req("act-0003-cccc", Some(2)), Some(2), 2, T0 + 11).unwrap());
        assert_eq!((r.state, r.retry), (State::Unknown, Retry::AfterRefresh));
        assert_eq!(r.reason.as_deref(), Some("interrupted"));
        // And it survives another restart as unknown, not as accepted again.
        let s2 = Store::open(&path(&d), T0 + 20).unwrap();
        assert_eq!(s2.get("act-0003-cccc").unwrap().state, State::Unknown);
    }

    // Given a worker moved on to attempt 3, When an action carrying attempt 2 arrives, Then it is rejected as
    // stale_generation with the current generation, and the effect never runs.
    #[test]
    fn stale_generation_is_rejected() {
        let d = tempdir();
        let mut s = Store::open(&path(&d), T0).unwrap();
        let r = done(s.begin(&req("act-0004-dddd", Some(2)), Some(3), 9, T0).unwrap());
        assert_eq!((r.state, r.reason.as_deref(), r.current_generation, r.retry), (State::Rejected, Some("stale_generation"), Some(3), Retry::AfterRefresh));
        // Replaying the id gives the same rejection even if the generation now matches.
        let again = done(s.begin(&req("act-0004-dddd", Some(2)), Some(2), 10, T0 + 1).unwrap());
        assert_eq!(again, r);
    }

    #[test]
    fn mcp_caller_cannot_take_a_human_only_action() {
        let d = tempdir();
        let mut s = Store::open(&path(&d), T0).unwrap();
        let mut q = req("act-0005-eeee", None);
        q.caller = Caller::Mcp;
        let r = done(s.begin(&q, None, 1, T0).unwrap());
        assert_eq!((r.state, r.reason.as_deref()), (State::Rejected, Some("human_only")));
    }

    #[test]
    fn reusing_an_action_id_for_another_action_never_replays_or_overwrites() {
        let d = tempdir();
        let mut s = Store::open(&path(&d), T0).unwrap();
        let first = run(s.begin(&req("act-0006-ffff", None), None, 1, T0).unwrap());
        let mut other = req("act-0006-ffff", None);
        other.target = "w2";
        let r = done(s.begin(&other, None, 1, T0 + 1).unwrap());
        assert_eq!(r.reason.as_deref(), Some("action_id_reuse"));
        assert_eq!(s.get("act-0006-ffff").unwrap(), &first);
    }

    #[test]
    fn invalid_action_ids_are_rejected_and_not_stored() {
        let d = tempdir();
        let mut s = Store::open(&path(&d), T0).unwrap();
        for bad in ["", "short", "has space in it", &"x".repeat(65), "bad/slash-1234"] {
            let r = done(s.begin(&req(bad, None), None, 1, T0).unwrap());
            assert_eq!(r.reason.as_deref(), Some("invalid_action_id"), "{bad}");
        }
        assert!(s.by_id.is_empty());
    }

    #[test]
    fn first_outcome_wins() {
        let d = tempdir();
        let mut s = Store::open(&path(&d), T0).unwrap();
        run(s.begin(&req("act-0007-gggg", None), None, 1, T0).unwrap());
        s.finish("act-0007-gggg", State::Failed, Some("io"), None, 2, T0 + 1).unwrap();
        let r = s.finish("act-0007-gggg", State::Done, None, None, 3, T0 + 2).unwrap();
        assert_eq!((r.state, r.retry), (State::Failed, Retry::Safe));
    }

    // A ledger that cannot be written means the effect must not run.
    #[test]
    fn receipt_before_effect_a_broken_ledger_refuses_to_start_the_action() {
        let d = tempdir();
        let mut s = Store::open(&path(&d), T0).unwrap();
        s.ledger.break_for_test();
        assert!(s.begin(&req("act-0008-hhhh", None), None, 1, T0).is_err());
        assert!(s.get("act-0008-hhhh").is_none());
    }

    #[test]
    fn old_receipts_fall_out_of_the_dedupe_window() {
        let d = tempdir();
        {
            let mut s = Store::open(&path(&d), T0).unwrap();
            run(s.begin(&req("act-0009-iiii", None), None, 1, T0).unwrap());
            s.finish("act-0009-iiii", State::Done, None, None, 2, T0).unwrap();
        }
        let s = Store::open(&path(&d), T0 + DEDUPE_MS + 1).unwrap();
        assert!(s.get("act-0009-iiii").is_none());
    }

    // The wire shape is a contract shared with the UI (`app/ui/tests/receipt-contract.test.ts` reads the same files).
    #[test]
    fn receipts_serialise_exactly_as_the_contract_fixtures() {
        let d = tempdir();
        let mut s = Store::open(&path(&d), T0).unwrap();
        run(s.begin(&req("act-0001-aaaa", Some(2)), Some(2), 4, T0 + 1).unwrap());
        let done_rc = s.finish("act-0001-aaaa", State::Done, None, None, 5, T0 + 1).unwrap();
        let want: Value = serde_json::from_str(include_str!("../../tests/contract/receipt_done.json")).unwrap();
        assert_eq!(serde_json::to_value(&done_rc).unwrap(), want);
        let stale = done(s.begin(&req("act-0004-dddd", Some(2)), Some(3), 9, T0).unwrap());
        let want: Value = serde_json::from_str(include_str!("../../tests/contract/receipt_stale.json")).unwrap();
        assert_eq!(serde_json::to_value(&stale).unwrap(), want);
    }

    // Audit: an outcome that cannot be saved must not leave the receipt `accepted` for ever.
    #[test]
    fn an_unsaved_outcome_reads_unknown_on_retry() {
        let d = tempdir();
        let mut s = Store::open(&path(&d), T0).unwrap();
        run(s.begin(&req("act-0010-jjjj", None), None, 1, T0).unwrap());
        s.ledger.break_for_test();
        assert!(s.finish("act-0010-jjjj", State::Done, None, None, 2, T0 + 1).is_err());
        let r = done(s.begin(&req("act-0010-jjjj", None), None, 3, T0 + 2).unwrap());
        assert_eq!((r.state, r.reason.as_deref(), r.retry), (State::Unknown, Some("outcome_not_recorded"), Retry::AfterRefresh));
    }

    // Audit: a crash mid-append leaves a line without a newline; the next record must not be glued onto it.
    #[test]
    fn a_torn_last_line_does_not_swallow_the_next_receipt() {
        let d = tempdir();
        let p = path(&d);
        std::fs::write(&p, "{\"v\":1,\"kind\":\"receipt\",\"action_id\":\"act-torn-0001\",\"rec").unwrap();
        {
            let mut s = Store::open(&p, T0).unwrap();
            run(s.begin(&req("act-0011-kkkk", None), None, 1, T0).unwrap());
        }
        let s = Store::open(&p, T0 + 1).unwrap();
        assert_eq!(s.get("act-0011-kkkk").unwrap().state, State::Unknown, "the record after the fragment was read back");
    }

    #[test]
    fn tail_read_drops_a_partial_first_line() {
        let d = tempdir();
        let p = path(&d);
        std::fs::write(&p, "{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n").unwrap();
        let v = Ledger::read_tail(&p, 12).unwrap();
        assert_eq!(v, vec![json!({"c": 3})]);
    }
}
