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

// ---------------------------------------------------------------- worktree_info (S2c)

fn git(cwd: &Path, args: &[&str]) {
    let o = std::process::Command::new("git").arg("-C").arg(cwd).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

/// A repo with one worktree under `.claude/worktrees/w1`, a registered run for it and a seeded worker whose transcript cwd is `wt_cwd`.
fn crew_fixture(t: &Tmp, wt_cwd: &dyn Fn(&Path) -> PathBuf) -> (App, PathBuf, PathBuf) {
    let repo = t.0.join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("f.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    let wt = repo.join(".claude/worktrees/w1");
    git(&repo, &["worktree", "add", "-q", "-b", "worker-1", wt.to_str().unwrap()]);
    fs::write(wt.join("f.txt"), "one\ntwo\n").unwrap();
    let sid = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    let (path, first) = seed_worker(t, &repo, sid);
    let cwd = wt_cwd(&repo);
    fs::write(&path, format!("{}\n", first.replace("\"cwd\":\"/repo\"", &format!("\"cwd\":{}", serde_json::to_string(&cwd).unwrap())))).unwrap();
    let app = t.app();
    lock(&app.runs).insert("run-x".into(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: repo.clone(), nonces: Redactor::default() });
    app.crew("c", "20261105-1230").unwrap();
    (app, repo, wt)
}

#[test]
fn worktree_info_returns_the_crew_worktree_its_branch_and_base() {
    let t = Tmp::new();
    let (app, _repo, wt) = crew_fixture(&t, &|r| r.join(".claude/worktrees/w1"));
    let i = app.worktree_info("aa870dd40540481f0").unwrap();
    assert_eq!(i["worktree"], wt.canonicalize().unwrap().to_string_lossy().as_ref(), "{i}");
    assert_eq!(i["branch"], "worker-1", "{i}");
    assert_eq!(i["base"].as_str().map(str::len), Some(40), "{i}");
    let d = app.diff(i["worktree"].as_str().unwrap(), i["base"].as_str().unwrap()).unwrap();
    assert!(d["diff"].as_str().unwrap().contains("+two"), "{d}");
}

#[test]
fn diff_lists_untracked_files_apart_and_respects_ignore_rules() {
    let t = Tmp::new();
    let (app, repo, wt) = crew_fixture(&t, &|r| r.join(".claude/worktrees/w1"));
    git(&wt, &["checkout", "-q", "--", "f.txt"]); // only new work remains: nothing tracked differs from the base
    fs::write(wt.join("new file.rs"), "fn main() {}\n").unwrap();
    fs::create_dir_all(wt.join("sub")).unwrap();
    fs::write(wt.join("sub/also.txt"), "x").unwrap();
    fs::write(repo.join(".git/info/exclude"), "ignored.log\n").unwrap();
    fs::write(wt.join("ignored.log"), "x").unwrap();
    let i = app.worktree_info("aa870dd40540481f0").unwrap();
    let d = app.diff(i["worktree"].as_str().unwrap(), i["base"].as_str().unwrap()).unwrap();
    assert_eq!(d["diff"].as_str().unwrap().trim(), "", "tracked files are unchanged: {d}");
    let mut u: Vec<String> = d["untracked"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    u.sort();
    assert_eq!(u, vec!["new file.rs".to_string(), "sub/also.txt".to_string()], "{d}");
    assert_eq!(d["untracked_truncated"], false);
}

#[test]
fn worktree_info_refuses_a_cwd_outside_the_crew_worktrees() {
    let t = Tmp::new();
    let (app, repo, _wt) = crew_fixture(&t, &|r| r.to_path_buf()); // the repo itself, not a worktree
    assert_eq!(code(&app.worktree_info("aa870dd40540481f0").unwrap_err()), "not_a_crew_worktree");
    let _ = repo;
}

#[test]
fn worktree_info_refuses_a_symlink_that_leaves_the_worktrees_dir() {
    let t = Tmp::new();
    let (app, repo, _wt) = crew_fixture(&t, &|r| r.join(".claude/worktrees/evil"));
    let outside = t.0.join("outside");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, repo.join(".claude/worktrees/evil")).unwrap();
    assert_eq!(code(&app.worktree_info("aa870dd40540481f0").unwrap_err()), "not_a_crew_worktree");
}

#[test]
fn worktree_info_rejects_a_bad_or_unknown_worker() {
    let t = Tmp::new();
    let app = t.app();
    assert_eq!(code(&app.worktree_info("../x").unwrap_err()), "invalid_args");
    assert_eq!(code(&app.worktree_info("aa870dd40540481f0").unwrap_err()), "not_found");
}

// ---------------------------------------------------------------- evidence_list (S4a)

/// A stand-in `vajra_tasks.py check-evidence`: the verdict comes from the evidence file's name.
fn fake_tasks_script(t: &Tmp) {
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let body = "import json, sys\np = sys.argv[2]\n\
        if 'slow' in p:\n    import time; time.sleep(30)\n\
        if 'boom' in p:\n    print(json.dumps({'ok': False, 'error': 'unreadable'})); sys.exit(1)\n\
        if 'swap' in p:\n    open(p, 'w').write('{\"a\":\"BBB\"}')\n\
        green = 'green' in p\n\
        try:\n    green = green and 'RED' not in open(p).read()\nexcept OSError:\n    pass\n\
        print(json.dumps({'ok': True, 'green': green, 'reasons': [] if green else ['a check failed']}))\n";
    fs::write(dir.join("vajra_tasks.py"), format!("#!/usr/bin/env python3\n{body}")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir.join("vajra_tasks.py"), fs::Permissions::from_mode(0o755)).unwrap();
}

fn ev_task(slug_dir: &Path, id: &str, status: &str, acceptance: &[&str], evidence: &str) -> Value {
    let ev = if evidence.is_empty() {
        Value::Null
    } else {
        let p = slug_dir.join("evidence").join(format!("{evidence}.json"));
        json!({"path": p, "sha256": "ab", "head": "a".repeat(40), "commands": 2})
    };
    json!({"id": id, "title": format!("Task {id}"), "owner_agent": "bhima", "deps": [], "acceptance": acceptance, "status": status, "attempts": 1, "evidence": ev})
}

fn campaign_with_tasks(t: &Tmp, tasks: Vec<Value>) -> (App, String) {
    fake_tasks_script(t);
    let slug = "deliver-20261105-1810";
    let camp = t.home().join(".claude/vajra/campaigns").join(slug);
    fs::create_dir_all(camp.join("evidence")).unwrap();
    let tasks: Vec<Value> = tasks.into_iter().map(|mut x| {
        if let Some(p) = x["evidence"]["path"].as_str().map(str::to_string).filter(|p| p.starts_with("/x/")) {
            x["evidence"]["path"] = json!(camp.canonicalize().unwrap().join("evidence").join(Path::new(&p).file_name().unwrap()).to_string_lossy());
        }
        x
    }).collect();
    fs::write(camp.join("tasks.json"), serde_json::to_vec(&json!({"campaign": slug, "schema": "vajra.tasks/v1", "tasks": tasks})).unwrap()).unwrap();
    (t.app(), slug.into())
}

fn row<'a>(r: &'a Value, task: &str, criterion: &str) -> &'a Value {
    r["rows"].as_array().unwrap().iter().find(|x| x["task"] == task && x["criterion"] == criterion).unwrap_or_else(|| panic!("no row {task}/{criterion} in {r}"))
}

