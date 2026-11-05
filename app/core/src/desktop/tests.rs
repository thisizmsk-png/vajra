use super::*;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

/// A temp dir holding `home/` and `data/`; removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        let d = std::env::temp_dir()
            .join(format!("vajra-main-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("home/.claude/vajra/campaigns")).unwrap();
        Self(d)
    }
    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
    fn app(&self) -> App {
        App::open(&self.0.join("data"), &self.home()).unwrap()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn code(v: &Value) -> &str {
    v["error"].as_str().unwrap_or("")
}

#[test]
fn crew_tails_worker_transcripts_into_streams_and_inspect() {
    // Review #8: the C5 tailer feeds per-worker streams and the Lead's inspect.
    let t = Tmp::new();
    let app = t.app();
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let cwd = t.0.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let sub = t.home().join(".claude/projects").join(project_slug(&cwd)).join(sid).join("subagents");
    fs::create_dir_all(&sub).unwrap();
    let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts/session/subagents");
    for f in ["agent-aa870dd40540481f0.jsonl", "agent-aa870dd40540481f0.meta.json"] {
        fs::copy(fx.join(f), sub.join(f)).unwrap();
    }
    let stream = format!("run-{sid}");
    app.sup
        .spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: cwd.clone(), rows: 24, cols: 80, redact: Redactor::default() })
        .unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: cwd.clone(), nonces: Redactor::default() });
    lock(&app.control).start_session(sid, "20261105-1230").unwrap();
    app.crew("c", "20261105-1230").unwrap();
    let w = app.bus.tail("w-aa870dd40540481f0", 0, MAX_PAGE);
    assert!(w.lines.iter().any(|l| l.text.contains("step1")), "worker stream has transcript lines");
    let i = lock(&app.control).inspect_worker(Actor::Lead, "aa870dd40540481f0", 50).unwrap();
    let lines = i["lines"].as_array().unwrap();
    assert!(!lines.is_empty() && lines.iter().all(|l| l["untrusted"] == true), "{i}");
    assert_eq!(i["cwd"], "/repo");
    // A second crew call reads only new bytes: no duplicate lines.
    let n = app.bus.tail("w-aa870dd40540481f0", 0, MAX_PAGE).lines.len();
    app.crew("c", "20261105-1230").unwrap();
    assert_eq!(app.bus.tail("w-aa870dd40540481f0", 0, MAX_PAGE).lines.len(), n);
    app.sup.kill(&stream).unwrap();
}

#[test]
fn pages_are_capped_at_64_kib() {
    assert_eq!(MAX_PAGE, 64 * 1024);
}

#[test]
fn one_active_run_at_a_time() {
    // The control service steers one session; a second concurrent run would
    // get the first run's messages and stops (review #4).
    let t = Tmp::new();
    let app = t.app();
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let stream = format!("run-{sid}");
    app.sup
        .spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: t.0.clone(), rows: 24, cols: 80, redact: Redactor::default() })
        .unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: t.0.clone(), nonces: Redactor::default() });
    let cwd = t.0.to_string_lossy().to_string();
    let input = RunInput { ask: "x".into(), max_rounds: 1, base: "main".into(), max_in_flight: 1, campaign: None };
    assert_eq!(code(&app.start_run(input, &cwd).unwrap_err()), "run_active");
    let other = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    assert_eq!(code(&app.resume_run(other, "20261105-1231", "c2", &cwd).unwrap_err()), "run_active");
    // A past run's crew view never retargets the control service.
    let before = lock(&app.control).nonce().map(str::to_string);
    let _ = app.crew("c-old", "20261101-0000");
    assert_eq!(lock(&app.control).nonce().map(str::to_string), before);
    app.sup.kill(&stream).unwrap();
}


#[test]
fn project_slug_matches_claude_code() {
    assert_eq!(project_slug(Path::new("/Users/a/my.repo")), "-Users-a-my-repo");
}

