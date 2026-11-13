//! W2: the hash-chained run log. Changed-log cases (dropped, reordered, edited, duplicated, spliced, cut short, rewritten),
//! the verifier-only check results (H1), restart after a torn line, deltas by seq, and anchors through the record.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::record::{Event, PlanRecord, RecordCard, RunRecord};
use super::runlog::{self, Anchor, Entry, Kind, Log, Refused, Source, Tokens, Weight, Why};
use super::runlog_store::{self, RunLog};

const RUN: &str = "20261112-0930-w2";

fn scratch() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("vajra-runlog-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn dispatched(n: u32) -> Kind {
    Kind::Dispatched { task: format!("{RUN}.t{n}"), ticket: format!("k{n}"), seat: "bhima".into() }
}

fn log_of(n: u32) -> Log {
    let mut l = Log::new(RUN);
    for i in 1..=n {
        l.append(Source::Core, dispatched(i), 1000 + u64::from(i)).unwrap();
    }
    l
}

fn entries(n: u32) -> Vec<Entry> {
    log_of(n).entries().to_vec()
}

fn brk(v: &[Entry]) -> Option<(u64, Why)> {
    runlog::verify(RUN, v).broken.map(|b| (b.seq, b.why))
}

// ---------------------------------------------------------------- the chain