#[test]
fn evidence_list_marks_each_criterion_pass_fail_not_run_or_unverified() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![
        ev_task(&d, "t1", "done", &["ac-1", "ac-2"], "green"),
        ev_task(&d, "t2", "done", &["ac-3"], "red"),
        ev_task(&d, "t3", "in_progress", &["ac-4"], ""),
        ev_task(&d, "t4", "done", &["ac-5"], "boom"),
        ev_task(&d, "t5", "done", &["ac-6"], ""),
    ]);
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "t1", "ac-1")["status"], "pass");
    assert_eq!(row(&r, "t1", "ac-2")["status"], "pass");
    assert_eq!(row(&r, "t2", "ac-3")["status"], "unverified", "evidence whose check is red is not a pass: {r}");
    assert_eq!(row(&r, "t3", "ac-4")["status"], "not_run");
    assert_eq!(row(&r, "t4", "ac-5")["status"], "unverified", "an unreadable check is never a pass");
    assert_eq!(row(&r, "t5", "ac-6")["status"], "not_run", "a closed task without evidence is never pass");
    assert_eq!(row(&r, "t1", "ac-1")["commands"], 2);
    assert!(row(&r, "t1", "ac-1")["head"].as_str().unwrap().len() == 40);
}

#[test]
fn a_failed_task_shows_fail_with_its_report() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let mut failed = ev_task(&d, "t1", "failed", &["ac-1"], "");
    failed["report"] = json!({"status": "failed", "summary": "tests red"});
    let (app, slug) = campaign_with_tasks(&t, vec![failed]);
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "t1", "ac-1")["status"], "fail");
    assert_eq!(row(&r, "t1", "ac-1")["report"], "tests red");
}

#[test]
fn a_failed_task_is_fail_even_when_its_evidence_checks_green() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let mut failed = ev_task(&d, "t1", "failed", &["ac-1"], "green");
    failed["report"] = json!({"status": "failed", "summary": "green evidence, but the task failed"});
    let (app, slug) = campaign_with_tasks(&t, vec![failed, ev_task(&d, "t2", "done", &["ac-2"], "green")]);
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "t1", "ac-1")["status"], "fail", "{r}");
    assert_eq!(row(&r, "t1", "ac-1")["report"], "green evidence, but the task failed");
    assert_eq!(row(&r, "t2", "ac-2")["status"], "pass", "a done task with green evidence still passes");
}

#[test]
fn evidence_checks_are_bounded_and_a_slow_one_is_unverified_not_a_hang() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let mut tasks: Vec<Value> = (0..9).map(|i| ev_task(&d, &format!("g{i}"), "done", &["ac"], "green")).collect();
    tasks.push(ev_task(&d, "s1", "done", &["ac"], "slow"));
    let (mut app, slug) = campaign_with_tasks(&t, tasks);
    app.evidence_timeout = Duration::from_millis(400);
    let start = std::time::Instant::now();
    let r = app.evidence_list(&slug).unwrap();
    assert!(start.elapsed() < Duration::from_secs(10), "{:?}", start.elapsed());
    assert_eq!(row(&r, "s1", "ac")["status"], "unverified");
    assert_eq!(row(&r, "g8", "ac")["status"], "pass");
}

#[test]
fn evidence_list_has_a_total_budget_and_unchecked_tasks_read_checking() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let tasks: Vec<Value> = (0..40).map(|i| ev_task(&d, &format!("s{i}"), "done", &["ac"], "slow")).collect();
    let (mut app, slug) = campaign_with_tasks(&t, tasks);
    app.evidence_timeout = Duration::from_millis(400);
    app.evidence_budget = Duration::from_millis(700);
    let start = std::time::Instant::now();
    let r = app.evidence_list(&slug).unwrap();
    // Unbounded this is 10 batches x 400 ms = 4 s.
    assert!(start.elapsed() < Duration::from_millis(2500), "{:?}", start.elapsed());
    assert_eq!(row(&r, "s39", "ac")["status"], "checking", "{r}");
    assert!(r["pending"].as_u64().unwrap() > 0, "{r}");
    assert_eq!(row(&r, "s0", "ac")["status"], "unverified", "a check that ran out its own full timeout is unverified, not checking");
    // The second batch started with 300 ms of budget left (< the 400 ms timeout): the budget cut it, so it is checking and counted.
    assert_eq!(row(&r, "s4", "ac")["status"], "checking", "{r}");
    assert_eq!(r["pending"].as_u64().unwrap(), 36, "{r}");
    // Budget-cut checks are not remembered (they get a real run next call).
    assert!(app.evidence_cache.lock().unwrap().values().all(|(_, until)| until.is_some()), "only the full-timeout failures are remembered, and only briefly");
}

