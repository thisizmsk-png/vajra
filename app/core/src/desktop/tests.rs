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
    let input = RunInput { ask: "x".into(), max_rounds: 1, base: "ea73cb05d639b945e21c667e17103e56404861a8".into(), max_in_flight: 1, campaign: None };
    assert_eq!(code(&app.start_run(input, &cwd).unwrap_err()), "run_active");
    let other = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    assert_eq!(code(&app.resume_run(other, "c2", &cwd).unwrap_err()), "run_active");
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
    let e = app.resume_run("bad id;rm", "deliver-x", &cwd).unwrap_err();
    assert_eq!(code(&e), "invalid_args");
    let e = app.resume_run("0f8fad5b-d9cb-469f-a165-70867728950e", "../x", &cwd).unwrap_err();
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
    assert!(app.run_by_id("c", "20261105-1230").is_some(), "an exited run stays viewable");
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

#[test]
fn start_run_resolves_a_branch_to_a_sha_through_git() {
    let t = Tmp::new();
    let app = t.app();
    let git = |a: &[&str]| {
        let o = std::process::Command::new("/usr/bin/git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t"]).args(a).current_dir(&t.0).output().unwrap();
        assert!(o.status.success(), "{a:?}");
        String::from_utf8(o.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["commit", "-q", "--allow-empty", "-m", "base"]);
    let sha = git(&["rev-parse", "HEAD"]);
    assert_eq!(app.resolve_base(&t.0, "main").unwrap(), sha);
    assert_eq!(app.resolve_base(&t.0, &sha).unwrap(), sha);
    for bad in ["nope", "main..x", "-x"] {
        assert_eq!(code(&app.resolve_base(&t.0, bad).unwrap_err()), "invalid_args", "{bad}");
    }
}

#[test]
fn oversized_worker_meta_and_journal_are_skipped_not_loaded() {
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
    // A journal over the cap with a VALID entry for the worker at its start: if it were read, the worker would take
    // the journal's label. It must be skipped by its size alone, so the label stays the meta file's.
    let journal = sub.join("journal.jsonl");
    fs::write(&journal, "{\"type\":\"started\",\"agentId\":\"aa870dd40540481f0\",\"label\":\"journal.only.label\",\"phase\":\"Execute\"}\n").unwrap();
    fs::OpenOptions::new().write(true).open(&journal).unwrap().set_len(9 * 1024 * 1024).unwrap();
    // A second worker whose meta file is over the cap is skipped.
    let big = sub.join("agent-bb870dd40540481f0.meta.json");
    fs::File::create(&big).unwrap().set_len(1024 * 1024).unwrap();
    let stream = format!("run-{sid}");
    app.sup
        .spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: cwd.clone(), rows: 24, cols: 80, redact: Redactor::default() })
        .unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: cwd.clone(), nonces: Redactor::default() });
    let run = lock(&app.runs).get(&stream).map(|r| (r.session_id.clone(), r.run_id.clone(), r.campaign.clone(), r.cwd.clone())).unwrap();
    let r = Run { session_id: run.0, run_id: run.1, campaign: run.2, cwd: run.3, nonces: Redactor::default() };
    let workers = app.discover_workers(&r).unwrap();
    let ids: Vec<&str> = workers.iter().map(|w| w.agent_id.as_str()).collect();
    assert_eq!(ids, vec!["aa870dd40540481f0"], "the normal worker is found; the oversized meta is skipped: {ids:?}");
    assert_ne!(workers[0].label, "journal.only.label", "an over-limit journal is not loaded");
    app.sup.kill(&stream).unwrap();
}

#[test]
fn two_runs_in_one_minute_get_distinct_ids_and_views_never_cross() {
    let t = Tmp::new();
    let app = t.app();
    let mk = |sid: &str, rid: &str, camp: &str| Run { session_id: sid.into(), run_id: rid.into(), campaign: camp.into(), cwd: t.0.clone(), nonces: Redactor::default() };
    let a = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let b = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    // Same minute, different campaigns (for example a resume and a fresh run): the lookup keys on both.
    lock(&app.runs).insert(format!("run-{a}"), mk(a, "20261105-1230", "deliver-old"));
    lock(&app.runs).insert(format!("run-{b}"), mk(b, "20261105-1230", "deliver-new"));
    for _ in 0..20 {
        assert_eq!(app.run_by_id("deliver-old", "20261105-1230").unwrap().1.session_id, a);
        assert_eq!(app.run_by_id("deliver-new", "20261105-1230").unwrap().1.session_id, b);
    }
    assert!(app.run_by_id("deliver-other", "20261105-1230").is_none());
    // A fresh run in a minute that is taken moves to the next free minute.
    let now = 1_793_881_859; // 2026-11-05 12:30:59 UTC
    assert_eq!(ids::run_id_at(now), "20261105-1230");
    let (t2, id2) = app.fresh_run_id(now);
    assert_eq!(id2, "20261105-1231");
    assert_eq!(t2, now + 60);
    lock(&app.runs).insert("run-x".into(), mk("2b4e28ba-2fa1-41d2-883f-0016d3cca427", "20261105-1231", "deliver-20261105-1231"));
    assert_eq!(app.fresh_run_id(now).1, "20261105-1232");
}

