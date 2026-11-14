//! 6i: the delivery over fake steps. Every step that passes is seen in order with exactly what it was given; every
//! failure ends not verified with a plain reason, runs nothing after it and still removes the evidence worktree.

use super::*;
use crate::desktop::runproto::TaskClose;
use crate::desktop::verify::{Status, VResult};
use std::sync::atomic::{AtomicUsize, Ordering};

const RUN: &str = "20261114-0900";
const SLUG: &str = "deliver-20261114-0900";
const HEAD: &str = "1111111111111111111111111111111111111111";
const BASE: &str = "2222222222222222222222222222222222222222";
const EV_SHA: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const APPROVAL_SHA: &str = "4444444444444444444444444444444444444444444444444444444444444444";

/// Where a fake step fails.
#[derive(Clone, Debug, PartialEq)]
enum At {
    Nowhere,
    Tree,
    Approve,
    RunErr,
    RunFailed,
    RunNotPassed,
    SignRefused,
    SignNoKey,
    SignUnknown,
    Check,
    /// The first write whose request debug text starts with this.
    Write(&'static str),
    Verify,
}

struct Fake {
    at: At,
    calls: Mutex<Vec<String>>,
    writes: Mutex<Vec<Request>>,
    asks: Mutex<Vec<Ask>>,
}

impl Fake {
    fn new(at: At) -> Arc<Fake> {
        Arc::new(Fake { at, calls: Mutex::new(Vec::new()), writes: Mutex::new(Vec::new()), asks: Mutex::new(Vec::new()) })
    }
    fn call(&self, c: &str) {
        crate::lock(&self.calls).push(c.into());
    }
    fn calls(&self) -> Vec<String> {
        crate::lock(&self.calls).clone()
    }
    fn writes(&self) -> Vec<Request> {
        crate::lock(&self.writes).clone()
    }
}

fn tree() -> Tree {
    Tree { worktree: "/wt/evidence-r1".into(), tmp: "/wt/tmp/evidence-r1/evidence-run".into(), git_common: "/repo/.git".into(), base: "/wt/evidence-r1/data/walkthroughs".into() }
}

/// The variant name of a request: `TasksClose`, `CampaignPass`, ...
fn kind(r: &Request) -> String {
    format!("{r:?}").split([' ', '{', '(']).next().unwrap_or_default().to_string()
}

impl Steps for Fake {
    fn tree(&self, _: &RoundInput) -> Result<Tree, String> {
        self.call("tree");
        if self.at == At::Tree { Err("it couldn't be made (worktree_add)".into()) } else { Ok(tree()) }
    }
    fn approve(&self, ask: &Ask, _: &dyn RunLife) -> Result<(), String> {
        self.call("approve");
        crate::lock(&self.asks).push(ask.clone());
        if self.at == At::Approve { Err("you declined the check commands".into()) } else { Ok(()) }
    }
    fn run(&self, ask: Ask, approved: bool, t: &Tree) -> Result<(Option<String>, RunOutcome), String> {
        assert_eq!(*t, tree());
        self.call(if approved { "run approved" } else { "run" });
        crate::lock(&self.asks).push(ask);
        let used = approved.then(|| APPROVAL_SHA.to_string());
        match self.at {
            At::RunErr => Err("the worktree isn't at the approved commit".into()),
            At::RunFailed => Ok((used, RunOutcome::Failed("the evidence pipe closed early".into()))),
            At::RunNotPassed => Ok((used, RunOutcome::Ran { passed: false, sha256: EV_SHA.into() })),
            _ => Ok((used, RunOutcome::Ran { passed: true, sha256: EV_SHA.into() })),
        }
    }
    fn sign(&self, slug: &str, sha256: &str, t: &Tree) -> SignOutcome {
        assert_eq!((slug, sha256, t), (SLUG, EV_SHA, &tree()), "sign gets exactly the sha the run recorded");
        self.call("sign");
        match self.at {
            At::SignRefused => SignOutcome::Refused("sha256 mismatch".into()),
            At::SignNoKey => SignOutcome::NoKey,
            At::SignUnknown => SignOutcome::Unknown("signing ended unexpectedly".into()),
            _ => SignOutcome::Signed,
        }
    }
    fn check(&self, evidence: &Path, head: &str, _: &Tree) -> Result<(), String> {
        assert_eq!((evidence, head), (Path::new("/wt/evidence-r1/data/walkthroughs/deliver-20261114-0900/evidence.json"), HEAD));
        self.call("check");
        if self.at == At::Check { Err("not green: git.head is not the integration commit".into()) } else { Ok(()) }
    }
    fn write(&self, req: &Request) -> Result<(), String> {
        self.call(&format!("write {}", kind(req)));
        crate::lock(&self.writes).push(req.clone());
        match &self.at {
            At::Write(k) if kind(req) == *k => Err("tasks.json is locked".into()),
            _ => Ok(()),
        }
    }
    fn verify(&self) -> Result<(), String> {
        self.call("verify");
        if self.at == At::Verify { Err("the record's signature check failed: signature INVALID".into()) } else { Ok(()) }
    }
    fn remove(&self, t: &Tree) {
        assert_eq!(*t, tree());
        self.call("remove");
    }
}

/// The run's life: live for the first `n` checks, then ended.
struct Life(AtomicUsize);

impl Life {
    fn always() -> Life {
        Life(AtomicUsize::new(usize::MAX))
    }
    fn for_checks(n: usize) -> Life {
        Life(AtomicUsize::new(n))
    }
}

impl RunLife for Life {
    fn start(&self, launch: &mut dyn FnMut()) -> bool {
        let live = self.0.load(Ordering::SeqCst) > 0;
        if live {
            self.0.fetch_sub(1, Ordering::SeqCst);
            launch();
        }
        live
    }
}

fn crit(id: &str) -> Criterion {
    Criterion { id: id.into(), criterion: format!("{id} holds"), check: format!("make {id}") }
}

fn round() -> RoundInput {
    let task = |id: &str, c: &str| super::super::plan::PlanTask {
        id: id.into(), title: format!("do {id}"), owner_role: "backend".into(), files: vec!["src/**".into()], deps: vec![], criteria: vec![c.into()],
        brief: "do it".into(), touches_security: false,
    };
    let close = |id: &str, n: char| TaskClose { id: format!("{RUN}.{id}"), commit: n.to_string().repeat(40), branch: format!("vajra/{RUN}/task-{id}-r1"), changed: vec![format!("src/{id}.rs")] };
    RoundInput {
        run: RUN.into(), round: 1, head: HEAD.into(), base: BASE.into(), criteria: vec![crit("ac-1"), crit("ac-2")],
        tasks: vec![task("t1", "ac-1"), task("t2", "ac-2")], security: false, security_writers: vec![], closes: vec![close("t1", 'a'), close("t2", 'b')],
    }
}

fn result(id: &str, cmd: &str) -> VResult {
    VResult { criterion_id: id.into(), status: Status::Pass, command: cmd.into(), notes: String::new(), exit_code: 0, evidence: "ok".into() }
}

fn verification() -> Verification {
    Verification {
        results: vec![result("ac-1", "cargo test ac1"), result("ac-2", "cargo test ac2")],
        suite_command: "cargo test".into(), suite_passes: true, regressions: vec![], setup_cmds: vec!["npm ci".into()],
    }
}

fn verifier() -> WorkerSeat {
    WorkerSeat {
        slot: "v1".into(), role: "verifier".into(), task: String::new(), account: "codex (work)".into(), model: None, effort: None, security: false, network: true,
        why: String::new(), source: "user".into(),
    }
}

fn delivery(fake: &Arc<Fake>, trust: Trust) -> Delivery {
    let slug = if trust == Trust::OutsideGuard { format!("{SLUG}-og") } else { SLUG.into() };
    Delivery::new(DeliveryConfig { run: RUN.into(), slug, trust }, fake.clone())
}

fn deliver(fake: &Arc<Fake>, trust: Trust) -> Delivered {
    delivery(fake, trust).deliver(&round(), &verification(), &verifier(), &Life::always())
}

const ALL_WRITES: [&str; 8] =
    ["TasksClose", "TasksClose", "CampaignArchiveEvidence", "CampaignPass", "CampaignPass", "CampaignProgress", "CampaignCheckpoint", "CampaignClose"];

fn guarded_calls() -> Vec<String> {
    let mut want = vec!["tree".to_string(), "run".into(), "sign".into(), "check".into()];
    want.extend(ALL_WRITES.iter().map(|k| format!("write {k}")));
    want.extend(["write CampaignReport".to_string(), "verify".into(), "remove".into()]);
    want
}

// ---------------------------------------------------------------- the whole delivery

#[test]
fn a_guarded_run_runs_signs_checks_records_and_verifies_in_order_then_reads_verified() {
    let f = Fake::new(At::Nowhere);
    let d = deliver(&f, Trust::Guarded);
    assert!(d.verified, "{d:?}");
    assert_eq!(d.reason, "round 1 passed and its evidence was checked");
    assert_eq!(f.calls(), guarded_calls(), "no approval for a guarded run, CampaignVerify last, the worktree removed");
    // The ask: setup first, each criterion's check in criteria order, then the suite; the verifier's seat and network.
    let ask = crate::lock(&f.asks)[0].clone();
    assert_eq!(ask.commands, ["npm ci", "cargo test ac1", "cargo test ac2", "cargo test"]);
    assert_eq!((ask.run_id.as_str(), ask.round, ask.commit.as_str(), ask.seat.as_str(), ask.account.as_str(), ask.network), (RUN, 1, HEAD, "v1", "codex (work)", true));
    let steps: Vec<&str> = d.steps.iter().map(|s| s["step"].as_str().unwrap()).collect();
    assert_eq!(steps, ["request", "worktree", "run", "sign", "check_evidence", "tasks_close", "archive", "pass", "close", "report", "verify"]);
}

#[test]
fn every_record_write_carries_the_evidence_the_commit_and_the_tasks_own_branch() {
    let f = Fake::new(At::Nowhere);
    assert!(deliver(&f, Trust::Guarded).verified);
    let w = f.writes();
    let ev = PathBuf::from("/wt/evidence-r1/data/walkthroughs/deliver-20261114-0900/evidence.json");
    let Request::TasksClose { slug, id, evidence, head, commit, branch, base_commit, changed, repo } = &w[0] else { panic!("{:?}", w[0]) };
    assert_eq!((slug.as_str(), id.as_str(), evidence, head.as_str(), commit.as_str()), (SLUG, "20261114-0900.t1", &ev, HEAD, "a".repeat(40).as_str()));
    assert_eq!((branch.as_str(), base_commit.as_str(), changed.clone(), repo.as_path()), ("vajra/20261114-0900/task-t1-r1", BASE, vec!["src/t1.rs".to_string()], Path::new("/wt/evidence-r1")));
    let Request::CampaignArchiveEvidence { dir, head, setup, .. } = &w[2] else { panic!("{:?}", w[2]) };
    assert_eq!((dir.as_path(), head.as_str(), setup.clone()), (Path::new("/wt/evidence-r1/data/walkthroughs/deliver-20261114-0900"), HEAD, vec!["npm ci".to_string()]));
    let Request::CampaignPass { feature, evidence, .. } = &w[3] else { panic!("{:?}", w[3]) };
    assert_eq!((feature.as_str(), evidence.as_str()), ("ac-1", "cargo test ac1 (exit 0)"), "the pass names the command that ran, as the workflow does");
    let Request::CampaignClose { status, .. } = &w[7] else { panic!("{:?}", w[7]) };
    assert_eq!(*status, CloseStatus::Done);
    let Request::CampaignReport { bytes, check, base64, .. } = &w[8] else { panic!("{:?}", w[8]) };
    assert_eq!(*check, fnv32(base64), "the bridge recomputes this before it spawns");
    assert!(*bytes > 0 && *bytes <= REPORT_MAX);
}

#[test]
fn an_approval_run_asks_first_runs_through_the_approval_and_signs_its_progress_line() {
    let f = Fake::new(At::Nowhere);
    let d = deliver(&f, Trust::GuardedByApproval);
    assert!(d.verified, "{d:?}");
    assert_eq!(d.reason, "round 1 passed and its evidence was checked; check commands approved by you");
    let calls = f.calls();
    assert_eq!(&calls[..4], ["tree", "approve", "run approved", "write CampaignProgress"]);
    // The person approved exactly the list that ran.
    let asks = crate::lock(&f.asks).clone();
    assert_eq!(asks[0], asks[1]);
    // Bound to the repository of the worktree the core made, not one a caller names (codex P1 on #249).
    assert_eq!(asks[0].repo, tree().git_common);
    let Request::CampaignProgress { text, .. } = &f.writes()[0] else { panic!() };
    assert_eq!(*text, progress_line(APPROVAL_SHA), "the proof needs this signed line (N20)");
}

#[test]
fn an_outside_guard_run_skips_the_approval_and_is_labelled_in_every_line_it_writes() {
    let f = Fake::new(At::Nowhere);
    let fake: Arc<dyn Steps> = f.clone();
    let d = Delivery::new(DeliveryConfig { run: RUN.into(), slug: SLUG.into(), trust: Trust::OutsideGuard }, fake).deliver(&round(), &verification(), &verifier(), &Life::always());
    assert!(d.verified, "{d:?}");
    assert!(d.reason.ends_with("the evidence ran outside the Vajra guard"), "{}", d.reason);
    assert!(!f.calls().contains(&"approve".to_string()));
    for w in f.writes() {
        let text = match &w {
            Request::CampaignPass { evidence, .. } => evidence.clone(),
            Request::CampaignProgress { text, .. } => text.clone(),
            Request::CampaignCheckpoint { label, .. } => label.clone(),
            _ => continue,
        };
        assert!(text.starts_with(OUTSIDE_GUARD), "{text}");
    }
}

// ---------------------------------------------------------------- every failure is not verified

#[test]
fn each_failing_step_ends_not_verified_with_its_reason_runs_nothing_after_it_and_removes_the_worktree() {
    let cases: Vec<(At, Trust, &str, &str)> = vec![
        (At::Tree, Trust::Guarded, "The evidence worktree: it couldn't be made (worktree_add)", "tree"),
        (At::Approve, Trust::GuardedByApproval, "Your approval: you declined the check commands", "approve"),
        (At::RunErr, Trust::Guarded, "The evidence run: the worktree isn't at the approved commit", "run"),
        (At::RunFailed, Trust::Guarded, "The evidence run: the evidence pipe closed early", "run"),
        (At::RunNotPassed, Trust::Guarded, "The evidence run: a check command failed when Vajra ran it", "run"),
        (At::SignRefused, Trust::Guarded, "Signing the evidence: refused (sha256 mismatch)", "sign"),
        (At::SignNoKey, Trust::Guarded, "Signing the evidence: Vajra's signing key isn't usable", "sign"),
        (At::SignUnknown, Trust::Guarded, "Signing the evidence: signing ended unexpectedly", "sign"),
        (At::Check, Trust::Guarded, "The evidence check: not green: git.head is not the integration commit", "check"),
        (At::Write("TasksClose"), Trust::Guarded, "Closing the tasks: task 20261114-0900.t1 didn't close: tasks.json is locked", "write TasksClose"),
        (At::Write("CampaignArchiveEvidence"), Trust::Guarded, "Archiving the evidence: tasks.json is locked", "write CampaignArchiveEvidence"),
        (At::Write("CampaignPass"), Trust::Guarded, "Passing the criteria: criterion ac-1 wasn't marked passed: tasks.json is locked", "write CampaignPass"),
        (At::Write("CampaignCheckpoint"), Trust::Guarded, "The record: the delivered checkpoint wasn't written: tasks.json is locked", "write CampaignCheckpoint"),
        (At::Write("CampaignClose"), Trust::Guarded, "The record: the done close wasn't written: tasks.json is locked", "write CampaignClose"),
        (At::Write("CampaignReport"), Trust::Guarded, "The delivery report: tasks.json is locked", "write CampaignReport"),
        (At::Verify, Trust::Guarded, "The run record: the record's signature check failed: signature INVALID", "verify"),
        (At::Write("CampaignProgress"), Trust::GuardedByApproval, "The record: the approval's progress line wasn't written: tasks.json is locked", "write CampaignProgress"),
    ];
    for (at, trust, reason, last) in cases {
        let f = Fake::new(at.clone());
        let d = deliver(&f, trust);
        assert!(!d.verified, "{at:?} must never read verified");
        assert_eq!(d.reason, reason, "{at:?}");
        let calls = f.calls();
        let mut want_last = vec![last.to_string()];
        if at != At::Tree {
            want_last.push("remove".into());
        }
        assert_eq!(calls[calls.len() - want_last.len()..], want_last[..], "{at:?}: nothing runs after the failing step; {calls:?}");
        assert!(!calls.contains(&"verify".to_string()) || at == At::Verify, "{at:?}");
        let last_step = d.steps.last().unwrap();
        assert_eq!(last_step["ok"], false, "{at:?}");
    }
}

#[test]
fn a_run_that_found_a_failing_check_still_signs_its_used_approval_on_the_record_and_is_not_verified() {
    let f = Fake::new(At::RunNotPassed);
    let d = deliver(&f, Trust::GuardedByApproval);
    assert!(!d.verified);
    assert_eq!(f.calls(), ["tree", "approve", "run approved", "write CampaignProgress", "remove"]);
}

// ---------------------------------------------------------------- refused before anything runs

#[test]
fn a_list_vajra_cannot_run_or_keep_whole_is_refused_before_any_worktree_is_made() {
    let f = Fake::new(At::Nowhere);
    let run = |v: Verification| delivery(&f, Trust::Guarded).deliver(&round(), &v, &verifier(), &Life::always());
    let mut v = verification();
    v.setup_cmds = (0..6).map(|i| format!("make s{i}")).collect();
    assert_eq!(run(v).reason, "The check commands: the verifier listed 6 setup commands, and Vajra can keep at most 5");
    let mut v = verification();
    v.setup_cmds = vec!["x".repeat(1001)];
    assert_eq!(run(v).reason, "The check commands: a setup command is longer than 1000 characters");
    let mut v = verification();
    v.results.pop();
    assert_eq!(run(v).reason, "The check commands: criterion ac-2 has no check in the verifier's answer");
    let mut v = verification();
    v.results[0].command = "  ".into();
    assert_eq!(run(v).reason, "The check commands: criterion ac-1 has no check command");
    // A command the dialog couldn't show truthfully (the ask's own check) is refused for every trust.
    let mut v = verification();
    v.results[0].command = "cargo test\u{2028}rm -rf .".into();
    assert!(run(v).reason.starts_with("The check commands: a command has a"), "the ask's check applies");
    assert!(f.calls().is_empty(), "nothing ran: {:?}", f.calls());
}

#[test]
fn the_list_drops_repeats_and_an_empty_suite_and_keeps_the_order() {
    let mut v = verification();
    v.setup_cmds = vec!["npm ci".into(), " npm ci ".into(), "".into()];
    v.results[1].command = "cargo test ac1".into();
    v.suite_command = String::new();
    let c = commands(&v, &[crit("ac-1"), crit("ac-2")]).unwrap();
    assert_eq!(c.setup, ["npm ci"]);
    assert_eq!(c.all, ["npm ci", "cargo test ac1"]);
    assert_eq!(c.checks, [("ac-1".to_string(), "cargo test ac1".to_string()), ("ac-2".to_string(), "cargo test ac1".to_string())]);
}

#[test]
fn a_task_with_no_commit_to_close_on_is_refused_before_anything_runs() {
    let f = Fake::new(At::Nowhere);
    let mut r = round();
    r.closes.pop();
    let d = delivery(&f, Trust::Guarded).deliver(&r, &verification(), &verifier(), &Life::always());
    assert_eq!(d.reason, "The task graph: task t2 has no commit to close on the evidence");
    assert!(f.calls().is_empty());
}

// ---------------------------------------------------------------- the run's end stops the delivery

#[test]
fn once_the_run_has_ended_nothing_more_runs_and_the_delivery_is_not_verified() {
    // Ended before the worktree: nothing at all.
    let f = Fake::new(At::Nowhere);
    let d = delivery(&f, Trust::Guarded).deliver(&round(), &verification(), &verifier(), &Life::for_checks(0));
    assert_eq!(d.reason, "The evidence check: the run ended before the evidence check finished");
    assert!(f.calls().is_empty());
    // Ended after the worktree, before the run: no command runs, the worktree goes.
    let f = Fake::new(At::Nowhere);
    let d = delivery(&f, Trust::Guarded).deliver(&round(), &verification(), &verifier(), &Life::for_checks(1));
    assert!(!d.verified);
    assert_eq!(f.calls(), ["tree", "remove"]);
    // Ended between two task closes: the current write finishes, nothing after it.
    let f = Fake::new(At::Nowhere);
    let d = delivery(&f, Trust::Guarded).deliver(&round(), &verification(), &verifier(), &Life::for_checks(3));
    assert_eq!(d.reason, "The record: the run ended before the evidence check finished");
    assert_eq!(f.calls(), ["tree", "run", "sign", "check", "write TasksClose", "remove"]);
}

// ---------------------------------------------------------------- the report

#[test]
fn the_report_is_built_from_the_runs_facts_only() {
    let f = Fake::new(At::Nowhere);
    assert!(deliver(&f, Trust::GuardedByApproval).verified);
    let Some(Request::CampaignReport { base64, bytes, .. }) = f.writes().into_iter().find(|w| kind(w) == "CampaignReport") else { panic!() };
    let text = String::from_utf8(super::super::record_ledger::unbase64_for_tests(&base64).unwrap()).unwrap();
    assert_eq!(text.len(), bytes);
    for want in ["Run 20261114-0900 delivered round 1 at commit 1111111111111111111111111111111111111111.", "Evidence: guarded, check commands approved by you.", "- ac-1: ac-1 holds\n  check: cargo test ac1", "- 20261114-0900.t2 at bbbb", "- npm ci"] {
        assert!(text.contains(want), "{want} in {text}");
    }
}