#[test]
fn a_second_call_during_a_sweep_runs_no_checks_and_unchanged_evidence_is_not_rechecked() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let tasks: Vec<Value> = (0..3).map(|i| ev_task(&d, &format!("g{i}"), "done", &["ac"], "green")).collect();
    let (app, slug) = campaign_with_tasks(&t, tasks);
    // A sweep of this campaign is "running": a call (another change event) must not start a parallel one.
    app.sweeping.lock().unwrap().insert(slug.clone());
    let start = std::time::Instant::now();
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "g0", "ac")["status"], "checking", "{r}");
    assert!(start.elapsed() < Duration::from_millis(300));
    app.sweeping.lock().unwrap().remove(&slug);
    // The verdict cache keys on the evidence file's mtime and size, so the file has to exist.
    let ev_dir = t.home().join(".claude/vajra/campaigns").join(&slug).join("evidence");
    fs::write(ev_dir.join("green.json"), b"{}").unwrap();
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "g0", "ac")["status"], "pass", "{r}");
    // Verdicts are cached: while another sweep holds the campaign, the same tasks still answer `pass`.
    app.sweeping.lock().unwrap().insert(slug.clone());
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "g0", "ac")["status"], "pass", "{r}");
    assert_eq!(r["pending"], 0);
}

#[test]
fn a_rewritten_evidence_file_with_the_same_size_and_mtime_is_rechecked_not_served_from_the_cache() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "g0", "done", &["ac"], "green")]);
    let file = t.home().join(".claude/vajra/campaigns").join(&slug).join("evidence/green.json");
    fs::write(&file, br#"{"a":"GRN"}"#).unwrap();
    assert_eq!(row(&app.evidence_list(&slug).unwrap(), "g0", "ac")["status"], "pass");
    let mtime = fs::metadata(&file).unwrap().modified().unwrap();
    fs::write(&file, br#"{"a":"RED"}"#).unwrap(); // same size
    fs::OpenOptions::new().write(true).open(&file).unwrap().set_modified(mtime).unwrap(); // utimes restores the old mtime
    assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), mtime);
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "g0", "ac")["status"], "unverified", "the cache must not serve the old pass: {r}");
}

#[test]
fn a_hanging_check_ends_unverified_with_nothing_pending_and_is_not_re_run_every_call() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let tasks = vec![
        ev_task(&d, "h", "done", &["ac"], "slow"),
        ev_task(&d, "g1", "done", &["ac"], "green"),
        ev_task(&d, "g2", "done", &["ac"], "green"),
    ];
    let (mut app, slug) = campaign_with_tasks(&t, tasks);
    // The shipped relation: a check's own timeout fits inside the call's budget.
    app.evidence_timeout = Duration::from_millis(400);
    app.evidence_budget = Duration::from_millis(500);
    let ev_dir = t.home().join(".claude/vajra/campaigns").join(&slug).join("evidence");
    fs::write(ev_dir.join("slow.json"), b"{}").unwrap();
    fs::write(ev_dir.join("green.json"), b"{}").unwrap();
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "h", "ac")["status"], "unverified", "{r}");
    assert_eq!(row(&r, "g1", "ac")["status"], "pass", "{r}");
    assert_eq!(r["pending"], 0, "{r}");
    // Remembered briefly: the next call does not spend 400 ms on it again.
    let start = std::time::Instant::now();
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "h", "ac")["status"], "unverified", "{r}");
    assert!(start.elapsed() < Duration::from_millis(300), "{:?}", start.elapsed());
}

#[test]
fn a_verdict_is_not_cached_when_the_file_is_swapped_while_the_check_runs() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "w", "done", &["ac"], "green-swap")]);
    let file = t.home().join(".claude/vajra/campaigns").join(&slug).join("evidence/green-swap.json");
    fs::write(&file, br#"{"a":"AAA"}"#).unwrap();
    // The check script rewrites the file to BBB after the key was read from AAA: that verdict belongs to BBB.
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "w", "ac")["status"], "pass", "{r}");
    assert_eq!(fs::read(&file).unwrap(), br#"{"a":"BBB"}"#);
    assert!(app.evidence_cache.lock().unwrap().is_empty(), "no cache entry for a verdict whose key bytes changed");
}

/// A task with green LIVE evidence (outside the campaign's evidence dir) whose run repo is a real git repo at `head`.
fn live_evidence_repo(t: &Tmp, git_repo: bool, ev_in_repo: bool) -> (App, String, std::path::PathBuf, String) {
    let repo = t.home().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git").current_dir(&repo).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let mut head = "a".repeat(40);
    if git_repo {
        git(&["init", "-q"]);
        fs::write(repo.join("f.txt"), b"one").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "one"]);
        head = git(&["rev-parse", "HEAD"]);
    }
    let ev = if ev_in_repo { repo.join("data/walkthroughs/deliver-20261105-1810") } else { t.home().join("live-ev") };
    fs::create_dir_all(&ev).unwrap();
    fs::write(ev.join("green.json"), b"{}").unwrap();
    let mut task = ev_task(Path::new("/x"), "t1", "done", &["ac"], "green");
    task["evidence"]["path"] = json!(ev.join("green.json").to_string_lossy());
    task["evidence"]["head"] = json!(head);
    let (app, slug) = campaign_with_tasks(t, vec![task]);
    lock(&app.runs).insert("run-x".into(), Run { session_id: "s".into(), run_id: "20261105-1230".into(), campaign: slug.clone(), cwd: repo.clone(), nonces: Redactor::default() });
    (app, slug, repo, head)
}

