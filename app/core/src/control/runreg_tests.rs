//! 6d2 transport tests: the tool table, the token file, the registry's gate (run, token, lead, peer) and the socket
//! routing of `vajra-run` calls past the Service lock. Fake workers and no harness; the peer check runs against
//! real processes this test spawns, each killed in a `Drop` guard.

use super::*;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command};
use std::sync::Condvar;
use std::time::Instant;

use crate::control::service::WorkerSeat;
use crate::crew::redact::Redactor;
use crate::desktop::runproto::{Integration, Limits, RunConfig, StubGate, SystemClock, Tick, Workers};
use crate::desktop::workers::{DispatchError, KillReason, State, TaskSpec};
use crate::vajra::gitw::GitwError;

// ---------------------------------------------------------------- fakes

/// Tickets that run until the test ends them. Only what the transport tests need.
#[derive(Default)]
struct FakeWorkers {
    tickets: Mutex<BTreeMap<String, Tick>>,
    changed: Condvar,
}

impl Workers for FakeWorkers {
    fn dispatch(&self, spec: TaskSpec, _: WorkerSeat) -> Result<String, DispatchError> {
        let id = format!("{}.{}.r{}", spec.run, spec.task, spec.round);
        let t = Tick { id: id.clone(), state: State::Running, outcome: None, reason: None, commit: None, attempt: 1, elapsed_s: 0, queue_position: None };
        crate::lock(&self.tickets).insert(id.clone(), t);
        Ok(id)
    }
    fn view(&self, ticket: &str) -> Option<Tick> {
        crate::lock(&self.tickets).get(ticket).cloned()
    }
    fn await_any(&self, tickets: &[String], wait: Duration) -> Option<Tick> {
        let deadline = Instant::now() + wait;
        let mut b = crate::lock(&self.tickets);
        loop {
            if let Some(t) = tickets.iter().filter_map(|id| b.get(id)).find(|t| t.state == State::Ended) {
                return Some(t.clone());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            b = self.changed.wait_timeout(b, left).unwrap().0;
        }
    }
    fn cancel(&self, _: &str) -> bool {
        false
    }
    fn kill_run(&self, _: &str, _: KillReason) {}
    fn integrate(&self, _: &Integration<'_>, _: &[String]) -> Result<String, GitwError> {
        Ok("f".repeat(40))
    }
}

fn seat(slot: &str, role: &str) -> WorkerSeat {
    WorkerSeat {
        slot: slot.into(), role: role.into(), task: String::new(), account: "codex".into(), model: None, effort: None, security: false, network: false,
        why: String::new(), source: "role_default".into(),
    }
}

const RUN: &str = "20261112-0930";

fn proto() -> RunProtocol {
    let cfg = RunConfig {
        project: "p".into(), run: RUN.into(), repo: "/nonexistent/repo".into(), base: "0".repeat(40), ask: "add a flag".into(),
        seats: vec![seat("w1", "backend"), seat("w2", "backend"), seat("w3", "verifier"), seat("w4", "reviewer")], max_in_flight: 4,
        limits: Limits::default(), redact: Redactor::default(),
    };
    RunProtocol::new(cfg, Arc::new(FakeWorkers::default()), Arc::new(StubGate), Arc::new(SystemClock::default()))
}

/// Two backend tasks, dispatched: their tickets run until the test ends.
fn two_tickets(p: &RunProtocol) -> (String, String) {
    let plan = json!({"round": 1, "criteria": [{"id": "ac-1", "criterion": "it works", "check": "true"}],
        "tasks": [{"id": "t1", "title": "a", "owner_role": "backend", "files": ["src/a/"], "criteria": ["ac-1"], "brief": "a"},
                  {"id": "t2", "title": "b", "owner_role": "backend", "files": ["src/b/"], "brief": "b"}]});
    call(p, "plan_tasks", &plan).unwrap();
    let t = |id: &str| call(p, "dispatch_worker", &json!({"task_id": id})).unwrap()["ticket"].as_str().unwrap().to_string();
    (t("t1"), t("t2"))
}

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        // Socket paths stay under 104 bytes, so not under $TMPDIR.
        let d = PathBuf::from(format!("/tmp/vrr-{tag}-{}-{}", std::process::id(), nonce::random_hex(4).unwrap()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A process this test spawned, killed and reaped on drop.
struct Kid(Child);

impl Drop for Kid {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A lead process: a child of this test (so never the app or its ancestor), killed on drop.
fn lead_kid() -> (Kid, LeadProc) {
    let k = Kid(Command::new("sleep").arg("30").spawn().unwrap());
    let l = LeadProc { pid: k.0.id(), stamp: crate::procid::start_stamp(k.0.id()).unwrap() };
    (k, l)
}

/// This test process as a socket peer.
fn me() -> Option<Peer> {
    Peer::now(std::process::id())
}

fn req(run: &str, token: &str, tool: &str, args: Value) -> Value {
    json!({"vajra_run": run, "token": token, "tool": tool, "args": args})
}

/// A registry serving `proto` for RUN behind a fresh token; returns the token.
fn registered(reg: &RunRegistry, p: RunProtocol, lead: Option<LeadProc>) -> String {
    let token = nonce::random_hex(32).unwrap();
    reg.register(RUN, p, sha256::digest(token.as_bytes()));
    if let Some(l) = lead {
        reg.set_lead(RUN, l).unwrap();
    }
    token
}

/// A registry whose ancestry check always passes: the same-process check stays real.
fn any_tree() -> RunRegistry {
    RunRegistry::with_peer_check(Box::new(|_, _| true))
}

// ---------------------------------------------------------------- the tool table

#[test]
fn tools_list_is_exactly_the_vajra_run_tools() {
    let tools = tools();
    let names: Vec<&str> = tools.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, TOOL_NAMES);
    for t in tools.as_array().unwrap() {
        assert_eq!(t["inputSchema"]["additionalProperties"], json!(false), "{}", t["name"]);
    }
    for gone in ["verify", "review", "propose_run", "message_worker"] {
        assert!(!is_run_tool(gone), "{gone} is not a lead tool");
    }
}

#[test]
fn an_await_waits_wait_s_plus_ten_and_other_calls_keep_the_default() {
    let d = Duration::from_secs(30);
    assert_eq!(client_timeout("await_any", &json!({"wait_s": 110}), d), Duration::from_secs(120));
    assert_eq!(client_timeout("await_worker", &json!({"wait_s": 5}), d), Duration::from_secs(15));
    assert_eq!(client_timeout("await_any", &json!({"wait_s": 9999}), d), Duration::from_secs(120), "never past 110 + 10");
    assert_eq!(client_timeout("plan_tasks", &json!({}), d), d);
    assert_eq!(client_timeout("integrate", &json!({"round": 1}), d), INTEGRATE_TIMEOUT, "a merge outlasts the default");
}

#[test]
fn a_call_with_bad_arguments_is_a_tool_error_not_a_panic() {
    let p = proto();
    assert_eq!(call(&p, "plan_tasks", &json!({"round": "one"})).unwrap_err()["error"], "invalid_args");
    assert_eq!(call(&p, "dispatch_worker", &json!({})).unwrap_err()["error"], "invalid_args");
    assert_eq!(call(&p, "await_any", &json!({"tickets": [1], "wait_s": 1})).unwrap_err()["error"], "invalid_args");
    assert_eq!(call(&p, "integrate", &json!([])).unwrap_err()["error"], "invalid_args");
    assert_eq!(call(&p, "list_workers", &json!({})).unwrap(), json!({"workers": [], "rounds": []}));
}

#[test]
fn list_and_inspect_show_only_this_runs_tickets() {
    let p = proto();
    let (a, _) = two_tickets(&p);
    let all = call(&p, "list_workers", &json!({})).unwrap();
    assert_eq!(all["workers"].as_array().unwrap().len(), 2);
    let one = call(&p, "inspect_worker", &json!({"ticket": a})).unwrap();
    assert_eq!(one["task"], "t1");
    assert_eq!(one["status"]["state"], "running");
    assert_eq!(call(&p, "inspect_worker", &json!({"ticket": "other.t1.r1"})).unwrap_err()["error"], "invalid");
}

// ---------------------------------------------------------------- the token file

#[test]
fn the_token_file_is_0600_and_only_its_hash_comes_back() {
    let t = Tmp::new("tok");
    let dir = t.0.join("runs").join(RUN);
    let (path, hash) = write_token(&dir).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let token = read_token(&path).unwrap();
    assert_eq!(sha256::digest(token.as_bytes()), hash);
    let (path2, hash2) = write_token(&dir).unwrap();
    assert_eq!(path, path2, "a rotation replaces the one file");
    assert_ne!(hash, hash2);
    assert_ne!(read_token(&path).unwrap(), token);
}

#[test]
fn a_token_file_that_is_readable_by_others_or_a_link_is_refused() {
    let t = Tmp::new("tokbad");
    let (path, _) = write_token(&t.0).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(read_token(&path).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = t.0.join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(read_token(&link).is_err());
    std::fs::write(t.0.join("short"), "abc").unwrap();
    std::fs::set_permissions(t.0.join("short"), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(read_token(&t.0.join("short")).unwrap_err().kind(), io::ErrorKind::InvalidData);
}

// ---------------------------------------------------------------- the gate

#[test]
fn the_gate_refuses_an_unknown_run_a_wrong_token_and_a_run_with_no_lead() {
    let reg = any_tree();
    let token = registered(&reg, proto(), None);
    assert_eq!(reg.serve(&req("20990101-0000", &token, "list_workers", json!({})), me())["error"], "run_gone");
    assert_eq!(reg.serve(&req(RUN, &"0".repeat(64), "list_workers", json!({})), me())["error"], "run_token");
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), me())["error"], "lead_not_running");
    let (_k, lead) = lead_kid();
    reg.set_lead(RUN, lead).unwrap();
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), me()), json!({"ok": {"workers": [], "rounds": []}}));
    assert_eq!(reg.serve(&req(RUN, &token, "propose_run", json!({})), me())["error"], "unknown_tool");
    assert_eq!(reg.serve(&json!({"vajra_run": RUN, "tool": "list_workers"}), me())["error"], "bad_request");
    assert!(reg.clear_lead(RUN, lead));
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), me())["error"], "lead_not_running", "a lead that exited calls nothing");
}

