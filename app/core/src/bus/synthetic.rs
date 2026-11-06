//! Synthetic producers: N streams pushing lines at a fixed rate. Used by the
//! bus tests and by the 10 streams x 200 lines/s memory measurement.

use super::Bus;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Deterministic line text for `stream`/`n`, about `len` bytes.
pub fn line_text(stream: usize, n: u64, len: usize) -> String {
    let mut s = format!("synthetic s{stream} n{n} ");
    while s.len() < len {
        s.push('x');
    }
    s.truncate(len.max(1));
    s
}

/// Stream id used for synthetic stream `i`.
pub fn stream_id(i: usize) -> String {
    format!("synthetic-{i}")
}

/// Push `lines` lines of `len` bytes to each of `streams` streams, as fast as
/// possible, round-robin. Returns the slowest single push.
pub fn burst(bus: &Bus, streams: usize, lines: u64, len: usize) -> Duration {
    let ids: Vec<String> = (0..streams).map(stream_id).collect();
    let mut worst = Duration::ZERO;
    for n in 0..lines {
        for (i, id) in ids.iter().enumerate() {
            let t = Instant::now();
            bus.push(id, &line_text(i, n, len)).expect("valid id");
            worst = worst.max(t.elapsed());
        }
    }
    worst
}

/// Background producers: one thread per stream, each pushing `rate` lines
/// per second until stopped.
pub struct Producer {
    stop: Arc<AtomicBool>,
    pushed: Arc<AtomicU64>,
    threads: Vec<JoinHandle<()>>,
}

impl Producer {
    pub fn start(bus: Arc<Bus>, streams: usize, rate_per_sec: u32, len: usize) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let pushed = Arc::new(AtomicU64::new(0));
        let period = Duration::from_secs(1) / rate_per_sec.max(1);
        let threads = (0..streams)
            .map(|i| {
                let (bus, stop, pushed) = (bus.clone(), stop.clone(), pushed.clone());
                std::thread::spawn(move || {
                    let id = stream_id(i);
                    let start = Instant::now();
                    let mut n = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        bus.push(&id, &line_text(i, n, len)).expect("valid id");
                        pushed.fetch_add(1, Ordering::Relaxed);
                        n += 1;
                        let due = start + period * n as u32;
                        if let Some(wait) = due.checked_duration_since(Instant::now()) {
                            std::thread::sleep(wait);
                        }
                    }
                })
            })
            .collect();
        Self {
            stop,
            pushed,
            threads,
        }
    }

    pub fn pushed(&self) -> u64 {
        self.pushed.load(Ordering::Relaxed)
    }

    /// Stop all threads and return the total lines pushed.
    pub fn stop(self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads {
            let _ = t.join();
        }
        self.pushed.load(Ordering::Relaxed)
    }
}