fn status_of(app: &App, slug: &str) -> String {
    lock(&app.worktree_state).clear(); // the 5 s state cache would otherwise hide the edit
    app.evidence_list(slug).unwrap()["rows"][0]["status"].as_str().unwrap().to_string()
}

#[test]
fn green_evidence_is_pass_only_while_the_repo_is_at_its_head_and_clean() {
    let t = Tmp::new();
    let (app, slug, repo, _head) = live_evidence_repo(&t, true, false);
    assert_eq!(status_of(&app, &slug), "pass");
    fs::write(repo.join("f.txt"), b"edited, not committed").unwrap();
    assert_eq!(status_of(&app, &slug), "stale", "a tracked edit, even uncommitted");
    let git = |args: &[&str]| { std::process::Command::new("git").current_dir(&repo).args(args).output().unwrap(); };
    git(&["checkout", "--", "f.txt"]);
    assert_eq!(status_of(&app, &slug), "pass", "clean again");
    fs::write(repo.join("new.txt"), b"x").unwrap();
    assert_eq!(status_of(&app, &slug), "stale", "an untracked file");
    fs::remove_file(repo.join("new.txt")).unwrap();
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "two"]);
    assert_eq!(status_of(&app, &slug), "stale", "the branch moved past the evidence's head");
}

#[test]
fn the_runs_own_untracked_output_does_not_make_its_evidence_stale() {
    let t = Tmp::new();
    // The real layout: <repo>/data/walkthroughs/<slug>/evidence.json, untracked, with its logs beside it.
    let (app, slug, repo, _head) = live_evidence_repo(&t, true, true);
    fs::write(repo.join("data/walkthroughs/deliver-20261105-1810/cmd-1.log"), b"ok").unwrap();
    fs::create_dir_all(repo.join(".claude/worktrees/w1")).unwrap();
    fs::write(repo.join(".claude/worktrees/w1/x"), b"crew worktree").unwrap();
    assert_eq!(status_of(&app, &slug), "pass", "the run's own output is not an edit");
    fs::write(repo.join("unrelated.txt"), b"x").unwrap();
    assert_eq!(status_of(&app, &slug), "stale", "an unrelated untracked file still counts");
}

#[test]
fn the_repo_state_reads_stop_at_the_calls_deadline() {
    let t = Tmp::new();
    let (app, _slug, repo, _head) = live_evidence_repo(&t, true, false);
    // Deadline already passed: git is not even started, nothing is remembered.
    assert!(app.worktree_state(&repo, std::time::Instant::now()).is_none());
    assert!(lock(&app.worktree_state).is_empty());
    let (head, dirty) = app.worktree_state(&repo, std::time::Instant::now() + Duration::from_secs(10)).unwrap();
    assert_eq!(head.len(), 40);
    assert!(!dirty);
}