#[test]
fn the_app_itself_or_its_ancestor_is_never_a_lead() {
    let reg = RunRegistry::default();
    registered(&reg, proto(), None);
    let pid = std::process::id();
    let mine = LeadProc { pid, stamp: crate::procid::start_stamp(pid).unwrap() };
    assert_eq!(reg.set_lead(RUN, mine), Err(LeadRefused::TooWide));
    let (ppid, _) = crate::procid::parent_and_stamp(pid).unwrap();
    if let Some(stamp) = crate::procid::start_stamp(ppid) {
        assert_eq!(reg.set_lead(RUN, LeadProc { pid: ppid, stamp }), Err(LeadRefused::TooWide), "every process of ours descends from it");
    }
    let (_k, lead) = lead_kid();
    assert_eq!(reg.set_lead("20990101-0000", lead), Err(LeadRefused::UnknownRun));
    assert_eq!(reg.set_lead(RUN, lead), Ok(()));
}

#[test]
fn a_peer_that_does_not_descend_from_the_lead_is_refused() {
    let reg = RunRegistry::default();
    let (lead_kid, lead) = lead_kid();
    let token = registered(&reg, proto(), Some(lead));
    // This test process is the lead's parent, never its descendant (as a core-spawned worker is never in its tree).
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), me())["error"], "peer_not_lead");
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), None)["error"], "peer_not_lead", "no peer pid, no call");
    assert!(!reg.in_lead_tree(me()));
    // The lead itself passes.
    let p = Peer::now(lead_kid.0.id());
    assert!(reg.serve(&req(RUN, &token, "list_workers", json!({})), p).get("ok").is_some());
    assert!(reg.in_lead_tree(p));
}

