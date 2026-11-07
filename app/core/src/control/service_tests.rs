use super::*;
use crate::crew::tree::{Status, Worker};
use std::fs;

const SID: &str = "62278db8-d6e8-48f7-b03e-14d8a8c989f2";

fn svc() -> (Service, PathBuf) {
    let dir = std::env::temp_dir().join(format!("vajra-svc-{}-{}", std::process::id(), nonce::random_hex(4).unwrap()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("wt")).unwrap();
    let mut s = Service::open(&dir, dir.join("audit.log"), None).unwrap();
    s.start_session(SID, "20261105-1810").unwrap();
    s.set_workers(vec![WorkerInfo {
        worker: Worker {
            agent_id: "aa870dd40540481f0".into(),
            engine_id: "bhima".into(),
            label: "task.t1.r1".into(),
            prompt_first_line: None,
            branch: None,
            status: Status::InProgress,
        },
        task_id: Some("20261105-1810.t1".into()),
        title: None,
        round: Some(1),
        live: true,
        last_activity: None,
        cwd: Some(dir.join("wt").to_string_lossy().into_owned()),
        lines: vec![],
    }]);
    (s, dir)
}

fn files(dir: &Path, sub: &str) -> usize {
    let d = dir.join("control").join(SID).join(sub);
    fs::read_dir(d).map(|r| r.filter_map(|e| e.ok()).filter(|e| !e.file_name().to_string_lossy().starts_with('.')).count()).unwrap_or(0)
}

#[test]
fn a_ledger_failure_leaves_no_queue_entry_and_no_stop_file() {
    let (mut s, dir) = svc();
    s.ledger.break_for_test();
    assert!(s.message_worker(Actor::Human, "bhima", "hello").is_err());
    assert_eq!(files(&dir, "queue"), 0, "the hook must not deliver a message the ledger never recorded");
    assert!(s.pending.is_empty());
    assert!(s.stop_worker(Actor::Human, "bhima", None).is_err());
    assert_eq!(files(&dir, "stops"), 0, "a stop reported as failed must not be enforced");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_resume_leaves_the_stop_in_force() {
    let (mut s, dir) = svc();
    s.stop_worker(Actor::Human, "bhima", None).unwrap();
    assert_eq!(files(&dir, "stops"), 1);
    s.ledger.break_for_test();
    assert!(s.resume_worker(Actor::Human, "bhima").is_err());
    assert_eq!(files(&dir, "stops"), 1, "a resume that could not be recorded must not clear the stop");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_resume_survives_a_restart() {
    let (mut s, dir) = svc();
    s.stop_worker(Actor::Human, "bhima", None).unwrap();
    s.resume_worker(Actor::Human, "bhima").unwrap();
    assert!(s.last_stop.is_empty());
    drop(s);
    let again = Service::open(&dir, dir.join("audit.log"), None).unwrap();
    assert!(again.last_stop.is_empty(), "replay must apply the resume record: {:?}", again.last_stop);
    assert!(again.pending.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_stop_without_a_resume_survives_a_restart() {
    let (mut s, dir) = svc();
    s.stop_worker(Actor::Human, "bhima", None).unwrap();
    drop(s);
    let again = Service::open(&dir, dir.join("audit.log"), None).unwrap();
    assert_eq!(again.last_stop.len(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_second_stop_never_rewrites_or_loses_the_first() {
    let (mut s, dir) = svc();
    let first = s.stop_worker(Actor::Human, "bhima", None).unwrap();
    assert_eq!(files(&dir, "stops"), 1);
    // With the ledger broken, a repeated stop must neither fail by deleting the stop nor replace it.
    s.ledger.break_for_test();
    let again = s.stop_worker(Actor::Human, "bhima", None).unwrap();
    assert_eq!(again["action_id"], first["action_id"], "the existing stop is reported, not a new one");
    assert_eq!(again["already_stopped"], true);
    assert_eq!(files(&dir, "stops"), 1, "an enforced stop must survive a failed second stop");
    let _ = fs::remove_dir_all(&dir);
}

fn ledger_states(dir: &Path) -> Vec<String> {
    Ledger::read_all(&dir.join("control/ledger.jsonl")).unwrap().iter().map(|r| str_field(r, "state").to_string()).collect()
}

#[test]
fn nothing_is_visible_to_the_hook_before_the_ledger_has_recorded_it() {
    // A hook that polls the queue and the stops dir while a message and a stop are attempted with a failing ledger
    // never sees an entry: publication happens only after a successful record.
    let (mut s, dir) = svc();
    s.ledger.break_for_test();
    let (q, st) = (dir.join("control").join(SID).join("queue"), dir.join("control").join(SID).join("stops"));
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (d2, s2) = (done.clone(), seen.clone());
    let watcher = std::thread::spawn(move || {
        while !d2.load(Ordering::SeqCst) {
            for p in [&q, &st] {
                let n = fs::read_dir(p).map(|r| r.filter_map(|e| e.ok()).filter(|e| !e.file_name().to_string_lossy().starts_with('.')).count()).unwrap_or(0);
                s2.fetch_add(n, Ordering::SeqCst);
            }
        }
    });
    for _ in 0..50 {
        assert!(s.message_worker(Actor::Human, "bhima", "hello").is_err());
        assert!(s.stop_worker(Actor::Human, "bhima", None).is_err());
    }
    done.store(true, Ordering::SeqCst);
    watcher.join().unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 0, "the hook must never see an action the ledger does not hold");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_publication_is_recorded_not_silent() {
    use std::os::unix::fs::PermissionsExt;
    let (mut s, dir) = svc();
    let q = dir.join("control").join(SID).join("queue");
    fs::set_permissions(&q, fs::Permissions::from_mode(0o500)).unwrap();
    assert!(s.message_worker(Actor::Human, "bhima", "hello").is_err());
    fs::set_permissions(&q, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(ledger_states(&dir), vec!["queued", "not_delivered"], "recorded first, then the failed publication");
    assert!(s.pending.is_empty());
    let st = dir.join("control").join(SID).join("stops");
    fs::set_permissions(&st, fs::Permissions::from_mode(0o500)).unwrap();
    assert!(s.stop_worker(Actor::Human, "bhima", None).is_err());
    fs::set_permissions(&st, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(ledger_states(&dir), vec!["queued", "not_delivered", "queued", "refused"]);
    assert_eq!(files(&dir, "stops"), 0);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn discovery_marking_a_worker_finished_expires_its_queued_messages() {
    // Review P2: completion arrives only through set_workers, which used to leave the message queued until the TTL.
    let (mut s, dir) = svc();
    let id = s.message_worker(Actor::Human, "bhima", "late note").unwrap()["action_id"].as_str().unwrap().to_string();
    assert_eq!(files(&dir, "queue"), 1);
    let mut ws = s.workers.clone();
    ws[0].live = false;
    ws[0].worker.status = Status::Done;
    s.set_workers(ws.clone());
    assert_eq!(files(&dir, "queue"), 0, "a finished worker never receives it later");
    assert!(!s.pending.contains_key(&id));
    // Repeating the same refresh is harmless.
    s.set_workers(ws);
    let _ = fs::remove_dir_all(&dir);
}

// ---- delivery state machine (Opus design, ADR note on #5) -------------------------------------------------------
// The hook claims queue/ -> inflight/, writes acks/, then removes inflight/. check() observes in the order
// queue, inflight, ack, expired: an "all absent" reading then proves the file was never published, because a hook that
// ran between the reads would have left an ack behind. Q = queue/, I = inflight/, A = acks/, E = expired/.

#[derive(Clone, Copy)]
enum Ack { None, Valid, Big, Array, Text, Symlink }

struct Case {
    name: &'static str,
    q: bool,
    i: bool,
    a: Ack,
    e: bool,
    /// Seconds after the first tick at which the case is checked (first tick is always at 0).
    at: u64,
    /// `None`: still pending. `Some((state, reason))`: settled.
    want: Option<(&'static str, Option<&'static str>)>,
}

fn last_record(s: &Service, id: &str) -> Value {
    let rec = super::super::ledger::Ledger::read_all(s.ledger.path()).unwrap();
    rec.into_iter().rev().find(|r| r["action_id"] == id).unwrap()
}

#[test]
fn every_delivery_crash_point_settles_as_the_table_says() {
    use Ack::*;
    let cases = [
        // Absence alone is not proof of non-publication: an ack and a claim removal that were never made durable look the
        // same after power loss. So it is delivery_unknown, never a terminal not_delivered.
        Case { name: "recorded, crashed before publish", q: false, i: false, a: None, e: false, at: 0, want: Some(("delivery_unknown", Some("no_evidence"))) },
        Case { name: "power loss: ack and claim removal not durable", q: false, i: false, a: None, e: false, at: 600, want: Some(("delivery_unknown", Some("no_evidence"))) },
        Case { name: "published, not yet claimed", q: true, i: false, a: None, e: false, at: 5, want: Option::None },
        Case { name: "forged ack beside a queued file", q: true, i: false, a: Valid, e: false, at: 5, want: Option::None },
        Case { name: "queued past the ttl", q: true, i: false, a: None, e: false, at: 31 * 60, want: Some(("not_delivered", Some("expired"))) },
        Case { name: "claimed, hook alive", q: false, i: true, a: None, e: false, at: 5, want: Option::None },
        Case { name: "claimed, no ack for 60s", q: false, i: true, a: None, e: false, at: 61, want: Some(("delivery_unknown", Some("claim_stale"))) },
        Case { name: "claimed and acked", q: false, i: true, a: Valid, e: false, at: 0, want: Some(("delivered", Option::None)) },
        Case { name: "acked, claim removed", q: false, i: false, a: Valid, e: false, at: 0, want: Some(("delivered", Option::None)) },
        Case { name: "acked long after the claim timer", q: false, i: false, a: Valid, e: false, at: 600, want: Some(("delivered", Option::None)) },
        Case { name: "oversized ack", q: false, i: false, a: Big, e: false, at: 0, want: Some(("delivery_unknown", Some("ack_invalid"))) },
        Case { name: "ack that is a json array", q: false, i: true, a: Array, e: false, at: 0, want: Some(("delivery_unknown", Some("ack_invalid"))) },
        Case { name: "ack that is not json", q: false, i: false, a: Text, e: false, at: 0, want: Some(("delivery_unknown", Some("ack_invalid"))) },
        Case { name: "ack planted as a symlink", q: false, i: false, a: Symlink, e: false, at: 0, want: Some(("delivery_unknown", Some("ack_invalid"))) },
        Case { name: "expired by rename, crashed before the record", q: false, i: false, a: None, e: true, at: 0, want: Some(("not_delivered", Some("expired"))) },
    ];
    for c in cases {
        let (mut s, dir) = svc();
        let id = s.message_worker(Actor::Human, "bhima", "x").unwrap()["action_id"].as_str().unwrap().to_string();
        let root = dir.join("control").join(SID);
        let qf = fs::read_dir(root.join("queue")).unwrap().next().unwrap().unwrap().path();
        let name = qf.file_name().unwrap().to_owned();
        if c.e {
            fs::rename(&qf, root.join("expired").join(&name)).unwrap();
        } else if c.i {
            fs::rename(&qf, root.join("inflight").join(format!("{id}.4242"))).unwrap();
        } else if !c.q {
            fs::remove_file(&qf).unwrap();
        }
        let ack = root.join("acks").join(format!("{id}.json"));
        fs::create_dir_all(root.join("acks")).unwrap();
        match c.a {
            None => {}
            Valid => fs::write(&ack, format!(r#"{{"action_id":"{id}","consumed_at":"2026-11-06T00:00:00Z","tool_name":"Bash"}}"#)).unwrap(),
            Big => fs::File::create(&ack).unwrap().set_len(MAX_ACK_BYTES + 1).unwrap(),
            Array => fs::write(&ack, b"[1,2]").unwrap(),
            Text => fs::write(&ack, b"nope").unwrap(),
            Symlink => std::os::unix::fs::symlink("/nonexistent/ack", &ack).unwrap(),
        }
        let t0 = SystemTime::now();
        s.tick_at(t0);
        if c.at > 0 {
            s.tick_at(t0 + Duration::from_secs(c.at));
        }
        match c.want {
            Option::None => assert!(s.pending.contains_key(&id), "{}: should still be pending", c.name),
            Some((state, reason)) => {
                assert!(!s.pending.contains_key(&id), "{}: should have settled", c.name);
                let r = last_record(&s, &id);
                assert_eq!(r["state"], state, "{}: {r}", c.name);
                if let Some(reason) = reason {
                    assert_eq!(r["reason"], reason, "{}: {r}", c.name);
                }
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn a_claimed_message_is_still_settled_after_its_session_ended() {
    let (mut s, dir) = svc();
    let id = s.message_worker(Actor::Human, "bhima", "x").unwrap()["action_id"].as_str().unwrap().to_string();
    let root = dir.join("control").join(SID);
    let qf = fs::read_dir(root.join("queue")).unwrap().next().unwrap().unwrap().path();
    fs::rename(&qf, root.join("inflight").join(format!("{id}.4242"))).unwrap();
    s.end_session();
    assert!(s.pending.contains_key(&id), "a claimed message outlives the session");
    fs::create_dir_all(root.join("acks")).unwrap();
    fs::write(root.join("acks").join(format!("{id}.json")), format!(r#"{{"action_id":"{id}"}}"#)).unwrap();
    s.tick_at(SystemTime::now());
    assert_eq!(last_record(&s, &id)["state"], "delivered");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_ack_arriving_between_two_ticks_wins_over_the_claim_timer() {
    let (mut s, dir) = svc();
    let id = s.message_worker(Actor::Human, "bhima", "x").unwrap()["action_id"].as_str().unwrap().to_string();
    let root = dir.join("control").join(SID);
    let qf = fs::read_dir(root.join("queue")).unwrap().next().unwrap().unwrap().path();
    let claim = root.join("inflight").join(format!("{id}.4242"));
    fs::rename(&qf, &claim).unwrap();
    let t0 = SystemTime::now();
    s.tick_at(t0);
    // The laptop slept: the hook acked and dropped its claim, then the app woke past the 60 s mark.
    fs::create_dir_all(root.join("acks")).unwrap();
    fs::write(root.join("acks").join(format!("{id}.json")), format!(r#"{{"action_id":"{id}"}}"#)).unwrap();
    fs::remove_file(claim).unwrap();
    s.tick_at(t0 + INFLIGHT_UNKNOWN + Duration::from_secs(30));
    assert_eq!(last_record(&s, &id)["state"], "delivered");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_fifo_at_the_ack_path_is_refused_without_blocking() {
    // Review P1: a worker (same user) can plant a FIFO; a blocking open froze the watcher while it held the mutex.
    let (mut s, dir) = svc();
    let id = s.message_worker(Actor::Human, "bhima", "x").unwrap()["action_id"].as_str().unwrap().to_string();
    let root = dir.join("control").join(SID);
    let qf = fs::read_dir(root.join("queue")).unwrap().next().unwrap().unwrap().path();
    fs::rename(&qf, root.join("inflight").join(format!("{id}.4242"))).unwrap();
    fs::create_dir_all(root.join("acks")).unwrap();
    let fifo = std::ffi::CString::new(root.join("acks").join(format!("{id}.json")).to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    let h = std::thread::spawn(move || {
        s.tick_at(SystemTime::now());
        tx.send(()).unwrap();
        s
    });
    assert!(rx.recv_timeout(Duration::from_secs(3)).is_ok(), "the watcher blocked on the FIFO");
    let s = h.join().unwrap();
    assert!(!s.pending.contains_key(&id));
    let r = last_record(&s, &id);
    assert_eq!((r["state"].as_str(), r["reason"].as_str()), (Some("delivery_unknown"), Some("ack_invalid")), "{r}");
    // The reader itself refuses it too.
    assert!(fsx::read_nofollow_capped(&root.join("acks").join(format!("{id}.json")), 1024).is_err());
    assert!(fsx::read_nofollow(&root.join("acks").join(format!("{id}.json"))).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_directory_scan_is_retried_not_settled() {
    // Review P2: find() swallowed read_dir errors, so an unreadable queue/ looked like "no queue file" and the message
    // was settled terminally while the hook could still deliver it.
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        return; // root reads any directory
    }
    let (mut s, dir) = svc();
    let id = s.message_worker(Actor::Human, "bhima", "x").unwrap()["action_id"].as_str().unwrap().to_string();
    let q = dir.join("control").join(SID).join("queue");
    fs::set_permissions(&q, fs::Permissions::from_mode(0o000)).unwrap();
    s.tick_at(SystemTime::now() + Duration::from_secs(600));
    fs::set_permissions(&q, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(s.pending.contains_key(&id), "an unreadable queue directory must not settle the message");
    assert_eq!(files(&dir, "queue"), 1, "and the file is still there for the hook");
    s.tick_at(SystemTime::now());
    assert!(s.pending.contains_key(&id), "readable again: it is simply still queued");
    let _ = fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------ ADR-13: Kind::Ask and the Lead audit line

const ASK: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const LEAD_SID: &str = "1a8fad5b-d9cb-469f-a165-70867728950f";

fn ask(s: &mut Service, state: AskState, text: Option<&str>, reason: Option<&str>) {
    s.record_ask(&AskRecord { ask_id: ASK, run_id: "20261105-1810", session_id: SID, lead_session_id: LEAD_SID, state, text, reason }).unwrap();
}

fn ledger_of(dir: &Path) -> Vec<Value> {
    Ledger::read_all(&dir.join("control/ledger.jsonl")).unwrap()
}

fn audit_of(dir: &Path) -> String {
    fs::read_to_string(dir.join("audit.log")).unwrap_or_default()
}

#[test]
fn ask_ledger_records_are_human_ui_and_carry_the_text_once() {
    let (mut s, dir) = svc();
    ask(&mut s, AskState::Queued, Some("what is bhima doing?"), None);
    ask(&mut s, AskState::Delivered, None, None);
    ask(&mut s, AskState::Done, None, None);
    let recs = ledger_of(&dir);
    let asks: Vec<&Value> = recs.iter().filter(|r| r["kind"] == "ask").collect();
    assert_eq!(asks.iter().map(|r| r["state"].as_str().unwrap()).collect::<Vec<_>>(), ["queued", "delivered", "done"]);
    for r in &asks {
        assert_eq!((r["actor"].as_str(), r["via"].as_str(), r["source"].as_str()), (Some("human"), Some("ui"), Some("human")));
        assert_eq!(r["action_id"], ASK);
        assert_eq!(r["run_id"], "20261105-1810");
    }
    assert_eq!(asks[0]["text"], "what is bhima doing?");
    assert_eq!(asks[0]["text_sha256"], sha256::hex_digest(b"what is bhima doing?"));
    assert_eq!(asks[0]["lead_session_id"], LEAD_SID);
    assert!(asks[1].get("text").is_none(), "the text is stored once");
    // A failure carries its reason.
    ask(&mut s, AskState::Failed, None, Some("cancelled"));
    let last = ledger_of(&dir).pop().unwrap();
    assert_eq!((last["state"].as_str(), last["reason"].as_str()), (Some("failed"), Some("cancelled")));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ask_adds_no_queue_entry_no_audit_line_and_no_pending_action() {
    // AC-3.1.3: an ask is not a control action. It reaches no worker, and (ADR-13 6) writes no audit.log line.
    let (mut s, dir) = svc();
    ask(&mut s, AskState::Queued, Some("hello"), None);
    ask(&mut s, AskState::Done, None, None);
    assert_eq!(files(&dir, "queue"), 0);
    assert_eq!(files(&dir, "stops"), 0);
    assert!(s.pending.is_empty() && s.last_stop.is_empty());
    assert_eq!(audit_of(&dir), "");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ask_records_survive_a_restart_without_becoming_pending() {
    let (mut s, dir) = svc();
    ask(&mut s, AskState::Queued, Some("hello"), None);
    drop(s);
    let again = Service::open(&dir, dir.join("audit.log"), None).unwrap();
    assert!(again.pending.is_empty(), "an ask is never replayed into the queue watcher");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_ledger_failure_is_an_error_not_a_silent_skip() {
    let (mut s, dir) = svc();
    s.ledger.break_for_test();
    let r = s.record_ask(&AskRecord { ask_id: ASK, run_id: "20261105-1810", session_id: SID, lead_session_id: LEAD_SID, state: AskState::Queued, text: Some("x"), reason: None });
    assert!(r.is_err(), "the caller must not spawn the Lead for an ask the ledger never recorded");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reconcile_ignores_ask() {
    let (mut s, dir) = svc();
    ask(&mut s, AskState::Queued, Some("hello"), None);
    ask(&mut s, AskState::Delivered, None, None);
    ask(&mut s, AskState::Failed, None, Some("lead_tools_mismatch"));
    let rep = crate::control::reconcile::run(&dir.join("control/ledger.jsonl"), &dir.join("audit.log")).unwrap();
    assert!(rep.is_clean(), "{rep:?}");
    let _ = fs::remove_dir_all(&dir);
}

fn guard_line(tool: &str) -> String {
    format!("{} ALLOW mcp__vajra-desktop__{tool} reason=\"x\"\n", time::iso_secs(SystemTime::now()))
}

#[test]
fn lead_mcp_audit_line_per_call() {
    let (mut s, dir) = svc();
    let calls: [(&str, Value); 5] = [
        ("list_workers", json!({})),
        ("inspect_worker", json!({"worker": "bhima"})),
        ("message_worker", json!({"worker": "bhima", "text": "hi"})),
        ("redirect_worker", json!({"worker": "bhima", "text": "go left"})),
        ("stop_worker", json!({"worker": "bhima"})),
    ];
    for (tool, args) in &calls {
        s.call_tool(tool, args).unwrap();
    }
    // A failed call is not an action and gets no line.
    assert!(s.call_tool("inspect_worker", &json!({"worker": "nobody"})).is_err());
    assert!(s.call_tool("rm_rf", &json!({})).is_err());

    let recs = ledger_of(&dir);
    let mcp_ids: Vec<&str> = recs.iter().filter(|r| r["via"] == "mcp").map(|r| r["action_id"].as_str().unwrap()).collect();
    let audit = audit_of(&dir);
    for (tool, _) in &calls {
        let n = audit.lines().filter(|l| l.contains(&format!("reason=\"mcp {tool} id="))).count();
        assert_eq!(n, 1, "one line for {tool}: {audit}");
    }
    assert_eq!(audit.lines().count(), 5, "{audit}");
    for l in audit.lines() {
        assert!(l.contains(" ALLOW desktop-control reason=\"mcp "), "{l}");
        let id = l.split("id=").nth(1).unwrap().trim_end_matches('"');
        assert!(mcp_ids.contains(&id), "{id} is a Lead action in the ledger");
    }

    // Reconcile joins by id, and with the guard's own lines present the report is clean.
    let rp = |dir: &Path| crate::control::reconcile::run(&dir.join("control/ledger.jsonl"), &dir.join("audit.log")).unwrap();
    let mut with_guard = audit.clone();
    for (tool, _) in &calls {
        with_guard.push_str(&guard_line(tool));
    }
    fs::write(dir.join("audit.log"), &with_guard).unwrap();
    assert!(rp(&dir).is_clean(), "{:?}", rp(&dir));

    // guard_absent: a Lead call whose guard line never appeared means the Lead ran unguarded.
    let without_one: String = with_guard.lines().filter(|l| !l.contains("mcp__vajra-desktop__redirect_worker")).map(|l| format!("{l}\n")).collect();
    fs::write(dir.join("audit.log"), without_one).unwrap();
    let rep = rp(&dir);
    assert_eq!(rep.ledger_without_audit.len(), 1, "{rep:?}");

    // An app line for an id the ledger never recorded is flagged.
    fs::write(dir.join("audit.log"), format!("{with_guard}{} ALLOW desktop-control reason=\"mcp stop_worker id=aDEADBEEF\"\n", time::iso_secs(SystemTime::now()))).unwrap();
    assert_eq!(rp(&dir).audit_without_ledger, vec!["aDEADBEEF".to_string()]);
    let _ = fs::remove_dir_all(&dir);
}