#[test]
fn a_log_chains_from_a_genesis_bound_to_its_run() {
    let v = entries(5);
    assert_eq!(v.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    assert_eq!(v[0].prev, runlog::genesis(RUN));
    for w in v.windows(2) {
        assert_eq!(w[1].prev, w[0].hash);
    }
    let c = runlog::verify(RUN, &v);
    assert_eq!((c.ok_to, c.broken), (5, None));
}

#[test]
fn an_entry_survives_a_json_round_trip_with_the_same_hash() {
    for e in entries(3) {
        let back: Entry = serde_json::from_slice(&serde_json::to_vec(&e).unwrap()).unwrap();
        assert_eq!(back.compute_hash(), e.hash);
    }
}

#[test]
fn an_edited_entry_is_detected() {
    let mut v = entries(5);
    v[2].kind = Kind::Dispatched { task: format!("{RUN}.t3"), ticket: "k3".into(), seat: "nakula".into() };
    assert_eq!(brk(&v), Some((3, Why::Edited)));
    let mut v = entries(5);
    v[1].at_ms += 1;
    assert_eq!(brk(&v), Some((2, Why::Edited)));
}

#[test]
fn an_edited_and_rehashed_entry_breaks_the_next_link() {
    let mut v = entries(5);
    v[2].kind = Kind::Dispatched { task: format!("{RUN}.t3"), ticket: "k3".into(), seat: "nakula".into() };
    v[2].hash = v[2].compute_hash();
    assert_eq!(brk(&v), Some((4, Why::Unlinked)));
}

#[test]
fn a_dropped_entry_is_detected() {
    let mut v = entries(5);
    v.remove(2);
    assert_eq!(brk(&v), Some((3, Why::Dropped)));
}

#[test]
fn a_dropped_and_renumbered_entry_is_detected() {
    let mut v = entries(5);
    v.remove(2);
    for (i, e) in v.iter_mut().enumerate() {
        e.seq = i as u64 + 1;
        e.hash = e.compute_hash();
    }
    assert_eq!(brk(&v), Some((3, Why::Unlinked)));
}

#[test]
fn reordered_entries_are_detected() {
    let mut v = entries(5);
    v.swap(1, 2);
    assert_eq!(brk(&v), Some((2, Why::Reordered)));
}

#[test]
fn a_repeated_entry_is_detected() {
    let mut v = entries(4);
    let again = v[1].clone();
    v.insert(2, again);
    assert_eq!(brk(&v), Some((3, Why::Reordered)));
}

#[test]
fn another_runs_log_does_not_verify_as_this_one() {
    let mut other = Log::new("20261112-0931-other");
    other.append(Source::Core, dispatched(1), 1).unwrap();
    assert_eq!(brk(other.entries()), Some((1, Why::WrongRun)));
}

#[test]
fn an_entry_of_another_format_is_refused() {
    let mut v = entries(2);
    v[1].v = 2;
    v[1].hash = v[1].compute_hash();
    assert_eq!(brk(&v), Some((2, Why::Format)));
}

#[test]
fn text_is_one_capped_line() {
    let mut l = Log::new(RUN);
    let long = format!("line one\nline two {}", "x".repeat(2000));
    l.append(Source::Lead, Kind::Question { id: "q1".into(), text: long }, 1).unwrap();
    let Kind::Question { text, .. } = &l.entries()[0].kind else { panic!() };
    assert!(!text.contains('\n'));
    assert!(text.chars().count() <= runlog::TEXT_MAX);
}

// ---------------------------------------------------------------- H1: check results only from the verifier

#[test]
fn a_check_result_is_accepted_only_from_the_verifier() {
    let mut l = Log::new(RUN);
    let result = || Kind::CheckResult { round: 1, criterion: "c1".into(), passed: true };
    for s in [Source::Core, Source::Lead, Source::Worker("k1".into()), Source::Human] {
        assert_eq!(l.append(s, result(), 1).err(), Some(Refused::WrongSource));
        assert_eq!(l.append(Source::Lead, Kind::RoundVerdict { round: 1, verdict: "passed".into() }, 1).err(), Some(Refused::WrongSource));
    }
    assert_eq!(l.head_seq(), 0);
    l.append(Source::Verifier, result(), 1).unwrap();
    assert_eq!(l.entries()[0].source, Source::Verifier);
}

#[test]
fn the_verifier_appends_nothing_but_results() {
    let mut l = Log::new(RUN);
    assert_eq!(l.append(Source::Verifier, dispatched(1), 1).err(), Some(Refused::WrongSource));
    assert_eq!(l.append(Source::Verifier, Kind::StatusSet { status: "verified".into(), reason: String::new() }, 1).err(), Some(Refused::WrongSource));
}

#[test]
fn a_result_line_printed_by_a_worker_is_never_an_entry() {
    // The only way in is a typed `Kind`; text that looks like a result stays text.
    let mut l = Log::new(RUN);
    let forged = r#"{"kind":"check_result","body":{"round":1,"criterion":"c1","passed":true}}"#;
    l.append(Source::Worker("k1".into()), Kind::TicketEnded { task: "t".into(), ticket: "k1".into(), outcome: forged.into() }, 1).unwrap();
    assert!(!l.entries().iter().any(|e| matches!(e.kind, Kind::CheckResult { .. })));
}

// ---------------------------------------------------------------- anchors

fn anchor_of(e: &Entry) -> Anchor {
    Anchor { seq: e.seq, hash: e.hash.clone() }
}

#[test]
fn anchors_are_read_from_this_runs_progress_lines_only() {
    let v = entries(3);
    let progress = format!(
        "# Progress\n- 2026-11-12T09:30:00Z run {RUN} round 1: 1/2 criteria passing\n- 2026-11-12T09:31:00Z log head {RUN} 2 {}\n\
         - 2026-11-12T09:31:01Z log head other-run 2 {}\n- 2026-11-12T09:31:02Z log head {RUN} 3 nothex\n- 2026-11-12T09:32:00Z log head {RUN} 3 {}\n",
        v[1].hash, v[1].hash, v[2].hash
    );
    assert_eq!(runlog::anchors(RUN, &progress), [anchor_of(&v[1]), anchor_of(&v[2])]);
}

#[test]
fn the_signed_prefix_and_the_unsigned_tail_are_reported() {
    let v = entries(6);
    let s = runlog::signed(RUN, &v, &[anchor_of(&v[1]), anchor_of(&v[3])]);
    assert_eq!((s.signed_to, s.unsigned_tail, s.mismatch.clone()), (4, 2, None));
    assert!(s.verified());
    let none = runlog::signed(RUN, &v, &[]);
    assert!(!none.verified());
    assert_eq!(none.unsigned_tail, 6);
}

#[test]
fn a_log_cut_short_after_it_was_signed_fails_its_anchor() {
    let v = entries(5);
    let a = [anchor_of(&v[4])];
    let s = runlog::signed(RUN, &v[..4], &a);
    assert_eq!(runlog::verify(RUN, &v[..4]).broken, None, "the chain alone can't see a cut tail");
    assert_eq!(s.mismatch, Some(a[0].clone()));
    assert!(!s.verified());
}

#[test]
fn a_log_rewritten_consistently_after_it_was_signed_fails_its_anchor() {
    let v = entries(4);
    let a = [anchor_of(&v[1]), anchor_of(&v[3])];
    // Rebuild the whole chain with one event changed: every link holds, but it isn't the signed one.
    let mut l = Log::new(RUN);
    for (i, e) in v.iter().enumerate() {
        let k = if i == 2 { Kind::Dispatched { task: "x".into(), ticket: "x".into(), seat: "x".into() } } else { e.kind.clone() };
        l.append(e.source.clone(), k, e.at_ms).unwrap();
    }
    assert_eq!(runlog::verify(RUN, l.entries()).broken, None);
    let s = runlog::signed(RUN, l.entries(), &a);
    assert_eq!(s.mismatch, Some(a[1].clone()));
    assert_eq!(s.signed_to, 0);
}

#[test]
fn an_anchor_past_a_break_does_not_count() {
    let mut v = entries(5);
    let a = [anchor_of(&v[4])];
    v[2].at_ms += 1;
    assert!(!runlog::signed(RUN, &v, &a).verified());
}

#[test]
fn receipts_are_inside_the_signed_prefix() {
    let mut l = log_of(2);
    let tokens = Tokens { fresh: 1200, cache_write: 300, cache_read: 50_000, output: 900, side: 0 };
    let weight = Some(Weight { app_footprint_peak_kib: 190_000, agent_rss_peak_kib: 400_000 });
    l.append(Source::Core, Kind::Receipt { scope: "run".into(), model: "opus".into(), tokens, usd_micros: 166_000, weight }, 9).unwrap();
    let head = anchor_of(l.head().unwrap());
    assert!(runlog::signed(RUN, l.entries(), std::slice::from_ref(&head)).verified());
    let mut v = l.entries().to_vec();
    if let Kind::Receipt { usd_micros, .. } = &mut v[2].kind {
        *usd_micros = 1;
    }
    v[2].hash = v[2].compute_hash();
    assert!(!runlog::signed(RUN, &v, &[head]).verified(), "a changed receipt isn't the signed one");
}

// ---------------------------------------------------------------- the store: restart, deltas, files

fn file(dir: &Path) -> PathBuf {
    dir.join(runlog_store::FILE)
}

#[test]
fn a_restart_resumes_after_the_last_entry() {
    let d = scratch();
    {
        let l = RunLog::open(&d, RUN).unwrap();
        for i in 1..=3 {
            l.append(Source::Core, dispatched(i)).unwrap();
        }
        l.flush();
    }
    let l = RunLog::open(&d, RUN).unwrap();
    assert_eq!(l.head_seq(), 3);
    assert_eq!(l.append(Source::Core, dispatched(4)).unwrap(), 4);
    l.flush();
    let back = runlog_store::parse(&std::fs::read(file(&d)).unwrap());
    assert_eq!(runlog::verify(RUN, &back.entries).ok_to, 4);
}

#[test]
fn a_torn_last_line_is_cut_and_recorded() {
    let d = scratch();
    {
        let l = RunLog::open(&d, RUN).unwrap();
        l.append(Source::Core, dispatched(1)).unwrap();
        l.append(Source::Core, dispatched(2)).unwrap();
        l.flush();
    }
    // A crash mid-append: half a line, no newline.
    let mut f = std::fs::OpenOptions::new().append(true).open(file(&d)).unwrap();
    let torn: &[u8] = br#"{"v":1,"seq":3,"prev":"ab"#;
    std::io::Write::write_all(&mut f, torn).unwrap();
    drop(f);
    let l = RunLog::open(&d, RUN).unwrap();
    l.flush();
    let (got, head) = l.since(0, 100);
    assert_eq!(head, 3);
    assert_eq!(got[2].kind, Kind::Recovered { dropped_bytes: torn.len() as u64, at_seq: 2 });
    assert_eq!(l.card().broken, None);
    let back = runlog_store::parse(&std::fs::read(file(&d)).unwrap());
    assert_eq!((back.torn, runlog::verify(RUN, &back.entries).ok_to), (0, 3));
}

#[test]
fn a_break_inside_the_log_is_never_repaired() {
    let d = scratch();
    {
        let l = RunLog::open(&d, RUN).unwrap();
        for i in 1..=4 {
            l.append(Source::Core, dispatched(i)).unwrap();
        }
        l.flush();
    }
    let edited = String::from_utf8(std::fs::read(file(&d)).unwrap()).unwrap().replacen("\"seat\":\"bhima\"", "\"seat\":\"other\"", 2);
    std::fs::write(file(&d), &edited).unwrap();
    let l = RunLog::open(&d, RUN).unwrap();
    let card = l.card();
    assert_eq!(card.broken.map(|b| (b.seq, b.why)), Some((1, Why::Edited)));
    assert_eq!(l.append(Source::Core, dispatched(5)).err(), Some(Refused::Broken(1)));
    l.flush();
    assert_eq!(std::fs::read_to_string(file(&d)).unwrap(), edited, "the file is left as found");
}

#[test]
fn a_line_that_is_not_an_entry_breaks_the_log_there() {
    let d = scratch();
    {
        let l = RunLog::open(&d, RUN).unwrap();
        l.append(Source::Core, dispatched(1)).unwrap();
        l.flush();
    }
    let mut f = std::fs::OpenOptions::new().append(true).open(file(&d)).unwrap();
    std::io::Write::write_all(&mut f, b"not json\n").unwrap();
    drop(f);
    let l = RunLog::open(&d, RUN).unwrap();
    assert_eq!(l.card().broken.map(|b| (b.seq, b.why)), Some((2, Why::Edited)));
    assert_eq!(l.head_seq(), 1);
}

#[test]
fn a_link_in_place_of_the_log_is_refused() {
    let d = scratch();
    std::fs::create_dir_all(&d).unwrap();
    let target = d.join("elsewhere");
    std::fs::write(&target, b"").unwrap();
    std::os::unix::fs::symlink(&target, file(&d)).unwrap();
    assert!(RunLog::open(&d, RUN).is_err());
}

#[test]
fn the_log_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let d = scratch();
    let l = RunLog::open(&d, RUN).unwrap();
    l.append(Source::Core, dispatched(1)).unwrap();
    l.flush();
    assert_eq!(std::fs::metadata(file(&d)).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
}

#[test]
fn since_returns_only_later_written_entries_and_a_reload_sees_the_same_list() {
    let d = scratch();
    let l = RunLog::open(&d, RUN).unwrap();
    for i in 1..=10 {
        l.append(Source::Core, dispatched(i)).unwrap();
    }
    l.flush();
    let (page, head) = l.since(4, 3);
    assert_eq!((page.iter().map(|e| e.seq).collect::<Vec<_>>(), head), (vec![5, 6, 7], 10));
    assert!(l.since(10, 50).0.is_empty());
    assert!(l.since(99, 50).0.is_empty());
    // A UI that reloads asks from 0 (or its kept seq) and gets the same entries: no gap, no duplicate.
    let mut seen = Vec::new();
    let mut after = 0;
    loop {
        let (page, _) = l.since(after, 4);
        if page.is_empty() {
            break;
        }
        after = page.last().unwrap().seq;
        seen.extend(page);
    }
    let all = l.since(0, 100).0;
    assert_eq!(seen, all);
    drop(l);
    let again = RunLog::open(&d, RUN).unwrap();
    assert_eq!(again.since(0, 100).0, all);
}

#[test]
fn a_page_is_capped() {
    let d = scratch();
    let l = RunLog::open(&d, RUN).unwrap();
    for i in 1..=(runlog_store::PAGE_MAX as u32 + 5) {
        l.append(Source::Core, dispatched(i)).unwrap();
    }
    l.flush();
    assert_eq!(l.since(0, usize::MAX).0.len(), runlog_store::PAGE_MAX);
}

// ---------------------------------------------------------------- notices

fn counting(l: &RunLog) -> Arc<Mutex<Vec<u64>>> {
    let told = Arc::new(Mutex::new(Vec::new()));
    let t = Arc::clone(&told);
    l.set_notify(Box::new(move |run, seq| {
        assert_eq!(run, RUN);
        t.lock().unwrap().push(seq);
    }));
    told
}

#[test]
fn ordinary_entries_are_told_in_one_batch_and_critical_ones_at_once() {
    let d = scratch();
    let l = RunLog::open(&d, RUN).unwrap();
    let told = counting(&l);
    for i in 1..=20 {
        l.append(Source::Core, dispatched(i)).unwrap();
    }
    std::thread::sleep(runlog_store::COALESCE * 3);
    assert_eq!(*told.lock().unwrap(), [20], "20 ordinary entries, one notice");
    let start = std::time::Instant::now();
    l.append(Source::Lead, Kind::Question { id: "q1".into(), text: "which seat?".into() }).unwrap();
    while told.lock().unwrap().len() < 2 && start.elapsed() < std::time::Duration::from_secs(2) {
        std::thread::yield_now();
    }
    assert!(start.elapsed() < std::time::Duration::from_millis(50), "a question is told at once, not batched");
    assert_eq!(*told.lock().unwrap(), [20, 21]);
}

#[test]
fn nothing_is_told_before_it_is_written() {
    let d = scratch();
    let l = RunLog::open(&d, RUN).unwrap();
    let dir = d.clone();
    l.set_notify(Box::new(move |_, seq| {
        let back = runlog_store::parse(&std::fs::read(dir.join(runlog_store::FILE)).unwrap());
        assert!(back.entries.iter().any(|e| e.seq == seq), "seq {seq} told before it was on disk");
    }));
    l.append(Source::Lead, Kind::Question { id: "q".into(), text: "?".into() }).unwrap();
    l.append(Source::Core, dispatched(2)).unwrap();
    l.flush();
}

// ---------------------------------------------------------------- anchors through the signed record

#[derive(Default)]
struct FakeRecord {
    sent: Mutex<Vec<Event>>,
}

impl RunRecord for FakeRecord {
    fn plan(&self, _: &PlanRecord) -> Result<(), String> {
        Ok(())
    }
    fn send(&self, e: Event) {
        self.sent.lock().unwrap().push(e);
    }
    fn card(&self) -> RecordCard {
        RecordCard::default()
    }
    fn repair(&self) -> RecordCard {
        RecordCard::default()
    }
    fn flush(&self) {}
}

impl FakeRecord {
    /// The progress text the scripts would hold: one `- <time> <text>` line per progress write.
    fn progress(&self) -> String {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| if let Event::Progress(t) = e { Some(format!("- 2026-11-12T09:30:00Z {t}\n")) } else { None })
            .collect()
    }
}