#[test]
fn a_reused_lead_pid_with_another_start_time_is_refused() {
    let reg = RunRegistry::default();
    let (k, real) = lead_kid();
    // The recorded lead has the process's number but another start time: what a reused pid looks like.
    let token = registered(&reg, proto(), Some(LeadProc { pid: real.pid, stamp: real.stamp + 1 }));
    let p = Peer::now(k.0.id());
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), p)["error"], "peer_not_lead");
    reg.set_lead(RUN, real).unwrap();
    assert!(reg.serve(&req(RUN, &token, "list_workers", json!({})), p).get("ok").is_some());
}

#[test]
fn a_peer_whose_pid_now_names_another_process_is_refused() {
    // What a connection handed to a process that later reused the peer's pid looks like: same pid, newer stamp.
    let reg = any_tree();
    let (_k, lead) = lead_kid();
    let token = registered(&reg, proto(), Some(lead));
    let then = Peer { pid: std::process::id(), stamp: me().unwrap().stamp - 1 };
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), Some(then))["error"], "peer_not_lead");
    assert!(!reg.in_lead_tree(Some(then)));
    assert!(reg.serve(&req(RUN, &token, "list_workers", json!({})), me()).get("ok").is_some());
}

#[test]
fn a_rotated_token_refuses_the_old_one() {
    let reg = any_tree();
    let (_k, lead) = lead_kid();
    let old = registered(&reg, proto(), Some(lead));
    let t = Tmp::new("rot");
    let (path, hash) = write_token(&t.0).unwrap();
    assert!(reg.rotate_token(RUN, hash));
    let new = read_token(&path).unwrap();
    assert_eq!(reg.serve(&req(RUN, &old, "list_workers", json!({})), me())["error"], "run_token");
    assert!(reg.serve(&req(RUN, &new, "list_workers", json!({})), me()).get("ok").is_some());
    reg.remove(RUN);
    assert_eq!(reg.serve(&req(RUN, &new, "list_workers", json!({})), me())["error"], "run_gone");
}