#[test]
fn inspect_returns_the_newest_output_once_more_than_16_kib_is_retained() {
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
    // 40 KiB of older output, then a unique final line: the oldest 16 KiB must not be what the Lead sees.
    for i in 0..400 {
        app.bus.push("w-aa870dd40540481f0", &format!("older line {i:04} {}", "x".repeat(90))).unwrap();
    }
    app.bus.push("w-aa870dd40540481f0", "FINAL-UNIQUE-QUESTION: which branch?").unwrap();
    app.crew("c", "20261105-1230").unwrap();
    let i = lock(&app.control).inspect_worker(Actor::Lead, "aa870dd40540481f0", 50).unwrap();
    let lines: Vec<&str> = i["lines"].as_array().unwrap().iter().filter_map(|l| l["text"].as_str()).collect();
    assert!(lines.iter().any(|l| l.contains("FINAL-UNIQUE-QUESTION")), "the newest line is returned: {lines:?}");
    assert!(!lines.iter().any(|l| l.contains("older line 0000")), "and not the oldest retained output");
    app.sup.kill(&stream).unwrap();
}

#[test]
fn two_resumes_of_one_campaign_get_distinct_run_ids() {
    // Review P1: a resume reused one identity, so its task ids and branches collided with the previous resume's.
    let t = Tmp::new();
    let app = t.app();
    let a = app.fresh_run_id(1_794_000_000).1;
    // The first resume's run entry is replaced by the second; the issued set still remembers it.
    lock(&app.runs).clear();
    let b = app.fresh_run_id(1_794_000_000).1;
    assert_ne!(a, b);
    assert!(ids::valid_run_id(&a) && ids::valid_run_id(&b));
}

#[test]
fn a_reap_in_progress_blocks_a_launch_until_it_finishes() {
    // Review P1: the launching guard used to drop before `reap_locked`, so a launch could install its session while an
    // earlier reap was still about to end "the old one". Hold the control lock so the reap stalls inside `reap_locked`,
    // and check the launch guard is still taken.
    let t = Tmp::new();
    let app = t.app();
    let sid = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    let cwd = t.0.to_string_lossy().into_owned();
    app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| Ok(vec!["true".into()]), false).unwrap();
    app.sup.wait(&format!("run-{sid}"), Duration::from_secs(5)).unwrap();
    let held = lock(&app.control);
    std::thread::scope(|sc| {
        let reaper = sc.spawn(|| app.reap_runs());
        let mut blocked = false;
        for _ in 0..200 {
            if matches!(app.launching.try_lock(), Err(std::sync::TryLockError::WouldBlock)) {
                blocked = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(blocked, "the reap released the launch guard before it finished");
        drop(held);
        reaper.join().unwrap();
    });
    assert!(app.launching.try_lock().is_ok(), "the guard is released once the reap is done");
}

#[test]
fn the_core_refreshes_workers_and_transcripts_without_a_crew_view() {
    // Review P2: discovery and tailing used to run only when the UI asked for the crew tree, so a run nobody was
    // looking at had no steerable workers and stale inspect output.
    let t = Tmp::new();
    let app = t.app();
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let cwd = t.0.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let (path, first) = seed_worker(&t, &cwd, sid);
    let stream = format!("run-{sid}");
    app.sup.spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: cwd.clone(), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: cwd.clone(), nonces: Redactor::default() });
    lock(&app.control).start_session(sid, "20261105-1230").unwrap();
    assert!(lock(&app.control).inspect_worker(Actor::Lead, "aa870dd40540481f0", 50).is_err(), "unknown before any refresh");
    app.refresh_workers();
    assert!(lock(&app.control).inspect_worker(Actor::Lead, "aa870dd40540481f0", 50).is_ok(), "discovered without crew()");
    // New transcript output shows up on the next refresh, still without crew().
    let echo = first.replace("Run these Bash commands", "FRESH-MARKER then run these Bash commands");
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    std::io::Write::write_all(&mut f, format!("{echo}\n").as_bytes()).unwrap();
    app.refresh_workers();
    let i = lock(&app.control).inspect_worker(Actor::Lead, "aa870dd40540481f0", 50).unwrap();
    assert!(i.to_string().contains("FRESH-MARKER"), "{i}");
    app.sup.kill(&stream).unwrap();
}