#[test]
fn changes_coalesce_into_one_payload() {
    let t = Tmp::new();
    let app = t.app();
    let sub = app.bus.subscribe();
    assert!(collect_changes(&sub, Duration::from_millis(20)).is_none());
    app.bus.push("s1", "a").unwrap();
    app.bus.push("s2", "b").unwrap();
    app.bus.push("s1", "c").unwrap();
    let p = collect_changes(&sub, Duration::from_secs(1)).unwrap();
    assert_eq!(p["streams"], json!(["s1", "s2"]));
    assert_eq!(p["snapshot_needed"], json!(false));
    assert!(sub.is_empty());
}

#[test]
fn tail_and_read_spill_return_json_pages() {
    let t = Tmp::new();
    let app = t.app();
    app.bus.push("s", "one").unwrap();
    app.bus.push("s", "two").unwrap();
    let v = app.tail("s", 0, 1 << 20).unwrap();
    assert_eq!(v["head_seq"], json!(2));
    assert_eq!(v["lines"][1]["text"], json!("two"));
    assert_eq!(code(&app.tail("../x", 0, 10).unwrap_err()), "invalid_args");
    let p = app.read_spill("s", 0, 1024).unwrap();
    assert!(p["lines"].as_array().unwrap().is_empty());
    assert_eq!(code(&app.read_spill("a/b", 0, 10).unwrap_err()), "invalid_args");
}

#[test]
fn start_run_rejects_bad_input_before_creating_a_session() {
    let t = Tmp::new();
    let app = t.app();
    let input = |ask: &str| RunInput {
        ask: ask.into(),
        max_rounds: 2,
        base: "main".into(),
        max_in_flight: 4,
        campaign: None,
    };
    let cwd = t.0.to_string_lossy().into_owned();
    assert_eq!(code(&app.start_run(input("   "), &cwd).unwrap_err()), "invalid_args");
    assert_eq!(code(&app.start_run(input("do it"), "relative/dir").unwrap_err()), "invalid_args");
    assert!(!t.0.join("data/control/sessions").exists() || fs::read_dir(t.0.join("data/control/sessions")).unwrap().next().is_none());
    assert_eq!(app.active_runs().unwrap(), json!({"count": 0, "runs": []}));
}

#[test]
fn resume_run_validates_ids() {
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.to_string_lossy().into_owned();
    let e = app.resume_run("bad id;rm", "20261105-1810", "deliver-x", &cwd).unwrap_err();
    assert_eq!(code(&e), "invalid_args");
    let e = app.resume_run("abc-12345678", "2026", "deliver-x", &cwd).unwrap_err();
    assert_eq!(code(&e), "invalid_args");
    let e = app.resume_run("abc-12345678", "20261105-1810", "../x", &cwd).unwrap_err();
    assert_eq!(code(&e), "invalid_args");
}

#[test]
fn control_commands_act_as_the_human_and_fail_closed_without_a_session() {
    let t = Tmp::new();
    let app = t.app();
    assert_eq!(code(&app.control_message("w1", "hi").unwrap_err()), "control_disabled");
    assert_eq!(code(&app.control_stop("w1", None).unwrap_err()), "control_disabled");
    assert_eq!(code(&app.confirm_stop("a000").unwrap_err()), "action_not_found");
    let l = app.ledger().unwrap();
    assert!(l["records"].is_array());
}

#[test]
fn take_over_on_an_unknown_stream_errors() {
    let t = Tmp::new();
    let app = t.app();
    assert_eq!(code(&app.take_over("run-none", true).unwrap_err()), "session_error");
}

#[test]
fn campaigns_and_crew_read_through_the_allowlist() {
    let t = Tmp::new();
    let camp = t.home().join(".claude/vajra/campaigns/deliver-20261105-1810");
    fs::create_dir_all(&camp).unwrap();
    fs::write(
        camp.join("tasks.json"),
        serde_json::to_vec(&json!({"campaign": "deliver-20261105-1810", "schema": "vajra.tasks/v1",
            "tasks": [{"id": "t1", "owner_agent": "bhima", "status": "in_progress"}]}))
        .unwrap(),
    )
    .unwrap();
    let app = t.app();
    let c = app.list_campaigns().unwrap();
    assert!(c["campaigns"].as_array().unwrap().iter().any(|x| x["slug"] == "deliver-20261105-1810"));
    let tree = app.crew("deliver-20261105-1810", "20261105-1810").unwrap();
    assert!(tree["kind"].is_string());
    let s = tree.to_string();
    assert!(s.contains("in_progress"), "{s}");
    assert_eq!(code(&app.task("deliver-20261105-1810", "t9").unwrap_err()), "task_not_found");
}