#[test]
fn past_the_per_run_bound_a_call_is_answered_busy() {
    let reg = Arc::new(any_tree());
    let (_k, lead) = lead_kid();
    let p = proto();
    let (a, _) = two_tickets(&p);
    let token = registered(&reg, p, Some(lead));
    let parked: Vec<_> = (0..MAX_RUN_CALLS)
        .map(|_| {
            let (reg, token, a) = (reg.clone(), token.clone(), a.clone());
            std::thread::spawn(move || reg.serve(&req(RUN, &token, "await_any", json!({"tickets": [a], "wait_s": 2})), me()))
        })
        .collect();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), me())["error"], "busy");
    for t in parked {
        assert!(t.join().unwrap().get("ok").is_some());
    }
    assert!(reg.serve(&req(RUN, &token, "list_workers", json!({})), me()).get("ok").is_some(), "the slots are freed");
}

// ---------------------------------------------------------------- the socket

fn service(t: &Tmp) -> Arc<Mutex<super::super::service::Service>> {
    let data = t.0.join("data");
    std::fs::create_dir_all(&data).unwrap();
    Arc::new(Mutex::new(super::super::service::Service::open(&data, t.0.join("audit.jsonl"), None).unwrap()))
}

fn serving(t: &Tmp, reg: Arc<RunRegistry>) -> (PathBuf, Arc<Mutex<super::super::service::Service>>) {
    let sock = t.0.join("s").join("app.sock");
    let listener = super::super::socket::bind(&sock).unwrap();
    let svc = service(t);
    let srv = svc.clone();
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    std::thread::spawn(move || super::super::socket::serve(listener, srv, reg, uid));
    (sock, svc)
}

/// Send `lines` raw on one connection; every reply line until the app closes it.
fn raw(sock: &Path, lines: &str) -> Vec<Value> {
    let mut c = std::os::unix::net::UnixStream::connect(sock).unwrap();
    // The app may answer and close before it has read everything (a refused oversized line): a broken pipe is fine.
    let _ = std::io::Write::write_all(&mut c, lines.as_bytes());
    let _ = c.shutdown(std::net::Shutdown::Write);
    std::io::BufRead::lines(std::io::BufReader::new(c)).map(|l| serde_json::from_str(&l.unwrap()).unwrap()).collect()
}

#[test]
fn two_concurrent_awaits_both_return_within_wait_s_plus_one_while_the_service_lock_is_held() {
    let t = Tmp::new("conc");
    let reg = any_tree();
    let (_k, lead) = lead_kid();
    let p = proto();
    let (a, b) = two_tickets(&p);
    let token = registered(&reg, p, Some(lead));
    let (sock, svc) = serving(&t, Arc::new(reg));
    // Hold the control Service lock for the whole test: a vajra-run call must never need it.
    let _held = crate::lock(&svc);
    let wait_s = 2u64;
    let started = Instant::now();
    let calls: Vec<_> = [a, b]
        .into_iter()
        .map(|tk| {
            let (sock, token) = (sock.clone(), token.clone());
            std::thread::spawn(move || {
                let args = json!({"tickets": [tk], "wait_s": wait_s});
                let to = client_timeout("await_any", &args, Duration::from_secs(30));
                let out = super::super::socket::request_run(&sock, RUN, &token, "await_any", &args, to).unwrap();
                (out, started.elapsed())
            })
        })
        .collect();
    for c in calls {
        let (out, took) = c.join().unwrap();
        assert!(out["ok"]["ended"].is_null(), "nothing ended: {out}");
        assert_eq!(out["ok"]["running"].as_array().unwrap().len(), 1);
        assert!(took < Duration::from_secs(wait_s + 1), "both awaits ran side by side: {took:?}");
    }
}