#[test]
fn task_metadata_is_derived_from_the_journal_label_and_resolves_task_calls() {
    // Review P2: every discovered worker used to publish task_id/round = None, so `t1` never resolved.
    assert_eq!(task_meta("20261105-1810", "task.t1.r2"), (Some("20261105-1810.t1".into()), Some(2)));
    assert_eq!(task_meta("20261105-1810", "verify.r1"), (None, None));
    assert_eq!(task_meta("20261105-1810", "kb.search"), (None, None));
    assert_eq!(task_meta("20261105-1810", "task."), (None, None));
    let t = Tmp::new();
    let app = t.app();
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let cwd = t.0.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let sub = t.home().join(".claude/projects").join(project_slug(&cwd)).join(sid).join("subagents");
    fs::create_dir_all(sub.join("workflows/wf_1")).unwrap();
    fs::write(sub.join("workflows/wf_1/agent-a2.meta.json"), r#"{"agentType":"vajra:arjuna","description":"d"}"#).unwrap();
    fs::write(sub.join("workflows/wf_1/journal.jsonl"), "{\"type\":\"started\",\"agentId\":\"a2\",\"label\":\"task.t1.r2\",\"phase\":\"Execute\"}\n").unwrap();
    let stream = format!("run-{sid}");
    app.sup.spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: cwd.clone(), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1810".into(), campaign: "c".into(), cwd: cwd.clone(), nonces: Redactor::default() });
    lock(&app.control).start_session(sid, "20261105-1810").unwrap();
    app.refresh_workers();
    let i = lock(&app.control).inspect_worker(Actor::Lead, "t1", 10).unwrap();
    assert_eq!(i["task_id"], "20261105-1810.t1", "{i}");
    assert_eq!(i["round"], 2, "{i}");
    app.sup.kill(&stream).unwrap();
}

// ---- verify_campaign (AC-1.2.1, AC-1.2.2) ------------------------------------------------------------------------

/// Plant a stand-in `vajra-campaign.sh` under the temp home; it records its argv and exits `code`.
fn fake_campaign_script(t: &Tmp, code: i32, stderr: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let log = t.0.join("argv.log");
    let sh = dir.join("vajra-campaign.sh");
    fs::write(
        &sh,
        format!("#!/bin/bash\necho \"$@\" >> '{log}'\necho '{stderr}' >&2\nexit {code}\n", log = log.display()),
    )
    .unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    log
}

#[test]
fn verify_campaign_exit_0_is_ok_and_runs_verify_with_the_slug() {
    let t = Tmp::new();
    let log = fake_campaign_script(&t, 0, "");
    let v = t.app().verify_campaign("my-camp").unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["slug"], "my-camp");
    assert_eq!(fs::read_to_string(log).unwrap().trim(), "verify my-camp");
}

#[test]
fn verify_campaign_exit_3_is_blocked_with_the_restore_command_for_the_latest_checkpoint() {
    let t = Tmp::new();
    fake_campaign_script(&t, 3, "signature INVALID");
    let ck = t.home().join(".claude/vajra/campaigns/my-camp/checkpoints");
    fs::create_dir_all(&ck).unwrap();
    for n in ["0001.json", "0002.json", "0010.json"] {
        fs::write(ck.join(n), "{}").unwrap();
    }
    let v = t.app().verify_campaign("my-camp").unwrap();
    assert_eq!(v["ok"], false, "{v}");
    assert_eq!(v["restore"], "vajra-campaign.sh resume my-camp --from-checkpoint 10", "{v}");
}

#[test]
fn verify_campaign_exit_3_without_checkpoints_has_no_restore_command() {
    let t = Tmp::new();
    fake_campaign_script(&t, 3, "signature INVALID");
    let v = t.app().verify_campaign("my-camp").unwrap();
    assert_eq!(v["ok"], false, "{v}");
    assert!(v["restore"].is_null(), "{v}");
}

#[test]
fn verify_campaign_other_exit_codes_are_an_error_not_a_pass() {
    let t = Tmp::new();
    fake_campaign_script(&t, 1, "no such campaign");
    let e = t.app().verify_campaign("my-camp").unwrap_err();
    assert_eq!(code(&e), "verify_failed", "{e}");
    assert!(e["detail"].as_str().unwrap().contains("no such campaign"), "{e}");
}