/// A "git" that never answers, to stand in for a slow repo.
fn slow_git(t: &Tmp) -> std::path::PathBuf {
    let p = t.home().join("slow-git.sh");
    fs::write(&p, "#!/bin/sh\nsleep 30\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[test]
fn a_slow_repo_does_not_eat_a_checks_full_timeout_and_is_not_asked_again_on_the_next_call() {
    let t = Tmp::new();
    let (app, slug, _repo, _head) = live_evidence_repo(&t, true, false);
    let mut app = app;
    app.bridge.git = slow_git(&t);
    app.evidence_timeout = Duration::from_millis(400);
    app.evidence_budget = Duration::from_secs(5);
    let start = std::time::Instant::now();
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(r["rows"][0]["status"], "pass", "the check's verdict stands when git is too slow: {r}");
    assert!(start.elapsed() < Duration::from_millis(3500), "{:?}", start.elapsed());
    // The second call does not pay for the slow repo again.
    let start = std::time::Instant::now();
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(r["rows"][0]["status"], "pass", "{r}");
    assert!(start.elapsed() < Duration::from_millis(300), "second call re-ran the slow git: {:?}", start.elapsed());
}

#[test]
fn checks_run_before_the_repo_scan_so_a_slow_check_gets_its_full_timeout() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let (_a, _s, repo, _h) = live_evidence_repo(&t, true, false);
    let ev = t.home().join("live-ev/green.json");
    let mut live = ev_task(&d, "t1", "done", &["ac"], "green");
    live["evidence"]["path"] = json!(ev.to_string_lossy());
    let (mut app, slug) = campaign_with_tasks(&t, vec![live, ev_task(&d, "t2", "done", &["ac2"], "slow")]);
    lock(&app.runs).insert("run-x".into(), Run { session_id: "s".into(), run_id: "20261105-1230".into(), campaign: slug.clone(), cwd: repo, nonces: Redactor::default() });
    app.bridge.git = slow_git(&t);
    app.evidence_timeout = Duration::from_millis(2500);
    app.evidence_budget = Duration::from_secs(3);
    let r = app.evidence_list(&slug).unwrap();
    // With the scan first, the slow git would have used 2 s and left the check under 1 s of its 2.5 s: it would read "checking".
    assert_eq!(row(&r, "t2", "ac2")["status"], "unverified", "a check that ran its full timeout is unverified, not cut by the budget: {r}");
    assert_eq!(row(&r, "t1", "ac")["status"], "pass", "{r}");
}

#[test]
fn a_repo_git_cannot_read_keeps_the_checks_own_verdict_and_nothing_is_remembered() {
    let t = Tmp::new();
    let (app, slug, _repo, _head) = live_evidence_repo(&t, false, false);
    assert_eq!(status_of(&app, &slug), "pass", "not a git repo: not stale");
    // Git had its full time and could not read it: remembered as unknown, so it is not asked again for a while.
    assert!(lock(&app.worktree_state).values().all(|(_, st)| st.is_none()));
}

#[test]
fn a_big_untracked_tree_does_not_blow_the_budget() {
    let t = Tmp::new();
    let (app, slug, repo, _head) = live_evidence_repo(&t, true, false);
    let big = repo.join("node_modules_like");
    fs::create_dir_all(&big).unwrap();
    for i in 0..3000 {
        fs::write(big.join(format!("f{i}.js")), b"x").unwrap();
    }
    let start = std::time::Instant::now();
    assert_eq!(status_of(&app, &slug), "stale");
    assert!(start.elapsed() < Duration::from_secs(3), "{:?}", start.elapsed());
}

fn detail_fixture(t: &Tmp, nonce: Option<&str>) -> (App, String, std::path::PathBuf) {
    let d = std::path::PathBuf::from("/x");
    let mut task = ev_task(&d, "t1", "failed", &["ac"], "evidence");
    task["report"] = json!({"status": "failed", "summary": format!("tests red {}", nonce.unwrap_or(""))});
    let (app, slug) = campaign_with_tasks(t, vec![task]);
    if let Some(n) = nonce {
        lock(&app.runs).insert("run-x".into(), Run { session_id: "s".into(), run_id: "20261105-1230".into(), campaign: slug.clone(), cwd: t.home().join("repo"), nonces: Redactor::new(Some(n)) });
    }
    let ev = t.home().join(".claude/vajra/campaigns").join(&slug).join("evidence");
    (app, slug, ev)
}

#[test]
fn evidence_detail_returns_each_command_its_exit_code_and_a_failed_ones_output_tail() {
    let t = Tmp::new();
    let (app, slug, ev) = detail_fixture(&t, None);
    fs::write(ev.join("evidence.json"), serde_json::to_vec(&json!({"commands": [
        {"cmd": "npm test", "exit": 1, "log": "cmd-1.log", "passed": false},
        {"cmd": "tsc --noEmit", "exit": 0, "log": "cmd-2.log", "passed": true},
        {"cmd": "evil", "exit": 1, "log": "../escape.log", "passed": false},
    ]})).unwrap()).unwrap();
    fs::write(ev.join("cmd-1.log"), format!("{}\x1b[31mFAIL src/a.test.ts\x1b[0m\n<script>alert(1)</script>\n", "noise\n".repeat(2000))).unwrap();
    fs::write(ev.join("cmd-2.log"), "all green\n").unwrap();
    fs::write(t.home().join(".claude/vajra/campaigns").join(&slug).join("escape.log"), "outside the evidence dir").unwrap();
    let r = app.evidence_detail(&slug, "t1").unwrap();
    let cmds = r["commands"].as_array().unwrap();
    assert_eq!(cmds.len(), 3, "{r}");
    assert_eq!(cmds[0]["cmd"], "npm test");
    assert_eq!(cmds[0]["exit_code"], 1);
    let tail = cmds[0]["output_tail"].as_str().unwrap();
    assert!(tail.contains("FAIL src/a.test.ts") && !tail.contains('\u{1b}'), "escapes stripped: {tail:?}");
    assert!(tail.contains("<script>alert(1)</script>"), "kept as inert text for the UI to render as text");
    assert!(tail.len() <= 4 * 1024 + 16, "tail is bounded: {}", tail.len());
    assert!(cmds[1]["output_tail"].is_null(), "a passing command carries no output");
    assert_eq!(cmds[2]["output_tail"], Value::Null, "a log name that climbs out is never read");
    assert_eq!(r["notes"], "tests red ");
}

#[test]
fn evidence_detail_redacts_a_planted_secret_from_commands_output_and_notes() {
    let t = Tmp::new();
    let secret = "STEERNONCE-9f8e7d6c5b";
    let (app, slug, ev) = detail_fixture(&t, Some(secret));
    fs::write(ev.join("evidence.json"), serde_json::to_vec(&json!({"commands": [
        {"cmd": format!("curl -H 'x: {secret}' host"), "exit": 1, "log": "cmd-1.log", "passed": false},
    ]})).unwrap()).unwrap();
    fs::write(ev.join("cmd-1.log"), format!("token={secret}\n")).unwrap();
    let r = app.evidence_detail(&slug, "t1").unwrap().to_string();
    assert!(!r.contains(secret), "secret leaked: {r}");
    assert!(r.contains("[redacted]"), "{r}");
}

#[test]
fn evidence_detail_bounds_the_evidence_file_and_refuses_a_link() {
    let t = Tmp::new();
    let (app, slug, ev) = detail_fixture(&t, None);
    fs::write(ev.join("evidence.json"), vec![b' '; 70 * 1024]).unwrap();
    let r = app.evidence_detail(&slug, "t1").unwrap();
    assert_eq!(r["truncated"], true, "{r}");
    assert_eq!(r["commands"].as_array().unwrap().len(), 0);
    fs::remove_file(ev.join("evidence.json")).unwrap();
    fs::write(ev.join("real.json"), br#"{"commands":[{"cmd":"x","exit":0}]}"#).unwrap();
    std::os::unix::fs::symlink(ev.join("real.json"), ev.join("evidence.json")).unwrap();
    let r = app.evidence_detail(&slug, "t1").unwrap();
    assert_eq!(r["commands"].as_array().unwrap().len(), 0, "a symlinked evidence file is not read: {r}");
    assert_eq!(code(&app.evidence_detail("../x", "t1").unwrap_err()), "invalid_args");
}

#[test]
fn evidence_list_rejects_a_bad_slug_and_reports_an_unknown_campaign() {
    let t = Tmp::new();
    let app = t.app();
    assert_eq!(code(&app.evidence_list("../x").unwrap_err()), "invalid_args");
    assert_eq!(code(&app.evidence_list("nope").unwrap_err()), "not_found");
}

#[test]
fn evidence_outside_the_campaign_or_with_a_dotdot_path_is_unverified_without_running_a_check() {
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let mut task = ev_task(&d, "t1", "done", &["ac-1"], "green");
    task["evidence"]["path"] = json!("/tmp/green.json");
    let (app, slug) = campaign_with_tasks(&t, vec![task]);
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "t1", "ac-1")["status"], "unverified", "{r}");
}