#[test]
fn a_vajra_run_connection_serves_one_request_and_a_lead_tree_peer_gets_no_service_calls() {
    let t = Tmp::new("one");
    let reg = any_tree();
    let (_k, lead) = lead_kid();
    let token = registered(&reg, proto(), Some(lead));
    let (sock, _svc) = serving(&t, Arc::new(reg));
    let two = format!("{}\n{}\n", req(RUN, &token, "list_workers", json!({})), req(RUN, &token, "list_workers", json!({})));
    let got = raw(&sock, &two);
    assert_eq!(got.len(), 1, "the connection closes after one vajra-run reply: {got:?}");
    assert!(got[0].get("ok").is_some());
    // Every peer passes the (faked) ancestry check here, so this test process is "in a lead's tree".
    let svc_call = format!("{}\n", json!({"tool": "list_workers", "args": {}}));
    assert_eq!(raw(&sock, &svc_call)[0]["error"], "run_lead_on_service");
}

#[test]
fn only_plan_tasks_from_a_lead_tree_may_use_the_larger_request_cap() {
    let t = Tmp::new("cap");
    let reg = any_tree();
    let (_k, lead) = lead_kid();
    let token = registered(&reg, proto(), Some(lead));
    let (sock, _svc) = serving(&t, Arc::new(reg));
    let big = "x".repeat(100 * 1024);
    let to = Duration::from_secs(10);
    // A 100 KiB plan passes the cap and reaches the plan rules (which refuse its 100 KiB brief).
    let plan = json!({"round": 1, "criteria": [{"id": "ac-1", "criterion": "c", "check": "true"}],
        "tasks": [{"id": "t1", "title": "a", "owner_role": "backend", "files": ["src/a/"], "criteria": ["ac-1"], "brief": big}]});
    let out = super::super::socket::request_run(&sock, RUN, &token, "plan_tasks", &plan, to).unwrap();
    assert_ne!(out["error"], "request_too_large", "{out}");
    assert!(out.get("tool_error").is_some(), "{out}");
    // Any other tool with 100 KiB is refused by the socket itself (sent raw: the client would not even send it).
    assert_eq!(raw(&sock, &format!("{}\n", req(RUN, &token, "finish_run", json!({"report": big}))))[0]["error"], "request_too_large");
    assert_eq!(super::super::socket::request_run(&sock, RUN, &token, "finish_run", &json!({"report": big}), to).unwrap()["error"], "request_too_large");
    let huge = json!({"round": 1, "tasks": [], "x": "y".repeat(MAX_PLAN_REQUEST)});
    assert_eq!(super::super::socket::request_run(&sock, RUN, &token, "plan_tasks", &huge, to).unwrap()["error"], "request_too_large");
}

#[test]
fn a_peer_outside_every_lead_tree_reads_no_more_than_the_small_cap() {
    let t = Tmp::new("small");
    let reg = RunRegistry::default();
    let (_k, lead) = lead_kid();
    let token = registered(&reg, proto(), Some(lead));
    let (sock, _svc) = serving(&t, Arc::new(reg));
    let plan = json!({"round": 1, "tasks": [], "pad": "x".repeat(100 * 1024)});
    assert_eq!(raw(&sock, &format!("{}\n", req(RUN, &token, "plan_tasks", plan)))[0]["error"], "request_too_large");
}

#[test]
fn a_service_request_still_works_beside_the_run_route() {
    let t = Tmp::new("svc");
    let (sock, _svc) = serving(&t, RunRegistry::new());
    let out = super::super::socket::request(&sock, "list_workers", &json!({})).unwrap();
    assert!(out.get("ok").is_some() || out.get("error").is_some(), "{out}");
    assert_ne!(out["error"], "run_gone", "a request without vajra_run never reaches the registry");
    assert_ne!(out["error"], "run_lead_on_service", "no lead is registered");
}

