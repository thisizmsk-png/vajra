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
        // FG-02c: a run starts only in a project's folder; `repo` is this test's project.
        test_project(&d.join("data"), &d.join("home"), &d.join("repo"));
        Self(d)
    }
    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
    fn app(&self) -> App {
        let mut app = App::open(&self.0.join("data"), &self.home()).unwrap();
        // Provider CLIs are looked for in a temp dir holding fakes, never on the machine's real PATH.
        app.provider_dirs = Some(vec![self.fake_clis(&["claude", "codex"])]);
        app
    }
    /// A dir holding fake provider CLIs (version and a signed-in status); only the named ones exist.
    fn fake_clis(&self, names: &[&str]) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let d = self.0.join(format!("fakebin-{}", names.join("-")));
        fs::create_dir_all(&d).unwrap();
        for n in names {
            let body = "#!/bin/sh\ncase \"$1\" in --version) echo '2.0.1 (fake)';; auth) echo '{\"loggedIn\": true}';; login) echo 'Logged in';; esac\n";
            fs::write(d.join(n), body).unwrap();
            fs::set_permissions(d.join(n), fs::Permissions::from_mode(0o755)).unwrap();
        }
        d
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
fn one_active_run_per_project() {
    // ADR-15 D5: a second run in the same project is held (two workflows would fight over its branches and
    // worktrees), with a reason; it is never queued.
    let t = Tmp::new();
    let app = t.app();
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let stream = format!("run-{sid}");
    app.sup
        .spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: t.0.join("repo"), rows: 24, cols: 80, redact: Redactor::default() })
        .unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: t.0.join("repo"), nonces: Redactor::default() });
    app.record_run("20261105-1230", &t.0.join("repo"), sid, "c", true, 1).unwrap(); // a launched run is always recorded with its project
    let cwd = t.0.join("repo").to_string_lossy().to_string();
    let input = RunInput { ask: "x".into(), max_rounds: 1, base: "ea73cb05d639b945e21c667e17103e56404861a8".into(), max_in_flight: 1, campaign: None, provider: None };
    let held = app.start_run(input, &cwd).unwrap_err();
    assert_eq!((code(&held), held["reason"].as_str()), ("run_held", Some("project_busy")), "{held}");
    assert!(held["detail"].as_str().is_some_and(|d| !d.is_empty()));
    let other = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    assert_eq!(app.resume_run(other, "c2", &cwd, None).unwrap_err()["reason"], "project_busy");
    // A past run's crew view never retargets the control service.
    let before = lock(&app.control).nonce(sid).map(str::to_string);
    let _ = app.crew("c-old", "20261101-0000");
    assert_eq!(lock(&app.control).nonce(sid).map(str::to_string), before);
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
        provider: None,
    };
    let cwd = t.0.join("repo").to_string_lossy().into_owned();
    assert_eq!(code(&app.start_run(input("   "), &cwd).unwrap_err()), "invalid_args");
    assert_eq!(code(&app.start_run(input("do it"), "relative/dir").unwrap_err()), "invalid_args");
    assert!(!t.0.join("data/control/sessions").exists() || fs::read_dir(t.0.join("data/control/sessions")).unwrap().next().is_none());
    assert_eq!(app.active_runs().unwrap(), json!({"count": 0, "runs": []}));
}

#[test]
fn resume_run_validates_ids() {
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.join("repo").to_string_lossy().into_owned();
    let e = app.resume_run("bad id;rm", "deliver-x", &cwd, None).unwrap_err();
    assert_eq!(code(&e), "invalid_args");
    let e = app.resume_run("0f8fad5b-d9cb-469f-a165-70867728950e", "../x", &cwd, None).unwrap_err();
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
    let c = with_badges(&app, false);
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
    let cwd = t.0.join("repo").to_string_lossy().into_owned();
    let sid = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    let argv = vec!["/nonexistent/vajra-no-such-binary".to_string()];
    let e = app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| Ok(argv.clone()), false).unwrap_err();
    assert_eq!(code(&e), "spawn_failed");
    assert!(lock(&app.control).nonce(sid).is_none(), "the control session ended with the failed spawn");
    assert_eq!(app.active_run_count(), 0);
}

#[test]
fn an_exited_run_ends_its_control_session() {
    // B4: once the run's PTY exits, steering must not target its workers.
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.join("repo").to_string_lossy().into_owned();
    let sid = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| Ok(vec!["true".into()]), false).unwrap();
    assert!(lock(&app.control).nonce(sid).is_some());
    app.sup.wait(&format!("run-{sid}"), Duration::from_secs(5)).unwrap();
    app.reap_runs();
    assert!(lock(&app.control).nonce(sid).is_none(), "end_session ran for the exited run");
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
    lock(&app.control).resume_session(sid).unwrap();
    let rotated = lock(&app.control).nonce(sid).unwrap().to_string();
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
    let nonce = lock(&app.control).nonce(sid).unwrap().to_string();
    app.sup.wait(&format!("run-{sid}"), Duration::from_secs(5)).unwrap();
    app.reap_runs();
    assert!(lock(&app.control).nonce(sid).is_none());
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
    let cwd = t.0.join("repo").to_string_lossy().into_owned();
    app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| Ok(vec!["true".into()]), false).unwrap();
    app.sup.wait(&format!("run-{sid}"), Duration::from_secs(5)).unwrap();
    app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| {
        app.reap_runs();
        Ok(vec!["sleep".into(), "5".into()])
    }, true).unwrap();
    assert!(lock(&app.control).nonce(sid).is_some(), "the resumed run is still steerable");
    app.sup.kill(&format!("run-{sid}")).unwrap();
}

#[test]
fn a_resumed_pty_redacts_the_launch_nonce_too() {
    // Optional: claude --resume replays the first prompt, which holds the old nonce.
    let t = Tmp::new();
    let app = t.app();
    let sid = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    let cwd = t.0.join("repo").to_string_lossy().into_owned();
    app.launch(sid, "20261105-1230", "c", Path::new(&cwd), |_| Ok(vec!["true".into()]), false).unwrap();
    let old = lock(&app.control).nonce(sid).unwrap().to_string();
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
    let (t2, id2) = app.fresh_run_id(now).unwrap();
    assert_eq!(id2, "20261105-1231");
    assert_eq!(t2, now + 60);
    lock(&app.runs).insert("run-x".into(), mk("2b4e28ba-2fa1-41d2-883f-0016d3cca427", "20261105-1231", "deliver-20261105-1231"));
    assert_eq!(app.fresh_run_id(now).unwrap().1, "20261105-1232");
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
    let a = app.fresh_run_id(1_794_000_000).unwrap().1;
    // The first resume's run entry is replaced by the second; the issued set still remembers it.
    lock(&app.runs).clear();
    let b = app.fresh_run_id(1_794_000_000).unwrap().1;
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
    let cwd = t.0.join("repo").to_string_lossy().into_owned();
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
        let l = with_badges(&t.app(), false);
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
    with_badges(&app, false);
    let l = with_badges(&app, false);
    assert_eq!(runs(&log), 2, "one verify per campaign; the second listing reuses the verdict");
    assert_eq!(l["campaigns"][0]["integrity"], "ok");
}

#[test]
fn a_verdict_older_than_the_ttl_is_verified_again() {
    let t = Tmp::new();
    let log = fake_campaign_script(&t, 0, "");
    campaign(&t, "c1", None);
    let (app, ms) = app_with_clock(&t);
    with_badges(&app, false);
    ms.fetch_add(4_999, std::sync::atomic::Ordering::SeqCst);
    with_badges(&app, false);
    assert_eq!(runs(&log), 1, "4.999 s: still inside the 5 s window");
    ms.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    with_badges(&app, false);
    assert_eq!(runs(&log), 2, "5 s: expired");
}

#[test]
fn refresh_verifies_again_inside_the_ttl() {
    let t = Tmp::new();
    let log = fake_campaign_script(&t, 0, "");
    campaign(&t, "c1", None);
    let (app, _) = app_with_clock(&t);
    with_badges(&app, false);
    with_badges(&app, true);
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
        let hs: Vec<_> = (0..8).map(|_| s.spawn(|| with_badges(&app, false)["campaigns"][0]["integrity"].clone())).collect();
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
    with_badges(&app, false);
    with_badges(&app, false);
    assert_eq!(runs(&log), 2, "an unknown verdict is retried on the next listing");
}

#[test]
fn verify_campaign_refreshes_that_campaigns_verdict() {
    let t = Tmp::new();
    fake_campaign_script(&t, 0, "");
    campaign(&t, "c1", Some(b"text"));
    let (app, _) = app_with_clock(&t);
    let verdict = |app: &App| with_badges(app, false)["campaigns"][0]["integrity"].clone();
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
    let verdict = |app: &App| with_badges(app, false)["campaigns"][0]["integrity"].clone();
    let fresh = |app: &App| with_badges(app, true)["campaigns"][0]["integrity"].clone();
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
    assert_eq!(with_badges(&app, false)["campaigns"][0]["integrity"], "unknown");
    assert!(started.elapsed() < Duration::from_secs(4), "the listing waited for the full sleep");
}

#[test]
fn list_campaigns_reverifies_after_an_edit_even_when_the_mtime_is_restored() {
    // The badge is the trust signal: a same-size edit with the old mtime put back must not keep a cached "ok".
    let t = Tmp::new();
    fake_campaign_script(&t, 0, "");
    campaign(&t, "c1", Some(b"AAAA"));
    let app = t.app();
    assert_eq!(with_badges(&app, false)["campaigns"][0]["integrity"], "ok");
    let f = t.home().join(".claude/vajra/campaigns/c1/report.md");
    let old = fs::metadata(&f).unwrap().modified().unwrap();
    fs::write(&f, b"BBBB").unwrap(); // same size
    fs::OpenOptions::new().write(true).open(&f).unwrap().set_modified(old).unwrap();
    assert_eq!(fs::metadata(&f).unwrap().modified().unwrap(), old, "mtime restored");
    fake_campaign_script(&t, 3, "signature INVALID"); // the script now says the campaign is tampered
    assert_eq!(with_badges(&app, true)["campaigns"][0]["integrity"], "failed");
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
        let lister = s.spawn(|| with_badges(&app, false));
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
    assert_eq!(with_badges(&app, false)["campaigns"][0]["integrity"], "unknown");
    fake_campaign_script(&t, 0, ""); // the install is repaired; the campaign files are untouched
    assert_eq!(with_badges(&app, false)["campaigns"][0]["integrity"], "ok");
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
    let l = with_badges(&app, false);
    assert_eq!((ok(&l, 0), ok(&l, 1)), (json!("ok"), json!("ok")));
    fs::write(&target, r#"{"features":[1]}"#).unwrap(); // edited in place, the link itself untouched
    fake_campaign_script(&t, 3, "signature INVALID");
    let l = with_badges(&app, true);
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
            let l = with_badges(&app, false);
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
    let a = { let app = app.clone(); std::thread::spawn(move || with_badges(&app, false)) };
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
    assert_eq!(with_badges(&app, false)["campaigns"][0]["integrity"], "failed");
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
    let l = with_badges(&t.app(), false);
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
    let l = with_badges(&t.app(), false);
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
    let entry = runindex::Entry { campaign: "deliver-old".into(), session_id: sid.into(), run_id: "20261105-1230".into(), cwd: t.0.clone(), lead_session_id: None, provider: None };
    runindex::append(&t.0.join("data").join("runs.jsonl"), &entry).ok();
    let now = 1_793_881_859; // 2026-11-05 12:30:59 UTC, minute 20261105-1230
    let (_, id) = app.fresh_run_id(now).unwrap();
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
    assert_eq!(with_badges(&app, false)["campaigns"][0]["integrity"], "failed");
}

#[test]
fn a_newer_report_failure_survives_an_older_listing_check_that_finishes_ok() {
    // The reverse order: a listing check starts against valid state and is held; a report read that starts later
    // fails; then the older listing check finishes with exit 0. The badge must keep the newer failure.
    let t = Tmp::new();
    let ctl = barrier_campaign_script(&t);
    campaign(&t, "c1", None); // only campaign.json: the held reader refuses it (required inputs missing)
    let app = Arc::new(t.app());
    let a = { let app = app.clone(); std::thread::spawn(move || with_badges(&app, false)) };
    let end = std::time::Instant::now() + Duration::from_secs(10);
    while !ctl.join("started").exists() {
        assert!(std::time::Instant::now() < end, "the listing check never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(app.read_report("c1").unwrap()["integrity"], "integrity_failed");
    fs::write(ctl.join("go"), "").unwrap();
    assert_eq!(a.join().unwrap()["campaigns"][0]["integrity"], "ok", "the held listing check saw the old state");
    assert_eq!(with_badges(&app, false)["campaigns"][0]["integrity"], "failed");
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
                *seen.borrow_mut() = Some(with_badges(&app, false)["campaigns"][0]["integrity"].clone());
            }
        };
        let r = app.read_report_with("real1", &seam, &|| {}).unwrap();
        let case = format!("{tamper}, cached failure {cached_failure}");
        assert_eq!(r["integrity"], "integrity_failed", "{case}: {r}");
        assert_eq!(seen.into_inner(), Some(json!("failed")), "{case}: the listing at the report check must not see ok");
        assert_eq!(with_badges(&app, false)["campaigns"][0]["integrity"], "failed", "{case}");
    }
}

// ---- #28: the list returns at once; each badge fills in from its own campaign_integrity call --------------------

/// The list as the UI sees it once every badge has filled in: the summaries, then one `campaign_integrity` per slug.
fn with_badges(app: &App, refresh: bool) -> Value {
    let mut l = app.list_campaigns_opt(refresh).unwrap();
    for c in l["campaigns"].as_array_mut().unwrap() {
        let slug = c["slug"].as_str().unwrap().to_string();
        c["integrity"] = app.campaign_integrity(&slug, refresh).unwrap()["integrity"].clone();
    }
    l
}

fn slow_campaign_script(t: &Tmp, secs: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let log = t.0.join("slow.log");
    let sh = dir.join("vajra-campaign.sh");
    fs::write(&sh, format!("#!/bin/bash\necho v >> '{}'\nsleep {secs}\nexit 0\n", log.display())).unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    log
}

#[test]
fn list_campaigns_answers_at_once_with_checking_and_starts_no_verify() {
    let t = Tmp::new();
    let log = slow_campaign_script(&t, "5");
    campaign(&t, "c1", None);
    campaign(&t, "c2", None);
    let started = std::time::Instant::now();
    let l = t.app().list_campaigns().unwrap();
    assert!(started.elapsed() < Duration::from_secs(2), "the listing waited for a verify");
    for c in l["campaigns"].as_array().unwrap() {
        assert_eq!(c["integrity"], "checking", "{c}");
        assert_eq!(c["goal"], "g", "the summary is still there: {c}");
    }
    assert_eq!(runs(&log), 0, "listing must not run the verify script");
}

#[test]
fn a_fresh_cached_verdict_is_listed_and_refresh_lists_checking_again() {
    let t = Tmp::new();
    let log = fake_campaign_script(&t, 0, "");
    campaign(&t, "c1", None);
    let (app, ms) = app_with_clock(&t);
    assert_eq!(app.campaign_integrity("c1", false).unwrap()["integrity"], "ok");
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "ok", "inside the TTL");
    assert_eq!(app.list_campaigns_opt(true).unwrap()["campaigns"][0]["integrity"], "checking", "refresh asks again");
    ms.fetch_add(5_000, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "checking", "an expired verdict is not shown");
    assert_eq!(runs(&log), 1, "listing never verifies");
}