#[test]
fn crew_nodes_carry_the_ui_role_name_next_to_the_engine_id() {
    use crate::crew::tree::{Node, NodeKind, Status};
    let n = Node { id: "agent:a1".into(), kind: NodeKind::Round, title: "round 1".into(), engine_id: Some("bhima".into()), status: Status::InProgress, overlays: vec![], children: vec![] };
    let j = node_json(&n);
    assert_eq!(j["role"], "Backend", "{j}");
    assert_eq!(j["engine_id"], "bhima");
    let g = node_json(&Node { engine_id: None, ..n });
    assert!(g["role"].is_null());
}

// ---------------------------------------------------------------- read_report (held bytes)

fn fnv32(s: &str) -> String {
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h = (h ^ b as u32).wrapping_mul(0x01000193);
    }
    format!("{h:08x}")
}

fn real_campaign_with_report(run: &impl Fn(&[&str]) -> std::process::Output, text: &str) {
    assert!(run(&["create", "real1", "--goal", "g", "--feature", "ac-1|x"]).status.success());
    let b64 = String::from_utf8(
        std::process::Command::new("sh").args(["-c", &format!("printf '%s' '{text}' | openssl base64 -A")]).output().unwrap().stdout,
    )
    .unwrap();
    let saved = run(&["report", "--campaign", "real1", "--bytes", &text.len().to_string(), "--check", &fnv32(&b64), "--base64", &b64]);
    assert!(saved.status.success(), "{}", String::from_utf8_lossy(&saved.stderr));
}

fn camp_file(t: &Tmp, name: &str) -> std::path::PathBuf {
    t.home().join(".claude/vajra/campaigns/real1").join(name)
}

#[test]
fn read_report_returns_the_signed_report() {
    let Some((t, run)) = real_script_home() else { return };
    real_campaign_with_report(&run, "Delivered on branch b");
    let r = t.app().read_report("real1").unwrap();
    assert_eq!((r["integrity"].as_str(), r["text"].as_str()), (Some("ok"), Some("Delivered on branch b")), "{r}");
}

#[test]
fn read_report_refuses_a_report_edited_after_it_was_signed() {
    let Some((t, run)) = real_script_home() else { return };
    real_campaign_with_report(&run, "Delivered on branch b");
    fs::write(camp_file(&t, "report.md"), "Delivered on branch X").unwrap();
    let r = t.app().read_report("real1").unwrap();
    assert_eq!(r["integrity"], "integrity_failed", "{r}");
    assert!(r.get("text").is_none());
}

#[test]
fn nul_in_report_breaks_hash() {
    let Some((t, run)) = real_script_home() else { return };
    real_campaign_with_report(&run, "ab");
    fs::write(camp_file(&t, "report.md"), b"a\0b").unwrap();
    assert_eq!(t.app().read_report("real1").unwrap()["integrity"], "integrity_failed");
}

#[test]
fn nul_in_signed_inputs_breaks_the_signature() {
    for name in ["campaign.json", "features.json", "progress.md"] {
        let Some((t, run)) = real_script_home() else { return };
        real_campaign_with_report(&run, "ab");
        let mut b = fs::read(camp_file(&t, name)).unwrap();
        b.insert(b.len() / 2, 0);
        fs::write(camp_file(&t, name), b).unwrap();
        assert_ne!(t.app().read_report("real1").unwrap()["integrity"], "ok", "{name}");
    }
}

#[test]
fn read_report_refuses_a_planted_report_without_a_recorded_hash() {
    let Some((t, run)) = real_script_home() else { return };
    assert!(run(&["create", "real1", "--goal", "g", "--feature", "ac-1|x"]).status.success());
    fs::write(camp_file(&t, "report.md"), "forged delivery report").unwrap();
    let r = t.app().read_report("real1").unwrap();
    assert_eq!(r["integrity"], "no_report", "{r}");
    assert!(r.get("text").is_none());
}

#[test]
fn report_is_symlink_to_valid_report() {
    let Some((t, run)) = real_script_home() else { return };
    real_campaign_with_report(&run, "Delivered on branch b");
    let real = camp_file(&t, "report.md");
    let moved = t.home().join("elsewhere.md");
    fs::rename(&real, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &real).unwrap();
    assert_eq!(t.app().read_report("real1").unwrap()["integrity"], "integrity_failed");
}

#[test]
fn files_rewritten_after_read_leave_the_verdict_and_text_on_the_held_bytes() {
    let Some((t, run)) = real_script_home() else { return };
    real_campaign_with_report(&run, "Delivered on branch b");
    let root = t.home().join(".claude/vajra/campaigns/real1");
    let between = || {
        for n in ["campaign.json", "features.json", "progress.md", "state.sig", "report.md"] {
            fs::write(root.join(n), b"rewritten after the read").unwrap();
        }
    };
    let r = t.app().read_report_with("real1", &|_| {}, &between).unwrap();
    assert_eq!((r["integrity"].as_str(), r["text"].as_str()), (Some("ok"), Some("Delivered on branch b")), "{r}");
}