#[test]
fn verify_campaign_refuses_a_bad_slug_without_running_anything() {
    let t = Tmp::new();
    let log = fake_campaign_script(&t, 0, "");
    for bad in ["../x", "-rf", "", "a b", ".hidden"] {
        let e = t.app().verify_campaign(bad).unwrap_err();
        assert_eq!(code(&e), "invalid_args", "{bad}: {e}");
    }
    assert!(!log.exists(), "the script must not run for a bad slug");
}

// ---- list_campaigns integrity badges (AC-6.1.3) -------------------------------------------------

fn campaign(t: &Tmp, slug: &str, report: Option<&[u8]>) {
    let d = t.home().join(".claude/vajra/campaigns").join(slug);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("campaign.json"), format!(r#"{{"slug":"{slug}","goal":"g","status":"active","created":"c","updated":"u"}}"#)).unwrap();
    if let Some(r) = report {
        fs::write(d.join("report.md"), r).unwrap();
    }
}

fn runs(log: &Path) -> usize {
    fs::read_to_string(log).map(|s| s.lines().count()).unwrap_or(0)
}

#[test]
fn list_campaigns_marks_each_campaign_ok_failed_or_unknown() {
    for (exit, want) in [(0, "ok"), (3, "failed"), (1, "unknown")] {
        let t = Tmp::new();
        fake_campaign_script(&t, exit, "x");
        campaign(&t, "c1", None);
        let l = t.app().list_campaigns().unwrap();
        assert_eq!(l["campaigns"][0]["integrity"], want, "exit {exit}: {l}");
    }
}

/// An app whose clock the test moves by hand (milliseconds).
fn app_with_clock(t: &Tmp) -> (App, Arc<std::sync::atomic::AtomicU64>) {
    let ms = Arc::new(std::sync::atomic::AtomicU64::new(1_000));
    let mut app = t.app();
    let m = ms.clone();
    app.clock = Arc::new(move || Duration::from_millis(m.load(std::sync::atomic::Ordering::SeqCst)));
    (app, ms)
}

#[test]
fn two_unchanged_listings_inside_the_ttl_verify_each_campaign_once() {
    let t = Tmp::new();
    let log = fake_campaign_script(&t, 0, "");
    campaign(&t, "c1", None);
    campaign(&t, "c2", None);
    let (app, _) = app_with_clock(&t);
    app.list_campaigns().unwrap();
    let l = app.list_campaigns().unwrap();
    assert_eq!(runs(&log), 2, "one verify per campaign; the second listing reuses the verdict");
    assert_eq!(l["campaigns"][0]["integrity"], "ok");
}

#[test]
fn a_verdict_older_than_the_ttl_is_verified_again() {
    let t = Tmp::new();
    let log = fake_campaign_script(&t, 0, "");
    campaign(&t, "c1", None);
    let (app, ms) = app_with_clock(&t);
    app.list_campaigns().unwrap();
    ms.fetch_add(4_999, std::sync::atomic::Ordering::SeqCst);
    app.list_campaigns().unwrap();
    assert_eq!(runs(&log), 1, "4.999 s: still inside the 5 s window");
    ms.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    app.list_campaigns().unwrap();
    assert_eq!(runs(&log), 2, "5 s: expired");
}

#[test]
fn refresh_verifies_again_inside_the_ttl() {
    let t = Tmp::new();
    let log = fake_campaign_script(&t, 0, "");
    campaign(&t, "c1", None);
    let (app, _) = app_with_clock(&t);
    app.list_campaigns().unwrap();
    app.list_campaigns_opt(true).unwrap();
    assert_eq!(runs(&log), 2);
}

#[test]
fn a_burst_of_listings_for_one_campaign_shares_one_verify() {
    // `changed` events arrive together; the verify takes a while, and nobody starts a second one meanwhile.
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new();
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let log = t.0.join("burst.log");
    let sh = dir.join("vajra-campaign.sh");
    fs::write(&sh, format!("#!/bin/bash\necho v >> '{}'\nsleep 0.5\nexit 0\n", log.display())).unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    campaign(&t, "c1", None);
    let app = t.app();
    let got: Vec<Value> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..8).map(|_| s.spawn(|| app.list_campaigns().unwrap()["campaigns"][0]["integrity"].clone())).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(got.iter().all(|v| v == "ok"), "{got:?}");
    assert_eq!(runs(&log), 1, "eight concurrent listings, one verify");
}

#[test]
fn unknown_is_never_reused() {
    let t = Tmp::new();
    let log = fake_campaign_script(&t, 1, "jq: not found");
    campaign(&t, "c1", None);
    let (app, _) = app_with_clock(&t);
    app.list_campaigns().unwrap();
    app.list_campaigns().unwrap();
    assert_eq!(runs(&log), 2, "an unknown verdict is retried on the next listing");
}