#[test]
fn campaign_integrity_answers_ok_failed_or_unknown_and_refuses_a_bad_slug() {
    for (exit, want) in [(0, "ok"), (3, "failed"), (1, "unknown")] {
        let t = Tmp::new();
        fake_campaign_script(&t, exit, "x");
        campaign(&t, "c1", None);
        let r = t.app().campaign_integrity("c1", false).unwrap();
        assert_eq!((r["slug"].as_str(), r["integrity"].as_str()), (Some("c1"), Some(want)), "exit {exit}: {r}");
    }
    let t = Tmp::new();
    let log = fake_campaign_script(&t, 0, "");
    for bad in ["../x", "", "a b", ".hidden"] {
        assert_eq!(code(&t.app().campaign_integrity(bad, false).unwrap_err()), "invalid_args", "{bad}");
    }
    // A name that looks like an option is a valid id but never reaches the script: it reads unknown.
    assert_eq!(t.app().campaign_integrity("-rf", false).unwrap()["integrity"], "unknown");
    assert_eq!(runs(&log), 0);
}

#[test]
fn a_failed_or_timed_out_verify_is_not_kept_as_a_verdict() {
    let t = Tmp::new();
    let log = fake_campaign_script(&t, 1, "jq: not found");
    campaign(&t, "c1", None);
    let (app, _) = app_with_clock(&t);
    assert_eq!(app.campaign_integrity("c1", false).unwrap()["integrity"], "unknown");
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "checking", "unknown is not listed as a verdict");
    fake_campaign_script(&t, 0, "");
    assert_eq!(app.campaign_integrity("c1", false).unwrap()["integrity"], "ok", "asked again, not served from a cached unknown");
    assert_eq!(runs(&log), 2);
}

#[test]
fn separate_integrity_calls_for_one_campaign_share_one_verify() {
    let t = Tmp::new();
    let log = slow_campaign_script(&t, "0.5");
    campaign(&t, "c1", None);
    let app = t.app();
    let got: Vec<Value> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..8).map(|_| s.spawn(|| app.campaign_integrity("c1", false).unwrap()["integrity"].clone())).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(got.iter().all(|v| v == "ok"), "{got:?}");
    assert_eq!(runs(&log), 1, "eight calls, one verify in flight");
}

#[test]
fn integrity_calls_for_many_campaigns_never_run_more_than_four_verifies_at_once() {
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new();
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let log = t.0.join("overlap2.log");
    let sh = dir.join("vajra-campaign.sh");
    fs::write(&sh, format!("#!/bin/bash\necho s >> '{l}'\nsleep 0.4\necho e >> '{l}'\nexit 0\n", l = log.display())).unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    for i in 0..10 {
        campaign(&t, &format!("c{i}"), None);
    }
    let app = t.app();
    let got: Vec<Value> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..10).map(|i| { let app = &app; s.spawn(move || app.campaign_integrity(&format!("c{i}"), false).unwrap()["integrity"].clone()) }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(got.iter().all(|v| v == "ok"), "{got:?}");
    let (mut cur, mut peak) = (0i32, 0i32);
    for line in fs::read_to_string(&log).unwrap().lines() {
        cur += if line == "s" { 1 } else { -1 };
        peak = peak.max(cur);
    }
    assert!((2..=4).contains(&peak), "peak concurrency {peak}");
}

#[test]
fn a_call_that_cannot_get_a_verify_slot_within_the_budget_reads_unknown_and_is_not_cached() {
    let t = Tmp::new();
    slow_campaign_script(&t, "1.5");
    for i in 0..5 {
        campaign(&t, &format!("c{i}"), None);
    }
    let mut app = t.app();
    app.verify_queue_budget = Duration::from_millis(200);
    let got: Vec<(usize, Value)> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..5).map(|i| { let app = &app; s.spawn(move || (i, app.campaign_integrity(&format!("c{i}"), false).unwrap()["integrity"].clone())) }).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let unknown: Vec<_> = got.iter().filter(|(_, v)| v == "unknown").collect();
    assert_eq!(unknown.len(), 1, "four hold slots, the fifth waits past its budget: {got:?}");
    assert_eq!(got.iter().filter(|(_, v)| v == "ok").count(), 4, "{got:?}");
    let slug = format!("c{}", unknown[0].0);
    assert_eq!(app.campaign_integrity(&slug, false).unwrap()["integrity"], "ok", "the queue timeout was not remembered as a verdict");
}

/// Review of #50: `list_campaigns(refresh=true)` is followed by the hook's `campaign_integrity(slug)` calls, which pass the
/// default `refresh=false`. A forced list refresh must still end in a new verify, not in the cached verdict.
#[test]
fn a_forced_list_refresh_reaches_the_hooks_default_calls_with_a_new_verify() {
    let t = Tmp::new();
    key_dependent_script(&t);
    campaign(&t, "c1", None);
    let key = t.home().join(".claude/vajra/stand-in.key");
    fs::write(&key, "K1").unwrap();
    let (app, _) = app_with_clock(&t);
    assert_eq!(app.campaign_integrity("c1", false).unwrap()["integrity"], "ok");
    fs::write(&key, "K2").unwrap(); // the verifier now says failed; the cached "ok" is 0 ms old
    let l = app.list_campaigns_opt(true).unwrap();
    assert_eq!(l["campaigns"][0]["integrity"], "checking");
    // What useCampaignIntegrity does for each "checking" row: the default refresh=false.
    assert_eq!(app.campaign_integrity("c1", false).unwrap()["integrity"], "failed", "the forced refresh verifies afresh");
    // The new verdict is cached again: further default calls reuse it.
    assert_eq!(app.campaign_integrity("c1", false).unwrap()["integrity"], "failed");
}

/// Review of #50: a forced list refresh that finds a verify already in flight must not be answered by that older
/// flight. The hook's follow-up (default `refresh=false`) has to end in a verify that started after the refresh.
#[test]
fn a_forced_list_refresh_during_a_running_verify_is_answered_by_a_newer_verify() {
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new();
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let sh = dir.join("vajra-campaign.sh");
    // Reads the key when it starts, then takes a while: the verdict reflects the key as of the start.
    let started = t.0.join("verify-started");
    fs::write(&sh, format!("#!/bin/bash\nk=$(cat \"$HOME/.claude/vajra/stand-in.key\" 2>/dev/null)\ntouch '{}'\nsleep 0.8\n[ \"$k\" = K1 ] && exit 0\nexit 3\n", started.display())).unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    campaign(&t, "c1", None);
    let key = t.home().join(".claude/vajra/stand-in.key");
    fs::write(&key, "K1").unwrap();
    let app = t.app();
    std::thread::scope(|s| {
        let old = s.spawn(|| app.campaign_integrity("c1", false).unwrap()["integrity"].clone());
        for _ in 0..500 { // the first verify has read K1 and is running once it says so
            if started.exists() { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(started.exists());
        fs::write(&key, "K2").unwrap();
        assert_eq!(app.list_campaigns_opt(true).unwrap()["campaigns"][0]["integrity"], "checking");
        // What the hook does for the "checking" row.
        assert_eq!(app.campaign_integrity("c1", false).unwrap()["integrity"], "failed", "not the pre-refresh flight's ok");
        assert_eq!(old.join().unwrap(), "ok", "the older flight itself still ends with what it saw");
    });
    // The newer verdict is the one kept.
    assert_eq!(app.campaign_integrity("c1", false).unwrap()["integrity"], "failed");
}

/// Review of #50: the verdict an older flight leaves behind must not be listed as fresh while a forced refresh is pending.
#[test]
fn a_verdict_settled_by_a_pre_refresh_verify_is_not_listed_while_the_refresh_is_pending() {
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new();
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    let started = t.0.join("verify-started2");
    let sh = dir.join("vajra-campaign.sh");
    fs::write(&sh, format!("#!/bin/bash\ntouch '{}'\nsleep 0.5\nexit 0\n", started.display())).unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
    campaign(&t, "c1", None);
    let app = t.app();
    std::thread::scope(|s| {
        let old = s.spawn(|| app.campaign_integrity("c1", false).unwrap()["integrity"].clone());
        for _ in 0..500 {
            if started.exists() { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(started.exists());
        assert_eq!(app.list_campaigns_opt(true).unwrap()["campaigns"][0]["integrity"], "checking");
        assert_eq!(old.join().unwrap(), "ok", "the older flight settles ok after the refresh was asked");
    });
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "checking", "not the pre-refresh ok");
    // The follow-up verify (after the floor) is listed again.
    assert_eq!(app.campaign_integrity("c1", false).unwrap()["integrity"], "ok");
    assert_eq!(app.list_campaigns().unwrap()["campaigns"][0]["integrity"], "ok");
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

#[test]
fn stream_window_serves_only_the_asked_rows_with_the_streams_bounds() {
    let t = Tmp::new();
    let app = t.app();
    for i in 1..=3_000u64 {
        app.bus.push("w-a", &format!("line {i}")).unwrap();
    }
    let newest = app.stream_window("w-a", None, 40, 64 * 1024).unwrap();
    let seqs = |v: &Value| v["lines"].as_array().unwrap().iter().map(|l| l["seq"].as_u64().unwrap()).collect::<Vec<_>>();
    assert_eq!(seqs(&newest), (2_961..=3_000).collect::<Vec<_>>());
    assert_eq!((newest["head_seq"].as_u64(), newest["first_seq"].as_u64()), (Some(3_000), Some(1)));
    assert_eq!((newest["at_end"].as_bool(), newest["at_start"].as_bool(), newest["gap"].as_bool()), (Some(true), Some(false), Some(false)));
    let mid = app.stream_window("w-a", Some(1_500), 40, 64 * 1024).unwrap();
    assert_eq!(seqs(&mid), (1_500..1_540).collect::<Vec<_>>());
    // Ceilings: a caller cannot ask for more than 500 rows, and a bad stream id is refused.
    assert_eq!(app.stream_window("w-a", Some(1), 100_000, usize::MAX).unwrap()["lines"].as_array().unwrap().len(), 500);
    assert_eq!(code(&app.stream_window("../x", None, 10, 10).unwrap_err()), "invalid_args");
    assert_eq!(app.stream_window("w-unknown", None, 10, 10).unwrap()["lines"], json!([]));
}

#[test]
fn a_run_id_with_a_guarded_record_is_never_reissued_after_the_index_is_lost() {
    // `runs.jsonl` is worker-writable. If it is emptied, a restarted app must still not hand out an id whose guarded
    // record (`control/lead/<id>/`) exists: `record_run` would overwrite its session and campaign but keep the old
    // Lead session id and approval-only marker, so the new run would resume another run's Lead.
    let t = Tmp::new();
    let run = "20261106-0911";
    let at = crate::control::time::parse_iso("2026-11-06T09:11:00Z").unwrap() as u64;
    {
        let app = t.app();
        app.record_run(run, &t.0, "0f8fad5b-d9cb-469f-a165-70867728950e", "deliver-x", true, at as i64).unwrap();
    }
    let _ = std::fs::remove_file(t.0.join("data/runs.jsonl"));
    let app = t.app();
    let (_, id) = app.fresh_run_id(at).unwrap();
    assert_ne!(id, run, "an id with a guarded record is taken");
}

#[test]
fn an_exhausted_run_id_search_never_falls_back_to_a_guarded_id() {
    // Review of #46 (P1): the guarded record holds the current minute and a forged index holds every other candidate
    // minute. The search must fail closed, never reissue the guarded id through a fallback.
    let t = Tmp::new();
    let run = "20261106-0911";
    let at = crate::control::time::parse_iso("2026-11-06T09:11:00Z").unwrap() as u64;
    {
        let app = t.app();
        app.record_run(run, &t.0, "0f8fad5b-d9cb-469f-a165-70867728950e", "deliver-x", true, at as i64).unwrap();
    }
    let index = t.0.join("data/runs.jsonl");
    let _ = std::fs::remove_file(&index);
    for k in 1..120u64 {
        let e = runindex::Entry { campaign: "deliver-forged".into(), session_id: "1f8fad5b-d9cb-469f-a165-70867728950e".into(), run_id: ids::run_id_at(at + 60 * k), cwd: t.0.clone(), lead_session_id: None, provider: None };
        runindex::append(&index, &e).unwrap();
    }
    let app = t.app();
    let got = app.fresh_run_id(at);
    assert_eq!(got.as_ref().map_err(code), Err("run_id_exhausted"), "the guarded id must not be reissued: {got:?}");
    // A launch fails closed too: nothing is spawned and the guarded record is not overwritten.
    assert!(!lock(&app.issued).contains(run));
    let rec = std::fs::read_to_string(t.0.join("data/control/lead").join(run).join("campaign")).unwrap();
    assert_eq!(rec.trim(), "deliver-x");
}

/// #13 item 3: `evidence_request` decides between an archived bundle and a repo path with a lexical `starts_with`, so a
/// path that climbs out of the archive with `..` must never be treated as archived (nor handed to the checker at all).
#[test]
fn evidence_request_refuses_a_path_that_climbs_out_of_the_archive() {
    use crate::vajra::bridge::Request;
    let t = Tmp::new();
    let d = std::path::PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t1", "done", &["ac-1"], "green")]);
    let archive = app.allow().unwrap().campaigns_dir().join(&slug).join("evidence");
    let task = |p: PathBuf| json!({"evidence": {"path": p, "head": "a".repeat(40)}});
    let inside = app.evidence_request(&slug, &task(archive.join("green.json")));
    assert!(matches!(inside, Some(Request::CheckEvidence { archived: Some(_), repo: None, .. })), "{inside:?}");
    // Lexically under the archive, physically somewhere else.
    for climbing in [archive.join("..").join("..").join("other").join("evidence").join("green.json"), archive.join("..").join("tasks.json")] {
        assert!(app.evidence_request(&slug, &task(climbing.clone())).is_none(), "{}", climbing.display());
    }
}

// ---------------------------------------------------------------- verify reasons (report and evidence)

fn campaign_script_body(t: &Tmp, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let sh = t.home().join(".claude/skills/vajra/scripts/vajra-campaign.sh");
    fs::write(&sh, format!("#!/bin/bash\n{body}\n")).unwrap();
    fs::set_permissions(&sh, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A signed real campaign, then `break_it` damages the install or the files; returns `read_report`'s answer.
fn report_after(timeout_ms: Option<u64>, break_it: impl Fn(&Tmp)) -> Option<Value> {
    let (t, run) = real_script_home()?;
    real_campaign_with_report(&run, "Delivered on branch b");
    break_it(&t);
    let mut app = t.app();
    if let Some(ms) = timeout_ms {
        app.verify_timeout = Duration::from_millis(ms);
    }
    Some(app.read_report("real1").unwrap())
}

#[test]
fn report_unknown_says_spawn_failed_when_the_script_is_missing() {
    let Some(r) = report_after(None, |t| fs::remove_file(t.home().join(".claude/skills/vajra/scripts/vajra-campaign.sh")).unwrap()) else { return };
    assert_eq!(r["integrity"], "unknown", "the old field stays: {r}");
    assert_eq!(r["reason"]["code"], "spawn_failed", "{r}");
}

#[test]
fn report_unknown_says_bad_exit_with_the_exit_code_and_the_scripts_message() {
    let Some(r) = report_after(None, |t| campaign_script_body(t, "echo 'verify-digest exploded' >&2\nexit 7")) else { return };
    assert_eq!(r["integrity"], "unknown", "{r}");
    assert_eq!(r["reason"]["code"], "bad_exit", "{r}");
    assert_eq!(r["reason"]["exit_code"], 7, "{r}");
    assert!(r["reason"]["detail"].as_str().unwrap().contains("exploded"), "{r}");
}

#[test]
fn report_unknown_says_timeout_when_the_check_overruns() {
    let Some(r) = report_after(Some(300), |t| campaign_script_body(t, "sleep 30")) else { return };
    assert_eq!(r["integrity"], "unknown", "{r}");
    assert_eq!(r["reason"]["code"], "timeout", "{r}");
}

#[test]
fn report_unknown_says_oversize_when_the_report_is_over_its_cap() {
    let Some(r) = report_after(None, |t| fs::write(camp_file(t, "report.md"), vec![b'x'; 70 * 1024]).unwrap()) else { return };
    assert_eq!(r["integrity"], "unknown", "{r}");
    assert_eq!(r["reason"]["code"], "oversize", "{r}");
    assert_eq!(r["reason"]["detail"], "report.md", "{r}");
}

#[test]
fn report_unknown_says_io_error_when_a_campaign_file_cannot_be_read() {
    use std::os::unix::fs::PermissionsExt;
    // Root can read a mode-000 file, so this case only means something for an ordinary user.
    if unsafe { libc::getuid() } == 0 {
        return;
    }
    let Some(r) = report_after(None, |t| fs::set_permissions(camp_file(t, "progress.md"), fs::Permissions::from_mode(0o000)).unwrap()) else { return };
    assert_eq!(r["integrity"], "unknown", "{r}");
    assert_eq!(r["reason"]["code"], "io_error", "{r}");
    assert!(r["reason"].get("detail").is_none(), "no OS error text, it carries paths: {r}");
}

#[test]
fn bridge_failures_map_to_fixed_codes() {
    use crate::vajra::bridge::BridgeError;
    let spawn = BridgeError::Spawn(std::io::Error::from(std::io::ErrorKind::NotFound));
    assert_eq!(bridge_failure(&spawn), "spawn_failed");
    assert_eq!(bridge_failure(&BridgeError::Invalid("jq not found".into())), "jq_missing");
    assert_eq!(bridge_failure(&BridgeError::Invalid("bad name: \"x\"".into())), "io_error");
}

#[test]
fn reason_text_is_bounded_cleaned_and_free_of_the_home_path() {
    let home = Path::new("/Users/someone");
    let raw = format!("line one\x1b[31m /Users/someone/.claude/vajra/.hmac-key {}\nsecond line", "y".repeat(500));
    let d = reason_detail(&raw, home);
    assert!(d.chars().count() <= 200, "{}", d.len());
    assert!(!d.contains('\x1b') && !d.contains('\n'), "{d:?}");
    assert!(!d.contains("/Users/someone"), "{d}");
    assert!(d.starts_with("line one"), "{d}");
}

#[test]
fn report_reason_text_has_no_home_path() {
    let Some((t, run)) = real_script_home() else { return };
    real_campaign_with_report(&run, "Delivered on branch b");
    campaign_script_body(&t, "echo \"cannot open $HOME/.claude/vajra/x\" >&2\nexit 9");
    let r = t.app().read_report("real1").unwrap();
    let d = r["reason"]["detail"].as_str().unwrap();
    assert_eq!(d, "cannot open ~/.claude/vajra/x", "{r}");
}

#[test]
fn evidence_without_a_registered_run_says_no_run_registered() {
    let t = Tmp::new();
    let d = PathBuf::from("/x");
    let mut task = ev_task(&d, "t1", "done", &["ac-1"], "green");
    task["evidence"]["path"] = json!("/tmp/green.json"); // live evidence, not an archived bundle
    let (app, slug) = campaign_with_tasks(&t, vec![task]);
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "t1", "ac-1")["status"], "unverified", "the old field stays: {r}");
    assert_eq!(row(&r, "t1", "ac-1")["why"]["code"], "no_run_registered", "{r}");
}

#[test]
fn evidence_that_climbs_out_says_outside_campaign() {
    let t = Tmp::new();
    let d = PathBuf::from("/x");
    let mut task = ev_task(&d, "t1", "done", &["ac-1"], "green");
    task["evidence"]["path"] = json!("/tmp/../etc/green.json");
    let (app, slug) = campaign_with_tasks(&t, vec![task]);
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "t1", "ac-1")["why"]["code"], "outside_campaign", "{r}");
}

#[test]
fn evidence_keeps_the_scripts_reason_when_the_check_is_not_green() {
    let t = Tmp::new();
    let d = PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t2", "done", &["ac-3"], "red")]);
    let r = app.evidence_list(&slug).unwrap();
    let w = &row(&r, "t2", "ac-3")["why"];
    assert_eq!((w["code"].as_str(), w["detail"].as_str()), (Some("refused"), Some("a check failed")), "{r}");
}

#[test]
fn evidence_says_bad_exit_when_the_check_itself_fails() {
    let t = Tmp::new();
    let d = PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t4", "done", &["ac-5"], "boom")]);
    let r = app.evidence_list(&slug).unwrap();
    let w = &row(&r, "t4", "ac-5")["why"];
    assert_eq!((w["code"].as_str(), w["exit_code"].as_i64(), w["detail"].as_str()), (Some("bad_exit"), Some(1), Some("unreadable")), "{r}");
}

#[test]
fn evidence_says_timeout_for_a_check_that_ran_out_its_time() {
    let t = Tmp::new();
    let d = PathBuf::from("/x");
    let (mut app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "s1", "done", &["ac"], "slow")]);
    app.evidence_timeout = Duration::from_millis(400);
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "s1", "ac")["why"]["code"], "timeout", "{r}");
}

