use super::*;
use std::sync::mpsc;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vajra-term-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

fn spec(home: &Path) -> OpenSpec {
    OpenSpec { cwd: home.to_path_buf(), rows: 24, cols: 80, shell: Some("/bin/sh".into()), args: Some(vec![]), home: Some(home.to_path_buf()) }
}

struct Collect(mpsc::Receiver<Event>);

fn sink() -> (Sink, Collect) {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    (Arc::new(move |e| { let _ = lock(&tx).send(e); }), Collect(rx))
}

impl Collect {
    /// Read until `needle` appears in the output (or `secs` pass); returns everything seen.
    fn until(&self, needle: &str, secs: u64) -> String {
        let mut all = Vec::new();
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if let Ok(Event::Data(d)) = self.0.recv_timeout(Duration::from_millis(100)) {
                all.extend(d);
                if String::from_utf8_lossy(&all).contains(needle) {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&all).into_owned()
    }
    fn exit(&self, secs: u64) -> Option<Option<i32>> {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if let Ok(Event::Exit(c)) = self.0.recv_timeout(Duration::from_millis(100)) {
                return Some(c);
            }
        }
        None
    }
}

fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[test]
fn types_a_command_and_sees_its_output_in_the_project_dir_with_the_temp_home() {
    let d = tmp("echo");
    let (s, out) = sink();
    let t = Terminals::new();
    let id = t.open(spec(&d), s).unwrap();
    t.write(id, b"pwd; echo HOME=$HOME; echo MARK$((20+22))\n").unwrap();
    let seen = out.until("MARK42", 5);
    assert!(seen.contains(d.to_str().unwrap()) && seen.contains(&format!("HOME={}", d.display())) && seen.contains("MARK42"), "{seen}");
}

#[test]
fn resize_reaches_the_shell() {
    let d = tmp("resize");
    let (s, out) = sink();
    let t = Terminals::new();
    let id = t.open(spec(&d), s).unwrap();
    t.resize(id, 40, 132).unwrap();
    t.write(id, b"stty size\n").unwrap();
    let seen = out.until("40 132", 5);
    assert!(seen.contains("40 132"), "{seen}");
    assert_eq!(t.resize(id, 0, 10), Err(TermError::InvalidSize));
}

#[test]
fn the_shell_gets_only_the_allowed_environment() {
    let d = tmp("env");
    // SAFETY: a unique name no other test reads.
    unsafe { std::env::set_var("VAJRA_SPIKE_SECRET", "leakme") };
    let (s, out) = sink();
    let t = Terminals::new();
    let id = t.open(spec(&d), s).unwrap();
    t.write(id, b"echo S=[$VAJRA_SPIKE_SECRET] T=$TERM\n").unwrap();
    let seen = out.until("T=xterm-256color", 5);
    assert!(seen.contains("S=[]") && seen.contains("T=xterm-256color"), "{seen}");
}

#[test]
fn rejects_a_bad_cwd_a_relative_cwd_and_a_zero_size() {
    let d = tmp("bad");
    let t = Terminals::new();
    let (s, _o) = sink();
    let mut sp = spec(&d);
    sp.cwd = d.join("missing");
    assert_eq!(t.open(sp.clone(), s.clone()).unwrap_err(), TermError::BadCwd);
    sp.cwd = PathBuf::from("relative");
    assert_eq!(t.open(sp.clone(), s.clone()).unwrap_err(), TermError::BadCwd);
    sp = spec(&d);
    sp.rows = 0;
    assert_eq!(t.open(sp, s).unwrap_err(), TermError::InvalidSize);
}

#[test]
fn close_kills_the_shell_and_what_it_started_in_the_background() {
    let d = tmp("close");
    let (s, out) = sink();
    let t = Terminals::new();
    let id = t.open(spec(&d), s).unwrap();
    t.write(id, b"sleep 300 & echo BG=$!\n").unwrap();
    // The tty echoes the typed line (`BG=$!`) first; the answer is the occurrence followed by digits.
    let mut seen = out.until("BG=1", 5);
    for _ in 0..50 {
        if seen.split("BG=").skip(1).any(|r| r.starts_with(|c: char| c.is_ascii_digit())) {
            break;
        }
        seen.push_str(&out.until("BG=", 1));
    }
    let bg: u32 = seen.split("BG=").skip(1).find_map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()).unwrap();
    let shell = t.pid(id).unwrap();
    assert!(alive(bg) && alive(shell));
    t.close(id).unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    while (alive(bg) || alive(shell)) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!alive(bg), "the background job outlived its tab");
    assert!(!alive(shell), "the shell outlived its tab");
    assert_eq!(t.count(), 0);
}