#[test]
fn verify_campaign_refreshes_that_campaigns_verdict() {
    let t = Tmp::new();
    fake_campaign_script(&t, 0, "");
    campaign(&t, "c1", Some(b"text"));
    let (app, _) = app_with_clock(&t);
    let verdict = |app: &App| app.list_campaigns().unwrap()["campaigns"][0]["integrity"].clone();
    assert_eq!(verdict(&app), "ok");
    fake_campaign_script(&t, 3, "signature INVALID");
    assert_eq!(verdict(&app), "ok", "inside the window the listing still shows the earlier verdict");
    app.verify_campaign("c1").unwrap(); // the user asked: this is a fresh check and it updates the badge
    assert_eq!(verdict(&app), "failed");
}

/// A stand-in `verify` whose verdict depends on a signing-key file outside the campaign, like the real one: 0 when
/// ~/.claude/vajra/stand-in.key holds "K1", 3 otherwise.
fn key_dependent_script(t: &Tmp) {
    use std::os::unix::fs::PermissionsExt;
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let sh = dir.join("vajra-campaign.sh");
    fs::write(&sh, "#!/bin/bash\n[ \"$(cat \"$HOME/.claude/vajra/stand-in.key\" 2>/dev/null)\" = K1 ] && exit 0\nexit 3\n").unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_replaced_signing_key_shows_within_five_seconds_or_at_once_with_refresh() {
    // Stated staleness: the key is not part of any cache key (there is none), so after a replacement the badge may keep
    // the earlier verdict for up to the 5 s window; refresh=true, an expired window, and verify_campaign show it at once.
    let t = Tmp::new();
    key_dependent_script(&t);
    campaign(&t, "c1", None);
    let key = t.home().join(".claude/vajra/stand-in.key");
    fs::write(&key, "K1").unwrap();
    let (app, ms) = app_with_clock(&t);
    let step = |n: u64| { ms.fetch_add(n, std::sync::atomic::Ordering::SeqCst); };
    let verdict = |app: &App| app.list_campaigns().unwrap()["campaigns"][0]["integrity"].clone();
    let fresh = |app: &App| app.list_campaigns_opt(true).unwrap()["campaigns"][0]["integrity"].clone();
    assert_eq!(verdict(&app), "ok");
    fs::write(&key, "K2").unwrap(); // install repair generates a new key; no campaign file is touched
    step(4_999);
    assert_eq!(verdict(&app), "ok", "4.999 s after the check the earlier verdict may still be shown (the bound)");
    step(1);
    assert_eq!(verdict(&app), "failed", "at 5 s the next listing reflects the replaced key, with no refresh");
    fs::write(&key, "K1").unwrap(); // the original key is restored
    assert_eq!(verdict(&app), "failed", "inside the window again: lags, within the bound");
    assert_eq!(fresh(&app), "ok", "refresh shows the restored key at once");
    fs::write(&key, "K2").unwrap();
    assert_eq!(verdict(&app), "ok", "lag again");
    app.verify_campaign("c1").unwrap();
    assert_eq!(verdict(&app), "failed", "a user-initiated verify shows it at once and updates the badge");
}

#[test]
fn a_verify_that_overruns_its_timeout_reads_unknown_never_ok() {
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new();
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let sh = dir.join("vajra-campaign.sh");
    fs::write(&sh, "#!/bin/bash\nsleep 5\nexit 0\n").unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    campaign(&t, "c1", None);
    let mut app = t.app();
    app.verify_timeout = Duration::from_millis(300);
    let started = std::time::Instant::now();
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "unknown");
    assert!(started.elapsed() < Duration::from_secs(4), "the listing waited for the full sleep");
}

#[test]
fn listing_runs_verifies_in_parallel_but_never_more_than_four_at_once() {
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new();
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let log = t.0.join("overlap.log");
    let sh = dir.join("vajra-campaign.sh");
    fs::write(&sh, format!("#!/bin/bash\necho s >> '{l}'\nsleep 0.4\necho e >> '{l}'\nexit 0\n", l = log.display())).unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    for i in 0..10 {
        campaign(&t, &format!("c{i}"), None);
    }
    let l = t.app().list_campaigns().unwrap();
    assert!(l["campaigns"].as_array().unwrap().iter().all(|c| c["integrity"] == "ok"), "{l}");
    let (mut cur, mut peak) = (0i32, 0i32);
    for line in fs::read_to_string(&log).unwrap().lines() {
        cur += if line == "s" { 1 } else { -1 };
        peak = peak.max(cur);
    }
    assert!((2..=4).contains(&peak), "peak concurrency {peak}");
}