#[test]
fn an_anchor_goes_through_the_records_progress_and_is_logged() {
    let d = scratch();
    let l = RunLog::open(&d, RUN).unwrap();
    let rec = Arc::new(FakeRecord::default());
    l.set_record(rec.clone());
    for i in 1..=3 {
        l.append(Source::Core, dispatched(i)).unwrap();
    }
    l.anchor();
    l.flush();
    let all = l.since(0, 100).0;
    assert_eq!(all.len(), 4);
    assert_eq!(all[3].kind, Kind::Anchored { seq: 3, hash: all[2].hash.clone() });
    let anchors = runlog::anchors(RUN, &rec.progress());
    assert_eq!(anchors, [Anchor { seq: 3, hash: all[2].hash.clone() }]);
    let s = runlog::signed(RUN, &all, &anchors);
    assert!(s.verified());
    assert_eq!((s.signed_to, s.unsigned_tail), (3, 1));
    // The anchor line passes the record's own progress check, so the bridge will send it.
    let line = rec.progress();
    let text = line.trim_end().split_once("Z ").unwrap().1;
    assert!(crate::vajra::bridge_record::text(text, crate::vajra::bridge_record::PROGRESS_MAX, true).is_ok());
}

#[test]
fn an_anchor_covers_only_what_was_appended_before_it() {
    let d = scratch();
    let l = RunLog::open(&d, RUN).unwrap();
    let rec = Arc::new(FakeRecord::default());
    l.set_record(rec.clone());
    l.append(Source::Core, dispatched(1)).unwrap();
    l.anchor();
    l.append(Source::Core, dispatched(2)).unwrap();
    l.flush();
    let all = l.since(0, 100).0;
    assert_eq!(runlog::anchors(RUN, &rec.progress()), [Anchor { seq: 1, hash: all[0].hash.clone() }]);
    assert_eq!(runlog::verify(RUN, &all).ok_to, 3);
    assert!(matches!(all[1].kind, Kind::Dispatched { .. }) && matches!(all[2].kind, Kind::Anchored { seq: 1, .. }));
}

