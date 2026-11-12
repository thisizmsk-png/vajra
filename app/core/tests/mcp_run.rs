//! The real `vajra-desktop --mcp-run` binary (ADR-16 16.5, 6d2): the token is read from its 0600 file and never shows
//! in the process's argv or environment, and a call goes stdio -> socket -> registry -> protocol. The peer check
//! passes for the server when it descends from the lead (this test process) and refuses it when the lead is another
//! process. Temp HOME and data dir only; every child is killed in a `Drop` guard.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use vajra_desktop::control::runreg::{self, LeadProc, RunRegistry};
use vajra_desktop::control::service::{Service, WorkerSeat};
use vajra_desktop::control::{nonce, socket};
use vajra_desktop::crew::redact::Redactor;
use vajra_desktop::desktop::runproto::{Integration, Limits, RunConfig, RunProtocol, StubGate, SystemClock, Tick, Workers};
use vajra_desktop::desktop::workers::{DispatchError, KillReason, TaskSpec};
use vajra_desktop::vajra::gitw::GitwError;

const RUN: &str = "20261112-0930";

/// No workers at all: these tests only need a protocol to answer `list_workers`.
struct NoWorkers;

impl Workers for NoWorkers {
    fn dispatch(&self, _: TaskSpec, _: WorkerSeat) -> Result<String, DispatchError> {
        Err(DispatchError::Invalid("no workers in this test"))
    }
    fn view(&self, _: &str) -> Option<Tick> {
        None
    }
    fn await_any(&self, _: &[String], _: Duration) -> Option<Tick> {
        None
    }
    fn cancel(&self, _: &str) -> bool {
        false
    }
    fn kill_run(&self, _: &str, _: KillReason) {}
    fn integrate(&self, _: &Integration<'_>, _: &[String]) -> Result<String, GitwError> {
        Err(GitwError::Invalid("no merges in this test"))
    }
}

fn proto() -> RunProtocol {
    let cfg = RunConfig {
        project: "p".into(), run: RUN.into(), repo: "/nonexistent/repo".into(), base: "0".repeat(40), ask: "a".into(), seats: vec![], max_in_flight: 1,
        limits: Limits::default(), redact: Redactor::default(),
    };
    RunProtocol::new(cfg, Arc::new(NoWorkers), Arc::new(StubGate), Arc::new(SystemClock::default()))
}

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Kid(Child);

impl Drop for Kid {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Rig {
    tmp: Tmp,
    reg: Arc<RunRegistry>,
    token: String,
    token_file: PathBuf,
    _svc: Arc<Mutex<Service>>,
}

fn rig() -> Rig {
    // Socket paths stay under 104 bytes, so not under $TMPDIR.
    let root = PathBuf::from(format!("/tmp/vmr-{}-{}", std::process::id(), nonce::random_hex(4).unwrap()));
    let _ = fs::remove_dir_all(&root);
    let tmp = Tmp(root.clone());
    let (home, data) = (root.join("h"), root.join("d"));
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&data).unwrap();
    let (token_file, hash) = runreg::write_token(&data.join("runs").join(RUN)).unwrap();
    let token = runreg::read_token(&token_file).unwrap();
    let reg = RunRegistry::new();
    reg.register(RUN, proto(), hash);
    let svc = Arc::new(Mutex::new(Service::open(&data, root.join("audit.jsonl"), None).unwrap()));
    let listener = socket::bind(&vajra_desktop::control::socket_path(&data)).unwrap();
    let (srv, r) = (svc.clone(), reg.clone());
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    std::thread::spawn(move || socket::serve(listener, srv, r, uid));
    Rig { tmp, reg, token, token_file, _svc: svc }
}

impl Rig {
    fn spawn(&self) -> (Kid, ChildStdin, BufReader<ChildStdout>) {
        let mut c = Command::new(env!("CARGO_BIN_EXE_vajra-desktop"))
            .arg("--mcp-run")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.tmp.0.join("h"))
            .env("VAJRA_DESKTOP_DATA", self.tmp.0.join("d"))
            .env(runreg::RUN_ENV, RUN)
            .env(runreg::TOKEN_FILE_ENV, &self.token_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let (stdin, stdout) = (c.stdin.take().unwrap(), c.stdout.take().unwrap());
        (Kid(c), stdin, BufReader::new(stdout))
    }
}

fn ask(stdin: &mut ChildStdin, out: &mut BufReader<ChildStdout>, msg: Value) -> Value {
    writeln!(stdin, "{msg}").unwrap();
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap_or_else(|e| panic!("{line:?}: {e}"))
}

fn me() -> LeadProc {
    let pid = std::process::id();
    LeadProc { pid, stamp: vajra_desktop::procid::start_stamp(pid).unwrap() }
}

/// The process's argv and environment as the OS shows them to other processes.
fn visible(pid: u32) -> String {
    if cfg!(target_os = "linux") {
        let r = |f: &str| fs::read(format!("/proc/{pid}/{f}")).map(|b| String::from_utf8_lossy(&b).replace('\0', " ")).unwrap_or_default();
        return format!("{} {}", r("cmdline"), r("environ"));
    }
    let o = Command::new("/bin/ps").args(["-ww", "-E", "-o", "command=", "-p", &pid.to_string()]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn the_token_never_shows_in_ps_or_argv_and_a_descendant_of_the_lead_is_served() {
    let r = rig();
    r.reg.set_lead(RUN, me());
    let (kid, mut stdin, mut out) = r.spawn();
    let init = ask(&mut stdin, &mut out, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
    assert_eq!(init["result"]["serverInfo"]["name"], "vajra_run");
    let shown = visible(kid.0.id());
    assert!(shown.contains("--mcp-run"), "ps shows the process: {shown}");
    assert!(shown.contains(runreg::TOKEN_FILE), "ps shows the token file's path, which is fine: {shown}");
    assert!(!shown.contains(&r.token), "the token is in ps: {shown}");
    let list = ask(&mut stdin, &mut out, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), runreg::TOOL_NAMES.len());
    let called = ask(&mut stdin, &mut out, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "list_workers", "arguments": {}}}));
    assert_eq!(called["result"]["isError"], false, "{called}");
    assert_eq!(called["result"]["structuredContent"], json!({"workers": []}));
}

#[test]
fn a_server_outside_the_leads_tree_is_refused() {
    let r = rig();
    // The lead is another process: the server, a child of this test, is not in its tree.
    let other = Kid(Command::new("sleep").arg("30").spawn().unwrap());
    r.reg.set_lead(RUN, LeadProc { pid: other.0.id(), stamp: vajra_desktop::procid::start_stamp(other.0.id()).unwrap() });
    let (_kid, mut stdin, mut out) = r.spawn();
    let called = ask(&mut stdin, &mut out, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "list_workers", "arguments": {}}}));
    assert_eq!(called["result"]["isError"], true);
    assert_eq!(called["result"]["structuredContent"]["error"], "peer_not_lead", "{called}");
}

#[test]
fn a_server_with_no_valid_token_file_refuses_to_start() {
    let r = rig();
    fs::write(&r.token_file, "not-a-token").unwrap();
    let (mut kid, stdin, _out) = r.spawn();
    drop(stdin);
    let st = kid.0.wait().unwrap();
    assert!(!st.success(), "a malformed token file is a refusal to start");
}