#[test]
fn list_campaigns_reverifies_after_an_edit_even_when_the_mtime_is_restored() {
    // The badge is the trust signal: a same-size edit with the old mtime put back must not keep a cached "ok".
    let t = Tmp::new();
    fake_campaign_script(&t, 0, "");
    campaign(&t, "c1", Some(b"AAAA"));
    let app = t.app();
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "ok");
    let f = t.home().join(".claude/vajra/campaigns/c1/report.md");
    let old = fs::metadata(&f).unwrap().modified().unwrap();
    fs::write(&f, b"BBBB").unwrap(); // same size
    fs::OpenOptions::new().write(true).open(&f).unwrap().set_modified(old).unwrap();
    assert_eq!(fs::metadata(&f).unwrap().modified().unwrap(), old, "mtime restored");
    fake_campaign_script(&t, 3, "signature INVALID"); // the script now says the campaign is tampered
    assert_eq!(app.list_campaigns_opt(true).unwrap()["campaigns"][0]["integrity"], "failed");
}

#[test]
fn a_hung_verify_does_not_block_other_reader_calls() {
    // list_campaigns runs on every `changed` event: a slow verify must not hold the reader lock.
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new();
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let go = t.0.join("go");
    let sh = dir.join("vajra-campaign.sh");
    fs::write(&sh, format!("#!/bin/bash\nwhile [ ! -f '{}' ]; do sleep 0.05; done\nexit 0\n", go.display())).unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    campaign(&t, "c1", None);
    let app = t.app();
    let reached = std::thread::scope(|s| {
        let lister = s.spawn(|| app.list_campaigns().unwrap());
        std::thread::sleep(Duration::from_millis(400)); // the lister is now inside verify
        let (tx, rx) = std::sync::mpsc::channel();
        let app = &app;
        s.spawn(move || {
            let _ = app.with_reader(|r| r.campaigns().map_err(|e| fail("io_error", e)));
            let _ = tx.send(());
        });
        let reached = rx.recv_timeout(Duration::from_secs(2)).is_ok();
        fs::write(&go, "").unwrap(); // release the verify either way, so the test cannot hang
        lister.join().unwrap();
        reached
    });
    assert!(reached, "another reader call waited behind a running verify");
}

/// A temp home with the real `vajra-campaign.sh` installed; None when jq or openssl is missing.
fn real_script_home() -> Option<(Tmp, impl Fn(&[&str]) -> std::process::Output)> {
    let have = |c: &str| std::process::Command::new("sh").args(["-c", &format!("command -v {c}")]).output().is_ok_and(|o| o.status.success());
    if !(have("jq") && have("openssl") && have("python3")) {
        return None;
    }
    let t = Tmp::new();
    let scripts = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&scripts).unwrap();
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/vajra-campaign.sh");
    fs::copy(&real, scripts.join("vajra-campaign.sh")).unwrap();
    let home = t.home();
    let run = move |args: &[&str]| std::process::Command::new("bash").arg(scripts.join("vajra-campaign.sh")).args(args).env("HOME", &home).output().unwrap();
    Some((t, run))
}

#[test]
fn an_unknown_verdict_clears_once_the_install_is_repaired() {
    // A check that could not run (script or jq missing) reads unknown; once the install is repaired the next listing is ok.
    let t = Tmp::new();
    fake_campaign_script(&t, 1, "jq: not found");
    campaign(&t, "c1", None);
    let app = t.app();
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "unknown");
    fake_campaign_script(&t, 0, ""); // the install is repaired; the campaign files are untouched
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "ok");
}