#[test]
fn evidence_says_spawn_failed_when_the_check_script_is_not_installed() {
    let t = Tmp::new();
    let d = PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t1", "done", &["ac-1"], "green")]);
    fs::remove_file(t.home().join(".claude/skills/vajra/scripts/vajra_tasks.py")).unwrap();
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "t1", "ac-1")["status"], "unverified", "{r}");
    assert_eq!(row(&r, "t1", "ac-1")["why"]["code"], "spawn_failed", "{r}");
}

#[test]
fn evidence_why_survives_the_verdict_cache_and_is_absent_for_other_statuses() {
    let t = Tmp::new();
    let d = PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t1", "done", &["ac-1"], "green"), ev_task(&d, "t2", "done", &["ac-3"], "red"), ev_task(&d, "t3", "in_progress", &["ac-4"], "")]);
    for _ in 0..2 {
        let r = app.evidence_list(&slug).unwrap();
        assert_eq!(row(&r, "t2", "ac-3")["why"]["code"], "refused", "the second call answers from the cache and still says why: {r}");
        assert!(row(&r, "t1", "ac-1").get("why").is_none_or(Value::is_null), "{r}");
        assert!(row(&r, "t3", "ac-4").get("why").is_none_or(Value::is_null), "{r}");
    }
}

#[test]
fn report_unknown_says_jq_missing_before_the_digest_script_runs() {
    // The campaign script dies on a missing jq before it dispatches verify-digest, so the digest step must not be the one to find out.
    let (t, run) = match real_script_home() { Some(x) => x, None => return };
    real_campaign_with_report(&run, "Delivered on branch b");
    let marker = t.0.join("ran-script");
    campaign_script_body(&t, &format!("touch '{}'\nexit 0", marker.display()));
    let mut app = t.app();
    app.bridge.jq_candidates.clear(); // no jq on this machine
    let r = app.read_report("real1").unwrap();
    assert_eq!(r["integrity"], "unknown", "{r}");
    assert_eq!(r["reason"]["code"], "jq_missing", "{r}");
    assert!(!marker.exists(), "jq is checked before any script runs");
}

#[test]
fn report_unknown_says_jq_missing_when_the_script_itself_reports_it() {
    // A jq the app found but the script's own PATH did not: the script's "jq is required" is a missing dependency, not a bad exit.
    let Some(r) = report_after(None, |t| campaign_script_body(t, "echo 'vajra-campaign: jq is required' >&2\nexit 1")) else { return };
    assert_eq!(r["integrity"], "unknown", "{r}");
    assert_eq!(r["reason"]["code"], "jq_missing", "{r}");
}

#[test]
fn report_unknown_says_oversize_when_the_scripts_output_hits_the_cap() {
    // Output cut at the bridge's cap is not a verifier failure: the exit code and the partial text say nothing reliable.
    let Some(r) = report_after(None, |t| campaign_script_body(t, "head -c 2200000 /dev/zero | tr '\\0' 'x'\nexit 7")) else { return };
    assert_eq!(r["integrity"], "unknown", "{r}");
    assert_eq!(r["reason"]["code"], "oversize", "{r}");
    assert!(r["reason"].get("exit_code").is_none(), "no exit code is reported for a cut run: {r}");
    assert!(r["reason"]["detail"].as_str().is_some_and(|d| d.contains("output")), "{r}");
}

#[test]
fn a_cut_verify_digest_that_exits_3_is_unknown_not_a_tamper() {
    let Some(r) = report_after(None, |t| campaign_script_body(t, "head -c 2200000 /dev/zero | tr '\\0' 'x'\nexit 3")) else { return };
    assert_eq!(r["integrity"], "unknown", "a cut exit 3 is not a tamper: {r}");
    assert_eq!(r["reason"]["code"], "oversize", "{r}");
}

#[test]
fn report_unknown_says_oversize_when_only_stderr_hits_the_cap() {
    let Some(r) = report_after(None, |t| campaign_script_body(t, "head -c 2200000 /dev/zero | tr '\\0' 'x' >&2\nexit 7")) else { return };
    assert_eq!(r["reason"]["code"], "oversize", "{r}");
}

#[test]
fn evidence_cut_output_that_still_parses_as_green_is_oversize_not_a_verdict() {
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new();
    let d = PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t1", "done", &["ac-1"], "green")]);
    let py = t.home().join(".claude/skills/vajra/scripts/vajra_tasks.py");
    fs::write(&py, "#!/usr/bin/env python3\nimport sys\nsys.stdout.write('{\"green\": true}')\nsys.stderr.write('x' * 2200000)\nsys.exit(0)\n").unwrap();
    fs::set_permissions(&py, fs::Permissions::from_mode(0o755)).unwrap();
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "t1", "ac-1")["status"], "unverified", "{r}");
    assert_eq!(row(&r, "t1", "ac-1")["why"]["code"], "oversize", "{r}");
}

#[test]
fn evidence_says_oversize_when_the_check_output_hits_the_cap() {
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new();
    let d = PathBuf::from("/x");
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(&d, "t1", "done", &["ac-1"], "green")]);
    let py = t.home().join(".claude/skills/vajra/scripts/vajra_tasks.py");
    fs::write(&py, "#!/usr/bin/env python3\nimport sys\nsys.stdout.write('x' * 2200000)\nsys.exit(0)\n").unwrap();
    fs::set_permissions(&py, fs::Permissions::from_mode(0o755)).unwrap();
    let r = app.evidence_list(&slug).unwrap();
    assert_eq!(row(&r, "t1", "ac-1")["status"], "unverified", "{r}");
    assert_eq!(row(&r, "t1", "ac-1")["why"]["code"], "oversize", "{r}");
}

// ------------------------------------------------ output cut at the cap is never interpreted
// Every reader of a bridge `Output` goes through `Output::exit` / `stdout_whole` (or checks `truncated` first). Each test
// below makes the tool print a plausible, parseable answer and then overflow the cap on stderr, so the kept stdout and
// the exit code look right and only `truncated` says otherwise.

/// An executable that prints `answer` on stdout, floods stderr past the cap, and exits with `code`.
fn cut_exe(path: &Path, interpreter_line: &str, answer: &str, code: i32) {
    use std::os::unix::fs::PermissionsExt;
    let body = if interpreter_line.contains("python") {
        format!("{interpreter_line}\nimport sys\nsys.stdout.write({answer:?})\nsys.stderr.write('x' * 2200000)\nsys.exit({code})\n")
    } else {
        format!("{interpreter_line}\nprintf '%s' '{answer}'\nhead -c 2200000 /dev/zero | tr '\\0' 'x' >&2\nexit {code}\n")
    };
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn output_exit_and_stdout_whole_hide_a_cut_or_killed_run() {
    use crate::vajra::bridge::Output;
    let o = |truncated, timed_out| Output { code: Some(0), stdout: "{}".into(), stderr: String::new(), truncated, timed_out };
    assert_eq!((o(false, false).exit(), o(false, false).stdout_whole()), (Some(0), Some("{}")));
    // A killed run that still carries a status (3, the tamper code) is not read as that status.
    let killed = Output { code: Some(3), stdout: String::new(), stderr: String::new(), truncated: false, timed_out: true };
    assert_eq!(killed.exit(), None);
    for cut in [o(true, false), o(false, true), o(true, true)] {
        assert_eq!((cut.exit(), cut.stdout_whole()), (None, None), "{cut:?}");
    }
}

#[test]
fn a_cut_campaign_verify_that_exits_3_is_unknown_not_failed() {
    let t = Tmp::new();
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    cut_exe(&dir.join("vajra-campaign.sh"), "#!/bin/bash", "", 3);
    let app = t.app();
    assert_eq!(app.integrity_run("deliver-20261105-1810", true).0, "unknown");
}

#[test]
fn a_cut_task_evidence_check_gives_no_code_and_no_result() {
    let t = Tmp::new();
    let (app, slug) = campaign_with_tasks(&t, vec![ev_task(Path::new("/x"), "t1", "done", &["ac-1"], "green")]);
    cut_exe(&t.home().join(".claude/skills/vajra/scripts/vajra_tasks.py"), "#!/usr/bin/env python3", "{\"ok\": true, \"green\": true, \"reasons\": []}", 0);
    let r = app.task(&slug, "t1").unwrap();
    assert_eq!(r["evidence"]["truncated"], true, "{r}");
    assert!(r["evidence"]["code"].is_null() && r["evidence"]["result"].is_null(), "a cut check has no code and no result: {r}");
}

#[test]
fn a_cut_kb_search_returns_no_hits() {
    let t = Tmp::new();
    fs::create_dir_all(t.home().join(".claude/skills/vajra/scripts")).unwrap();
    cut_exe(&t.home().join(".claude/skills/vajra/scripts/vajra_kb.py"), "#!/usr/bin/env python3", "kb-abcdef fact a title\n", 0);
    let app = t.app();
    let r = app.kb_search("q", 5, t.home().to_str().unwrap()).unwrap();
    assert_eq!(r["truncated"], true, "{r}");
    assert!(r["hits"].as_array().unwrap().is_empty() && r["code"].is_null(), "a cut search lists nothing: {r}");
}

#[test]
fn a_cut_git_answer_is_not_a_base_a_head_or_a_worktree_fact() {
    let sha = "a".repeat(40);
    // resolve_base: a 40-hex answer from a cut run is not trusted.
    let t = Tmp::new();
    let mut app = t.app();
    app.bridge.git = t.home().join("cut-git.sh");
    cut_exe(&app.bridge.git, "#!/bin/sh", &sha, 0);
    assert!(app.resolve_base(&t.home(), "main").is_err());
    // worktree_state (evidence staleness) learns nothing from a cut head or status.
    assert_eq!(app.worktree_state(&t.home(), std::time::Instant::now() + Duration::from_secs(10)), None);
    // worktree_info: no branch and no base from a cut run.
    let t2 = Tmp::new();
    let (mut app2, _repo, _wt) = crew_fixture(&t2, &|r| r.join(".claude/worktrees/w1"));
    app2.bridge.git = t2.home().join("cut-git.sh");
    cut_exe(&app2.bridge.git, "#!/bin/sh", &sha, 0);
    let i = app2.worktree_info("aa870dd40540481f0").unwrap();
    assert!(i["branch"].is_null() && i["base"].is_null(), "{i}");
}

#[test]
fn a_cut_verify_run_never_resumes_and_a_cut_diff_stays_viewable() {
    let t = Tmp::new();
    let dir = t.home().join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&dir).unwrap();
    for code in [0, 3] {
        cut_exe(&dir.join("vajra-campaign.sh"), "#!/bin/bash", "", code);
        let app = t.app();
        let e = app.verify_campaign("deliver-20261105-1810").unwrap_err();
        assert_eq!(code_of_err(&e), "verify_failed", "a cut exit {code} is neither ok nor a signature failure: {e}");
    }
    // diff stays viewable: a cut `git diff` keeps its code and partial text and says `truncated`, for the UI's notice.
    let mut app = t.app();
    app.bridge.git = t.home().join("cut-git.sh");
    cut_exe(&app.bridge.git, "#!/bin/sh", "diff --git a/f b/f\n", 0);
    let d = app.diff(t.home().to_str().unwrap(), "main").unwrap();
    assert_eq!((d["truncated"].clone(), d["code"].clone()), (json!(true), json!(0)), "{d}");
    assert!(d["diff"].as_str().unwrap().starts_with("diff --git"), "{d}");
}