#[test]
fn a_shell_that_ignores_hangup_is_killed() {
    let d = tmp("trap");
    let (s, out) = sink();
    let t = Terminals::new();
    let id = t.open(spec(&d), s).unwrap();
    t.write(id, b"trap '' HUP TERM; echo PID=$$; while :; do sleep 1; done\n").unwrap();
    let seen = out.until("PID=", 5);
    let pid = t.pid(id).unwrap();
    assert!(seen.contains("PID="));
    let t0 = Instant::now();
    t.close(id).unwrap();
    assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
    std::thread::sleep(Duration::from_millis(100));
    assert!(!alive(pid));
}

#[test]
fn shutdown_closes_every_terminal_and_refuses_new_ones() {
    let d = tmp("quit");
    let t = Terminals::new();
    let mut pids = Vec::new();
    for _ in 0..3 {
        let (s, _o) = sink();
        let id = t.open(spec(&d), s).unwrap();
        pids.push(t.pid(id).unwrap());
    }
    t.shutdown();
    std::thread::sleep(Duration::from_millis(100));
    assert!(pids.iter().all(|p| !alive(*p)));
    let (s, _o) = sink();
    assert_eq!(t.open(spec(&d), s).unwrap_err(), TermError::ShuttingDown);
}

#[test]
fn dropping_the_registry_kills_the_shells() {
    let d = tmp("drop");
    let pid = {
        let t = Terminals::new();
        let (s, _o) = sink();
        let id = t.open(spec(&d), s).unwrap();
        t.pid(id).unwrap()
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(!alive(pid));
}

#[test]
fn reports_the_exit_code_when_the_shell_ends_by_itself() {
    let d = tmp("exit");
    let (s, out) = sink();
    let t = Terminals::new();
    let id = t.open(spec(&d), s).unwrap();
    t.write(id, b"exit 7\n").unwrap();
    assert_eq!(out.exit(5), Some(Some(7)));
    t.close(id).unwrap();
}

#[test]
fn output_waits_for_acks_instead_of_queueing_without_bound() {
    let d = tmp("flow");
    let (s, out) = sink();
    let t = Terminals::new();
    let id = t.open(spec(&d), s).unwrap();
    t.write(id, b"yes | head -c 20000000; echo DO$((1+1))NE\n").unwrap();
    std::thread::sleep(Duration::from_millis(800));
    // Nobody acks: the reader stops near the high-water mark.
    let mut got = 0usize;
    while let Ok(Event::Data(d)) = out.0.try_recv() {
        got += d.len();
    }
    assert!((HIGH_WATER..=HIGH_WATER + 2 * CHUNK).contains(&got), "read {got} bytes before the first ack");
    t.ack(id, got).unwrap();
    // Acking lets it finish.
    let end = Instant::now() + Duration::from_secs(20);
    let mut tail = String::new();
    while Instant::now() < end && !tail.contains("DO2NE") {
        if let Ok(Event::Data(d)) = out.0.recv_timeout(Duration::from_millis(100)) {
            t.ack(id, d.len()).unwrap();
            tail.push_str(&String::from_utf8_lossy(&d[d.len().saturating_sub(64)..]));
        }
    }
    assert!(tail.contains("DO2NE"), "the flood never finished after acks; tail={:?} {}", &tail[tail.len().saturating_sub(120)..], tail.len());
}

/// `cargo test --release --lib terminal::tests::bench -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_throughput_cat_50mb() {
    let d = tmp("bench");
    let file = d.join("big.bin");
    // 50 MB of text-like output (lines), the shape `cat` of a log has.
    let line = b"2026-11-08T04:43:09.1728745Z the quick brown fox jumps over the lazy dog 0123456789\n";
    let mut f = std::fs::File::create(&file).unwrap();
    let mut written = 0usize;
    while written < 50 * 1024 * 1024 {
        f.write_all(line).unwrap();
        written += line.len();
    }
    drop(f);
    let (s, out) = sink();
    let t = Terminals::new();
    let id = t.open(spec(&d), s).unwrap();
    out.until("$", 3);
    let t0 = Instant::now();
    t.write(id, format!("cat {}; echo END$((1+1))MARK\n", file.display()).as_bytes()).unwrap();
    let (mut bytes, mut chunks, mut tail) = (0usize, 0usize, String::new());
    while !tail.contains("END2MARK") && t0.elapsed() < Duration::from_secs(120) {
        if let Ok(Event::Data(d)) = out.0.recv_timeout(Duration::from_millis(50)) {
            bytes += d.len();
            chunks += 1;
            t.ack(id, d.len()).unwrap();
            tail.push_str(&String::from_utf8_lossy(&d[d.len().saturating_sub(64)..]));
            if tail.len() > 256 { tail = tail[tail.len() - 128..].to_string(); }
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    println!("BENCH throughput: {} bytes in {:.2}s = {:.1} MB/s, {} chunks (avg {} B)", bytes, secs, bytes as f64 / secs / 1e6, chunks, bytes / chunks.max(1));
}

fn ps(pid: u32, col: &str) -> String {
    let o = std::process::Command::new("ps").args(["-o", &format!("{col}="), "-p", &pid.to_string()]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn cpu_secs(pid: u32) -> f64 {
    // "M:SS.ss" or "H:MM:SS.ss"
    ps(pid, "time").split(':').fold(0.0, |a, p| a * 60.0 + p.parse::<f64>().unwrap_or(0.0))
}

/// Memory per open terminal (core-side thread/buffers and the login shell) and idle CPU.
#[test]
#[ignore]
fn bench_memory_and_idle_cpu() {
    let d = tmp("mem");
    let me = std::process::id();
    let rss_kb = |pid: u32| ps(pid, "rss").parse::<u64>().unwrap_or(0);
    let zsh = std::path::Path::new("/bin/zsh").exists();
    let mk = |t: &Terminals| {
        let (s, o) = sink();
        let mut sp = spec(&d);
        if zsh {
            // The user's real login shell shape: a login shell that reads the profile (HOME is the temp dir, so none).
            sp.shell = Some("/bin/zsh".into());
            sp.args = Some(vec!["-l".into()]);
        }
        let id = t.open(sp, s).unwrap();
        (id, o)
    };
    let t = Terminals::new();
    std::thread::sleep(Duration::from_millis(300));
    let base = rss_kb(me);
    let mut keep = Vec::new();
    for _ in 0..8 {
        keep.push(mk(&t));
    }
    std::thread::sleep(Duration::from_secs(2));
    // Drain what the shells printed so the (unbounded here) test channel does not count.
    for (id, o) in &keep {
        while let Ok(Event::Data(d)) = o.0.try_recv() {
            t.ack(*id, d.len()).unwrap();
        }
    }
    let after = rss_kb(me);
    let shells: Vec<u64> = keep.iter().map(|(id, _)| rss_kb(t.pid(*id).unwrap())).collect();
    println!("BENCH core RSS before {} KB, after 8 terminals {} KB => {} KB per terminal (core side)", base, after, after.saturating_sub(base) / 8);
    println!("BENCH shell RSS per terminal: {:?} KB (avg {})", shells, shells.iter().sum::<u64>() / shells.len() as u64);
    let pids: Vec<u32> = keep.iter().map(|(id, _)| t.pid(*id).unwrap()).collect();
    let (c0, s0) = (cpu_secs(me), pids.iter().map(|p| cpu_secs(*p)).sum::<f64>());
    std::thread::sleep(Duration::from_secs(10));
    let (c1, s1) = (cpu_secs(me), pids.iter().map(|p| cpu_secs(*p)).sum::<f64>());
    println!("BENCH idle CPU over 10 s with 8 idle terminals: core {:.2}s ({:.2}%), shells {:.2}s total ({:.2}%)", c1 - c0, (c1 - c0) * 10.0, s1 - s0, (s1 - s0) * 10.0);
    println!("BENCH idle threads: {} terminals => {} extra threads", keep.len(), 2 * keep.len());
}
