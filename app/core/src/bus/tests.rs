use super::synthetic;
use super::*;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

struct TmpDir(PathBuf);

impl TmpDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("vajra-desktop-bus-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cfg(ring_bytes: usize, spill_cap_bytes: u64, queue_cap: usize) -> Config {
    Config {
        ring_bytes,
        spill_cap_bytes,
        queue_cap,
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Read every spilled line of `stream` through `read_spill`.
fn all_spill(bus: &Bus, stream: &str) -> (Vec<Line>, bool) {
    let mut offset = bus.tail(stream, 0, 0).spill_start;
    let mut lines = Vec::new();
    let mut gap = false;
    loop {
        let page = bus.read_spill(stream, offset, 100).unwrap();
        gap |= page.gap;
        lines.extend(page.lines);
        offset = page.next_offset;
        if page.end {
            return (lines, gap);
        }
    }
}

#[test]
fn bus_seq_is_monotonic_per_stream() {
    let tmp = TmpDir::new("seq");
    let bus = Bus::open(&tmp.0, Config::default()).unwrap();
    assert_eq!(bus.push("a", "one").unwrap(), 1);
    assert_eq!(bus.push("a", "two").unwrap(), 2);
    assert_eq!(bus.push("b", "one").unwrap(), 1);
    assert_eq!(bus.push("a", "three").unwrap(), 3);
    let t = bus.tail("a", 0, usize::MAX);
    let seqs: Vec<u64> = t.lines.iter().map(|l| l.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3]);
    assert_eq!(t.head_seq, 3);
    assert!(!t.gap);
    assert_eq!(bus.tail("a", 2, usize::MAX).lines[0].text.as_ref(), "three");
    assert!(bus.tail("a", 3, usize::MAX).lines.is_empty());
    assert!(bus.tail("missing", 0, 10).lines.is_empty());
}

#[test]
fn bus_seq_monotonic_across_threads() {
    let tmp = TmpDir::new("seqthreads");
    let bus = Arc::new(Bus::open(&tmp.0, Config::default()).unwrap());
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let bus = bus.clone();
            std::thread::spawn(move || {
                let mut last = 0;
                for _ in 0..1000 {
                    let seq = bus.push("shared", "x").unwrap();
                    assert!(seq > last);
                    last = seq;
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let t = bus.tail("shared", 0, usize::MAX);
    assert_eq!(t.head_seq, 4000);
    assert!(t.lines.windows(2).all(|w| w[1].seq == w[0].seq + 1));
}

#[test]
fn bus_rejects_unsafe_stream_ids() {
    let tmp = TmpDir::new("ids");
    let bus = Bus::open(&tmp.0, Config::default()).unwrap();
    for bad in ["", "../x", "a/b", ".hidden", "a b", &"x".repeat(129)] {
        assert!(bus.push(bad, "x").is_err(), "{bad:?} accepted");
    }
    assert!(bus.push("run-1.agent_2", "x").is_ok());
}

#[test]
fn bus_default_ring_budget_is_256_kib() {
    assert_eq!(RING_BYTES, 256 * 1024, "NFR-PERF-2: 256 KiB retained tail per worker");
    assert_eq!(Config::default().ring_bytes, RING_BYTES);
}

#[test]
fn bus_byte_budget_evicts_oldest() {
    let tmp = TmpDir::new("budget");
    let bus = Bus::open(&tmp.0, cfg(1000, u64::MAX, 16)).unwrap();
    for _ in 0..100 {
        bus.push("s", &"y".repeat(68)).unwrap(); // cost 100 each
    }
    let st = bus.stats();
    assert!(st.ring_bytes <= 1000, "ring {} over budget", st.ring_bytes);
    assert_eq!(st.ring_lines, 10);
    let t = bus.tail("s", 0, usize::MAX);
    assert_eq!(t.lines.first().unwrap().seq, 91);
    assert_eq!(t.lines.last().unwrap().seq, 100);
}

#[test]
fn bus_oversized_line_is_truncated_and_kept() {
    let tmp = TmpDir::new("oversize");
    let bus = Bus::open(&tmp.0, cfg(100, u64::MAX, 16)).unwrap();
    let big = "é".repeat(MAX_LINE_BYTES); // 2 bytes each
    bus.push("s", &big).unwrap();
    let t = bus.tail("s", 0, usize::MAX);
    assert_eq!(t.lines.len(), 1);
    assert!(t.lines[0].text.len() <= MAX_LINE_BYTES);
}

#[test]
fn bus_tail_sets_gap_when_after_seq_fell_off_ring() {
    let tmp = TmpDir::new("gap");
    let bus = Bus::open(&tmp.0, cfg(1000, u64::MAX, 16)).unwrap();
    for _ in 0..100 {
        bus.push("s", &"y".repeat(68)).unwrap();
    }
    // Ring holds 91..=100.
    let t = bus.tail("s", 5, usize::MAX);
    assert!(t.gap);
    assert_eq!(t.lines.first().unwrap().seq, 91);
    assert!(!bus.tail("s", 90, usize::MAX).gap);
    assert!(!bus.tail("s", 95, usize::MAX).gap);
    assert!(bus.tail("s", 89, usize::MAX).gap);
    assert!(t.spill_end > t.spill_start);
}

#[test]
fn bus_tail_respects_max_bytes_but_always_progresses() {
    let tmp = TmpDir::new("maxbytes");
    let bus = Bus::open(&tmp.0, Config::default()).unwrap();
    for _ in 0..10 {
        bus.push("s", "0123456789").unwrap();
    }
    assert_eq!(bus.tail("s", 0, 35).lines.len(), 3);
    assert_eq!(bus.tail("s", 0, 1).lines.len(), 1);
}

#[test]
fn bus_spill_file_is_0600_in_private_dir() {
    let tmp = TmpDir::new("perms");
    let dir = tmp.0.join("spill");
    let bus = Bus::open(&dir, cfg(200, u64::MAX, 16)).unwrap();
    for i in 0..50 {
        bus.push("s", &format!("line {i}")).unwrap();
    }
    let file = dir.join("s.log");
    assert!(file.is_file());
    assert_eq!(mode(&file), 0o600);
    assert_eq!(mode(&dir), 0o700);
}

#[test]
fn bus_read_spill_returns_evicted_lines_in_order() {
    let tmp = TmpDir::new("readspill");
    let bus = Bus::open(&tmp.0, cfg(500, u64::MAX, 16)).unwrap();
    for i in 1..=200u64 {
        // Include a newline to prove framing survives it.
        bus.push("s", &format!("line {i}\nsecond half")).unwrap();
    }
    let ring_first = bus.tail("s", 0, usize::MAX).lines[0].seq;
    let (spilled, gap) = all_spill(&bus, "s");
    assert!(!gap);
    let seqs: Vec<u64> = spilled.iter().map(|l| l.seq).collect();
    assert_eq!(seqs, (1..ring_first).collect::<Vec<_>>());
    assert_eq!(spilled[41].text.as_ref(), "line 42\nsecond half");
    // Paging: a single page with a small budget does not return everything.
    let page = bus.read_spill("s", 0, 50).unwrap();
    assert!(page.lines.len() < spilled.len());
    assert!(!page.end);
    let next = bus.read_spill("s", page.next_offset, 50).unwrap();
    assert_eq!(next.lines[0].seq, page.lines.last().unwrap().seq + 1);
}

#[test]
fn bus_read_spill_bad_offset_is_an_error_not_garbage() {
    let tmp = TmpDir::new("badoffset");
    let bus = Bus::open(&tmp.0, cfg(200, u64::MAX, 16)).unwrap();
    for i in 0..50 {
        bus.push("s", &format!("line {i}")).unwrap();
    }
    assert!(bus.read_spill("s", 3, 100).is_err());
    assert!(bus.read_spill("missing", 0, 100).unwrap().end);
}

#[test]
fn bus_spill_lru_cap_evicts_least_recent_stream() {
    let tmp = TmpDir::new("lru");
    let cap = 2000; // each stream spills about 1.2 KB
    let bus = Bus::open(&tmp.0, cfg(300, cap, 16)).unwrap();
    for s in ["old", "mid", "new"] {
        for i in 0..60 {
            bus.push(s, &format!("{s} line {i:04}")).unwrap();
        }
    }
    let st = bus.stats();
    assert!(st.spill_bytes <= cap, "spill {} over cap", st.spill_bytes);
    // "old" was written least recently, so its spill went first.
    assert!(!tmp.0.join("old.log").exists());
    assert!(tmp.0.join("new.log").exists());
    let on_disk: u64 = std::fs::read_dir(&tmp.0)
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert_eq!(on_disk, st.spill_bytes);
    // Reading the evicted stream from 0 reports the gap.
    let page = bus.read_spill("old", 0, 1000).unwrap();
    assert!(page.gap);
    // Its ring is intact.
    assert_eq!(bus.tail("old", 0, usize::MAX).head_seq, 60);
}

#[test]
fn bus_spill_cap_counts_and_evicts_leftover_files_first() {
    let tmp = TmpDir::new("orphans");
    std::fs::create_dir_all(&tmp.0).unwrap();
    std::fs::write(tmp.0.join("stale.log"), vec![b'z'; 3000]).unwrap();
    let bus = Bus::open(&tmp.0, cfg(300, 4000, 16)).unwrap();
    assert_eq!(bus.stats().spill_bytes, 3000);
    for i in 0..60 {
        bus.push("live", &format!("live line {i:04}")).unwrap();
    }
    assert!(!tmp.0.join("stale.log").exists());
    assert!(tmp.0.join("live.log").exists());
    assert!(bus.stats().spill_bytes <= 4000);
}

#[test]
fn bus_remove_stream_deletes_spill() {
    let tmp = TmpDir::new("remove");
    let bus = Bus::open(&tmp.0, cfg(200, u64::MAX, 16)).unwrap();
    for i in 0..50 {
        bus.push("s", &format!("line {i}")).unwrap();
    }
    bus.remove_stream("s");
    assert!(!tmp.0.join("s.log").exists());
    assert_eq!(bus.stats().spill_bytes, 0);
    assert_eq!(bus.push("s", "fresh").unwrap(), 1);
}

#[test]
fn bus_subscriber_overflow_drops_to_snapshot() {
    let tmp = TmpDir::new("snapshot");
    let bus = Bus::open(&tmp.0, cfg(RING_BYTES, u64::MAX, 8)).unwrap();
    let sub = bus.subscribe();
    for i in 0..100 {
        bus.push("s", &format!("l{i}")).unwrap();
    }
    assert_eq!(sub.len(), 1);
    assert_eq!(sub.try_recv(), Some(Event::SnapshotNeeded));
    let snap = bus.snapshot("s", 5).unwrap();
    assert_eq!(snap.head_seq, 100);
    assert_eq!(snap.lines.len(), 5);
    assert_eq!(snap.lines[0].seq, 96);
    // After taking the marker, events flow again.
    bus.push("s", "after").unwrap();
    match sub.recv_timeout(Duration::from_millis(100)) {
        Some(Event::Line { line, .. }) => assert_eq!(line.seq, 101),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn bus_dropped_subscriber_is_removed() {
    let tmp = TmpDir::new("unsub");
    let bus = Bus::open(&tmp.0, Config::default()).unwrap();
    let sub = bus.subscribe();
    assert_eq!(bus.stats().subscribers, 1);
    drop(sub);
    bus.push("s", "x").unwrap();
    assert_eq!(bus.stats().subscribers, 0);
}

/// Seeded property loop: random line sizes and stream counts; a reader
/// thread subscribes and then stalls (never drains). Push stays fast and
/// memory stays bounded after 100k lines.
#[test]
fn bus_property_push_never_blocks_on_stalled_reader_and_memory_bounded() {
    let tmp = TmpDir::new("property");
    let ring = 64 * 1024;
    let cap = 4 * 1024 * 1024;
    let queue_cap = 256;
    let bus = Arc::new(Bus::open(&tmp.0, cfg(ring, cap, queue_cap)).unwrap());

    // Stalled reader: subscribes, holds its queue, and blocks forever on a
    // lock the main thread holds, so it never drains.
    let stall = Arc::new(Mutex::new(()));
    let held = stall.lock().unwrap();
    let started = Arc::new(AtomicBool::new(false));
    let reader = {
        let (bus, stall, started) = (bus.clone(), stall.clone(), started.clone());
        std::thread::spawn(move || {
            let sub = bus.subscribe();
            started.store(true, Ordering::SeqCst);
            let _g = stall.lock().unwrap();
            sub.len()
        })
    };
    while !started.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }

    let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let streams = 1 + (next() % 8) as usize;
    let mut worst = Duration::ZERO;
    let start = Instant::now();
    for n in 0..100_000u64 {
        let i = (next() % streams as u64) as usize;
        let len = 1 + (next() % 400) as usize;
        let t = Instant::now();
        bus.push(&synthetic::stream_id(i), &synthetic::line_text(i, n, len))
            .unwrap();
        worst = worst.max(t.elapsed());
    }
    let total = start.elapsed();

    let st = bus.stats();
    assert!(
        st.ring_bytes <= streams * ring,
        "ring {} unbounded",
        st.ring_bytes
    );
    assert!(st.spill_bytes <= cap, "spill {} over cap", st.spill_bytes);
    assert!(
        st.queued_events <= queue_cap,
        "queue {} unbounded",
        st.queued_events
    );
    assert_eq!(st.subscribers, 1);
    // No push waits on the reader: generous bounds that a blocked push
    // (which would never return) or a per-push O(n) scan would blow.
    assert!(worst < Duration::from_millis(250), "slowest push {worst:?}");
    assert!(
        total < Duration::from_secs(20),
        "100k pushes took {total:?}"
    );

    drop(held);
    assert_eq!(
        reader.join().unwrap(),
        1,
        "stalled queue holds only the marker"
    );
}

#[test]
fn bus_synthetic_producer_pushes_at_rate() {
    let tmp = TmpDir::new("synthetic");
    let bus = Arc::new(Bus::open(&tmp.0, Config::default()).unwrap());
    let p = synthetic::Producer::start(bus.clone(), 10, 200, 80);
    std::thread::sleep(Duration::from_millis(300));
    let pushed = p.stop();
    // 10 x 200/s x 0.3 s = 600, allow slack for scheduling.
    assert!((300..=900).contains(&pushed), "pushed {pushed}");
    assert_eq!(bus.streams().len(), 10);
    assert!(synthetic::burst(&bus, 2, 10, 20) < Duration::from_secs(1));
}