fn code_of_err(v: &Value) -> &str {
    v["error"].as_str().unwrap_or("")
}

/// ADR-15 2.6 step 1: with two runs live, each keeps its own control session, and the end of one ends only its own.
#[test]
fn two_live_runs_keep_their_own_control_sessions_and_reap_independently() {
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.join("repo");
    let (a, b) = ("1b4e28ba-2fa1-41d2-883f-0016d3cca427", "0f8fad5b-d9cb-469f-a165-70867728950e");
    for (sid, run, argv) in [(a, "20261108-0900", vec!["true".to_string()]), (b, "20261108-0901", vec!["sleep".into(), "5".into()])] {
        let stream = format!("run-{sid}");
        app.sup.spawn(&stream, SpawnSpec { argv, cwd: cwd.clone(), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
        lock(&app.runs).insert(stream, Run { session_id: sid.into(), run_id: run.into(), campaign: format!("c-{run}"), cwd: cwd.clone(), nonces: Redactor::default() });
        lock(&app.control).start_session(sid, run).unwrap();
    }
    let nb = lock(&app.control).nonce(b).unwrap().to_string();
    assert_ne!(lock(&app.control).nonce(a).unwrap(), nb);
    app.sup.wait(&format!("run-{a}"), Duration::from_secs(5)).unwrap();
    app.reap_runs();
    let svc = lock(&app.control);
    assert!(!svc.has_session(a), "the exited run's session ended");
    assert_eq!(svc.nonce(b), Some(nb.as_str()), "the live run's session is untouched");
    drop(svc);
    app.refresh_workers(); // every live run is refreshed; must not panic or touch the ended one
    app.sup.kill(&format!("run-{b}")).unwrap();
}

/// Two folders, each its own project (a `.git` dir), under the test's temp dir.
fn repos(t: &Tmp, names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(|n| {
        let d = t.0.join(n);
        fs::create_dir_all(d.join(".git")).unwrap();
        test_project(&t.0.join("data"), &t.home(), &d);
        d
    }).collect()
}

const SIDS: [&str; 3] = ["1b4e28ba-2fa1-41d2-883f-0016d3cca427", "0f8fad5b-d9cb-469f-a165-70867728950e", "7c9e6679-7425-40de-944b-e07fc1f90ae7"];
const RUNS: [&str; 3] = ["20261108-1000", "20261108-1001", "20261108-1002"];

fn sleeper(_: &str) -> Result<Vec<String>, Value> {
    Ok(vec!["sleep".into(), "5".into()])
}

/// ADR-15 D5: runs in two projects go at once (2 by default); a third is held until the setting allows it.
#[test]
fn two_projects_run_at_once_and_the_app_cap_holds_a_third() {
    let t = Tmp::new();
    let app = t.app();
    let r = repos(&t, &["one", "two", "three"]);
    // Two workers each, so the worker ceiling (8) is not what holds the third run.
    let go = |i: usize, repo: &Path| app.launch_kind(SIDS[i], RUNS[i], &format!("c{i}"), repo, sleeper, false, true, Engine::Claude, 2);
    go(0, &r[0]).unwrap();
    go(1, &r[1]).unwrap();
    assert_eq!(app.active_run_count(), 2);
    {
        let svc = lock(&app.control);
        assert!(svc.has_session(SIDS[0]) && svc.has_session(SIDS[1]));
        assert_ne!(svc.nonce(SIDS[0]), svc.nonce(SIDS[1]));
        assert!(caps::project_key(&r[0]) != caps::project_key(&r[1]));
    }
    let held = go(2, &r[2]).unwrap_err();
    assert_eq!((code(&held), held["reason"].as_str()), ("run_held", Some("app_runs_full")), "{held}");
    assert!(!lock(&app.control).has_session(SIDS[2]), "a held start takes no control session");
    assert_eq!(app.active_run_count(), 2, "nothing running was stopped");
    assert_eq!(code(&app.set_run_limits(5).unwrap_err()), "invalid_input");
    assert_eq!(app.set_run_limits(3).unwrap()["max_runs"], 3);
    go(2, &r[2]).unwrap();
    assert_eq!(app.active_run_count(), 3);
    // A lower cap never stops running work; it only holds new starts.
    app.set_run_limits(1).unwrap();
    assert_eq!(app.active_run_count(), 3);
    for s in SIDS {
        app.sup.kill(&format!("run-{s}")).unwrap();
    }
}

/// ADR-15 D5: the worker ceiling (8) counts every concurrent run's maxInFlight; a lone run is never held by it.
#[test]
fn the_worker_ceiling_holds_a_second_run_that_would_go_over() {
    let t = Tmp::new();
    let app = t.app();
    let r = repos(&t, &["one", "two"]);
    app.launch_kind(SIDS[0], RUNS[0], "c0", &r[0], sleeper, false, true, Engine::Claude, 6).unwrap();
    let held = app.launch_kind(SIDS[1], RUNS[1], "c1", &r[1], sleeper, false, true, Engine::Claude, 4).unwrap_err();
    assert_eq!(held["reason"], "workers_full", "{held}");
    app.launch_kind(SIDS[1], RUNS[1], "c1", &r[1], sleeper, false, true, Engine::Claude, 2).unwrap();
    for s in &SIDS[..2] {
        app.sup.kill(&format!("run-{s}")).unwrap();
    }
    // With both ended, a lone run may use more than the ceiling (today's 1 to 16 range).
    app.sup.wait(&format!("run-{}", SIDS[0]), Duration::from_secs(5)).unwrap();
    app.sup.wait(&format!("run-{}", SIDS[1]), Duration::from_secs(5)).unwrap();
    app.launch_kind(SIDS[2], RUNS[2], "c2", &r[0], sleeper, false, true, Engine::Claude, 16).unwrap();
    app.sup.kill(&format!("run-{}", SIDS[2])).unwrap();
}

#[test]
fn a_launched_run_belongs_to_its_projects_director_general() {
    let t = Tmp::new();
    let app = t.app();
    let r = repos(&t, &["alpha", "beta"]);
    app.launch_kind(SIDS[0], RUNS[0], "c0", &r[0], sleeper, false, true, Engine::Claude, 2).unwrap();
    app.launch_kind(SIDS[1], RUNS[1], "c1", &r[1], sleeper, false, true, Engine::Claude, 2).unwrap();
    let (pa, pb) = (test_project(&t.0.join("data"), &t.home(), &r[0]), test_project(&t.0.join("data"), &t.home(), &r[1]));
    assert_ne!(pa, pb);
    assert_eq!(std::fs::read_to_string(t.0.join("data/control/lead").join(RUNS[0]).join("project")).unwrap(), pa, "pinned in the guarded record");
    {
        let mut svc = lock(&app.control);
        svc.set_generation(&pa, 1).unwrap();
        let idle = test_project(&t.0.join("data"), &t.home(), &t.0.join("gamma"));
        svc.set_generation(&idle, 1).unwrap();
        svc.set_generation("k_00000000000000000000000000000000", 1).unwrap();
        // Alpha's Director-General has a live session in scope (an unknown worker is "not found"); a project with no
        // live run has none at all (control is off for it), so nothing falls back to every session.
        let msg = json!({"worker": "nobody", "text": "x"});
        assert!(matches!(svc.call_tool_for_project(&pa, 1, "message_worker", &msg), Err(crate::control::service::ControlError::WorkerNotFound)));
        assert!(matches!(svc.call_tool_for_project(&idle, 1, "message_worker", &msg), Err(crate::control::service::ControlError::ControlDisabled)));
        // FG-02c: a stand-in id is no store project, so its calls are refused outright.
        assert!(matches!(svc.call_tool_for_project("k_00000000000000000000000000000000", 1, "message_worker", &msg), Err(crate::control::service::ControlError::ProjectMissing)));
    }
    for s in &SIDS[..2] {
        app.sup.kill(&format!("run-{s}")).unwrap();
    }
}

#[test]
fn terminal_roots_are_the_run_dirs_of_runs_this_app_started_not_the_worker_writable_index() {
    let t = Tmp::new();
    let app = t.app();
    assert!(app.terminal_roots().is_empty());
    let (a, b) = (t.home().join("repo-a"), t.home().join("repo-b"));
    for (k, cwd) in [("s1", &a), ("s2", &a), ("s3", &b)] {
        lock(&app.runs).insert(k.into(), Run { session_id: k.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: cwd.clone(), nonces: Redactor::default() });
    }
    assert_eq!(app.terminal_roots(), vec![a.clone(), b]);
    // A line a worker appends to runs.jsonl names a dir; it grants nothing.
    fs::write(t.0.join("data/runs.jsonl"), format!("{{\"campaign\":\"c\",\"session_id\":\"x\",\"run_id\":\"20261105-1231\",\"cwd\":\"{}\"}}\n", t.home().join("elsewhere").display())).unwrap();
    assert!(!app.terminal_roots().iter().any(|r| r.ends_with("elsewhere")));
}

/// FG-02c (decision Q1, 3.1 test 3 and 3.4 test 18): a run starts only in a project's folder. Elsewhere it is refused
/// with the add action and the folder, and leaves no control session, no stream and no record; an archived project
/// is refused too. A run that starts gets a `kind: run` line in its project's sessions, parented to the orchestrator.
#[test]
fn a_run_starts_only_in_an_active_project_and_is_recorded_under_its_orchestrator() {
    let t = Tmp::new();
    let app = t.app();
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let loose = t.0.join("loose");
    fs::create_dir_all(&loose).unwrap();
    let e = app.launch(sid, "20261105-1230", "c", &loose, |_| Ok(vec!["true".into()]), false).unwrap_err();
    assert_eq!((code(&e), e["action"].as_str(), e["path"].as_str()), ("not_a_project", Some("add_project"), loose.to_str()), "{e}");
    assert!(!lock(&app.control).has_session(sid) && lock(&app.runs).is_empty(), "nothing started");
    assert!(!t.0.join("data/control/lead/20261105-1230").exists(), "no run record");
    let archived = test_project(&t.0.join("data"), &t.home(), &loose);
    app.projects.archive(&archived, None).unwrap();
    assert_eq!(code(&app.launch(sid, "20261105-1230", "c", &loose, |_| Ok(vec!["true".into()]), false).unwrap_err()), "project_archived");
    // In the project: pinned, and recorded as a run of its orchestrator.
    let repo = t.0.join("repo");
    let pid = test_project(&t.0.join("data"), &t.home(), &repo);
    app.launch(sid, "20261105-1230", "deliver-x", &repo, |_| Ok(vec!["true".into()]), false).unwrap();
    assert_eq!(fs::read_to_string(t.0.join("data/control/lead/20261105-1230/project")).unwrap(), pid);
    let log = app.projects.sessions(&pid).unwrap().sessions;
    let run = log.iter().find(|s| s.kind == crate::projects::contract::SessionKind::Run).expect("a run line");
    let orch = fs::read_to_string(t.0.join("data/control/projects").join(&pid).join("orchestrator/record")).unwrap();
    assert_eq!((run.run_id.as_deref(), run.campaign.as_deref(), run.parent.as_deref()), (Some("20261105-1230"), Some("deliver-x"), Some(orch.as_str())));
    let _ = app.sup.wait(&format!("run-{sid}"), Duration::from_secs(5));
}

// ---- FG-09: the provider a run starts and resumes on

const PSID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

fn input_for(provider: Option<&str>) -> RunInput {
    RunInput { ask: "do it".into(), max_rounds: 1, base: "ea73cb05d639b945e21c667e17103e56404861a8".into(), max_in_flight: 1, campaign: None, provider: provider.map(str::to_string) }
}

#[test]
fn g12_a_provider_that_is_not_installed_is_refused_and_no_other_one_is_tried() {
    let t = Tmp::new();
    let mut app = t.app();
    app.provider_dirs = Some(vec![t.fake_clis(&["claude"])]); // claude is there, codex is not
    let cwd = t.0.join("repo").to_string_lossy().into_owned();
    let e = app.start_run(input_for(Some("codex")), &cwd).unwrap_err();
    assert_eq!(code(&e), "provider_not_installed", "{e}");
    let e = app.resume_run(PSID, "deliver-x", &cwd, Some("codex")).unwrap_err();
    assert_eq!(code(&e), "provider_not_installed", "{e}");
    assert_eq!(app.active_runs().unwrap(), json!({"count": 0, "runs": []}), "nothing was started on claude instead");
    assert!(!t.0.join("data/runs.jsonl").exists());
}

#[test]
fn an_unknown_provider_is_invalid_not_defaulted() {
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.join("repo").to_string_lossy().into_owned();
    for bad in ["gemini", "", "Claude", "claude "] {
        let e = app.start_run(input_for(Some(bad)), &cwd).unwrap_err();
        assert_eq!(code(&e), "invalid_args", "{bad:?}: {e}");
    }
}

#[test]
fn a_claude_run_is_recorded_and_offered_for_resume_but_a_codex_run_is_not() {
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.join("repo").to_string_lossy().into_owned();
    assert_eq!(app.chosen_engine(None).unwrap(), Engine::Claude, "no preference means claude");
    assert_eq!(app.chosen_engine(Some("codex")).unwrap(), Engine::Codex);
    app.launch_engine(Engine::Claude, PSID, "20261108-1200", "deliver-claude", Path::new(&cwd), |_| Ok(vec!["true".into()]), false, caps::DEFAULT_RUN_WORKERS).unwrap();
    let other = "1a8fad5b-d9cb-469f-a165-70867728950f";
    // `true` exits at once on an idle machine, but under load its PTY can still be open, and a second launch is refused
    // while a run is active. Wait (bounded) for the first run to finish instead of assuming it already has.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while app.active_run_count() > 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(app.active_run_count(), 0, "the first run never finished");
    app.launch_engine(Engine::Codex, other, "20261108-1201", "deliver-codex", Path::new(&cwd), |_| Ok(vec!["true".into()]), false, caps::DEFAULT_RUN_WORKERS).unwrap();
    let idx = fs::read_to_string(t.0.join("data/runs.jsonl")).unwrap();
    assert!(idx.contains("\"provider\":\"codex\""), "the run is still recorded with its provider");
    let targets = app.resume_targets().unwrap();
    assert_eq!(targets["targets"]["deliver-claude"]["provider"], "claude");
    assert!(targets["targets"].get("deliver-codex").is_none(), "the app-made id means nothing to codex resume: {targets}");
    // A worker rewrites the Codex row to say claude: the guarded record still wins, so it is not offered on Claude.
    let flipped = idx.replace("\"provider\":\"codex\"", "\"provider\":\"claude\"");
    assert_ne!(flipped, idx);
    fs::write(t.0.join("data/runs.jsonl"), flipped).unwrap();
    let targets = app.resume_targets().unwrap();
    assert!(targets["targets"].get("deliver-codex").is_none(), "an edited index row cannot move a Codex run onto Claude: {targets}");
    let e = app.resume_run(other, "deliver-codex", &cwd, Some("claude")).unwrap_err();
    assert_eq!(code(&e), "provider_mismatch", "{e}");
    // And a direct resume of it is refused, never handed to `codex resume <app uuid>`.
    let e = app.resume_run(other, "deliver-codex", &cwd, Some("codex")).unwrap_err();
    assert_eq!(code(&e), "provider_cannot_resume", "{e}");
    // A line written before providers existed reads as claude; an unknown id in the index is not believed.
    let mut f = fs::OpenOptions::new().append(true).open(t.0.join("data/runs.jsonl")).unwrap();
    let line = |c: &str, p: Option<&str>| json!({"campaign": c, "session_id": "2b8fad5b-d9cb-469f-a165-70867728950f", "run_id": "20261108-1202", "cwd": cwd, "provider": p}).to_string();
    std::io::Write::write_all(&mut f, format!("{}\n{}\n", line("old", None), line("forged", Some("evil"))).as_bytes()).unwrap();
    let targets = app.resume_targets().unwrap();
    assert_eq!(targets["targets"]["old"]["provider"], "claude");
    assert_eq!(targets["targets"]["forged"]["provider"], "claude");
}

#[test]
fn provider_list_answers_the_contract_from_the_machines_clis_without_touching_a_real_home() {
    let t = Tmp::new();
    let mut app = t.app();
    app.provider_dirs = Some(vec![t.fake_clis(&["claude"])]);
    let v = app.provider_list().unwrap();
    let l = crate::providers::contract::parse_provider_list(v.to_string().as_bytes()).unwrap();
    let by = |id: &str| l.providers.iter().find(|p| p.id == id).unwrap();
    assert!(by("claude").installed && !by("codex").installed);
    assert_eq!(by("claude").login.state, crate::providers::contract::LoginState::SignedIn);
}

#[test]
fn the_login_check_runs_only_when_provider_list_is_called_and_is_cached_otherwise() {
    let t = Tmp::new();
    let app = t.app();
    let counter = t.0.join("auth-runs");
    let d = t.0.join("counting");
    fs::create_dir_all(&d).unwrap();
    let script = format!("#!/bin/sh\ncase \"$1\" in --version) echo '2.0.1';; auth) echo x >> '{}'; echo '{{\"loggedIn\": true}}';; esac\n", counter.display());
    fs::write(d.join("claude"), script).unwrap();
    std::os::unix::fs::PermissionsExt::set_mode(&mut fs::metadata(d.join("claude")).unwrap().permissions(), 0o755);
    fs::set_permissions(d.join("claude"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut app = app;
    app.provider_dirs = Some(vec![d]);
    assert!(app.provider_cached().is_none());
    let runs = || fs::read_to_string(&counter).map(|s| s.lines().count()).unwrap_or(0);
    // Reading state and cache any number of times runs nothing.
    for _ in 0..5 {
        app.project_state("p_x", None).unwrap();
        let _ = app.provider_cached();
    }
    assert_eq!(runs(), 0);
    app.provider_list().unwrap();
    assert_eq!(runs(), 1);
    for _ in 0..5 {
        app.project_state("p_x", None).unwrap();
    }
    assert_eq!(runs(), 1, "no idle work");
    assert_eq!(app.provider_cached().unwrap().providers[0].login.state, crate::providers::contract::LoginState::SignedIn);
}


// ---- #80 review: a run's project is the one pinned at launch, never re-read from the worker-writable `.git`

/// A `.git` the project store accepts (HEAD and objects), and a linked worktree of it with its back-link, as git makes.
fn store_git(dir: &Path) {
    fs::create_dir_all(dir.join(".git/objects")).unwrap();
    fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
}

fn store_worktree(repo: &Path, wt: &Path, name: &str) {
    let gd = repo.join(".git/worktrees").join(name);
    fs::create_dir_all(&gd).unwrap();
    fs::create_dir_all(wt).unwrap();
    fs::write(gd.join("commondir"), "../..\n").unwrap();
    fs::write(gd.join("gitdir"), format!("{}\n", wt.join(".git").display())).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", gd.display())).unwrap();
}

#[test]
fn a_rewritten_active_worktree_git_neither_frees_its_project_nor_holds_another() {
    let t = Tmp::new();
    let app = t.app();
    let (a, b, wt) = (t.0.join("a"), t.0.join("b"), t.0.join("a-wt"));
    store_git(&a);
    store_git(&b);
    store_worktree(&a, &wt, "a-wt");
    let (pa, pb) = (test_project(&t.0.join("data"), &t.home(), &a), test_project(&t.0.join("data"), &t.home(), &b));
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let stream = format!("run-{sid}");
    app.sup.spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: wt.clone(), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: wt.clone(), nonces: Redactor::default() });
    app.record_run("20261105-1230", &wt, sid, "c", true, 1).unwrap(); // pinned to A at launch
    assert_eq!(app.pinned_project("20261105-1230").as_deref(), Some(pa.as_str()));
    // A worker of the active run re-links its worktree to B (a full, back-linked worktree of B).
    store_worktree(&b, &wt, "a-wt");
    assert_eq!(app.projects.find_by_path(&wt).unwrap().map(|p| p.id), Some(pb.clone()), "the rewrite took: the folder now looks like B's");
    assert_eq!(app.launch_project(&wt).unwrap().1.id, pa, "a resume in the worktree keeps A");
    let busy = app.within_caps(&pa, 1).unwrap_err();
    assert_eq!(busy["reason"], "project_busy", "A is still busy: {busy}");
    app.within_caps(&pb, 1).expect("B is still eligible");
    app.sup.kill(&stream).unwrap();
}

#[test]
fn an_active_run_with_no_recorded_project_holds_every_start() {
    let t = Tmp::new();
    let app = t.app();
    let elsewhere = test_project(&t.0.join("data"), &t.home(), &t.0.join("elsewhere"));
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let stream = format!("run-{sid}");
    app.sup.spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: t.0.clone(), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: t.0.clone(), nonces: Redactor::default() });
    let e = app.within_caps(&elsewhere, 1).unwrap_err();
    assert_eq!(code(&e), "project_not_recorded", "{e}");
    app.sup.kill(&stream).unwrap();
}