#[test]
fn an_app_that_takes_the_call_and_never_answers_is_a_timeout_not_a_refusal() {
    let t = Tmp::new("silent");
    let sock = t.0.join("app.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    // Accept and hold the connection without answering.
    let hold = std::thread::spawn(move || {
        let held = listener.accept();
        std::thread::sleep(Duration::from_millis(600));
        drop(held);
    });
    let e = super::super::socket::request_run(&sock, RUN, "t", "integrate", &json!({"round": 1}), Duration::from_millis(200)).unwrap_err();
    assert!(matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut), "{e:?}: the client maps this to no_answer");
    let _ = hold.join();
}

#[test]
fn a_late_exit_of_the_old_lead_never_clears_its_successor_and_its_parked_calls_free_no_slot_of_the_new_one() {
    let reg = Arc::new(any_tree());
    let (_k1, old) = lead_kid();
    let (_k2, new) = lead_kid();
    let p = proto();
    let (a, _) = two_tickets(&p);
    let token = registered(&reg, p, Some(old));
    // The old lead's calls park, filling its slots.
    let parked: Vec<_> = (0..MAX_RUN_CALLS)
        .map(|_| {
            let (reg, token, a) = (reg.clone(), token.clone(), a.clone());
            std::thread::spawn(move || reg.serve(&req(RUN, &token, "await_any", json!({"tickets": [a], "wait_s": 2})), me()))
        })
        .collect();
    std::thread::sleep(Duration::from_millis(400));
    // The restart: the new lead is set before the old lead's exit is handled.
    reg.set_lead(RUN, new).unwrap();
    assert!(!reg.clear_lead(RUN, old), "a stale exit leaves the new lead in place");
    assert!(reg.serve(&req(RUN, &token, "list_workers", json!({})), me()).get("ok").is_some(), "the new lead has its own slots");
    for t in parked {
        t.join().unwrap();
    }
    assert!(reg.clear_lead(RUN, new));
}

#[test]
fn the_schema_caps_are_the_protocols_caps() {
    use crate::desktop::runproto::{MAX_AWAIT_TICKETS, MAX_CRITERIA, MAX_PLAN_DECISIONS, MAX_TASK_DEPS, MAX_TASK_FILES};
    let t = tools();
    let tool = |n: &str| t.as_array().unwrap().iter().find(|x| x["name"] == n).unwrap()["inputSchema"]["properties"].clone();
    let plan = tool("plan_tasks");
    assert_eq!(plan["criteria"]["maxItems"], MAX_CRITERIA);
    assert_eq!(plan["decisions"]["maxItems"], MAX_PLAN_DECISIONS);
    let task = &plan["tasks"]["items"]["properties"];
    assert_eq!(task["criteria"]["maxItems"], MAX_CRITERIA);
    assert_eq!(task["deps"]["maxItems"], MAX_TASK_DEPS);
    assert_eq!(task["files"]["maxItems"], MAX_TASK_FILES);
    assert_eq!(tool("await_any")["tickets"]["maxItems"], MAX_AWAIT_TICKETS);
    assert!(client_timeout("await_any", &json!({"wait_s": 110}), Duration::ZERO) <= MAX_CLIENT_TIMEOUT);
    assert!(client_timeout("integrate", &json!({}), Duration::ZERO) <= MAX_CLIENT_TIMEOUT);
    assert!(MAX_CLIENT_TIMEOUT < Duration::from_secs(130), "below the Codex lead's tool_timeout_sec");
}

#[test]
fn the_schema_text_limits_are_the_protocols_and_a_plan_at_them_is_accepted() {
    use crate::desktop::runproto::{MAX_ACTION, MAX_ACTION_WHY, MAX_DECISION_TITLE, MAX_RATIONALE};
    let t = tools();
    let plan = t.as_array().unwrap().iter().find(|x| x["name"] == "plan_tasks").unwrap()["inputSchema"]["properties"].clone();
    let action = &plan["protected_actions"]["items"]["properties"];
    assert_eq!((action["action"]["maxLength"].as_u64(), action["why"]["maxLength"].as_u64()), (Some(MAX_ACTION as u64), Some(MAX_ACTION_WHY as u64)));
    let decision = &plan["decisions"]["items"]["properties"];
    assert_eq!((decision["title"]["maxLength"].as_u64(), decision["rationale"]["maxLength"].as_u64()), (Some(MAX_DECISION_TITLE as u64), Some(MAX_RATIONALE as u64)));
}