#[test]
fn workers_are_discovered_from_the_session_subagents_dir() {
    let t = Tmp::new();
    let repo = t.0.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let sid = "11111111-2222-4333-8444-555555555555";
    let sub = t.home().join(".claude/projects").join(project_slug(&repo)).join(sid).join("subagents");
    fs::create_dir_all(sub.join("workflows/wf_1")).unwrap();
    fs::write(sub.join("agent-a1.meta.json"), r#"{"agentType":"bhima","description":"top"}"#).unwrap();
    fs::write(sub.join("workflows/wf_1/agent-a2.meta.json"), r#"{"agentType":"vajra:arjuna","description":"d"}"#).unwrap();
    fs::write(
        sub.join("workflows/wf_1/journal.jsonl"),
        "{\"type\":\"started\",\"agentId\":\"a2\",\"label\":\"task.t1.r1\",\"phase\":\"Execute\"}\n{\"type\":\"result\",\"agentId\":\"a2\"}\n",
    )
    .unwrap();
    let app = t.app();
    let run = Run { session_id: sid.into(), run_id: "20261105-1810".into(), campaign: "c".into(), cwd: repo, nonces: Redactor::default() };
    let ws = app.discover_workers(&run).unwrap();
    assert_eq!(ws.len(), 2);
    let a2 = ws.iter().find(|w| w.agent_id == "a2").unwrap();
    assert_eq!(a2.label, "task.t1.r1");
    assert_eq!(a2.status, Status::Done);
    assert_eq!(ws.iter().find(|w| w.agent_id == "a1").unwrap().status, Status::Unknown);
}

#[test]
fn missing_claude_dir_is_an_error_not_a_panic() {
    let t = Tmp::new();
    fs::remove_dir_all(t.home().join(".claude")).unwrap();
    let app = t.app();
    assert_eq!(code(&app.list_campaigns().unwrap_err()), "claude_dir_missing");
    assert_eq!(code(&app.preflight().unwrap_err()), "claude_dir_missing");
}

fn rss_kb() -> u64 {
    let out = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// Perf probe, run by hand: `cargo test --release --bin vajra-desktop
/// synthetic_streams_memory -- --ignored --nocapture`. 10 streams x
/// 200 lines/s through the bus with one subscriber draining it the way
/// the `changed` emitter does. Fails if RSS growth passes the
/// NFR-PERF-2 budget (15 MB per stream).
#[test]
#[ignore]
fn synthetic_streams_memory() {
    let t = Tmp::new();
    let app = t.app();
    let sub = app.bus.subscribe();
    let before = rss_kb();
    let p = bus::synthetic::Producer::start(app.bus.clone(), 10, 200, 120);
    let end = std::time::Instant::now() + Duration::from_secs(30);
    let mut payloads = 0;
    while std::time::Instant::now() < end {
        if collect_changes(&sub, Duration::from_millis(200)).is_some() {
            payloads += 1;
        }
    }
    let after = rss_kb();
    let pushed = p.stop();
    let st = app.bus.stats();
    println!(
        "synthetic: pushed={pushed} payloads={payloads} rss_before_kb={before} rss_after_kb={after} delta_kb={} ring_bytes={} spill_bytes={}",
        after.saturating_sub(before),
        st.ring_bytes,
        st.spill_bytes
    );
    assert!(after.saturating_sub(before) <= 10 * 15 * 1024);
}

#[test]
fn node_json_is_recursive() {
    let leaf = Node {
        id: "w".into(),
        kind: tree::NodeKind::Worker,
        title: "Bhima".into(),
        engine_id: Some("bhima".into()),
        status: Status::Done,
        overlays: vec!["stopped"],
        children: vec![],
    };
    let root = Node { id: "r".into(), kind: tree::NodeKind::Run, title: "Run".into(), engine_id: None, status: Status::InProgress, overlays: vec![], children: vec![leaf] };
    let v = node_json(&root);
    assert_eq!(v["kind"], "run");
    assert_eq!(v["children"][0]["kind"], "worker");
    assert_eq!(v["children"][0]["overlays"], json!(["stopped"]));
}

#[test]
fn a_spawn_failure_rolls_the_control_session_back() {
    // B4: start_session ran, then the PTY spawn failed; no session may stay live.
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.to_string_lossy().into_owned();
    let sid = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    let argv = vec!["/nonexistent/vajra-no-such-binary".to_string()];
    let e = app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| Ok(argv.clone()), false).unwrap_err();
    assert_eq!(code(&e), "spawn_failed");
    assert!(lock(&app.control).nonce().is_none(), "the control session ended with the failed spawn");
    assert_eq!(app.active_run_count(), 0);
}

#[test]
fn an_exited_run_ends_its_control_session() {
    // B4: once the run's PTY exits, steering must not target its workers.
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.to_string_lossy().into_owned();
    let sid = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| Ok(vec!["true".into()]), false).unwrap();
    assert!(lock(&app.control).nonce().is_some());
    app.sup.wait(&format!("run-{sid}"), Duration::from_secs(5)).unwrap();
    app.reap_runs();
    assert!(lock(&app.control).nonce().is_none(), "end_session ran for the exited run");
    assert_eq!(app.active_run_count(), 0);
    assert_eq!(app.active_runs().unwrap()["count"], 0);
    // The finished run is still known, so its crew view keeps its workers.
    assert!(app.run_by_id("20261105-1230").is_some(), "an exited run stays viewable");
}

#[test]
fn worker_transcripts_redact_the_rotated_nonce() {
    // B4 / thermo non-blocking: a transcript tailed before a resume must
    // redact the nonce the resume rotated to, not only the old one.
    let t = Tmp::new();
    let app = t.app();
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let cwd = t.0.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let sub = t.home().join(".claude/projects").join(project_slug(&cwd)).join(sid).join("subagents");
    fs::create_dir_all(&sub).unwrap();
    let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts/session/subagents");
    fs::copy(fx.join("agent-aa870dd40540481f0.meta.json"), sub.join("agent-aa870dd40540481f0.meta.json")).unwrap();
    let first = fs::read_to_string(fx.join("agent-aa870dd40540481f0.jsonl")).unwrap();
    let first_line = first.lines().next().unwrap().to_string();
    fs::write(sub.join("agent-aa870dd40540481f0.jsonl"), format!("{first_line}\n")).unwrap();
    let stream = format!("run-{sid}");
    app.sup.spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: cwd.clone(), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: cwd.clone(), nonces: Redactor::default() });
    lock(&app.control).start_session(sid, "20261105-1230").unwrap();
    app.crew("c", "20261105-1230").unwrap();
    lock(&app.control).resume_session().unwrap();
    let rotated = lock(&app.control).nonce().unwrap().to_string();
    // The worker now echoes the rotated nonce.
    let echo = first_line.replace("Run these Bash commands", &format!("steer {rotated} then run these Bash commands"));
    assert_ne!(echo, first_line);
    let mut f = fs::OpenOptions::new().append(true).open(sub.join("agent-aa870dd40540481f0.jsonl")).unwrap();
    std::io::Write::write_all(&mut f, format!("{echo}\n").as_bytes()).unwrap();
    app.crew("c", "20261105-1230").unwrap();
    let w = app.bus.tail("w-aa870dd40540481f0", 0, MAX_PAGE);
    assert!(w.lines.iter().any(|l| l.text.contains("[redacted]")), "{:?}", w.lines);
    assert!(w.lines.iter().all(|l| !l.text.contains(&rotated)), "rotated nonce leaked");
    app.sup.kill(&stream).unwrap();
}