#[test]
fn a_symlink_alias_of_a_pinned_folder_reuses_its_pinned_project_after_a_git_rewrite() {
    use std::os::unix::fs::symlink;
    let t = Tmp::new();
    let app = t.app();
    let (a, b, wt, alias) = (t.0.join("a"), t.0.join("b"), t.0.join("a-wt"), t.0.join("a-wt-alias"));
    store_git(&a);
    store_git(&b);
    store_worktree(&a, &wt, "a-wt");
    symlink(&wt, &alias).unwrap();
    let (pa, pb) = (test_project(&t.0.join("data"), &t.home(), &a), test_project(&t.0.join("data"), &t.home(), &b));
    app.record_run("20261105-1230", &wt, "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    // A worker re-links the worktree to B.
    store_worktree(&b, &wt, "a-wt");
    assert_eq!(app.projects.find_by_path(&alias).unwrap().map(|p| p.id), Some(pb), "the rewrite took, seen through the alias");
    assert_eq!(app.launch_project(&alias).unwrap().1.id, pa, "a resume through the alias keeps A");
    assert_eq!(app.launch_project(&wt).unwrap().1.id, pa);
}

#[test]
fn recorded_runs_of_one_folder_that_disagree_on_its_project_refuse_the_launch() {
    use std::os::unix::fs::symlink;
    let t = Tmp::new();
    let app = t.app();
    let (a, alias) = (t.0.join("a"), t.0.join("a-alias"));
    fs::create_dir_all(a.join(".git")).unwrap();
    symlink(&a, &alias).unwrap();
    test_project(&t.0.join("data"), &t.home(), &a);
    let other = test_project(&t.0.join("data"), &t.home(), &t.0.join("other"));
    app.record_run("20261105-1230", &a, "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    app.record_run_as("20261105-1231", &alias, Some(&other), "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "c", true, 1).unwrap();
    let e = app.launch_project(&a).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "{e}");
}

/// #80 codex P1a, on the store id: a start in a SUBFOLDER of a pinned folder takes the folder's pinned project, not the
/// one a worker re-linked its worktree `.git` to. The cap and the other project's deny rules both follow the pin.
#[test]
fn a_subfolder_of_a_pinned_folder_keeps_its_pinned_project_after_a_git_rewrite() {
    let t = Tmp::new();
    let app = t.app();
    let (a, b, wt) = (t.0.join("a"), t.0.join("b"), t.0.join("a-wt"));
    store_git(&a);
    store_git(&b);
    store_worktree(&a, &wt, "a-wt");
    fs::create_dir_all(wt.join("src")).unwrap();
    let (pa, pb) = (test_project(&t.0.join("data"), &t.home(), &a), test_project(&t.0.join("data"), &t.home(), &b));
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let stream = format!("run-{sid}");
    app.sup.spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: wt.clone(), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: wt.clone(), nonces: Redactor::default() });
    app.record_run("20261105-1230", &wt, sid, "c", true, 1).unwrap();
    app.record_run("20261105-1229", &b, "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "d", true, 1).unwrap();
    assert_eq!(app.pinned_project("20261105-1230").as_deref(), Some(pa.as_str()));
    // A worker of A's run re-links A's worktree to B.
    store_worktree(&b, &wt, "a-wt");
    assert_eq!(app.projects.find_by_path(&wt.join("src")).unwrap().map(|p| p.id), Some(pb.clone()), "the rewrite took, seen from the subfolder");
    let p = app.launch_project(&wt.join("src")).unwrap().1;
    assert_eq!(p.id, pa, "a start in a subfolder keeps A");
    let busy = app.within_caps(&p.id, 1).unwrap_err();
    assert_eq!(busy["reason"], "project_busy", "A is still busy: {busy}");
    // The subfolder's run is recorded under A, so B's Director-General still has A's folders denied.
    app.record_run_as("20261105-1231", &wt.join("src"), Some(&p.id), "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "e", true, 1).unwrap();
    let denied = app.other_project_roots(&asking(&app, &pb), &b).unwrap();
    let real: Vec<PathBuf> = denied.iter().map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.clone())).collect();
    for want in [wt.join("src"), wt.clone(), a.clone()] {
        assert!(real.contains(&fs::canonicalize(&want).unwrap()), "{want:?} not denied: {denied:?}");
    }
    app.sup.kill(&stream).unwrap();
}