#[test]
fn planted_report_swap_during_verify() {
    // Live files are tampered (report changed); a genuine copy is put back only while the check runs. The verdict
    // comes from the bytes read, so the tamper is still seen.
    let Some((t, run)) = real_script_home() else { return };
    real_campaign_with_report(&run, "Delivered on branch b");
    let report = camp_file(&t, "report.md");
    let genuine = fs::read(&report).unwrap();
    fs::write(&report, "Delivered on branch X").unwrap();
    let r2 = report.clone();
    let between = move || fs::write(&r2, &genuine).unwrap();
    let r = t.app().read_report_with("real1", &|_| {}, &between).unwrap();
    assert_eq!(r["integrity"], "integrity_failed", "{r}");
}

#[test]
fn duplicate_key_campaign_json_refused() {
    let Some((t, run)) = real_script_home() else { return };
    real_campaign_with_report(&run, "Delivered on branch b");
    let f = camp_file(&t, "campaign.json");
    let s = fs::read_to_string(&f).unwrap();
    assert!(s.contains("\"goal\""), "fixture has a goal key");
    fs::write(&f, s.replacen('{', "{\"goal\":\"dup\",", 1)).unwrap(); // goal now appears twice
    assert_eq!(t.app().read_report("real1").unwrap()["integrity"], "integrity_failed");
}

#[test]
fn held_verdict_equals_verify_over_the_tamper_corpus() {
    type Tamper = fn(&std::path::Path);
    let corpus: [(&str, Tamper); 6] = [
        ("clean", |_| {}),
        ("campaign byte", |d| { let f = d.join("campaign.json"); let mut b = fs::read(&f).unwrap(); b.push(b' '); fs::write(f, b).unwrap(); }),
        ("features byte", |d| { let f = d.join("features.json"); let mut b = fs::read(&f).unwrap(); b.push(b' '); fs::write(f, b).unwrap(); }),
        ("progress byte", |d| { let f = d.join("progress.md"); let mut b = fs::read(&f).unwrap_or_default(); b.push(b'x'); fs::write(f, b).unwrap(); }),
        ("sig flipped", |d| { let f = d.join("state.sig"); let s = fs::read_to_string(&f).unwrap(); let c = if s.starts_with('0') { "1" } else { "0" }; fs::write(f, format!("{c}{}", &s[1..])).unwrap(); }),
        ("sig removed", |d| fs::remove_file(d.join("state.sig")).unwrap()),
    ];
    for (name, tamper) in corpus {
        let Some((t, run)) = real_script_home() else { return };
        real_campaign_with_report(&run, "Delivered on branch b");
        tamper(&camp_file(&t, "").parent().unwrap().join("real1"));
        let app = t.app();
        let plain = app.verify_campaign("real1").map(|v| v["ok"] == true).unwrap_or(false);
        let held = app.read_report("real1").unwrap()["integrity"] == "ok";
        assert_eq!(held, plain, "{name}");
    }
}

#[test]
fn read_report_rejects_a_bad_slug_and_reports_a_missing_campaign() {
    let t = Tmp::new();
    assert_eq!(code(&t.app().read_report("../x").unwrap_err()), "invalid_args");
    assert_eq!(code(&t.app().read_report("nope").unwrap_err()), "not_found");
}

#[test]
fn sig_over_256_bytes_is_integrity_failed() {
    let Some((t, run)) = real_script_home() else { return };
    real_campaign_with_report(&run, "Delivered on branch b");
    fs::write(camp_file(&t, "state.sig"), vec![b'a'; 300]).unwrap();
    assert_eq!(t.app().read_report("real1").unwrap()["integrity"], "integrity_failed");
}

#[test]
fn verify_digest_never_opens_campaign_dir() {
    // After the read, the campaigns dir is made unreadable: the digest check and the features-def check must not
    // need it, so the verdict and text are unchanged.
    use std::os::unix::fs::PermissionsExt;
    let Some((t, run)) = real_script_home() else { return };
    real_campaign_with_report(&run, "Delivered on branch b");
    let dir = t.home().join(".claude/vajra/campaigns");
    let d2 = dir.clone();
    let between = move || fs::set_permissions(&d2, fs::Permissions::from_mode(0o000)).unwrap();
    let r = t.app().read_report_with("real1", &|_| {}, &between);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    let r = r.unwrap();
    assert_eq!((r["integrity"].as_str(), r["text"].as_str()), (Some("ok"), Some("Delivered on branch b")), "{r}");
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

#[test]
fn a_run_id_recorded_before_a_restart_is_not_issued_again() {
    // Campaign A was resumed in minute M by an earlier app life; it has no `deliver-M` directory. After a restart
    // in the same minute, campaign B must not get M: the Lead's run lookup and MCP binding key on the run id.
    let t = Tmp::new();
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    std::fs::create_dir_all(t.0.join("data")).ok();
    let app = t.app();
    let entry = runindex::Entry { campaign: "deliver-old".into(), session_id: sid.into(), run_id: "20261105-1230".into(), cwd: t.0.clone(), lead_session_id: None };
    runindex::append(&t.0.join("data").join("runs.jsonl"), &entry).ok();
    let now = 1_793_881_859; // 2026-11-05 12:30:59 UTC, minute 20261105-1230
    let (_, id) = app.fresh_run_id(now);
    assert_ne!(id, "20261105-1230", "the persisted id is taken");
}

// ---- review #34 (P2): a report read and the shared integrity cache ----------------------------------------------

#[test]
fn an_older_report_read_cannot_overwrite_a_newer_explicit_verify() {
    // The report-read equivalent of `an_older_listing_check_cannot_overwrite_a_newer_explicit_verify`.
    let Some((t, run)) = real_script_home() else { return };
    real_campaign_with_report(&run, "Delivered on branch b");
    let app = t.app();
    let report = camp_file(&t, "report.md");
    // 1. The report read has held valid bytes. 2. While it is paused, the report is tampered with and the user
    // verifies explicitly: that newer check fails and caches "failed".
    let between = || {
        fs::write(&report, "Delivered on branch X").unwrap();
        let v = app.verify_campaign("real1").unwrap();
        assert_eq!(v["ok"], false, "the newer explicit verify sees the tamper: {v}");
    };
    // 3. The older read resumes: its held bytes are genuine, so it returns them...
    let r = app.read_report_with("real1", &|_| {}, &between).unwrap();
    assert_eq!((r["integrity"].as_str(), r["text"].as_str()), (Some("ok"), Some("Delivered on branch b")), "{r}");
    // 4. ...but its older "ok" must not replace the newer failure: the next listing keeps it.
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "failed");
}