/// The fixture worker's first transcript line under `cwd`'s project dir for `sid`.
fn seed_worker(t: &Tmp, cwd: &Path, sid: &str) -> (PathBuf, String) {
    let sub = t.home().join(".claude/projects").join(project_slug(cwd)).join(sid).join("subagents");
    fs::create_dir_all(&sub).unwrap();
    let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transcripts/session/subagents");
    fs::copy(fx.join("agent-aa870dd40540481f0.meta.json"), sub.join("agent-aa870dd40540481f0.meta.json")).unwrap();
    let first = fs::read_to_string(fx.join("agent-aa870dd40540481f0.jsonl")).unwrap().lines().next().unwrap().to_string();
    let path = sub.join("agent-aa870dd40540481f0.jsonl");
    fs::write(&path, format!("{first}\n")).unwrap();
    (path, first)
}

fn append_echo(path: &Path, first: &str, nonce: &str) {
    let echo = first.replace("Run these Bash commands", &format!("steer {nonce} then run these Bash commands"));
    assert_ne!(echo, first);
    let mut f = fs::OpenOptions::new().append(true).open(path).unwrap();
    std::io::Write::write_all(&mut f, format!("{echo}\n").as_bytes()).unwrap();
}

#[test]
fn transcripts_stay_redacted_after_the_run_exits() {
    // Code-review R1: once the control session ends, a finished run's crew view
    // must still redact the nonce that run was launched with.
    let t = Tmp::new();
    let app = t.app();
    let sid = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    let cwd = t.0.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let (path, first) = seed_worker(&t, &cwd, sid);
    app.launch(sid, "20261105-1230", "c", &cwd, |_| Ok(vec!["true".into()]), false).unwrap();
    let nonce = lock(&app.control).nonce().unwrap().to_string();
    app.sup.wait(&format!("run-{sid}"), Duration::from_secs(5)).unwrap();
    app.reap_runs();
    assert!(lock(&app.control).nonce().is_none());
    append_echo(&path, &first, &nonce);
    app.crew("c", "20261105-1230").unwrap();
    let w = app.bus.tail("w-aa870dd40540481f0", 0, MAX_PAGE);
    assert!(w.lines.iter().all(|l| !l.text.contains(&nonce)), "nonce leaked after the session ended");
    assert!(w.lines.iter().any(|l| l.text.contains("[redacted]")));
}

