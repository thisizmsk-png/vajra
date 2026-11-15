//! New-item notifications (FG-07 S7.6): `inbox_new {project_id, kind, count}`, coalesced over a 5 s window.
//!
//! The inbox is derived on every read and stored nowhere, so "new" means "a key this process has not seen waiting
//! before". The first read only sets the baseline (opening the app is not an arrival). Arrivals are counted per
//! (project, kind); the first arrival opens a window, everything that arrives inside it joins the same count, and one
//! event per (project, kind) leaves when the window ends. The event carries the project id, the kind and a count and
//! nothing from the item: its text is untrusted and may hold secrets. Pure and clock-free so tests pin it.

use std::collections::{BTreeMap, HashSet};

/// How long arrivals are held to be told as one.
pub const WINDOW_MS: u64 = 5_000;

/// What leaves: one line per (project, kind).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrival {
    pub project_id: String,
    pub kind: String,
    pub count: u64,
}

#[derive(Default)]
pub struct Arrivals {
    /// Keys seen waiting at the last read; None until the first read.
    seen: Option<HashSet<String>>,
    pending: BTreeMap<(String, String), u64>,
    /// When the open window ends, if one is open.
    due: Option<u64>,
}

impl Arrivals {
    /// Record one read of the inbox: `(key, project_id, kind)` for every waiting item. Returns when to flush
    /// (ms, absolute) if this read opened a window.
    pub fn note(&mut self, items: &[(String, String, String)], now: u64, window: u64) -> Option<u64> {
        let now_keys: HashSet<String> = items.iter().map(|(k, _, _)| k.clone()).collect();
        let before = self.seen.replace(now_keys)?;
        let mut opened = None;
        for (key, project, kind) in items {
            if before.contains(key) {
                continue;
            }
            *self.pending.entry((project.clone(), kind.clone())).or_insert(0) += 1;
            if self.due.is_none() {
                self.due = Some(now + window);
                opened = self.due;
            }
        }
        opened
    }

    /// The events that are due at `now` (none while the window is still open). Closes the window.
    pub fn flush(&mut self, now: u64) -> Vec<Arrival> {
        if self.due.is_none_or(|d| now < d) {
            return Vec::new();
        }
        self.due = None;
        std::mem::take(&mut self.pending).into_iter().map(|((project_id, kind), count)| Arrival { project_id, kind, count }).collect()
    }
}

/// Emits a named event to the UI (`inbox_new`). Installed by the shell next to the `changed` sink.
pub type EventSink = Box<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// The arrivals state plus where its events go. Shared (`Arc`) so a window's one-shot flush thread can finish it.
pub struct Hub {
    sink: std::sync::Mutex<Option<EventSink>>,
    state: std::sync::Mutex<Arrivals>,
    window_ms: std::sync::atomic::AtomicU64,
}

impl Default for Hub {
    fn default() -> Self {
        Self { sink: Default::default(), state: Default::default(), window_ms: WINDOW_MS.into() }
    }
}