#[test]
fn a_newer_report_failure_survives_an_older_listing_check_that_finishes_ok() {
    // The reverse order: a listing check starts against valid state and is held; a report read that starts later
    // fails; then the older listing check finishes with exit 0. The badge must keep the newer failure.
    let t = Tmp::new();
    let ctl = barrier_campaign_script(&t);
    campaign(&t, "c1", None); // only campaign.json: the held reader refuses it (required inputs missing)
    let app = Arc::new(t.app());
    let a = { let app = app.clone(); std::thread::spawn(move || app.list_campaigns().unwrap()) };
    let end = std::time::Instant::now() + Duration::from_secs(10);
    while !ctl.join("started").exists() {
        assert!(std::time::Instant::now() < end, "the listing check never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(app.read_report("c1").unwrap()["integrity"], "integrity_failed");
    fs::write(ctl.join("go"), "").unwrap();
    assert_eq!(a.join().unwrap()["campaigns"][0]["integrity"], "ok", "the held listing check saw the old state");
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "failed");
}

#[test]
fn settle_orders_verdicts_by_ticket_and_never_loses_a_failure() {
    let t = Tmp::new();
    let app = t.app();
    let now = |app: &App| match lock(&app.verdicts).by_slug.get("c") {
        Some(Verdict::Fresh(_, v)) => *v,
        Some(Verdict::Flying(_)) => "flying",
        None => "none",
    };
    let (t1, t2, t3, t4) = (app.ticket(), app.ticket(), app.ticket(), app.ticket());
    assert!(t1 < t2 && t2 < t3 && t3 < t4, "tickets increase");
    app.settle("c", "failed", t2);
    app.settle("c", "ok", t1); // older success after a newer failure: dropped
    assert_eq!(now(&app), "failed");
    app.settle("c", "unknown", t1); // an older "could not run" cannot clear it either
    assert_eq!(now(&app), "failed");
    app.settle("c", "ok", t3); // a newer success replaces it (the repair shows)
    assert_eq!(now(&app), "ok");
    app.settle("c", "failed", t1); // an older failure still lands: fail closed
    assert_eq!(now(&app), "failed");
    app.settle("c", "ok", t4);
    assert_eq!(now(&app), "ok");
}

#[test]
fn a_listing_during_the_report_check_cannot_see_a_partial_success() {
    // review #34 (P2, 54a2df9): "ok" used to be published after the signature and features checks but before the
    // report check, so a listing at that moment reused it for a campaign whose recorded report was edited or
    // deleted (or cleared an already cached failure). Pause exactly there and list.
    for (tamper, cached_failure) in [("edit", false), ("edit", true), ("delete", false), ("delete", true)] {
        let Some((t, run)) = real_script_home() else { return };
        real_campaign_with_report(&run, "Delivered on branch b");
        let report = camp_file(&t, "report.md");
        if tamper == "edit" {
            fs::write(&report, "Delivered on branch X").unwrap();
        } else {
            fs::remove_file(&report).unwrap();
        }
        let app = t.app();
        if cached_failure {
            assert_eq!(app.verify_campaign("real1").unwrap()["ok"], false);
        }
        let seen = std::cell::RefCell::new(None);
        let seam = |st: &str| {
            if st == "check:report" {
                *seen.borrow_mut() = Some(app.list_campaigns().unwrap()["campaigns"][0]["integrity"].clone());
            }
        };
        let r = app.read_report_with("real1", &seam, &|| {}).unwrap();
        let case = format!("{tamper}, cached failure {cached_failure}");
        assert_eq!(r["integrity"], "integrity_failed", "{case}: {r}");
        assert_eq!(seen.into_inner(), Some(json!("failed")), "{case}: the listing at the report check must not see ok");
        assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "failed", "{case}");
    }
}

#[test]
fn run_overview_scopes_a_campaign_with_two_runs_to_the_current_one() {
    let t = Tmp::new();
    let task = |id: &str, st: &str| json!({"id": id, "title": id, "owner_agent": "bhima", "status": st, "deps": []});
    let (app, slug) = campaign_with_tasks(&t, vec![task("r1.t1", "closed"), task("r2.t1", "in_progress"), task("r2.t2", "pending")]);
    let camp = t.home().join(".claude/vajra/campaigns").join(&slug);
    fs::write(camp.join("progress.md"), "- 2026-11-07T00:00:00Z run r1 round 1: 1/1 criteria passing; x\n- 2026-11-07T00:01:00Z resumed by vajra-deliver run r2\n").unwrap();
    let o = app.run_overview(&slug).unwrap();
    assert_eq!(o["run_id"], "r2", "the Crew link must name the run whose workers these are");
    let ids: Vec<&str> = o["graph"]["tasks"].as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["r2.t1", "r2.t2"]);
    assert_eq!(o["timeline"]["current"], "Execute", "the previous run's failed round is not carried into the resumed run");
}