/// The ancestor rule takes only a Git project's pin: a run in a plain (non-Git) project folder does not absorb a repo
/// that is its own project inside it.
#[test]
fn a_plain_folders_pin_is_not_taken_by_a_repo_inside_it() {
    let t = Tmp::new();
    let app = t.app();
    let (plain, inner) = (t.0.join("notes"), t.0.join("notes/tool"));
    fs::create_dir_all(&plain).unwrap();
    store_git(&inner);
    let pp = test_project(&t.0.join("data"), &t.home(), &plain);
    let pi = app.projects.add(&inner, true).unwrap().project().id.clone();
    app.record_run("20261105-1230", &plain, "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    assert_eq!(app.pinned_project("20261105-1230").as_deref(), Some(pp.as_str()));
    assert_eq!(app.launch_project(&inner).unwrap().1.id, pi, "the repo is its own project");
    // A plain subfolder with no record of its own is looked up in the store as usual (which finds the plain project).
    fs::create_dir_all(plain.join("sub")).unwrap();
    assert_eq!(app.launch_project(&plain.join("sub")).unwrap().1.id, pp);
}

/// A record with a project location but no store pin (an FG-02a/b record, before the store).
fn without_pin(app: &App, run: &str) {
    fs::remove_file(app.data.join("control/lead").join(run).join(super::orchestrator::PROJECT_FILE)).unwrap();
}

/// The shipped build's format: cwd, session, campaign and started, nothing about projects.
fn main_format(app: &App, run: &str) {
    let old = app.data.join("control/lead").join(run);
    for f in [super::orchestrator::PROJECT_FILE, "project_loc", "project_main", "cwd_real"] {
        let _ = fs::remove_file(old.join(f));
    }
}

/// #80 review (main format), on the store id: a run record written by the shipped build must not block a new launch in that
/// folder or anywhere near it; it pins nothing, is never migrated or written to, and the new launch pins the folder.
#[test]
fn a_main_format_record_does_not_block_a_new_launch_in_its_folder_and_is_left_untouched() {
    let t = Tmp::new();
    let app = t.app();
    let (a, far) = (t.0.join("a"), t.0.join("far"));
    store_git(&a);
    store_git(&far);
    fs::create_dir_all(a.join("src")).unwrap();
    let pa = test_project(&t.0.join("data"), &t.home(), &a);
    let pf = test_project(&t.0.join("data"), &t.home(), &far);
    app.record_run("20261105-1230", &a, "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    main_format(&app, "20261105-1230");
    let old = app.data.join("control/lead/20261105-1230");
    let files = || { let mut v: Vec<_> = fs::read_dir(&old).unwrap().flatten().map(|e| e.file_name()).collect(); v.sort(); v };
    let before = files();
    for (folder, want) in [(&a, &pa), (&a.join("src"), &pa), (&far, &pf)] {
        assert_eq!(&app.launch_project(folder).expect("not blocked by the old record").1.id, want, "{folder:?}");
    }
    assert_eq!(before, files(), "the old record was neither migrated nor written to");
    // After the new launch is recorded, the Director-General can still be asked: the old record of the same folder is not an ambiguity.
    app.record_run("20261105-1240", &a, "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "c", true, 2).unwrap();
    assert!(app.other_project_roots("20261105-1240", &a).is_ok(), "the old record of the same folder does not refuse the ask");
}

/// #80 review (main format), on the store id: an old record for folder X pins nothing, but X is still some project's folder:
/// another project's Director-General is denied it, as it is any other project's.
#[test]
fn a_main_format_record_still_denies_its_folder_to_another_projects_lead() {
    let t = Tmp::new();
    let app = t.app();
    let (x, b) = (t.0.join("x"), t.0.join("b"));
    store_git(&b);
    fs::create_dir_all(x.join("src")).unwrap();
    test_project(&t.0.join("data"), &t.home(), &b);
    app.record_run("20261105-1230", &x, "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    main_format(&app, "20261105-1230");
    app.record_run("20261105-1240", &b, "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "d", true, 2).unwrap();
    let denied = app.other_project_roots("20261105-1240", &b).unwrap();
    assert!(denied.contains(&x), "the old record's folder stays denied to another project's Director-General: {denied:?}");
}

/// #80 codex P1b, on the store id: a record with a project location but no store pin, and one with a pre-store `k_` stand-in
/// pin, are migrated once from their guarded location, so a fresh start in a previously used folder works.
#[test]
fn an_old_record_without_a_store_pin_is_migrated_once_and_a_fresh_start_works() {
    let t = Tmp::new();
    let app = t.app();
    let a = t.0.join("a");
    store_git(&a);
    let pa = test_project(&t.0.join("data"), &t.home(), &a);
    app.record_run("20261105-1230", &a, "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    without_pin(&app, "20261105-1230");
    assert_eq!(app.pinned_project("20261105-1230"), None, "no store pin yet");
    assert_eq!(app.launch_project(&a).expect("the old record is recognised, not refused").1.id, pa);
    assert_eq!(app.pinned_project("20261105-1230").as_deref(), Some(pa.as_str()), "written back under the guard");
    assert_eq!(app.launch_project(&a).unwrap().1.id, pa, "and stable on the next call");
    // A pre-store stand-in pin that its folder still gives migrates the same way.
    let k = super::orchestrator::project_ref(&a).0;
    fs::write(app.data.join("control/lead/20261105-1230").join(super::orchestrator::PROJECT_FILE), &k).unwrap();
    assert_eq!(app.launch_project(&a).unwrap().1.id, pa);
    assert_eq!(app.pinned_project("20261105-1230").as_deref(), Some(pa.as_str()));
}

#[test]
fn an_old_record_that_conflicts_or_is_malformed_still_refuses() {
    let fresh = || {
        let t = Tmp::new();
        let app = t.app();
        let a = t.0.join("a");
        store_git(&a);
        test_project(&t.0.join("data"), &t.home(), &a);
        app.record_run("20261105-1230", &a, "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
        without_pin(&app, "20261105-1230");
        (t, app, a)
    };
    let refused = |app: &App, a: &Path| code(&app.launch_project(a).unwrap_err()).to_string();
    // Conflict: an old record, and a pinned record of the same folder under another project.
    let (t, app, a) = fresh();
    let other = test_project(&t.0.join("data"), &t.home(), &t.0.join("other"));
    app.record_run_as("20261105-1231", &a, Some(&other), "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "c", true, 1).unwrap();
    assert_eq!(refused(&app, &a), "project_membership_inconsistent");
    // Malformed: a cwd record with no session and no pin is not the base format.
    let (_t, app, a) = fresh();
    fs::remove_file(app.data.join("control/lead/20261105-1230/session")).unwrap();
    assert_eq!(refused(&app, &a), "project_membership_inconsistent");
    // A pin file that is neither a store id nor a stand-in, and a stand-in the folder does not give.
    let pin = |app: &App, v: &str| fs::write(app.data.join("control/lead/20261105-1230").join(super::orchestrator::PROJECT_FILE), v).unwrap();
    let (_t, app, a) = fresh();
    pin(&app, "garbage");
    assert_eq!(refused(&app, &a), "project_membership_inconsistent");
    let (_t, app, a) = fresh();
    pin(&app, "k_0123456789abcdef0123456789abcdef");
    assert_eq!(refused(&app, &a), "project_membership_inconsistent");
    assert_eq!(app.pinned_project("20261105-1230").as_deref(), Some("k_0123456789abcdef0123456789abcdef"), "nothing was overwritten");
}

/// #80 codex P1 (round 2), on the store id: the first run was in a SUBFOLDER of a worktree. Its location is the worktree's
/// `.git`, so a later start in the worktree's root or in a sibling folder keeps that pin too, even after a worker re-linked
/// the worktree to another project.
#[test]
fn a_run_first_pinned_in_a_subfolder_keeps_the_whole_project_after_a_git_rewrite() {
    let t = Tmp::new();
    let app = t.app();
    let (a, b, wt) = (t.0.join("a"), t.0.join("b"), t.0.join("a-wt"));
    store_git(&a);
    store_git(&b);
    store_worktree(&a, &wt, "a-wt");
    for d in [wt.join("src"), wt.join("tests")] {
        fs::create_dir_all(d).unwrap();
    }
    let (pa, pb) = (test_project(&t.0.join("data"), &t.home(), &a), test_project(&t.0.join("data"), &t.home(), &b));
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let stream = format!("run-{sid}");
    app.sup.spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: wt.join("src"), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: wt.join("src"), nonces: Redactor::default() });
    app.record_run("20261105-1230", &wt.join("src"), sid, "c", true, 1).unwrap(); // A, first started in a-wt/src
    app.record_run("20261105-1229", &b, "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "d", true, 1).unwrap(); // B, finished
    assert_eq!(app.pinned_project("20261105-1230").as_deref(), Some(pa.as_str()));
    store_worktree(&b, &wt, "a-wt");
    assert_eq!(app.projects.find_by_path(&wt.join("tests")).unwrap().map(|p| p.id), Some(pb.clone()), "the rewrite took");
    for (n, dir) in [wt.clone(), wt.join("tests")].into_iter().enumerate() {
        let p = app.launch_project(&dir).unwrap().1;
        assert_eq!(p.id, pa, "{}: a start here keeps A", dir.display());
        let busy = app.within_caps(&p.id, 1).unwrap_err();
        assert_eq!(busy["reason"], "project_busy", "{}: A is still busy: {busy}", dir.display());
        app.record_run_as(&format!("20261105-123{}", n + 1), &dir, Some(&p.id), "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "e", true, 1).unwrap();
    }
    let denied = app.other_project_roots(&asking(&app, &pb), &b).unwrap();
    let real: Vec<PathBuf> = denied.iter().map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.clone())).collect();
    for want in [wt.clone(), wt.join("tests"), wt.join("src"), a.clone()] {
        assert!(real.contains(&fs::canonicalize(&want).unwrap()), "{want:?} not denied: {denied:?}");
    }
    app.sup.kill(&stream).unwrap();
}

/// #80 round 4, on the store id: genuinely nested repositories (their own `.git`, their own store projects) are separate:
/// neither held busy by the other, and the outer Director-General denies the inner project's folder.
#[test]
fn nested_repos_with_their_own_git_stay_separate_projects() {
    let t = Tmp::new();
    let app = t.app();
    let (outer, inner) = (t.0.join("outer"), t.0.join("outer/inner"));
    store_git(&outer);
    store_git(&inner);
    let po = test_project(&t.0.join("data"), &t.home(), &outer);
    let pi = test_project(&t.0.join("data"), &t.home(), &inner);
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let stream = format!("run-{sid}");
    app.sup.spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "5".into()], cwd: outer.clone(), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: outer.clone(), nonces: Redactor::default() });
    app.record_run("20261105-1230", &outer, sid, "c", true, 1).unwrap();
    let p = app.launch_project(&inner).unwrap().1;
    assert_eq!(p.id, pi, "the inner repo is its own project");
    assert_ne!(p.id, po);
    app.within_caps(&p.id, 1).expect("the outer run does not hold the inner project");
    app.record_run_as("20261105-1231", &inner, Some(&p.id), "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "d", true, 1).unwrap();
    assert_eq!(app.launch_project(&inner).unwrap().1.id, pi, "and it stays so on the next launch");
    assert_eq!(app.launch_project(&outer).unwrap().1.id, po);
    let denied = app.other_project_roots(&asking(&app, &po), &outer).unwrap();
    let real: Vec<PathBuf> = denied.iter().map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.clone())).collect();
    assert!(real.contains(&fs::canonicalize(&inner).unwrap()), "the outer Director-General denies the inner project: {denied:?}");
    app.sup.kill(&stream).unwrap();
}

/// #80 residual, on the store id: a NEW `.git` location found inside an already-pinned project's tree is admitted as its
/// own project only when it is a self-contained nested repo or a submodule of the enclosing project. A pointer into
/// anything else is refused and never pinned.
/// The newest recorded run pinned to store project `pid`: the run a Director-General of that project is asked through.
fn asking(app: &App, pid: &str) -> String {
    app.recorded_runs().into_iter().find(|r| app.pinned_project(r).as_deref() == Some(pid)).expect("a recorded run of the project")
}

fn pinned_a_and_b(t: &Tmp) -> (App, PathBuf, PathBuf, String, String) {
    let app = t.app();
    let (a, b) = (t.0.join("a"), t.0.join("b"));
    store_git(&a);
    store_git(&b);
    fs::create_dir_all(b.join(".git/worktrees/x")).unwrap();
    fs::create_dir_all(a.join("sub")).unwrap();
    let (pa, pb) = (test_project(&t.0.join("data"), &t.home(), &a), test_project(&t.0.join("data"), &t.home(), &b));
    app.record_run("20261105-1230", &a, "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    app.record_run("20261105-1229", &b, "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "d", true, 1).unwrap();
    (app, a, b, pa, pb)
}

#[test]
fn a_planted_git_pointer_into_another_project_under_a_pinned_tree_is_refused_and_never_pinned() {
    let t = Tmp::new();
    let (app, a, b, pa, pb) = pinned_a_and_b(&t);
    fs::write(a.join("sub/.git"), format!("gitdir: {}\n", b.join(".git/worktrees/x").display())).unwrap();
    let e = app.launch_project(&a.join("sub")).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "{e}");
    assert_eq!(app.launch_project(&a).unwrap().1.id, pa, "A's pin is unchanged");
    let denied = app.other_project_roots(&asking(&app, &pb), &b).unwrap();
    let real: Vec<PathBuf> = denied.iter().map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.clone())).collect();
    assert!(real.contains(&fs::canonicalize(&a).unwrap()), "B's Director-General still denies A: {denied:?}");
}

#[test]
fn a_symlinked_or_redirected_git_under_a_pinned_tree_is_refused() {
    use std::os::unix::fs::symlink;
    let t = Tmp::new();
    let (app, a, b, _, _) = pinned_a_and_b(&t);
    symlink(b.join(".git"), a.join("sub/.git")).unwrap();
    let e = app.launch_project(&a.join("sub")).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "symlink: {e}");
    // A real directory whose commondir redirects elsewhere is not self-contained.
    fs::create_dir_all(a.join("cd/.git")).unwrap();
    fs::write(a.join("cd/.git/commondir"), format!("{}\n", b.join(".git").display())).unwrap();
    let e = app.launch_project(&a.join("cd")).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "commondir: {e}");
    // A gitdir file whose target does not resolve.
    fs::create_dir_all(a.join("gone")).unwrap();
    fs::write(a.join("gone/.git"), "gitdir: /no/such/place\n").unwrap();
    let e = app.launch_project(&a.join("gone")).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "unresolvable: {e}");
    // The enclosing Git project's `.git` removed: its subfolder is now a location with no `.git`, which never escapes.
    fs::create_dir_all(a.join("plain")).unwrap();
    fs::rename(a.join(".git"), t.0.join("a-git-moved")).unwrap();
    let e = app.launch_project(&a.join("plain")).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "removed .git: {e}");
}

#[test]
fn a_self_contained_nested_clone_and_a_real_submodule_are_their_own_projects() {
    let t = Tmp::new();
    let (app, a, _, pa, _) = pinned_a_and_b(&t);
    store_git(&a.join("clone"));
    let pc = test_project(&t.0.join("data"), &t.home(), &a.join("clone"));
    let p = app.launch_project(&a.join("clone")).expect("a self-contained nested repo").1;
    assert_eq!(p.id, pc);
    assert_ne!(p.id, pa);
    // A submodule: its gitdir file points into the enclosing project's own common dir.
    fs::create_dir_all(a.join(".git/modules/sub/objects")).unwrap();
    fs::write(a.join(".git/modules/sub/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(a.join("sub/.git"), "gitdir: ../.git/modules/sub\n").unwrap();
    let ps = test_project(&t.0.join("data"), &t.home(), &a.join("sub"));
    let p = app.launch_project(&a.join("sub")).expect("a submodule of the enclosing project").1;
    assert_eq!(p.id, ps);
    assert_ne!(p.id, pa, "the submodule is its own project");
}

/// #80 round 5: the submodule target passes the "inside A's common dir" check, but a `commondir` file there would redirect
/// the key to another project. It is refused anywhere on the resolution path, and the pinned key is the validated target.
#[test]
fn an_internal_gitdir_whose_commondir_points_at_another_project_is_refused() {
    let t = Tmp::new();
    let (app, a, b, pa, pb) = pinned_a_and_b(&t);
    fs::create_dir_all(a.join(".git/modules/fake")).unwrap();
    fs::write(a.join(".git/modules/fake/commondir"), format!("{}\n", b.join(".git").display())).unwrap();
    fs::write(a.join("sub/.git"), "gitdir: ../.git/modules/fake\n").unwrap();
    let e = app.launch_project(&a.join("sub")).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "{e}");
    assert_eq!(app.launch_project(&a).unwrap().1.id, pa, "A's pin is unchanged");
    let denied = app.other_project_roots(&asking(&app, &pb), &b).unwrap();
    let real: Vec<PathBuf> = denied.iter().map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.clone())).collect();
    assert!(real.contains(&fs::canonicalize(&a).unwrap()), "B's deny roots are intact: {denied:?}");
    // A commondir further up the resolution path (on the modules dir itself) is refused too.
    fs::create_dir_all(a.join(".git/modules/deep/inner")).unwrap();
    fs::write(a.join(".git/modules/deep/commondir"), format!("{}\n", b.join(".git").display())).unwrap();
    fs::create_dir_all(a.join("sub2")).unwrap();
    fs::write(a.join("sub2/.git"), "gitdir: ../.git/modules/deep/inner\n").unwrap();
    let e = app.launch_project(&a.join("sub2")).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "path: {e}");
}

#[test]
fn a_submodules_pinned_key_is_the_validated_target() {
    let t = Tmp::new();
    let (app, a, _, pa, _) = pinned_a_and_b(&t);
    fs::create_dir_all(a.join(".git/modules/sub")).unwrap();
    fs::write(a.join("sub/.git"), "gitdir: ../.git/modules/sub\n").unwrap();
    // An admitted submodule that is not in the store is not a project: never the enclosing project by an ancestor walk.
    let e = app.launch_project(&a.join("sub")).unwrap_err();
    assert_eq!(code(&e), "not_a_project", "{e}");
    let ps = test_project(&t.0.join("data"), &t.home(), &a.join("sub"));
    let (loc, p) = app.launch_project(&a.join("sub")).unwrap();
    assert_eq!((loc, p.id.clone()), (fs::canonicalize(a.join("sub")).unwrap(), ps));
    assert_ne!(p.id, pa);
}

