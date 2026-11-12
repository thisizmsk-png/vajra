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

fn me() -> LeadProc {
    let pid = std::process::id();
    LeadProc { pid, stamp: crate::procid::start_stamp(pid).unwrap() }
}

fn req(run: &str, token: &str, tool: &str, args: Value) -> Value {
    json!({"vajra_run": run, "token": token, "tool": tool, "args": args})
}

/// A registry serving `proto()` for RUN behind a fresh token; returns the token.
fn registered(reg: &RunRegistry, p: RunProtocol, lead: Option<LeadProc>) -> String {
    let token = nonce::random_hex(32).unwrap();
    reg.register(RUN, p, sha256::digest(token.as_bytes()));
    if let Some(l) = lead {
        reg.set_lead(RUN, l);
    }
    token
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
    assert_eq!(call(&p, "list_workers", &json!({})).unwrap(), json!({"workers": []}));
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
    let reg = RunRegistry::with_peer_check(Box::new(|_, _| true));
    let token = registered(&reg, proto(), None);
    let peer = Some(std::process::id());
    assert_eq!(reg.serve(&req("20990101-0000", &token, "list_workers", json!({})), peer)["error"], "run_unknown");
    assert_eq!(reg.serve(&req(RUN, &"0".repeat(64), "list_workers", json!({})), peer)["error"], "run_token");
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), peer)["error"], "lead_not_running");
    reg.set_lead(RUN, me());
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), peer), json!({"ok": {"workers": []}}));
    assert_eq!(reg.serve(&req(RUN, &token, "propose_run", json!({})), peer)["error"], "unknown_tool");
    assert_eq!(reg.serve(&json!({"vajra_run": RUN, "tool": "list_workers"}), peer)["error"], "bad_request");
    reg.clear_lead(RUN);
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), peer)["error"], "lead_not_running", "a lead that exited calls nothing");
}

#[test]
fn a_peer_that_does_not_descend_from_the_lead_is_refused() {
    let reg = RunRegistry::default();
    // The lead is a process this test spawned; this test process is its parent, never its descendant.
    let lead = Kid(Command::new("sleep").arg("30").spawn().unwrap());
    let lead_proc = LeadProc { pid: lead.0.id(), stamp: crate::procid::start_stamp(lead.0.id()).unwrap() };
    let token = registered(&reg, proto(), Some(lead_proc));
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), Some(std::process::id()))["error"], "peer_not_lead");
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), None)["error"], "peer_not_lead", "no peer pid, no call");
    // The lead itself passes.
    assert!(reg.serve(&req(RUN, &token, "list_workers", json!({})), Some(lead.0.id())).get("ok").is_some());
}

#[test]
fn a_reused_lead_pid_with_another_start_time_is_refused() {
    let reg = RunRegistry::default();
    let real = me();
    // The recorded lead has this process's number but another start time: what a reused pid looks like.
    let token = registered(&reg, proto(), Some(LeadProc { pid: real.pid, stamp: real.stamp + 1 }));
    let child = Kid(Command::new("sleep").arg("30").spawn().unwrap());
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), Some(child.0.id()))["error"], "peer_not_lead");
    assert_eq!(reg.serve(&req(RUN, &token, "list_workers", json!({})), Some(real.pid))["error"], "peer_not_lead");
    reg.set_lead(RUN, real);
    assert!(reg.serve(&req(RUN, &token, "list_workers", json!({})), Some(child.0.id())).get("ok").is_some(), "a child of the true lead passes");
}

#[test]
fn a_rotated_token_refuses_the_old_one() {
    let reg = RunRegistry::with_peer_check(Box::new(|_, _| true));
    let old = registered(&reg, proto(), Some(me()));
    let t = Tmp::new("rot");
    let (path, hash) = write_token(&t.0).unwrap();
    assert!(reg.rotate_token(RUN, hash));
    let new = read_token(&path).unwrap();
    let peer = Some(std::process::id());
    assert_eq!(reg.serve(&req(RUN, &old, "list_workers", json!({})), peer)["error"], "run_token");
    assert!(reg.serve(&req(RUN, &new, "list_workers", json!({})), peer).get("ok").is_some());
    reg.remove(RUN);
    assert_eq!(reg.serve(&req(RUN, &new, "list_workers", json!({})), peer)["error"], "run_unknown");
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

#[test]
fn two_concurrent_awaits_both_return_within_wait_s_plus_one_while_the_service_lock_is_held() {
    let t = Tmp::new("conc");
    // The peer is this test process: it is the lead.
    let reg = RunRegistry::default();
    let p = proto();
    let (a, b) = two_tickets(&p);
    let token = registered(&reg, p, Some(me()));
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
fn only_plan_tasks_may_use_the_larger_request_cap() {
    let t = Tmp::new("cap");
    let reg = RunRegistry::with_peer_check(Box::new(|_, _| true));
    let token = registered(&reg, proto(), Some(me()));
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
    let raw = format!("{}\n", req(RUN, &token, "finish_run", json!({"report": big})));
    let mut c = std::os::unix::net::UnixStream::connect(&sock).unwrap();
    std::io::Write::write_all(&mut c, raw.as_bytes()).unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(c), &mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["error"], "request_too_large");
    assert_eq!(super::super::socket::request_run(&sock, RUN, &token, "finish_run", &json!({"report": big}), to).unwrap()["error"], "request_too_large");
    let huge = json!({"round": 1, "tasks": [], "x": "y".repeat(MAX_PLAN_REQUEST)});
    assert_eq!(super::super::socket::request_run(&sock, RUN, &token, "plan_tasks", &huge, to).unwrap()["error"], "request_too_large");
}

#[test]
fn a_service_request_still_works_beside_the_run_route() {
    let t = Tmp::new("svc");
    let (sock, _svc) = serving(&t, RunRegistry::new());
    let out = super::super::socket::request(&sock, "list_workers", &json!({})).unwrap();
    assert!(out.get("ok").is_some() || out.get("error").is_some(), "{out}");
    assert_ne!(out["error"], "run_unknown", "a request without vajra_run never reaches the registry");
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