#[test]
fn list_campaigns_reverifies_when_a_symlinked_input_changes_its_target() {
    // verify follows links like features.json; a stamp of the link alone would keep a cached "ok" after the target
    // (another campaign's file) is edited in place.
    let t = Tmp::new();
    fake_campaign_script(&t, 0, "");
    campaign(&t, "c1", None);
    campaign(&t, "c2", None);
    let camps = t.home().join(".claude/vajra/campaigns");
    let target = camps.join("c2/features.json");
    fs::write(&target, r#"{"features":[]}"#).unwrap();
    std::os::unix::fs::symlink(&target, camps.join("c1/features.json")).unwrap();
    let app = t.app();
    let ok = |l: &Value, i: usize| l["campaigns"][i]["integrity"].clone();
    let l = app.list_campaigns().unwrap();
    assert_eq!((ok(&l, 0), ok(&l, 1)), (json!("ok"), json!("ok")));
    fs::write(&target, r#"{"features":[1]}"#).unwrap(); // edited in place, the link itself untouched
    fake_campaign_script(&t, 3, "signature INVALID");
    let l = app.list_campaigns_opt(true).unwrap();
    assert_eq!(ok(&l, 0), "failed", "c1 follows the edited target: {l}");
}

#[test]
#[ignore = "measurement: cargo test --lib listing_latency -- --ignored --nocapture"]
fn listing_latency_with_the_real_script() {
    let Some((t, run)) = real_script_home() else { return };
    let app = t.app();
    let mut made = 0;
    for n in [1, 10, 30] {
        while made < n {
            made += 1;
            assert!(run(&["create", &format!("lat{made}"), "--goal", "g", "--feature", "ac-1|x"]).status.success());
        }
        let mut ms = Vec::new();
        for _ in 0..3 {
            let t0 = std::time::Instant::now();
            let l = app.list_campaigns().unwrap();
            ms.push(t0.elapsed().as_millis());
            assert!(l["campaigns"].as_array().unwrap().iter().all(|c| c["integrity"] == "ok"), "{l}");
        }
        println!("LATENCY campaigns={n} listing_ms={ms:?}");
    }
}

// ---- resume_targets: which session to resume for a campaign ---------------------------------------------------------

const SID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

fn launch_sleep(app: &App, sid: &str, run_id: &str, campaign: &str, cwd: &Path, rotate: bool) {
    app.launch(sid, run_id, campaign, cwd, |_| Ok(vec!["sleep".into(), "5".into()]), rotate).unwrap();
}

#[test]
fn a_launch_is_recorded_and_resume_targets_names_its_session_and_cwd() {
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    assert_eq!(app.resume_targets().unwrap()["targets"], json!({}));
    launch_sleep(&app, SID, "20261106-1000", "camp-a", &cwd, false);
    let r = app.resume_targets().unwrap();
    assert_eq!(r["targets"]["camp-a"]["session_id"], SID, "{r}");
    assert_eq!(r["targets"]["camp-a"]["cwd"], cwd.to_string_lossy().as_ref(), "{r}");
    assert_eq!(r["targets"]["camp-a"]["run_id"], "20261106-1000");
    app.sup.kill(&format!("run-{SID}")).unwrap();
}

#[test]
fn the_record_survives_a_restart_and_a_resume_updates_the_run_id_only() {
    let t = Tmp::new();
    let cwd = t.0.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    {
        let app = t.app();
        launch_sleep(&app, SID, "20261106-1000", "camp-a", &cwd, false);
        app.sup.kill(&format!("run-{SID}")).unwrap();
    }
    let app = t.app(); // a new App over the same data dir: nothing is in memory
    let r = app.resume_targets().unwrap();
    assert_eq!(r["targets"]["camp-a"]["session_id"], SID, "{r}");
    launch_sleep(&app, SID, "20261106-1100", "camp-a", &cwd, true);
    let r = app.resume_targets().unwrap();
    assert_eq!(r["targets"]["camp-a"]["session_id"], SID);
    assert_eq!(r["targets"]["camp-a"]["run_id"], "20261106-1100");
    app.sup.kill(&format!("run-{SID}")).unwrap();
}

#[test]
fn a_target_whose_directory_is_gone_is_not_offered() {
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    launch_sleep(&app, SID, "20261106-1000", "camp-a", &cwd, false);
    app.sup.kill(&format!("run-{SID}")).unwrap();
    fs::remove_dir_all(&cwd).unwrap();
    assert_eq!(app.resume_targets().unwrap()["targets"], json!({}));
}

#[test]
fn the_crew_tree_shows_message_pending_and_stopped_by_the_human_from_the_control_ledger() {
    let t = Tmp::new();
    let app = t.app();
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let cwd = t.0.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let sub = t.home().join(".claude/projects").join(project_slug(&cwd)).join(sid).join("subagents");
    fs::create_dir_all(sub.join("workflows/wf_1")).unwrap();
    fs::write(sub.join("workflows/wf_1/agent-a2.meta.json"), r#"{"agentType":"vajra:arjuna","description":"d"}"#).unwrap();
    fs::write(sub.join("workflows/wf_1/journal.jsonl"), "{\"type\":\"started\",\"agentId\":\"a2\",\"label\":\"task.t1.r1\",\"phase\":\"Execute\"}\n").unwrap();
    let stream = format!("run-{sid}");
    app.sup.spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: cwd.clone(), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: cwd.clone(), nonces: Redactor::default() });
    lock(&app.control).start_session(sid, "20261105-1230").unwrap();
    let overlays = |tree: &Value| -> Vec<String> {
        fn find<'a>(n: &'a Value, id: &str) -> Option<&'a Value> {
            if n["id"] == id { return Some(n) }
            n["children"].as_array()?.iter().find_map(|c| find(c, id))
        }
        find(tree, "agent:a2").map(|n| n["overlays"].as_array().unwrap().iter().map(|o| o.as_str().unwrap().to_string()).collect()).unwrap_or_else(|| panic!("worker missing: {tree}"))
    };
    assert!(overlays(&app.crew("c", "20261105-1230").unwrap()).is_empty());
    app.control_message("a2", "use the kit Button").unwrap();
    assert_eq!(overlays(&app.crew("c", "20261105-1230").unwrap()), vec!["message pending"]);
    app.control_stop("a2", None).unwrap();
    let o = overlays(&app.crew("c", "20261105-1230").unwrap());
    assert!(o.contains(&"stopped by the human".to_string()), "{o:?}");
    app.sup.kill(&stream).unwrap();
}