/// #80 round 5 (second finding): the location and key checked at launch are the pair that is recorded, and the record is
/// written before the process starts, so a `.git` created later cannot change what was pinned.
#[test]
fn the_checked_location_is_what_is_recorded_even_if_a_git_appears_afterwards() {
    let t = Tmp::new();
    let app = t.app();
    let (a, b) = (t.0.join("a"), t.0.join("b"));
    store_git(&a);
    store_git(&b);
    fs::create_dir_all(a.join("src")).unwrap();
    let pa = test_project(&t.0.join("data"), &t.home(), &a);
    test_project(&t.0.join("data"), &t.home(), &b);
    let (loc, p) = app.launch_project(&a.join("src")).unwrap();
    assert_eq!((loc.clone(), p.id.clone()), (fs::canonicalize(&a).unwrap(), pa.clone()));
    // After the identity was chosen, the launched process plants a nearer .git.
    fs::create_dir_all(a.join("src/.git")).unwrap();
    app.record_run_pinned("20261105-1230", &a.join("src"), &loc, Some(&p.id), "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    let rec = fs::read_to_string(app.data.join("control/lead/20261105-1230/project_loc")).unwrap();
    assert_eq!(Path::new(rec.trim()), loc, "the checked location is what was recorded");
    fs::remove_dir_all(a.join(".git")).unwrap();
    fs::write(a.join(".git"), format!("gitdir: {}\n", b.join(".git").display())).unwrap();
    assert_eq!(app.launch_project(&a).unwrap().1.id, pa, "a start at A still gets the project that was checked");
}

/// vajra-13: a run in a plain (non-Git) folder such as HOME must not block starts in a plain subfolder under it; the
/// subfolder is its own plain location. A Git enclosing project still refuses an unadmitted nested location.
#[test]
fn a_plain_folder_pin_does_not_block_a_plain_subfolder_but_a_git_pin_still_refuses_one() {
    let t = Tmp::new();
    let app = t.app();
    // A plain folder like HOME (the fixture's HOME itself is refused by the store, so a stand-in).
    let home = t.0.join("plain-home");
    fs::create_dir_all(home.join("notes")).unwrap();
    let ph = test_project(&t.0.join("data"), &t.home(), &home);
    app.record_run("20261105-1230", &home, "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    let (loc, p) = app.launch_project(&home.join("notes")).expect("a plain subfolder of a plain pinned folder is allowed");
    assert_eq!(loc, fs::canonicalize(home.join("notes")).unwrap(), "it is its own plain location");
    assert_eq!(p.id, ph, "looked up in the store as usual");
    // A worktree of another repo under the plain folder is not constrained by its pin either.
    let other = t.0.join("other");
    store_git(&other);
    let po = test_project(&t.0.join("data"), &t.home(), &other);
    store_worktree(&other, &home.join("wt"), "wt");
    assert_eq!(app.launch_project(&home.join("wt")).unwrap().1.id, po);
    // A Git enclosing project still refuses a nested location it does not admit.
    let t2 = Tmp::new();
    let (app2, a, _, _, _) = pinned_a_and_b(&t2);
    fs::write(a.join("sub/.git"), "gitdir: /no/such/place\n").unwrap();
    assert_eq!(code(&app2.launch_project(&a.join("sub")).unwrap_err()), "project_membership_inconsistent");
}

/// #82 review (P1): an external worktree reached through a symlink inside the main checkout is a run folder whose canonical path
/// is NOT covered by the checkout. It keeps its secret-file deny rules (supplied and canonical forms both reach the generator).
#[test]
fn an_external_worktree_reached_through_a_symlink_keeps_its_secret_file_rules() {
    use std::os::unix::fs::symlink;
    let t = Tmp::new();
    let app = t.app();
    let (repo, wt) = (t.0.join("repo"), t.0.join("repo-wt"));
    store_git(&repo);
    store_worktree(&repo, &wt, "wt");
    let pr = test_project(&t.0.join("data"), &t.home(), &repo);
    let link = repo.join("wt-link");
    symlink(&wt, &link).unwrap();
    app.record_run_pinned("20261105-1230", &repo, &repo, Some(&pr), "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    app.record_run_pinned("20261105-1231", &link, &wt, Some(&pr), "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "d", true, 1).unwrap();
    let folders = app.own_project_folders(&asking(&app, &pr), &repo).unwrap();
    let rules = crate::control::lead_settings::deny_rules_for(&t.home(), &folders, &t.0.join("data"), &[]).unwrap();
    let real_wt = fs::canonicalize(&wt).unwrap();
    let want = format!("{}/**/.env)", real_wt.to_string_lossy().trim_start_matches('/'));
    assert!(rules.iter().any(|r| r.ends_with(&want)), "the external worktree's .env stays denied: {want} in {rules:?}");
}

/// #82 review (P1): a worktree run launched in a SUBFOLDER (`/repo-wt/src`) guards the whole checkout root, so a root-level or sibling
/// `.env` is denied too, not only the run's own subfolder.
#[test]
fn a_worktree_run_started_in_a_subfolder_guards_the_whole_checkout_root() {
    let t = Tmp::new();
    let app = t.app();
    let (repo, wt) = (t.0.join("repo"), t.0.join("repo-wt"));
    store_git(&repo);
    store_worktree(&repo, &wt, "wt");
    fs::create_dir_all(wt.join("src")).unwrap();
    fs::create_dir_all(wt.join("docs")).unwrap();
    let pr = test_project(&t.0.join("data"), &t.home(), &repo);
    let (real_repo, real_wt) = (fs::canonicalize(&repo).unwrap(), fs::canonicalize(&wt).unwrap());
    app.record_run_pinned("20261105-1230", &repo, &real_repo, Some(&pr), "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    app.record_run_pinned("20261105-1231", &wt.join("src"), &real_wt, Some(&pr), "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "d", true, 1).unwrap();
    let folders = app.own_project_folders("20261105-1230", &repo).unwrap();
    assert!(folders.contains(&real_wt) || folders.contains(&wt), "the checkout root is guarded: {folders:?}");
    let rules = crate::control::lead_settings::deny_rules_for(&t.home(), &folders, &t.0.join("data"), &[]).unwrap();
    let want = format!("{}/**/.env)", real_wt.to_string_lossy().trim_start_matches('/'));
    assert!(rules.iter().any(|r| r.ends_with(&want)), "root-level and sibling secrets of the worktree are denied: {want} in {rules:?}");
}

/// #82 review (P2): a finished run no longer makes the Director-General "running"; the normal reaper is what ends it.
#[test]
fn a_finished_run_stops_the_director_general_reading_running_after_the_normal_reaper() {
    let t = Tmp::new();
    let app = t.app();
    let p = test_project(&t.0.join("data"), &t.home(), &t.0.join("repo"));
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let stream = format!("run-{sid}");
    app.sup.spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "30".into()], cwd: t.0.join("repo"), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: t.0.join("repo"), nonces: Redactor::default() });
    app.record_run("20261105-1230", &t.0.join("repo"), sid, "c", true, 1).unwrap();
    assert!(app.running_in_project(&p));
    app.sup.kill(&stream).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while app.sup.is_running(&stream).unwrap_or(false) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    app.reap_runs();
    assert!(lock(&app.runs).contains_key(&stream), "the entry stays for the crew view");
    assert!(!app.running_in_project(&p), "but the project is no longer running");
}

/// #82 review (P2), on the store id: a submodule first launched from a subfolder and then from its root pins the same
/// project, whose root is the checkout root the store recorded, never the subfolder.
#[test]
fn a_submodule_first_launched_from_a_subfolder_pins_its_checkout_root() {
    let t = Tmp::new();
    let app = t.app();
    let (parent, child) = (t.0.join("parent"), t.0.join("parent/child"));
    fs::create_dir_all(parent.join(".git/modules/child")).unwrap();
    fs::create_dir_all(child.join("src")).unwrap();
    let pc = test_project(&t.0.join("data"), &t.home(), &child);
    let loc = fs::canonicalize(&child).unwrap();
    app.record_run_pinned("20261105-1230", &child.join("src"), &loc, Some(&pc), "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    app.record_run_pinned("20261105-1231", &child, &loc, Some(&pc), "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "d", true, 1).unwrap();
    for run in ["20261105-1230", "20261105-1231"] {
        assert_eq!(app.pinned_project(run).as_deref(), Some(pc.as_str()), "{run}");
    }
    assert_eq!(fs::canonicalize(app.projects.get(&pc).unwrap().root).unwrap(), loc, "the root is the checkout, not the subfolder");
}

/// #80 round 6: a pinned common dir that was replaced by a symlink is no longer the project it was pinned as; it cannot admit a
/// new location, so a planted `gitdir:` into another project's common dir is refused.
#[test]
fn a_redirected_pinned_common_dir_cannot_admit_a_new_location() {
    use std::os::unix::fs::symlink;
    let t = Tmp::new();
    let (app, a, b, _, pb) = pinned_a_and_b(&t);
    fs::remove_dir_all(a.join(".git")).unwrap();
    symlink(b.join(".git"), a.join(".git")).unwrap();
    fs::write(a.join("sub/.git"), format!("gitdir: {}\n", b.join(".git").display())).unwrap();
    // Re-canonicalized, A's stored common dir would now be B's, and the planted pointer would land the folder in B.
    let e = app.launch_project(&a.join("sub")).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "{e}");
    let denied = app.other_project_roots(&asking(&app, &pb), &b).unwrap();
    let real: Vec<PathBuf> = denied.iter().map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.clone())).collect();
    assert!(denied.contains(&a) || real.contains(&fs::canonicalize(&t.0).unwrap().join("a")), "B's deny roots still cover A: {denied:?}");
}

/// #80 round 6 (P2): a legitimate linked worktree nested under an already-pinned repo (`.claude/worktrees/w`, whose git dir has
/// the normal `commondir` of `../..`) is admitted and pinned to the enclosing project, through the real launch path.
#[test]
fn a_linked_worktree_nested_under_its_pinned_repo_is_admitted_with_the_repos_key() {
    let t = Tmp::new();
    let (app, a, b, pa, _) = pinned_a_and_b(&t);
    let wt = a.join(".claude/worktrees/w");
    store_worktree(&a, &wt, "w");
    let (loc, p) = app.launch_project(&wt).expect("a worktree of the enclosing project");
    assert_eq!(p.id, pa, "it is the enclosing project");
    assert_eq!(loc, fs::canonicalize(&wt).unwrap());
    // A worktree whose commondir redirects to another project is still refused.
    let bad = a.join(".claude/worktrees/bad");
    store_worktree(&a, &bad, "bad");
    fs::write(a.join(".git/worktrees/bad/commondir"), format!("{}\n", b.join(".git").display())).unwrap();
    let e = app.launch_project(&bad).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "{e}");
}

/// #80 review (P2): a linked worktree of a SUBMODULE, started outside the submodule checkout but inside the pinned superproject, is its
/// own project keyed by the submodule's common dir. A `commondir` that leaves the enclosing common dir, lands on another module that
/// does not hold the worktree's git dir, or names a module that itself redirects, is still refused.
#[test]
fn a_linked_worktree_of_a_submodule_inside_the_pinned_superproject_is_admitted_under_the_submodules_key() {
    let t = Tmp::new();
    let (app, a, b, pa, _) = pinned_a_and_b(&t);
    let module = a.join(".git/modules/sub");
    fs::create_dir_all(module.join("worktrees/w")).unwrap();
    fs::write(module.join("worktrees/w/commondir"), "../..\n").unwrap();
    let wt = a.join("sub-wt");
    fs::create_dir_all(&wt).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", module.join("worktrees/w").display())).unwrap();
    // The submodule's worktree is its own store project (registered at its folder), never the superproject.
    let pw = test_project(&t.0.join("data"), &t.home(), &wt);
    let (loc, p) = app.launch_project(&wt).expect("a worktree of a submodule of the enclosing project");
    assert_eq!(p.id, pw, "it is the submodule's project, not the superproject's");
    assert_ne!(p.id, pa);
    assert_eq!(loc, fs::canonicalize(&wt).unwrap());
    // Redirects are refused: to another project, to a sibling module that does not hold the worktree's git dir, and to a module whose own
    // `commondir` redirects.
    fs::create_dir_all(a.join(".git/modules/other")).unwrap();
    fs::create_dir_all(a.join(".git/modules/hop")).unwrap();
    fs::write(a.join(".git/modules/hop/commondir"), format!("{}\n", b.join(".git").display())).unwrap();
    for (n, redirect) in [b.join(".git"), a.join(".git/modules/other"), a.join(".git/modules/hop")].into_iter().enumerate() {
        let gd = module.join(format!("worktrees/r{n}"));
        fs::create_dir_all(&gd).unwrap();
        fs::write(gd.join("commondir"), format!("{}\n", redirect.display())).unwrap();
        let bad = a.join(format!("bad-wt{n}"));
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join(".git"), format!("gitdir: {}\n", gd.display())).unwrap();
        let e = app.launch_project(&bad).unwrap_err();
        assert_eq!(code(&e), "project_membership_inconsistent", "{redirect:?}: {e}");
    }
}

/// #80 round 6 (P1): another project's Lead denies the other run's pinned location too, so a submodule launched from a subfolder
/// has its checkout root denied, not only the subfolder.
#[test]
fn another_leads_deny_roots_include_a_submodules_pinned_location_not_just_the_run_subfolder() {
    let t = Tmp::new();
    let app = t.app();
    let (sup, sub, b) = (t.0.join("super"), t.0.join("super/sub"), t.0.join("b"));
    fs::create_dir_all(sup.join(".git/modules/sub")).unwrap();
    fs::create_dir_all(sub.join("src")).unwrap();
    store_git(&b);
    let pb = test_project(&t.0.join("data"), &t.home(), &b);
    let loc = fs::canonicalize(&sub).unwrap();
    // A run of a project the store does not hold (no pin), launched from the submodule's subfolder.
    app.record_run_pinned("20261105-1230", &sub.join("src"), &loc, None, "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    app.record_run("20261105-1229", &b, "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "d", true, 1).unwrap();
    let denied = app.other_project_roots(&asking(&app, &pb), &b).unwrap();
    assert!(denied.contains(&loc), "the submodule's checkout root is denied to B's Director-General: {denied:?}");
}

/// #80 round 7: resuming a session that is already running, with another project's folder, is refused before any control state
/// changes: its nonce, control session and process stay exactly as they were.
#[test]
fn resuming_a_live_session_into_another_project_is_refused_before_touching_its_control_state() {
    let t = Tmp::new();
    let app = t.app();
    let (a, b) = (t.0.join("a"), t.0.join("b"));
    fs::create_dir_all(a.join(".git")).unwrap();
    fs::create_dir_all(b.join(".git")).unwrap();
    test_project(&t.0.join("data"), &t.home(), &b);
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let stream = format!("run-{sid}");
    app.sup.spawn(&stream, SpawnSpec { argv: vec!["sleep".into(), "30".into()], cwd: a.clone(), rows: 24, cols: 80, redact: Redactor::default() }).unwrap();
    lock(&app.runs).insert(stream.clone(), Run { session_id: sid.into(), run_id: "20261105-1230".into(), campaign: "c".into(), cwd: a.clone(), nonces: Redactor::default() });
    app.record_run("20261105-1230", &a, sid, "c", true, 1).unwrap();
    lock(&app.control).start_session(sid, "20261105-1230").unwrap();
    let nonce = lock(&app.control).nonce(sid).map(str::to_string);
    assert!(nonce.is_some());
    let e = app.launch_engine(Engine::Claude, sid, "20261105-1231", "c", &b, |_| Ok(vec!["sleep".into(), "30".into()]), true, caps::DEFAULT_RUN_WORKERS).unwrap_err();
    assert_eq!(code(&e), "session_running", "{e}");
    assert!(lock(&app.control).has_session(sid), "the control session is intact");
    assert_eq!(lock(&app.control).nonce(sid).map(str::to_string), nonce, "the nonce was not rotated");
    assert!(app.sup.is_running(&stream).unwrap(), "the original process still runs");
    app.sup.kill(&stream).unwrap();
}

/// #80 round 7 (P1): a submodule whose first app run is from an OUTSIDE linked worktree still has its main checkout pinned and
/// denied to other Leads; when that checkout cannot be resolved the asking Lead is refused rather than left unprotected.
#[test]
fn a_submodule_run_from_an_outside_worktree_still_denies_the_main_checkout_to_other_leads() {
    let t = Tmp::new();
    let app = t.app();
    let (sup, sub, wt, b) = (t.0.join("super"), t.0.join("super/sub"), t.0.join("sub-wt"), t.0.join("b"));
    let git = sup.join(".git/modules/sub");
    fs::create_dir_all(git.join("worktrees/wt")).unwrap();
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir_all(&wt).unwrap();
    store_git(&b);
    fs::write(sub.join(".git"), "gitdir: ../.git/modules/sub\n").unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", git.join("worktrees/wt").display())).unwrap();
    fs::write(git.join("worktrees/wt/commondir"), "../..\n").unwrap();
    let ps = test_project(&t.0.join("data"), &t.home(), &sub);
    test_project(&t.0.join("data"), &t.home(), &b);
    let loc = fs::canonicalize(&wt).unwrap();
    // The submodule's first run is from an outside worktree; its main checkout is pinned at launch from the store.
    app.record_run_pinned("20261105-1230", &wt, &loc, Some(&ps), "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    app.record_run("20261105-1229", &b, "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "d", true, 1).unwrap();
    // The project then leaves the store: its checkout is still denied, from the launch-time record alone.
    let rec = t.0.join("data/projects").join(&ps);
    if rec.is_dir() { fs::remove_dir_all(&rec).unwrap() } else { fs::remove_file(&rec).unwrap() }
    assert!(app.projects.get(&ps).is_err(), "gone from the store");
    let denied = app.other_project_roots("20261105-1229", &b).unwrap();
    let real: Vec<PathBuf> = denied.iter().map(|d| fs::canonicalize(d).unwrap_or_else(|_| d.clone())).collect();
    assert!(real.contains(&fs::canonicalize(&sub).unwrap()), "the submodule's main checkout is denied: {denied:?}");
    // A later run whose main checkout could not be resolved at launch (its pin unreadable in the store): the earlier run of the
    // same project knew the checkout, so that knowledge repairs this one; with none left, the asking Director-General is refused.
    app.record_run_pinned("20261105-1232", &wt, &loc, Some(&ps), "2c8fad5b-d9cb-469f-a165-70867728950e", "e", true, 1).unwrap();
    assert_eq!(fs::read_to_string(app.data.join("control/lead/20261105-1232/project_main")).unwrap(), "-");
    assert!(app.other_project_roots("20261105-1229", &b).is_ok(), "any guarded record of the project resolves it");
    fs::write(app.data.join("control/lead/20261105-1230/project_main"), "-").unwrap();
    assert!(app.other_project_roots("20261105-1229", &b).is_err(), "an unprotectable checkout refuses the Director-General");
}