#[test]
fn a_starting_leads_call_waits_for_its_read_back_and_passes_only_once_confirmed() {
    let reg = Arc::new(any_tree());
    let token = registered(&reg, proto(), None);
    let (_k, lead) = lead_kid();
    reg.start_lead(RUN, lead).unwrap();
    assert!(reg.in_lead_tree(me()), "a starting lead's tree is held to vajra-run tools from its first call");
    let (r2, t2) = (reg.clone(), token.clone());
    let started = std::time::Instant::now();
    let call = std::thread::spawn(move || r2.serve(&req(RUN, &t2, "list_workers", json!({})), me()));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!call.is_finished(), "nothing passes before the read-back");
    assert!(reg.confirm_lead(RUN, lead));
    assert_eq!(call.join().unwrap(), json!({"ok": {"workers": [], "rounds": []}}));
    assert!(started.elapsed() < CONFIRM_WAIT);
    // A lead cleared while starting (its read-back failed) is refused at once, and confirming it later does nothing.
    let (_k2, next) = lead_kid();
    reg.start_lead(RUN, next).unwrap();
    assert!(reg.clear_lead(RUN, next));
    assert!(!reg.confirm_lead(RUN, next));
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), me())["error"], "lead_not_running");
}

/// A peer connects (a lead-tree process here: the ancestry check is faked to pass) and makes one Service call, then
/// forks a child that keeps the socket and exits. Once the test has reaped it, the child sends a Service request on
/// the inherited connection and writes the reply to `<dir>/reply`.
const HANDOFF: &str = r#"
import json, os, socket, sys, time
sock, d = sys.argv[1], sys.argv[2]
c = socket.socket(socket.AF_UNIX)
c.connect(sock)
f = c.makefile("rw")
f.write(json.dumps({"tool": "list_workers", "args": {}}) + "\n"); f.flush()
open(os.path.join(d, "first"), "w").write(f.readline())
if os.fork() == 0:
    while not os.path.exists(os.path.join(d, "go")):
        time.sleep(0.02)
    f.write(json.dumps({"tool": "list_workers", "args": {}}) + "\n"); f.flush()
    open(os.path.join(d, "reply.tmp"), "w").write(f.readline())
    os.rename(os.path.join(d, "reply.tmp"), os.path.join(d, "reply"))
    os._exit(0)
os._exit(0)
"#;

#[test]
fn a_connection_handed_on_by_a_lead_tree_peer_that_exited_never_reaches_the_service() {
    for lead_tree in [true, false] {
        let t = Tmp::new(if lead_tree { "hand" } else { "hand2" });
        // With no lead registered, the peer is an ordinary client: its gone identity is what refuses the call.
        let reg = any_tree();
        let (_k, lead) = lead_kid();
        let _token = registered(&reg, proto(), lead_tree.then_some(lead));
        let (sock, _svc) = serving(&t, Arc::new(reg));
        let mut first = Command::new("/usr/bin/python3").args(["-I", "-c", HANDOFF, sock.to_str().unwrap(), t.0.to_str().unwrap()]).spawn().unwrap();
        // Reap it: its pid and start stamp are gone before the child speaks.
        assert!(first.wait().unwrap().success());
        let seen: Value = serde_json::from_str(&std::fs::read_to_string(t.0.join("first")).unwrap()).unwrap();
        if lead_tree {
            assert_eq!(seen["error"], "run_lead_on_service");
        } else {
            assert_ne!(seen["error"], "peer_gone", "an ordinary live client is served: {seen}");
        }
        std::fs::write(t.0.join("go"), "").unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while !t.0.join("reply").exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        let out: Value = serde_json::from_str(&std::fs::read_to_string(t.0.join("reply")).unwrap()).unwrap();
        let want = if lead_tree { "run_lead_on_service" } else { "peer_gone" };
        assert_eq!(out["error"], want, "lead_tree={lead_tree}: {out}");
    }
}
