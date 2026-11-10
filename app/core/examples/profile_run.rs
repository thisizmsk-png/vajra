//! Resource-profile harness (bench only, never shipped): the real core (App, supervisor, bus, crew tailing, the 1 s change
//! loop of main.rs, user terminals) driven without a window, one scenario per launch. FG-12 S12.1.
//!
//! `profile_run HOME DATA SCENARIO FAKE_AGENT_PY FIXTURE_DIR SECONDS`
//!
//! Scenarios: idle | run1 | run4 | term_flood | term_flood_unacked | terms8.
//! * run1 = one run session (a stand-in agent in a real PTY); run4 = the same plus 4 worker transcripts at 20 lines/s each.
//! * term_flood = one run (a terminal needs a project root today) plus one terminal printing at full speed with an
//!   attentive view (every chunk acknowledged at once); term_flood_unacked = the same with no view reading, so the 1 MiB
//!   credit window is what holds the flood back.
//! * terms8 = 8 idle terminals (the ADR-15 per-project cap).
//! * Not here: "3 projects open". This core allows one active run (`run_active`) and holds no project records yet (the
//!   store is FG-01), so that shape exists only in the webview; it is measured with the shipping binary once the shell has it.
//!
//! Output (stdout): `PID n`, then `RUNS [pid..]` once the runs exist, then at the end `STATS {json}`. The harness ends by
//! itself after SECONDS (a hard upper bound), so nothing keeps running if the sampler is cancelled.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vajra_desktop::desktop::{collect_changes, App};
use vajra_desktop::terminal::{Event, Limits, Terminals, UserPathEnv};

const SIDS: [&str; 1] = ["1b4e28ba-2fa1-41d2-883f-0016d3cca427"];

/// Set by SIGTERM or SIGINT: the loop ends and the harness shuts its terminals and runs down itself, so a cancelled
/// profile leaves no shell behind (they are sessions of their own, which a process-group kill never reaches).
static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn on_stop(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn main() {
    // SAFETY: the handler only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGTERM, on_stop as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_stop as extern "C" fn(libc::c_int) as libc::sighandler_t);
    }
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 7 {
        eprintln!("usage: profile_run HOME DATA SCENARIO FAKE_AGENT_PY FIXTURE_DIR SECONDS");
        std::process::exit(2);
    }
    let (home, data, scenario, agent, fixture) = (PathBuf::from(&a[1]), PathBuf::from(&a[2]), a[3].clone(), a[4].clone(), a[5].clone());
    let secs = a[6].parse::<u64>().unwrap().min(900); // hard cap: a bench never runs unbounded
    std::fs::create_dir_all(home.join(".claude/vajra/campaigns")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    println!("PID {}", std::process::id());
    let app = App::open(&data, &home).unwrap();
    let sub = app.bus.subscribe();

    let (runs, workers, terms) = match scenario.as_str() {
        "idle" => (0, 0, 0),
        "run1" => (1, 0, 0),
        "run4" => (1, 4, 0),
        "term_flood" | "term_flood_unacked" => (1, 0, 1),
        "terms8" => (1, 0, 8),
        other => panic!("unknown scenario {other}"),
    };
    let mut repos = Vec::new();
    for (i, sid) in SIDS.iter().enumerate().take(runs) {
        let repo = home.join(format!("repo{i}"));
        std::fs::create_dir_all(&repo).unwrap();
        let (h, r) = (home.display().to_string(), repo.display().to_string());
        let rate = if workers > 0 { 20.0 } else { 0.0 };
        let argv = vec!["env".to_string(), format!("FG12_FIXTURE={fixture}"), "python3".into(), agent.clone(), h, r, (*sid).into(), workers.to_string(), rate.to_string(), secs.to_string()];
        app.launch(sid, "20261108-0900", &format!("deliver-20261108-0900-{i}"), &repo, |_| Ok(argv), false).unwrap();
        repos.push(repo);
    }
    let pids: Vec<u32> = SIDS.iter().take(runs).filter_map(|s| app.sup.pid(&format!("run-{s}")).ok()).collect();
    println!("RUNS {pids:?}");

    // Terminals: the real module, a real login shell, the credit window at its default.
    let terms_core = Arc::new(Terminals::with(Limits::default(), Arc::new(UserPathEnv::app(home.clone()))));
    let received = Arc::new(AtomicU64::new(0));
    let unacked = scenario == "term_flood_unacked";
    let roots = app.terminal_roots();
    for t in 0..terms {
        let cwd = repos[t % repos.len().max(1)].clone();
        let id = Arc::new(AtomicU32::new(0));
        let (core, got, id2) = (terms_core.clone(), received.clone(), id.clone());
        let sink: vajra_desktop::terminal::Sink = Arc::new(move |e| {
            if let Event::Data(d) = e {
                got.fetch_add(d.len() as u64, Ordering::Relaxed);
                if !unacked {
                    let _ = core.ack(id2.load(Ordering::SeqCst), d.len());
                }
            }
            true
        });
        match terms_core.open(&cwd, &roots, 24, 80, sink) {
            Ok(info) => {
                id.store(info.id, Ordering::SeqCst);
                if scenario.starts_with("term_flood") {
                    let _ = terms_core.write(info.id, b"yes 'flood flood flood flood flood flood flood flood flood flood flood flood'\n");
                }
            }
            Err(e) => eprintln!("terminal {t}: {e}"),
        }
    }

    let t0 = Instant::now();
    // The same loop main.rs runs: coalesce bus events for 1 s, refresh the crew's workers.
    while t0.elapsed() < Duration::from_secs(secs) && !STOP.load(Ordering::SeqCst) {
        let _ = collect_changes(&sub, Duration::from_secs(1));
        app.refresh_workers();
        app.reap_runs();
    }
    println!("STATS {}", serde_json::json!({"scenario": scenario, "terminals_open": terms_core.count(), "terminal_bytes": received.load(Ordering::Relaxed), "seconds": t0.elapsed().as_secs()}));
    terms_core.shutdown();
    app.sup.shutdown();
}