impl Hub {
    pub fn set_sink(&self, sink: EventSink) {
        *self.sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);
    }

    /// Send one named event now (the run log's `run_log`, which batches on its own thread). No sink: nothing.
    pub fn emit(&self, name: &str, payload: serde_json::Value) {
        if let Some(sink) = self.sink.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            sink(name, payload);
        }
    }

    #[cfg(test)]
    pub fn set_window_ms(&self, ms: u64) {
        self.window_ms.store(ms, std::sync::atomic::Ordering::Relaxed);
    }

    /// One read of the inbox. With no sink (tests, the socket server) nothing is held or sent.
    pub fn observe(self: &std::sync::Arc<Self>, items: impl Iterator<Item = (String, String, String)>) {
        if self.sink.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
            return;
        }
        let items: Vec<_> = items.collect();
        let now = super::state::now_ms();
        let window = self.window_ms.load(std::sync::atomic::Ordering::Relaxed);
        let due = self.state.lock().unwrap_or_else(|e| e.into_inner()).note(&items, now, window);
        if let Some(due) = due {
            // One short-lived thread per window, not an always-on timer: it sleeps until the window ends, tells, exits.
            let me = std::sync::Arc::clone(self);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(due.saturating_sub(super::state::now_ms())));
                let out = me.state.lock().unwrap_or_else(|e| e.into_inner()).flush(super::state::now_ms().max(due));
                if let Some(sink) = me.sink.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    for a in out {
                        sink("inbox_new", serde_json::json!({"project_id": a.project_id, "kind": a.kind, "count": a.count}));
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(key: &str, project: &str, kind: &str) -> (String, String, String) {
        (key.into(), project.into(), kind.into())
    }

    // Given the first read of the inbox, Then it is the baseline: items already waiting are not arrivals.
    #[test]
    fn the_first_read_is_a_baseline_not_an_arrival() {
        let mut a = Arrivals::default();
        assert_eq!(a.note(&[it("k1", "p1", "plan")], 0, WINDOW_MS), None);
        assert!(a.flush(60_000).is_empty());
    }

    // Given a burst inside 5 s, Then one window opens and one event per project and kind leaves, with the count.
    #[test]
    fn a_burst_inside_the_window_is_one_event_per_project_and_kind() {
        let mut a = Arrivals::default();
        a.note(&[], 0, WINDOW_MS);
        let first = a.note(&[it("k1", "p1", "plan")], 1_000, WINDOW_MS);
        assert_eq!(first, Some(6_000));
        assert_eq!(a.note(&[it("k1", "p1", "plan"), it("k2", "p1", "plan"), it("k3", "p1", "question"), it("k4", "p2", "plan")], 3_000, WINDOW_MS), None);
        assert!(a.flush(5_999).is_empty(), "held until the window ends");
        let out = a.flush(6_000);
        assert_eq!(
            out,
            vec![
                Arrival { project_id: "p1".into(), kind: "plan".into(), count: 2 },
                Arrival { project_id: "p1".into(), kind: "question".into(), count: 1 },
                Arrival { project_id: "p2".into(), kind: "plan".into(), count: 1 },
            ]
        );
        assert!(a.flush(60_000).is_empty(), "told once");
    }

    // Given an item that stays waiting across reads, Then it is never an arrival again.
    #[test]
    fn a_waiting_item_is_not_new_on_the_next_read() {
        let mut a = Arrivals::default();
        a.note(&[], 0, WINDOW_MS);
        a.note(&[it("k1", "p1", "plan")], 1, WINDOW_MS);
        a.flush(10_000);
        assert_eq!(a.note(&[it("k1", "p1", "plan")], 20_000, WINDOW_MS), None);
        assert!(a.flush(40_000).is_empty());
    }

    // Given an item resolved and a changed plan (a new key) arrives, Then the new key is an arrival.
    #[test]
    fn a_changed_plan_is_a_new_arrival() {
        let mut a = Arrivals::default();
        a.note(&[it("plan:r.0@aaa", "p1", "plan")], 0, WINDOW_MS);
        assert!(a.note(&[it("plan:r.0@bbb", "p1", "plan")], 1_000, WINDOW_MS).is_some());
        assert_eq!(a.flush(7_000), vec![Arrival { project_id: "p1".into(), kind: "plan".into(), count: 1 }]);
    }

    // Given the window ended and another item arrives later, Then it opens a new window of its own.
    #[test]
    fn a_later_arrival_opens_a_new_window() {
        let mut a = Arrivals::default();
        a.note(&[], 0, WINDOW_MS);
        a.note(&[it("k1", "p1", "plan")], 1_000, WINDOW_MS);
        assert_eq!(a.flush(6_000).len(), 1);
        assert_eq!(a.note(&[it("k1", "p1", "plan"), it("k2", "p1", "plan")], 20_000, WINDOW_MS), Some(25_000));
        assert_eq!(a.flush(25_000), vec![Arrival { project_id: "p1".into(), kind: "plan".into(), count: 1 }]);
    }
}