/// #82 review: a run whose `project_loc` record EXISTS but cannot be read leaves its checkout root unknown, so the ask is refused rather
/// than guarding only the run's subfolder. A record with no location at all (the shipped format) is not that case.
#[test]
fn an_unreadable_project_loc_of_an_own_run_refuses_instead_of_leaving_the_root_uncovered() {
    let t = Tmp::new();
    let app = t.app();
    let (repo, wt) = (t.0.join("repo"), t.0.join("repo-wt"));
    store_git(&repo);
    store_worktree(&repo, &wt, "wt");
    fs::create_dir_all(wt.join("src")).unwrap();
    let pr = test_project(&t.0.join("data"), &t.home(), &repo);
    let (real_repo, real_wt) = (fs::canonicalize(&repo).unwrap(), fs::canonicalize(&wt).unwrap());
    app.record_run_pinned("20261105-1230", &repo, &real_repo, Some(&pr), "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    app.record_run_pinned("20261105-1231", &wt.join("src"), &real_wt, Some(&pr), "1b4e28ba-2fa1-41d2-883f-0016d3cca427", "d", true, 1).unwrap();
    assert!(app.own_project_folders("20261105-1230", &repo).is_ok());
    let loc = app.data.join("control/lead/20261105-1231/project_loc");
    fs::write(&loc, "not-an-absolute-path").unwrap();
    let e = app.own_project_folders("20261105-1230", &repo).unwrap_err();
    assert!(format!("{e:?}").contains("GuardMissing"), "{e:?}");
    // No location file at all: the shipped format pins nothing, and the run's own folder is still guarded.
    fs::remove_file(&loc).unwrap();
    assert!(app.own_project_folders("20261105-1230", &repo).is_ok());
}

/// #82 review (P1): the same fixture seen from the OWN project: the Director-General's root is the outside worktree, and the recorded
/// main checkout (`super/sub`, which no run was launched in) gets the secret-file rules too.
#[test]
fn a_submodule_run_from_an_outside_worktree_guards_its_own_main_checkouts_secrets() {
    let t = Tmp::new();
    let app = t.app();
    let (sup, sub, wt) = (t.0.join("super"), t.0.join("super/sub"), t.0.join("sub-wt"));
    let git = sup.join(".git/modules/sub");
    fs::create_dir_all(git.join("worktrees/wt")).unwrap();
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir_all(&wt).unwrap();
    fs::write(git.join("config"), "[core]\n\tworktree = ../../../sub\n").unwrap();
    fs::write(sub.join(".git"), "gitdir: ../.git/modules/sub\n").unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", git.join("worktrees/wt").display())).unwrap();
    fs::write(git.join("worktrees/wt/commondir"), "../..\n").unwrap();
    let ps = test_project(&t.0.join("data"), &t.home(), &sub);
    let loc = fs::canonicalize(&wt).unwrap();
    app.record_run_pinned("20261105-1230", &wt, &loc, Some(&ps), "0f8fad5b-d9cb-469f-a165-70867728950e", "c", true, 1).unwrap();
    let folders = app.own_project_folders("20261105-1230", &wt).unwrap();
    let real_sub = fs::canonicalize(&sub).unwrap();
    assert!(folders.contains(&real_sub), "the recorded main checkout is guarded: {folders:?}");
    let rules = crate::control::lead_settings::deny_rules_for(&t.home(), &folders, &t.0.join("data"), &[]).unwrap();
    let want = format!("{}/**/.env)", real_sub.to_string_lossy().trim_start_matches('/'));
    assert!(rules.iter().any(|r| r.ends_with(&want)), "the main checkout's .env is denied: {want} in {rules:?}");
}

/// #80 sweep, class (a): the guarded records are written before any control state changes, so a record that cannot be written
/// refuses the launch with the session's nonce and control session exactly as they were.
#[test]
fn a_record_that_cannot_be_written_refuses_a_resume_before_the_nonce_rotates() {
    let t = Tmp::new();
    let app = t.app();
    let a = t.0.join("a");
    fs::create_dir_all(a.join(".git")).unwrap();
    test_project(&t.0.join("data"), &t.home(), &a);
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    lock(&app.control).start_session(sid, "20261105-1230").unwrap();
    let nonce = lock(&app.control).nonce(sid).map(str::to_string);
    app.record_run("20261105-1230", &a, sid, "c", true, 1).unwrap(); // makes control/lead
    fs::write(app.data.join("control/lead/20261105-1231"), b"in the way").unwrap(); // the next run's record dir cannot be created
    let e = app.launch_engine(Engine::Claude, sid, "20261105-1231", "c", &a, |_| Ok(vec!["sleep".into(), "30".into()]), true, caps::DEFAULT_RUN_WORKERS).unwrap_err();
    assert_eq!(code(&e), "io", "{e}");
    assert!(lock(&app.control).has_session(sid), "the control session is intact");
    assert_eq!(lock(&app.control).nonce(sid).map(str::to_string), nonce, "the nonce was not rotated");
}

/// #80 sweep, class (b): a resume runs in the guarded record's folder. The folder the caller names comes from the worker-writable
/// index, so a different one is refused before anything starts.
#[test]
fn a_resume_into_a_different_folder_than_the_guarded_record_is_refused() {
    let t = Tmp::new();
    let app = t.app();
    let (a, b) = (t.0.join("a"), t.0.join("b"));
    fs::create_dir_all(a.join(".git")).unwrap();
    fs::create_dir_all(b.join(".git")).unwrap();
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    app.record_run("20261105-1230", &a, sid, "c", true, 1).unwrap();
    // The index (worker-writable) is made to say this session lives in b.
    let line = runindex::Entry { campaign: "c".into(), session_id: sid.into(), run_id: "20261105-1230".into(), cwd: b.clone(), lead_session_id: None, provider: None };
    runindex::append(&app.data.join("runs.jsonl"), &line).unwrap();
    let e = app.resume_run(sid, "c", &b.to_string_lossy(), None).unwrap_err();
    assert_eq!(code(&e), "resume_cwd_mismatch", "{e}");
    // The index line can name any run id: a forged, unused one must not slip past the guarded membership.
    let forged = runindex::Entry { campaign: "c".into(), session_id: sid.into(), run_id: "20261105-1299".into(), cwd: b.clone(), lead_session_id: None, provider: None };
    runindex::append(&app.data.join("runs.jsonl"), &forged).unwrap();
    let e = app.resume_run(sid, "c", &b.to_string_lossy(), None).unwrap_err();
    assert_eq!(code(&e), "resume_cwd_mismatch", "forged run id: {e}");
    assert!(!lock(&app.control).has_session(sid), "and no control state was created");
}

/// #80 review (P1): a resume compares against the folder pinned at LAUNCH (`cwd_real`), never a re-resolution of the recorded path.
/// A session recorded in a/src, whose folder is then renamed and replaced by a symlink to another project b, must not resume into b
/// (by b's own path, or by the old path that now leads there); no record or control state is written.
#[test]
fn a_resume_after_the_recorded_folder_is_replaced_by_a_symlink_to_another_project_is_refused() {
    let t = Tmp::new();
    let app = t.app();
    let (a, b) = (t.0.join("a"), t.0.join("b"));
    fs::create_dir_all(a.join(".git")).unwrap();
    fs::create_dir_all(a.join("src")).unwrap();
    fs::create_dir_all(b.join(".git")).unwrap();
    let sid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    app.record_run("20261105-1230", &a.join("src"), sid, "c", true, 1).unwrap();
    fs::rename(a.join("src"), a.join("src.old")).unwrap();
    std::os::unix::fs::symlink(&b, a.join("src")).unwrap();
    let line = runindex::Entry { campaign: "c".into(), session_id: sid.into(), run_id: "20261105-1230".into(), cwd: b.clone(), lead_session_id: None, provider: None };
    runindex::append(&app.data.join("runs.jsonl"), &line).unwrap();
    let records = || fs::read_dir(app.data.join("control/lead")).unwrap().count();
    let before = records();
    for ask in [b.clone(), a.join("src")] {
        let e = app.resume_run(sid, "c", &ask.to_string_lossy(), None).unwrap_err();
        assert_eq!(code(&e), "resume_cwd_mismatch", "{ask:?}: {e}");
    }
    assert_eq!(records(), before, "no run record was written");
    assert!(!lock(&app.control).has_session(sid), "and no control state was created");
}

/// #80 round 8 (P2): every read of repository-controlled Git metadata is bounded (64 KiB) and never follows a link or blocks on
/// a FIFO; an oversized or non-regular file is a refusal, not a truncation. Each oversized file here is otherwise valid, so an
/// unbounded read would accept it.
#[test]
fn oversized_or_non_regular_git_metadata_is_refused_not_read() {
    let t = Tmp::new();
    let (app, a, _, _, _) = pinned_a_and_b(&t);
    let pad = " ".repeat(200 * 1024);
    // An oversized nested `.git` file whose first line is a valid submodule gitdir.
    fs::create_dir_all(a.join(".git/modules/sub")).unwrap();
    fs::write(a.join("sub/.git"), format!("gitdir: ../.git/modules/sub{pad}\n")).unwrap();
    let e = app.launch_project(&a.join("sub")).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "oversized .git: {e}");
    // An oversized commondir on a linked worktree that would otherwise be admitted.
    let wt = a.join(".claude/worktrees/w");
    store_worktree(&a, &wt, "w");
    fs::write(a.join(".git/worktrees/w/commondir"), format!("../..{pad}\n")).unwrap();
    let e = app.launch_project(&wt).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "oversized commondir: {e}");
    // A FIFO as the nested `.git` and as a commondir is refused without blocking.
    let fifo = |p: &Path| {
        let c = std::ffi::CString::new(p.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    };
    fs::create_dir_all(a.join("pipe")).unwrap();
    fifo(&a.join("pipe/.git"));
    let e = app.launch_project(&a.join("pipe")).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "fifo .git: {e}");
    fs::create_dir_all(a.join(".git/modules/fifo")).unwrap();
    fifo(&a.join(".git/modules/fifo/commondir"));
    fs::create_dir_all(a.join("sub3")).unwrap();
    fs::write(a.join("sub3/.git"), "gitdir: ../.git/modules/fifo\n").unwrap();
    let e = app.launch_project(&a.join("sub3")).unwrap_err();
    assert_eq!(code(&e), "project_membership_inconsistent", "fifo commondir: {e}");
}

/// #80 sweep: a first launch at a location outside any pinned tree reads its `.git` once; metadata that exists but cannot be
/// read (oversized, a FIFO, a link) refuses, and is never read as "a separate project" that skips the one-run-per-project cap.
#[test]
fn unreadable_git_metadata_at_a_first_launch_refuses_instead_of_keying_a_separate_project() {
    let big = " ".repeat(200 * 1024);
    let fifo = |p: &Path| {
        let c = std::ffi::CString::new(p.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    };
    for case in ["big-commondir", "fifo-commondir", "big-gitfile", "fifo-gitfile", "symlinked-git"] {
        let t = Tmp::new();
        let app = t.app();
        let (a, wt) = (t.0.join("a"), t.0.join("a-wt"));
        store_git(&a);
        store_worktree(&a, &wt, "w");
        let pa = test_project(&t.0.join("data"), &t.home(), &a);
        let cd = a.join(".git/worktrees/w/commondir");
        match case {
            "big-commondir" => fs::write(&cd, format!("../..{big}\n")).unwrap(),
            "fifo-commondir" => {
                fs::remove_file(&cd).unwrap();
                fifo(&cd);
            }
            "big-gitfile" => fs::write(wt.join(".git"), format!("gitdir: {}{big}\n", a.join(".git/worktrees/w").display())).unwrap(),
            "fifo-gitfile" => {
                fs::remove_file(wt.join(".git")).unwrap();
                fifo(&wt.join(".git"));
            }
            _ => {
                fs::remove_file(wt.join(".git")).unwrap();
                std::os::unix::fs::symlink(a.join(".git"), wt.join(".git")).unwrap();
            }
        }
        // On the store id, a first launch outside any pinned tree is the store's lookup of the folder: the worktree is never
        // given a separate project of its own, whatever its unreadable metadata says.
        let r = app.launch_project(&wt);
        assert!(r.as_ref().map_or(true, |(_, p)| p.id == pa), "{case}: never a separate project: {r:?}");
    }
}

/// #80 sweep: a guarded record of the folder that exists but cannot be read (cwd, location, or the directory of runs itself)
/// refuses the launch. It is never skipped, which would pin the folder afresh and could give it another project.
#[test]
fn an_unreadable_guarded_record_refuses_a_launch_instead_of_re_pinning_the_folder() {
    use std::os::unix::fs::PermissionsExt;
    for what in ["cwd", "project_loc", "LIST"] {
        let t = Tmp::new();
        let (app, a, _, _, _) = pinned_a_and_b(&t);
        assert!(app.launch_project(&a).is_ok(), "{what}: healthy first");
        let lead = app.data.join("control/lead");
        if what == "LIST" {
            fs::set_permissions(&lead, fs::Permissions::from_mode(0o300)).unwrap();
        } else {
            for r in ["20261105-1230", "20261105-1229"] {
                let f = lead.join(r).join(what);
                let _ = fs::remove_file(&f);
                fs::create_dir(&f).unwrap();
            }
        }
        let r = app.launch_project(&a);
        let _ = fs::set_permissions(&lead, fs::Permissions::from_mode(0o700));
        assert!(r.is_err(), "{what}: refused, not re-pinned: {r:?}");
    }
}

#[path = "board_tests.rs"]
mod board_cards;

/// #80 optional: a finished run's worker count is forgotten when it is reaped, not only on the next start, so a long
/// session of runs that start and finish keeps the map bounded by the runs still going.
#[test]
fn reaping_forgets_finished_runs_worker_counts() {
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.join("repo");
    for i in 0..6u32 {
        let sid = format!("1b4e28ba-2fa1-41d2-883f-0016d3cca4{i:02}");
        app.launch(&sid, &format!("20261105-12{i:02}"), "c", &cwd, |_| Ok(vec!["true".into()]), false).unwrap();
        app.sup.wait(&format!("run-{sid}"), Duration::from_secs(5)).unwrap();
        app.reap_runs();
        assert!(lock(&app.run_workers).is_empty(), "after run {i}: {:?}", lock(&app.run_workers));
    }
}

/// #80 optional: a bad maxInFlight is `invalid_args` even next to a running run (never `workers_full`), from
/// `start_run` and at the caps door itself.
#[test]
fn a_bad_max_in_flight_is_invalid_args_before_any_cap() {
    let t = Tmp::new();
    let app = t.app();
    let cwd = t.0.join("repo");
    let sid = "1b4e28ba-2fa1-41d2-883f-0016d3cca499";
    app.launch_kind(sid, "20261105-1259", "c", &cwd, |_| Ok(vec!["sleep".into(), "5".into()]), false, true, Engine::Claude, 16).unwrap();
    for bad in [0u32, 17, 99] {
        let input = RunInput { ask: "x".into(), max_rounds: 1, base: "main".into(), max_in_flight: bad, campaign: None, provider: None };
        assert_eq!(code(&app.start_run(input, cwd.to_str().unwrap()).unwrap_err()), "invalid_args", "{bad}");
        let other = "1b4e28ba-2fa1-41d2-883f-0016d3cca498";
        assert_eq!(code(&app.launch_kind(other, "20261105-1258", "c", &cwd, |_| Ok(vec!["true".into()]), false, true, Engine::Claude, bad).unwrap_err()), "invalid_args", "caps door: {bad}");
    }
    let _ = app.sup.kill(&format!("run-{sid}"));
}