#[test]
fn a_reap_during_a_resume_keeps_the_new_session() {
    // Code-review R3: a UI poll that reaps between start_session and spawn must
    // not end the session the resume just took.
    let t = Tmp::new();
    let app = t.app();
    let sid = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    let cwd = t.0.to_string_lossy().into_owned();
    app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| Ok(vec!["true".into()]), false).unwrap();
    app.sup.wait(&format!("run-{sid}"), Duration::from_secs(5)).unwrap();
    app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| {
        app.reap_runs();
        Ok(vec!["sleep".into(), "5".into()])
    }, true).unwrap();
    assert!(lock(&app.control).nonce().is_some(), "the resumed run is still steerable");
    app.sup.kill(&format!("run-{sid}")).unwrap();
}

#[test]
fn a_resumed_pty_redacts_the_launch_nonce_too() {
    // Optional: claude --resume replays the first prompt, which holds the old nonce.
    let t = Tmp::new();
    let app = t.app();
    let sid = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    let cwd = t.0.to_string_lossy().into_owned();
    app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| Ok(vec!["true".into()]), false).unwrap();
    let old = lock(&app.control).nonce().unwrap().to_string();
    app.sup.wait(&format!("run-{sid}"), Duration::from_secs(5)).unwrap();
    let echo = format!("printf 'replay {old}\\n'");
    app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| Ok(vec!["/bin/sh".into(), "-c".into(), echo.clone()]), true).unwrap();
    app.sup.wait(&format!("run-{sid}"), Duration::from_secs(5)).unwrap();
    let l = app.bus.tail(&format!("run-{sid}"), 0, MAX_PAGE);
    assert!(l.lines.iter().any(|x| x.text.contains("replay")), "{:?}", l.lines);
    assert!(l.lines.iter().all(|x| !x.text.contains(&old)), "old nonce shown on resume");
}