// ---- review follow-ups: explicit verify vs an in-flight listing check; unreadable campaigns ------------------------

/// First invocation: signal `started`, wait for `go`, then exit 0 (it saw the old, valid state). Any later one exits 3.
fn barrier_campaign_script(t: &Tmp) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let ctl = t.0.join("ctl");
    fs::create_dir_all(&ctl).unwrap();
    let sh = dir.join("vajra-campaign.sh");
    let body = format!(
        "#!/bin/bash\nif mkdir '{c}/first' 2>/dev/null; then\n  touch '{c}/started'\n  while [ ! -e '{c}/go' ]; do sleep 0.05; done\n  exit 0\nfi\necho 'signature INVALID' >&2\nexit 3\n",
        c = ctl.display()
    );
    fs::write(&sh, body).unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    ctl
}

#[test]
fn an_older_listing_check_cannot_overwrite_a_newer_explicit_verify() {
    let t = Tmp::new();
    let ctl = barrier_campaign_script(&t);
    campaign(&t, "c1", None);
    let app = Arc::new(t.app());
    // 1. A listing check starts against valid state and is held at the barrier.
    let a = { let app = app.clone(); std::thread::spawn(move || app.list_campaigns().unwrap()) };
    let end = std::time::Instant::now() + Duration::from_secs(10);
    while !ctl.join("started").exists() {
        assert!(std::time::Instant::now() < end, "the listing check never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    // 2. The key is replaced; the user verifies explicitly. It must join the flight, not race past it.
    let b = { let app = app.clone(); std::thread::spawn(move || app.verify_campaign("c1").unwrap()) };
    std::thread::sleep(Duration::from_millis(300));
    assert!(!b.is_finished(), "an explicit verify waits for the campaign's running check, then runs its own");
    // 3. The old check finishes (exit 0), then the explicit one runs and fails.
    fs::write(ctl.join("go"), "").unwrap();
    let listed = a.join().unwrap();
    assert_eq!(listed["campaigns"][0]["integrity"], "ok");
    let v = b.join().unwrap();
    assert_eq!(v["ok"], false, "{v}");
    // 4. The badge shows the newer failure, not the older check's ok.
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "failed");
}

#[test]
fn a_campaign_the_reader_cannot_summarise_still_gets_an_integrity_verdict() {
    // campaign.json missing: plain verify exits 3, and the listing must say so instead of leaving the field out.
    let t = Tmp::new();
    fake_campaign_script(&t, 3, "signature INVALID");
    let d = t.home().join(".claude/vajra/campaigns/gone");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("progress.md"), "p").unwrap();
    campaign(&t, "bad", None);
    fs::write(t.home().join(".claude/vajra/campaigns/bad/campaign.json"), "{ truncated").unwrap();
    let l = t.app().list_campaigns().unwrap();
    for s in ["gone", "bad"] {
        let c = l["campaigns"].as_array().unwrap().iter().find(|c| c["slug"] == s).unwrap_or_else(|| panic!("{s} missing: {l}"));
        assert!(c["error"].is_string(), "{s}: {c}");
        assert_eq!(c["integrity"], "failed", "{s}: {c}");
    }
}

#[test]
fn an_unreadable_campaign_whose_check_cannot_run_is_unknown_never_missing() {
    let t = Tmp::new();
    fake_campaign_script(&t, 1, "jq: not found");
    let d = t.home().join(".claude/vajra/campaigns/gone");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("progress.md"), "p").unwrap();
    let l = t.app().list_campaigns().unwrap();
    assert_eq!(l["campaigns"][0]["integrity"], "unknown", "{l}");
}
