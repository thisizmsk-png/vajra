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
