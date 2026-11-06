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