#[test]
fn an_empty_log_sends_no_anchor() {
    let d = scratch();
    let l = RunLog::open(&d, RUN).unwrap();
    let rec = Arc::new(FakeRecord::default());
    l.set_record(rec.clone());
    l.anchor();
    l.flush();
    assert!(rec.sent.lock().unwrap().is_empty());
    assert_eq!(l.head_seq(), 0);
}

// ---------------------------------------------------------------- the app's reads and the run view

#[test]
fn the_run_view_ignores_anchor_lines() {
    let base = "# Progress\n- 2026-11-12T09:30:00Z run 20261112-0930 round 1: 1/2 criteria passing\n";
    let anchored = format!("{base}- 2026-11-12T09:31:00Z log head 20261112-0930 7 {}\n", "a".repeat(64));
    assert_eq!(super::runview::timeline(base, None), super::runview::timeline(&anchored, None));
}

#[test]
fn the_app_reads_a_runs_log_by_seq_and_refuses_a_bad_run_id() {
    let d = scratch();
    let app = crate::desktop::App::open(&d.join("data"), &d.join("home")).unwrap();
    for bad in ["../x", "20261112-0930/..", "", "r1"] {
        assert_eq!(app.run_log_since(bad, 0, 10)["error"], "invalid", "{bad}");
    }
    assert_eq!(app.run_log_since("20261112-0930", 0, 10)["error"], "not_found");
    let log = app.run_log("20261112-0930").unwrap();
    for i in 1..=3 {
        log.append(Source::Core, dispatched(i)).unwrap();
    }
    log.flush();
    let v = app.run_log_since("20261112-0930", 1, 10);
    assert_eq!(v["head_seq"], 3);
    assert_eq!(v["entries"].as_array().unwrap().iter().map(|e| e["seq"].as_u64().unwrap()).collect::<Vec<_>>(), [2, 3]);
    assert!(std::ptr::eq(Arc::as_ptr(&app.run_log("20261112-0930").unwrap()), Arc::as_ptr(&log)), "one writer per run");
    // No seats, so no signed record: hash-chained, not verified.
    let ver = app.run_log_verify("20261112-0930");
    assert_eq!((ver["verified"].as_bool(), ver["chain"]["ok_to"].as_u64()), (Some(false), Some(3)));
    assert!(ver["record_error"].is_string());
}
