use super::*;
use serde_json::json;

fn core(ticket: &str, task: &str, round: u32, status: Status, check: bool) -> CoreWorker {
    CoreWorker {
        ticket: ticket.into(), task: task.into(), round, check, status, overlays: Vec::new(),
        provider: Some(ProviderTag::new("codex", "model-a", "plan", None)),
        controls: json!({"message": {"enabled": false}}), diff: Some(json!({"added": 3})),
    }
}

/// 16.7: every core worker is a `core:<ticket>` node under its task node, with its controls, diff and provider; a
/// check ticket sits under its round's verification group.
#[test]
fn core_workers_are_core_nodes_under_their_task_with_controls_diff_and_provider() {
    let tasks = json!({"tasks": [{"id": "r1.t1", "owner_agent": "bhima", "status": "in_progress"}]});
    let cores = [core("r1.t1.r1", "t1", 1, Status::InProgress, false), core("r1.t2.r1", "t2", 1, Status::Done, false), core("r1.verify.r1", "verify", 1, Status::Done, true)];
    let signals = Signals::default();
    let root = build(&CrewInput { run_id: "r1", lead: None, run_status: Status::InProgress, tasks: Some(&tasks), workers: &[], core: &cores, signals: &signals });
    let run = &root.children[0];
    let t1 = run.children.iter().find(|n| n.id == "task:t1").unwrap();
    let w = &t1.children[0];
    assert_eq!((w.id.as_str(), w.kind, w.title.as_str(), w.status), ("core:r1.t1.r1", NodeKind::Round, "round 1", Status::InProgress));
    assert_eq!(w.controls.as_ref().unwrap()["message"]["enabled"], false);
    assert_eq!(w.diff.as_ref().unwrap()["added"], 3);
    assert_eq!(w.provider.as_ref().unwrap().account, "codex");
    assert!(run.children.iter().any(|n| n.id == "task:t2"), "a task seen only on a ticket still gets its node");
    let v = run.children.iter().find(|n| n.id == "verification:r1").unwrap();
    assert_eq!(v.children[0].id, "core:r1.verify.r1");
}
