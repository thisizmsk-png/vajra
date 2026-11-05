//! C6 control service: nonce, queue, ledger, stops, socket and the MCP stdio
//! server. Every test runs in its own temp HOME. The hook tests run the REAL
//! hooks/desktop-control.sh from ao/vajra-6/m1-engine (read with git show).
//!
//! Tests sit at the crate top level (no wrapping mod) so that
//! `cargo test queue_entry_matches_hook -- --exact` finds the test by name.

use std::fs;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use vajra_desktop::control::audit;
use vajra_desktop::control::ledger::Ledger;
use vajra_desktop::control::mcp::{self, SocketClient};
use vajra_desktop::control::nonce;
use vajra_desktop::control::reconcile;
use vajra_desktop::control::service::{Actor, ControlError, Service, WorkerInfo};
use vajra_desktop::control::socket::{self, MAX_REQUEST};
use vajra_desktop::crew::tree::{Status, Worker};
use vajra_desktop::logging::Logger;

static N: AtomicUsize = AtomicUsize::new(0);

const SID: &str = "62278db8-d6e8-48f7-b03e-14d8a8c989f2";
const RUN: &str = "20261105-1810";
const AID: &str = "aa870dd40540481f0";
const AID2: &str = "ab11223344556677e";

fn uniq() -> usize {
    N.fetch_add(1, Ordering::SeqCst)
}

/// A temp HOME laid out like the real one: the app data dir under
/// `Library/Application Support/VajraDesktop` (where the hook looks) and
/// `~/.claude/vajra/` for audit.log. Plus two fake worktrees.
struct Home {
    home: PathBuf,
}

impl Home {
    fn new() -> Self {
        let home = std::env::temp_dir().join(format!("vajra-control-{}-{}", std::process::id(), uniq()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join(".claude/vajra")).unwrap();
        fs::create_dir_all(home.join("wt/one")).unwrap();
        fs::create_dir_all(home.join("wt/two")).unwrap();
        Self { home }
    }
    fn data(&self) -> PathBuf {
        self.home.join("Library/Application Support/VajraDesktop")
    }
    fn session_dir(&self) -> PathBuf {
        self.data().join("control").join(SID)
    }
    fn audit(&self) -> PathBuf {
        self.home.join(".claude/vajra/audit.log")
    }
    fn wt(&self, n: &str) -> String {
        self.home.join("wt").join(n).to_string_lossy().into_owned()
    }
    fn service(&self) -> Service {
        let logger = Logger::open(&self.data().join("logs")).unwrap();
        Service::open(&self.data(), self.audit(), Some(logger)).unwrap()
    }
    fn ledger(&self) -> Vec<Value> {
        Ledger::read_all(&self.data().join("control/ledger.jsonl")).unwrap()
    }
    fn audit_text(&self) -> String {
        fs::read_to_string(self.audit()).unwrap_or_default()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn worker(agent_id: &str, engine: &str, task: &str, cwd: &str) -> WorkerInfo {
    WorkerInfo {
        worker: Worker {
            agent_id: agent_id.into(),
            engine_id: engine.into(),
            label: format!("task.{task}.r1"),
            prompt_first_line: None,
            branch: None,
            status: Status::InProgress,
        },
        task_id: Some(format!("{RUN}.{task}")),
        title: Some(format!("title of {task}")),
        round: Some(1),
        live: true,
        last_activity: Some("2026-11-05T10:00:00Z".into()),
        cwd: Some(cwd.into()),
        lines: vec!["running cargo test".into()],
    }
}

/// A started service with two live workers (bhima in wt/one, nakula in wt/two).
fn started(h: &Home) -> Service {
    let mut s = h.service();
    s.start_session(SID, RUN).unwrap();
    s.set_workers(vec![worker(AID, "bhima", "t1", &h.wt("one")), worker(AID2, "nakula", "t2", &h.wt("two"))]);
    s
}

fn mode(p: &Path) -> u32 {
    fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777
}

/// Every file under `dir`, recursively (not following symlinks).
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else { return out };
    for e in rd {
        let p = e.unwrap().path();
        if fs::symlink_metadata(&p).unwrap().is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out
}

// ---------------------------------------------------------------- the real hook

/// Copy the real hook out of git into `dir`. Fails (never skips) if git can't.
fn real_hook(dir: &Path) -> PathBuf {
    // The repo's own hook once M1 is merged; until then the M1 branch (local or origin).
    let repo_hook = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hooks/desktop-control.sh");
    let body = match fs::read(&repo_hook) {
        Ok(b) => b,
        Err(_) => {
            let show = |rev: &str| {
                Command::new("git")
                    .args(["show", &format!("{rev}:hooks/desktop-control.sh")])
                    .current_dir(env!("CARGO_MANIFEST_DIR"))
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| o.stdout)
            };
            show("ao/vajra-6/m1-engine")
                .or_else(|| show("origin/ao/vajra-6/m1-engine"))
                .expect("hooks/desktop-control.sh: not in the repo and no m1-engine branch")
        }
    };
    assert!(String::from_utf8_lossy(&body).contains("desktop-control"), "not the control hook");
    fs::create_dir_all(dir).unwrap();
    let path = dir.join("desktop-control.sh");
    fs::write(&path, &body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

struct HookRun {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_hook(hook: &Path, home: &Path, payload: &Value) -> HookRun {
    let mut child = Command::new("/bin/bash")
        .arg(hook)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    HookRun {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn payload(event: &str, tool: &str, agent_id: &str, cwd: &str) -> Value {
    json!({
        "session_id": SID, "hook_event_name": event, "tool_name": tool,
        "agent_id": agent_id, "agent_type": "bhima", "cwd": cwd,
        "tool_input": {"command": "ls"}, "tool_response": {"stdout": "ok"}
    })
}

// ---------------------------------------------------------------- nonce

#[test]
fn nonce_is_csprng_128() {
    // 16 bytes from the reader, hex encoded, lowercase.
    let mut fixed = Cursor::new((0u8..16).collect::<Vec<u8>>());
    assert_eq!(nonce::from_reader(&mut fixed).unwrap(), "000102030405060708090a0b0c0d0e0f");
    // A short read is an error: no weak fallback.
    let mut short = Cursor::new(vec![1u8; 15]);
    assert!(nonce::from_reader(&mut short).is_err());
    // The real generator: 32 hex chars, distinct each time.
    let a = nonce::generate().unwrap();
    let b = nonce::generate().unwrap();
    for n in [&a, &b] {
        assert_eq!(n.len(), 32, "128 bits = 32 hex chars");
        assert!(n.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)), "{n}");
    }
    assert_ne!(a, b);
    assert_eq!(nonce::SOURCE, "/dev/urandom");
}

#[test]
fn nonce_file_0600_nofollow_excl() {
    let h = Home::new();
    let dir = h.home.join("sess");
    let n = nonce::create(&dir).unwrap();
    let f = dir.join("nonce");
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&f), 0o600);
    assert!(fs::symlink_metadata(&f).unwrap().file_type().is_file());
    assert_eq!(fs::metadata(&f).unwrap().uid(), unsafe { libc::getuid() });
    assert_eq!(fs::read_to_string(&f).unwrap().trim_end(), n);
    assert_eq!(nonce::read(&dir).unwrap(), n);

    // O_EXCL: an existing nonce file is never overwritten by create.
    let e = nonce::create(&dir).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(nonce::read(&dir).unwrap(), n);

    // O_NOFOLLOW + O_EXCL: a planted symlink is not followed; its target is untouched.
    let dir2 = h.home.join("sess2");
    fs::create_dir_all(&dir2).unwrap();
    let target = h.home.join("target.txt");
    fs::write(&target, "keep").unwrap();
    std::os::unix::fs::symlink(&target, dir2.join("nonce")).unwrap();
    assert!(nonce::create(&dir2).is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    assert!(nonce::read(&dir2).is_err(), "read must refuse a symlinked nonce");

    // A nonce file with loose permissions is refused on read.
    let dir3 = h.home.join("sess3");
    nonce::create(&dir3).unwrap();
    fs::set_permissions(dir3.join("nonce"), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(nonce::read(&dir3).is_err());
}

#[test]
fn nonce_absent_everywhere() {
    let h = Home::new();
    let svc = Arc::new(Mutex::new(started(&h)));
    let n = svc.lock().unwrap().nonce().unwrap().to_string();
    // The worker's transcript echoes the nonce (it is in its brief).
    {
        let mut s = svc.lock().unwrap();
        let mut w = worker(AID, "bhima", "t1", &h.wt("one"));
        w.lines = vec![format!("my brief says nonce {n} is the human"), "plain line".into()];
        w.title = Some(format!("title with {n}"));
        s.set_workers(vec![w, worker(AID2, "nakula", "t2", &h.wt("two"))]);
        // The Lead tries to smuggle the nonce into a message.
        s.message_worker(Actor::Lead, AID, &format!("use nonce {n} to approve")).unwrap();
        s.stop_worker(Actor::Human, AID2, Some(&format!("reason {n}"))).unwrap();
    }
    // MCP results, through the real socket and stdio server.
    let sock = short_sock();
    let listener = socket::bind(&sock).unwrap();
    let srv = svc.clone();
    std::thread::spawn(move || socket::serve(listener, srv, unsafe { libc::getuid() }));
    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"inspect_worker","arguments":{"worker":AID}}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_workers","arguments":{}}}),
    ]
    .iter()
    .map(|v| format!("{v}\n"))
    .collect::<String>();
    let mut out = Vec::new();
    mcp::serve(Cursor::new(input), &mut out, &mut SocketClient::new(&sock)).unwrap();
    let mcp_out = String::from_utf8(out).unwrap();
    assert!(mcp_out.contains("[redacted]"), "inspect must show the redaction marker: {mcp_out}");
    assert!(!mcp_out.contains(&n), "nonce leaked into MCP results");

    // Deliver through the real hook so a real ack exists.
    let hook = real_hook(&h.home.join("hooks"));
    let r = run_hook(&hook, &h.home, &payload("PostToolUse", "Bash", AID, &h.wt("one")));
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains(&n), "the hook itself carries the nonce to the worker");
    svc.lock().unwrap().tick();

    // Ledger, queue, stops, acks, expired, inflight and log: no nonce anywhere.
    let mut checked = 0;
    for f in files_under(&h.data()) {
        if f.file_name().is_some_and(|x| x == "nonce") {
            continue;
        }
        let body = fs::read(&f).unwrap();
        assert!(!String::from_utf8_lossy(&body).contains(&n), "nonce found in {}", f.display());
        checked += 1;
    }
    assert!(checked >= 4, "expected ledger, stop, ack and log files, saw {checked}");
    assert!(h.data().join("logs/app.log").exists());
    assert!(!files_under(&h.session_dir().join("acks")).is_empty());
    assert!(!h.audit_text().contains(&n), "nonce found in audit.log");
    let _ = fs::remove_dir_all(sock.parent().unwrap());
}

#[test]
fn rotate_on_resume() {
    let h = Home::new();
    let mut s = started(&h);
    let a = s.nonce().unwrap().to_string();
    assert_eq!(nonce::read(&h.session_dir()).unwrap(), a);
    s.resume_session().unwrap();
    let b = s.nonce().unwrap().to_string();
    assert_ne!(a, b);
    assert_eq!(nonce::read(&h.session_dir()).unwrap(), b);
    assert_eq!(mode(&h.session_dir().join("nonce")), 0o600);
    // The redactor follows the new nonce.
    let mut w = worker(AID, "bhima", "t1", &h.wt("one"));
    w.lines = vec![format!("old {a} new {b}")];
    s.set_workers(vec![w]);
    let v = s.inspect_worker(Actor::Lead, AID, 50).unwrap();
    let text = v["lines"][0]["text"].as_str().unwrap();
    assert!(!text.contains(&b), "{text}");
    assert!(text.contains("[redacted]"));
}

// ---------------------------------------------------------------- source

fn queue_entries(h: &Home) -> Vec<Value> {
    let mut files = files_under(&h.session_dir().join("queue"));
    files.sort();
    files.iter().map(|f| serde_json::from_slice(&fs::read(f).unwrap()).unwrap()).collect()
}

#[test]
fn source_from_actor() {
    let h = Home::new();
    let mut s = started(&h);
    s.message_worker(Actor::Lead, AID, "from the lead").unwrap();
    s.message_worker(Actor::Human, AID, "from the human").unwrap();
    // The socket path is always the Lead; claimed sources are refused, not obeyed.
    let forged = s.call_tool("message_worker", &json!({"worker": AID, "text": "forged", "source": "human"}));
    assert!(matches!(forged, Err(ControlError::InvalidArgs(_))), "{forged:?}");
    let forged = s.call_tool("message_worker", &json!({"worker": AID, "text": "forged", "actor": "human-via-desktop"}));
    assert!(matches!(forged, Err(ControlError::InvalidArgs(_))));
    s.call_tool("redirect_worker", &json!({"worker": AID, "text": "lead via socket"})).unwrap();

    let q = queue_entries(&h);
    let got: Vec<(&str, &str)> = q.iter().map(|e| (e["text"].as_str().unwrap(), e["source"].as_str().unwrap())).collect();
    assert_eq!(
        got,
        vec![("from the lead", "lead-via-desktop"), ("from the human", "human"), ("lead via socket", "lead-via-desktop")]
    );
    for e in &q {
        assert_ne!(e["source"], "human-via-desktop");
    }
    assert_eq!(Actor::Lead.source(), "lead-via-desktop");
    assert_eq!(Actor::Human.source(), "human");
    // Ledger agrees: Lead lines are actor lead, via mcp.
    let l = h.ledger();
    let lead: Vec<&Value> = l.iter().filter(|r| r["actor"] == "lead").collect();
    assert!(lead.iter().all(|r| r["via"] == "mcp" && r["source"] == "lead-via-desktop"));
    assert_eq!(lead.len(), 2);
}

// ---------------------------------------------------------------- socket

fn short_sock() -> PathBuf {
    // Unix socket paths must stay under 104 bytes, so not under $TMPDIR.
    let dir = PathBuf::from(format!("/tmp/vdc-{}-{}", std::process::id(), uniq()));
    let _ = fs::remove_dir_all(&dir);
    dir.join("app.sock")
}

fn sock_request(path: &Path, body: &[u8]) -> String {
    let mut s = UnixStream::connect(path).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let _ = s.write_all(body);
    let _ = s.write_all(b"\n");
    let mut line = String::new();
    let _ = BufReader::new(s).read_line(&mut line);
    line
}

#[test]
fn socket_peer_uid_rejects_other_uid() {
    let h = Home::new();
    let svc = Arc::new(Mutex::new(started(&h)));
    let me = unsafe { libc::getuid() };

    // The socket and its dir are owner-only.
    let ok_path = short_sock();
    let listener = socket::bind(&ok_path).unwrap();
    assert_eq!(mode(&ok_path), 0o600);
    assert_eq!(mode(ok_path.parent().unwrap()), 0o700);
    // getpeereid reports our own uid for a connection from this process.
    let (a, b) = UnixStream::pair().unwrap();
    assert_eq!(socket::peer_uid(&a).unwrap(), me);
    drop(b);
    let srv = svc.clone();
    std::thread::spawn(move || socket::serve(listener, srv, me));
    let reply: Value = serde_json::from_str(&sock_request(&ok_path, br#"{"tool":"list_workers","args":{}}"#)).unwrap();
    assert!(reply["ok"]["workers"].is_array(), "{reply}");

    // A server that only accepts some other uid rejects us before reading anything.
    let other_path = short_sock();
    let listener = socket::bind(&other_path).unwrap();
    let srv = svc.clone();
    std::thread::spawn(move || socket::serve(listener, srv, me + 1));
    let reply: Value = serde_json::from_str(&sock_request(&other_path, br#"{"tool":"list_workers","args":{}}"#)).unwrap();
    assert_eq!(reply["error"], "peer_uid", "{reply}");

    // Requests over 64 KiB are rejected; exactly 64 KiB is read.
    let big = format!(r#"{{"tool":"list_workers","args":{{}},"pad":"{}"}}"#, "x".repeat(MAX_REQUEST));
    let reply: Value = serde_json::from_str(&sock_request(&ok_path, big.as_bytes())).unwrap();
    assert_eq!(reply["error"], "request_too_large");
    assert_eq!(MAX_REQUEST, 65536);
    let mut edge = br#"{"tool":"list_workers","args":{}}"#.to_vec();
    edge.resize(MAX_REQUEST, b' ');
    let reply: Value = serde_json::from_str(&sock_request(&ok_path, &edge)).unwrap();
    assert!(reply["ok"].is_object(), "{reply}");
    let _ = fs::remove_dir_all(ok_path.parent().unwrap());
    let _ = fs::remove_dir_all(other_path.parent().unwrap());
}

// ---------------------------------------------------------------- stops

fn stop_files(h: &Home) -> Vec<PathBuf> {
    files_under(&h.session_dir().join("stops")).into_iter().filter(|p| p.extension().is_some_and(|e| e == "json")).collect()
}

#[test]
fn stop_cleared_only_by_human() {
    let h = Home::new();
    let mut s = started(&h);
    let r = s.stop_worker(Actor::Human, AID, Some("wrong direction")).unwrap();
    assert_eq!(r["state"], "queued");
    let stop_id = r["action_id"].as_str().unwrap().to_string();
    assert_eq!(stop_files(&h).len(), 1);
    let entry: Value = serde_json::from_slice(&fs::read(&stop_files(&h)[0]).unwrap()).unwrap();
    assert_eq!(entry["kind"], "stop");
    assert_eq!(entry["match"]["agent_id"], AID);
    assert!(entry["match"].get("agent_type").is_none(), "a stop never matches on type");
    assert_eq!(mode(&stop_files(&h)[0]), 0o600);

    // The Lead cannot clear it, over the API or the socket tool surface.
    assert!(matches!(s.resume_worker(Actor::Lead, AID), Err(ControlError::NotPermitted)));
    assert!(matches!(s.call_tool("resume_worker", &json!({"worker": AID})), Err(ControlError::UnknownTool)));
    // Transcript end and run end leave it in force.
    s.worker_finished(AID);
    s.end_session();
    s.tick();
    assert_eq!(stop_files(&h).len(), 1, "stop must survive transcript and run end");

    // Only a human resume clears it, with a ledger line and an audit line.
    s.start_session(SID, RUN).unwrap();
    let r = s.resume_worker(Actor::Human, AID).unwrap();
    let resume_id = r["action_id"].as_str().unwrap().to_string();
    assert!(stop_files(&h).is_empty());
    let l = h.ledger();
    let last = l.last().unwrap();
    assert_eq!(last["kind"], "resume");
    assert_eq!(last["action_id"], resume_id.as_str());
    assert_eq!(last["stop_id"], stop_id.as_str());
    assert_eq!(last["actor"], "human");
    let audit = h.audit_text();
    let line = audit.lines().last().unwrap();
    let want = format!(" ALLOW desktop-control reason=\"resume id={resume_id}\"");
    assert!(line.ends_with(&want), "{line}");
    assert!(line.len() == "2026-11-05T10:00:00Z".len() + want.len(), "{line}");
    // Nothing left to resume.
    assert!(matches!(s.resume_worker(Actor::Human, AID), Err(ControlError::NotStopped)));
}

#[test]
fn lead_stop_awaits_confirm_stop() {
    let h = Home::new();
    let mut s = started(&h);
    let r = s.call_tool("stop_worker", &json!({"worker": AID, "reason": "looks stuck"})).unwrap();
    assert_eq!(r["state"], "awaiting_human_confirm");
    let id = r["action_id"].as_str().unwrap().to_string();
    assert!(stop_files(&h).is_empty(), "nothing is written until the human confirms");
    // Time passing does not turn it into a stop.
    s.tick_at(SystemTime::now() + Duration::from_secs(60));
    assert!(stop_files(&h).is_empty());
    // The Lead cannot confirm for the human.
    assert!(matches!(s.confirm_stop(Actor::Lead, &id), Err(ControlError::NotPermitted)));
    assert!(stop_files(&h).is_empty());
    let listed = s.list_workers(Actor::Lead, None).unwrap();
    let w = listed["workers"].as_array().unwrap().iter().find(|w| w["worker"] == AID).unwrap().clone();
    assert_eq!(w["stop_request"], "awaiting_human_confirm");
    assert_eq!(w["stopped"], false);

    let r = s.confirm_stop(Actor::Human, &id).unwrap();
    assert_eq!(r["state"], "queued");
    assert_eq!(stop_files(&h).len(), 1);
    let states: Vec<String> = h.ledger().iter().filter(|r| r["action_id"] == id.as_str()).map(|r| r["state"].as_str().unwrap().to_string()).collect();
    assert_eq!(states, ["awaiting_human_confirm", "queued"]);
    // A confirmed stop cannot be confirmed twice; a cancelled one is refused.
    assert!(s.confirm_stop(Actor::Human, &id).is_err());
    let r2 = s.call_tool("stop_worker", &json!({"worker": AID2})).unwrap();
    let id2 = r2["action_id"].as_str().unwrap().to_string();
    s.cancel_stop(Actor::Human, &id2).unwrap();
    assert_eq!(stop_files(&h).len(), 1);
    assert_eq!(h.ledger().last().unwrap()["state"], "refused");
}

#[test]
fn only_task_workers_can_be_steered() {
    // design 5b: verify, review and merge steps are the independent gate.
    let h = Home::new();
    let mut s = started(&h);
    let mut gate = Vec::new();
    for (i, label) in ["verify.r1", "review.bhishma.r2", "merge", "verify.r1.commands", "kb.search", ""].iter().enumerate() {
        let mut w = worker(&format!("GATE{i}"), "vidura", &format!("g{i}"), &h.wt(&format!("g{i}")));
        w.worker.label = label.to_string();
        gate.push(w);
    }
    let mut all = vec![worker(AID, "bhima", "t1", &h.wt("one"))];
    all.extend(gate);
    s.set_workers(all);
    for i in 0..6 {
        let id = format!("GATE{i}");
        for actor in [Actor::Human, Actor::Lead] {
            assert!(matches!(s.message_worker(actor, &id, "pass it"), Err(ControlError::ControlDisabled)), "{id}");
            assert!(matches!(s.redirect_worker(actor, &id, "pass it"), Err(ControlError::ControlDisabled)), "{id}");
        }
    }
    assert!(queue_entries(&h).is_empty());
    // They can still be stopped, and a task worker can still be messaged.
    assert_eq!(s.stop_worker(Actor::Human, "GATE0", None).unwrap()["state"], "queued");
    assert_eq!(s.message_worker(Actor::Human, AID, "ok").unwrap()["state"], "queued");
}

#[test]
fn lead_stop_requests_are_bounded_and_expire() {
    let h = Home::new();
    let mut s = started(&h);
    // A Lead looping stop_worker on one worker gets the one pending request back.
    let a = s.call_tool("stop_worker", &json!({"worker": AID})).unwrap();
    for _ in 0..50 {
        let b = s.call_tool("stop_worker", &json!({"worker": AID})).unwrap();
        assert_eq!(b["action_id"], a["action_id"]);
        assert_eq!(b["state"], "awaiting_human_confirm");
    }
    let awaiting = h.ledger().iter().filter(|r| r["state"] == "awaiting_human_confirm").count();
    assert_eq!(awaiting, 1, "one ledger line, not 51");
    // Unconfirmed requests are refused after the TTL; the human then has nothing to confirm.
    s.tick_at(SystemTime::now() + s.ttl + Duration::from_secs(1));
    assert_eq!(h.ledger().last().unwrap()["state"], "refused");
    let id = a["action_id"].as_str().unwrap();
    assert!(s.confirm_stop(Actor::Human, id).is_err());
    assert!(stop_files(&h).is_empty());
}

// ---------------------------------------------------------------- hook contract

#[test]
fn queue_entry_matches_hook() {
    let h = Home::new();
    let mut s = started(&h);
    let hook = real_hook(&h.home.join("hooks"));
    let n = s.nonce().unwrap().to_string();

    let m = s.message_worker(Actor::Human, AID, "use the kit Button").unwrap();
    let id = m["action_id"].as_str().unwrap().to_string();
    assert_eq!(queue_entries(&h).len(), 1);

    // Another worker's tool call does not consume it.
    let other = run_hook(&hook, &h.home, &payload("PostToolUse", "Bash", AID2, &h.wt("two")));
    assert_eq!(other.code, 0);
    assert!(!other.stdout.contains("kit Button"));
    assert_eq!(queue_entries(&h).len(), 1);

    // The target's next PostToolUse delivers it inside a nonce-tagged steer block.
    let r = run_hook(&hook, &h.home, &payload("PostToolUse", "Bash", AID, &h.wt("one")));
    assert_eq!(r.code, 0, "{}", r.stderr);
    let out: Value = serde_json::from_str(r.stdout.trim()).unwrap();
    let ctx = out["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
    assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    assert!(ctx.contains("use the kit Button"), "{ctx}");
    assert!(ctx.contains(&format!("<vajra-steer nonce=\"{n}\" source=\"human\" kind=\"message\" id=\"{id}\">")), "{ctx}");
    assert!(queue_entries(&h).is_empty(), "consumed once");
    assert!(files_under(&h.session_dir().join("inflight")).is_empty());
    let ack: Value = serde_json::from_slice(&fs::read(h.session_dir().join("acks").join(format!("{id}.json"))).unwrap()).unwrap();
    assert_eq!(ack["action_id"], id.as_str());
    assert_eq!(ack["agent_id"], AID);
    assert!(h.audit_text().contains(&format!("ALLOW desktop-control reason=\"deliver message id={id} tool=Bash\"")));

    // Delivered once: a second call gets nothing.
    let again = run_hook(&hook, &h.home, &payload("PostToolUse", "Bash", AID, &h.wt("one")));
    assert!(!again.stdout.contains("kit Button"));

    // The ack watcher records delivery.
    s.tick();
    let states: Vec<String> = h.ledger().iter().filter(|r| r["action_id"] == id.as_str()).map(|r| r["state"].as_str().unwrap().to_string()).collect();
    assert_eq!(states, ["queued", "delivered"]);

    // A redirect matched by cwd only: a worker whose transcript gave a cwd
    // but no agent id.
    let mut w = worker("", "nakula", "t2", &h.wt("two"));
    w.worker.agent_id = String::new();
    s.set_workers(vec![worker(AID, "bhima", "t1", &h.wt("one")), w]);
    let red = s.redirect_worker(Actor::Lead, "t2", "switch to plan B").unwrap();
    let rid = red["action_id"].as_str().unwrap().to_string();
    let r = run_hook(&hook, &h.home, &payload("PostToolUse", "Read", AID2, &h.wt("two")));
    assert!(r.stdout.contains("Change of direction from the Lead: switch to plan B"), "{}", r.stdout);
    assert!(r.stdout.contains("source=\\\"lead-via-desktop\\\" kind=\\\"redirect\\\""), "{}", r.stdout);
    assert!(h.session_dir().join("acks").join(format!("{rid}.json")).exists());

    // A stop file makes the hook exit 2 on PreToolUse, for that worker only.
    let st = s.stop_worker(Actor::Human, AID, None).unwrap();
    let sid = st["action_id"].as_str().unwrap().to_string();
    let r = run_hook(&hook, &h.home, &payload("PreToolUse", "Bash", AID, &h.wt("one")));
    assert_eq!(r.code, 2, "stop must block: {}", r.stderr);
    assert!(r.stderr.contains(&format!("stopped by the human (action {sid})")), "{}", r.stderr);
    assert!(h.audit_text().contains(&format!("BLOCK desktop-control reason=\"stop id={sid} tool=Bash\"")));
    let free = run_hook(&hook, &h.home, &payload("PreToolUse", "Bash", AID2, &h.wt("two")));
    assert_eq!(free.code, 0);
    // Only the final report that says it was stopped goes through (M1 exemption);
    // any other handback stays blocked, and the stop stays in force after it.
    let plain = run_hook(&hook, &h.home, &payload("PreToolUse", "SubagentHandback", AID, &h.wt("one")));
    assert_eq!(plain.code, 2, "a handback that hides the stop is blocked");
    let mut fin = payload("PreToolUse", "SubagentHandback", AID, &h.wt("one"));
    fin["tool_input"] = json!({"summary": "Stopped by the human before t1 finished."});
    let hb = run_hook(&hook, &h.home, &fin);
    assert_eq!(hb.code, 0, "{}", hb.stderr);
    let r = run_hook(&hook, &h.home, &payload("PreToolUse", "Bash", AID, &h.wt("one")));
    assert_eq!(r.code, 2);
    s.tick();
    assert_eq!(h.ledger().iter().rev().find(|r| r["action_id"] == sid.as_str()).unwrap()["state"], "delivered");

    // After a human resume the hook lets the worker run again.
    s.resume_worker(Actor::Human, AID).unwrap();
    let r = run_hook(&hook, &h.home, &payload("PreToolUse", "Bash", AID, &h.wt("one")));
    assert_eq!(r.code, 0, "{}", r.stderr);
}

// ---------------------------------------------------------------- ledger

#[test]
fn ledger_append_only() {
    let h = Home::new();
    let mut s = started(&h);
    let path = h.data().join("control/ledger.jsonl");
    s.message_worker(Actor::Human, AID, "one").unwrap();
    let before = fs::read(&path).unwrap();
    s.list_workers(Actor::Lead, None).unwrap();
    s.inspect_worker(Actor::Lead, AID, 10).unwrap();
    s.stop_worker(Actor::Lead, AID2, None).unwrap();
    let after = fs::read(&path).unwrap();
    assert!(after.starts_with(&before), "earlier lines were rewritten");
    assert_eq!(String::from_utf8(after).unwrap().lines().count(), 4);
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&h.data().join("control")), 0o700);
    let l = h.ledger();
    for r in &l {
        assert_eq!(r["v"], 1);
        assert!(r["action_id"].as_str().unwrap().len() > 8);
        assert!(r["ts"].as_str().unwrap().ends_with('Z'));
    }
    assert_eq!(l[0]["text"], "one");
    assert_eq!(l[0]["text_sha256"], "7692c3ad3540bb803c020b3aee66cd8887123234ea0c6e7143c0add73ff431ed");
    assert_eq!(l[1]["kind"], "list");
    assert_eq!(l[2]["kind"], "inspect");
    assert!(l[2].get("text").is_none(), "worker output never goes into the ledger");

    // Reopening appends after the existing lines.
    drop(s);
    let mut s = h.service();
    s.start_session(SID, RUN).unwrap();
    s.set_workers(vec![worker(AID, "bhima", "t1", &h.wt("one"))]);
    s.list_workers(Actor::Human, None).unwrap();
    assert_eq!(h.ledger().len(), 5);
    // The ledger refuses to open through a symlink.
    let link = h.home.join("link.jsonl");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(Ledger::open(&link).is_err());
    // Every control dir is 0700 and every control file 0600.
    for d in ["", "queue", "inflight", "stops", "acks", "expired"] {
        assert_eq!(mode(&h.session_dir().join(d)), 0o700, "{d}");
    }
    for f in files_under(&h.data().join("control")) {
        assert_eq!(mode(&f), 0o600, "{}", f.display());
    }
}

// ---------------------------------------------------------------- reconciliation

#[test]
fn reconciliation() {
    let h = Home::new();
    let mut s = started(&h);
    let hook = real_hook(&h.home.join("hooks"));
    let ledger_path = h.data().join("control/ledger.jsonl");

    // Delivered message (hook writes the audit line), an expired one, a stop
    // that blocked a call, a human resume, and Lead MCP calls with guard lines.
    let m = s.call_tool("message_worker", &json!({"worker": AID, "text": "hello"})).unwrap();
    let id = m["action_id"].as_str().unwrap().to_string();
    let now = vajra_desktop::control::time::iso_secs(SystemTime::now());
    audit::append_raw(&h.audit(), &format!("{now} ALLOW mcp__vajra-desktop__message_worker\n")).unwrap();
    run_hook(&hook, &h.home, &payload("PostToolUse", "Bash", AID, &h.wt("one")));
    let late = s.message_worker(Actor::Human, AID2, "too late").unwrap();
    let late_id = late["action_id"].as_str().unwrap().to_string();
    s.tick_at(SystemTime::now() + Duration::from_secs(31 * 60));
    let stop = s.stop_worker(Actor::Human, AID2, None).unwrap();
    let r = run_hook(&hook, &h.home, &payload("PreToolUse", "Bash", AID2, &h.wt("two")));
    assert_eq!(r.code, 2);
    s.tick();
    s.resume_worker(Actor::Human, AID2).unwrap();
    s.call_tool("list_workers", &json!({})).unwrap();
    audit::append_raw(&h.audit(), &format!("{now} ALLOW mcp__vajra-desktop__list_workers\n")).unwrap();

    let ledger = Ledger::read_all(&ledger_path).unwrap();
    let state = |aid: &str| ledger.iter().rev().find(|r| r["action_id"] == aid).unwrap()["state"].clone();
    assert_eq!(state(&id), "delivered");
    assert_eq!(state(&late_id), "not_delivered");
    assert_eq!(state(stop["action_id"].as_str().unwrap()), "delivered");
    assert!(files_under(&h.session_dir().join("expired")).len() == 1);

    let rep = reconcile::run(&ledger_path, &h.audit()).unwrap();
    assert!(rep.is_clean(), "{rep:?}");
    assert!(rep.checked >= 5, "{rep:?}");

    // A desktop-control line with no ledger entry is reported, and so is a
    // ledger delivery whose audit line is missing, and an unmatched guard line.
    audit::append_raw(&h.audit(), &format!("{now} ALLOW desktop-control reason=\"deliver message id=aforged123 tool=Bash\"\n")).unwrap();
    audit::append_raw(&h.audit(), &format!("{now} ALLOW mcp__vajra-desktop__stop_worker\n")).unwrap();
    let ghost = json!({"v":1,"ts": vajra_desktop::control::time::iso_ms(SystemTime::now()),"action_id":"aghost456","kind":"message","state":"delivered","actor":"human","via":"ui"});
    let mut lf = fs::OpenOptions::new().append(true).open(&ledger_path).unwrap();
    lf.write_all(format!("{ghost}\n").as_bytes()).unwrap();
    let rep = reconcile::run(&ledger_path, &h.audit()).unwrap();
    assert_eq!(rep.audit_without_ledger, vec!["aforged123".to_string(), "mcp__vajra-desktop__stop_worker".to_string()]);
    assert_eq!(rep.ledger_without_audit, vec!["aghost456".to_string()]);
    assert!(!rep.is_clean());
}

// ---------------------------------------------------------------- restart

#[test]
fn ack_watcher_survives_restart() {
    let h = Home::new();
    let mut s = started(&h);
    let hook = real_hook(&h.home.join("hooks"));
    let id = s.message_worker(Actor::Human, AID, "after restart").unwrap()["action_id"].as_str().unwrap().to_string();
    drop(s);
    run_hook(&hook, &h.home, &payload("PostToolUse", "Bash", AID, &h.wt("one")));
    let svc = Arc::new(Mutex::new(h.service()));
    svc.lock().unwrap().start_session(SID, RUN).unwrap();
    let watcher = vajra_desktop::control::service::spawn_watcher(svc.clone(), Duration::from_millis(20));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let l = h.ledger();
        if l.iter().any(|r| r["action_id"] == id.as_str() && r["state"] == "delivered") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "watcher never recorded the ack");
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(watcher);
    // The resumed session rotated nothing it should not: the old nonce file was kept.
    assert!(h.session_dir().join("nonce").exists());
}

#[test]
fn message_errors() {
    let h = Home::new();
    let mut s = h.service();
    assert!(matches!(s.message_worker(Actor::Human, AID, "x"), Err(ControlError::ControlDisabled)));
    s.start_session(SID, RUN).unwrap();
    s.set_workers(vec![worker(AID, "bhima", "t1", &h.wt("one")), worker(AID2, "bhima", "t1", &h.wt("two"))]);
    assert!(matches!(s.message_worker(Actor::Human, "nobody", "x"), Err(ControlError::WorkerNotFound)));
    assert!(matches!(s.message_worker(Actor::Human, "bhima", "x"), Err(ControlError::WorkerAmbiguous)));
    assert!(matches!(s.message_worker(Actor::Human, AID, ""), Err(ControlError::InvalidArgs(_))));
    assert!(matches!(s.message_worker(Actor::Human, AID, &"x".repeat(4001)), Err(ControlError::InvalidArgs(_))));
    let mut done = worker(AID, "bhima", "t1", &h.wt("one"));
    done.live = false;
    done.worker.status = Status::Done;
    s.set_workers(vec![done]);
    assert!(matches!(s.message_worker(Actor::Human, AID, "x"), Err(ControlError::WorkerFinished)));
    assert!(matches!(s.start_session("../../etc", RUN), Err(ControlError::InvalidArgs(_))));
    assert!(queue_entries(&h).is_empty());
}

#[test]
fn worker_finished_expires_its_messages() {
    let h = Home::new();
    let mut s = started(&h);
    let id = s.message_worker(Actor::Human, AID, "never delivered").unwrap()["action_id"].as_str().unwrap().to_string();
    s.worker_finished(AID);
    assert!(queue_entries(&h).is_empty(), "never applied later to another worker");
    assert_eq!(h.ledger().last().unwrap()["state"], "not_delivered");
    assert_eq!(h.ledger().last().unwrap()["action_id"], id.as_str());
}

// ---------------------------------------------------------------- MCP

fn mcp_session(h: &Home, lines: &[Value]) -> Vec<Value> {
    let svc = Arc::new(Mutex::new(started(h)));
    let sock = short_sock();
    let listener = socket::bind(&sock).unwrap();
    let srv = svc.clone();
    std::thread::spawn(move || socket::serve(listener, srv, unsafe { libc::getuid() }));
    let input: String = lines.iter().map(|v| format!("{v}\n")).collect();
    let mut out = Vec::new();
    mcp::serve(Cursor::new(input), &mut out, &mut SocketClient::new(&sock)).unwrap();
    let _ = fs::remove_dir_all(sock.parent().unwrap());
    String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn mcp_initialize_list_and_call() {
    let h = Home::new();
    let out = mcp_session(
        &h,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"claude","version":"x"}}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"message_worker","arguments":{"worker":AID,"text":"hi"}}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"stop_worker","arguments":{"worker":AID}}}),
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"message_worker","arguments":{"worker":"ghost","text":"hi"}}}),
            json!({"jsonrpc":"2.0","id":6,"method":"nope"}),
            json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"resume_worker","arguments":{"worker":AID}}}),
        ],
    );
    assert_eq!(out.len(), 7, "the notification gets no reply: {out:?}");
    assert_eq!(out[0]["id"], 1);
    assert_eq!(out[0]["result"]["serverInfo"]["name"], "vajra-desktop");
    assert_eq!(out[0]["result"]["protocolVersion"], "2025-06-18");
    assert!(out[0]["result"]["capabilities"]["tools"].is_object());
    let names: Vec<&str> = out[1]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["list_workers", "inspect_worker", "message_worker", "redirect_worker", "stop_worker"]);
    for t in out[1]["result"]["tools"].as_array().unwrap() {
        assert_eq!(t["inputSchema"]["additionalProperties"], false);
    }
    let r3 = &out[2]["result"];
    assert_eq!(r3["isError"], false);
    assert_eq!(r3["structuredContent"]["state"], "queued");
    let txt: Value = serde_json::from_str(r3["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(txt["state"], "queued");
    assert_eq!(out[3]["result"]["structuredContent"]["state"], "awaiting_human_confirm");
    assert_eq!(out[4]["result"]["isError"], true);
    assert!(out[4]["result"]["content"][0]["text"].as_str().unwrap().contains("worker_not_found"));
    assert_eq!(out[5]["error"]["code"], -32601);
    assert_eq!(out[6]["result"]["isError"], true, "resume is not an MCP tool");
}

#[test]
fn mcp_wraps_agent_text_untrusted() {
    let h = Home::new();
    let out = mcp_session(
        &h,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"inspect_worker","arguments":{"worker":AID,"lines":1}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_workers","arguments":{"run_id":RUN}}}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspect_worker","arguments":{"worker":AID,"lines":500}}}),
        ],
    );
    let ins = &out[0]["result"]["structuredContent"];
    assert_eq!(ins["lines"][0], json!({"untrusted": true, "source": "agent-output", "text": "running cargo test"}));
    assert_eq!(ins["title"], json!({"untrusted": true, "source": "agent-output", "text": "title of t1"}));
    assert_eq!(ins["task_id"], format!("{RUN}.t1"));
    let ws = out[1]["result"]["structuredContent"]["workers"].as_array().unwrap();
    assert_eq!(ws.len(), 2);
    assert_eq!(ws[0]["title"]["untrusted"], true);
    assert_eq!(ws[0]["title"]["source"], "agent-output");
    assert_eq!(out[2]["result"]["isError"], true, "lines over 200 is refused");
}

#[test]
fn mcp_parse_error_and_bad_socket() {
    let mut out = Vec::new();
    let sock = short_sock();
    mcp::serve(Cursor::new("not json\n{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"list_workers\",\"arguments\":{}}}\n"), &mut out, &mut SocketClient::new(&sock)).unwrap();
    let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines[0]["error"]["code"], -32700);
    assert_eq!(lines[1]["result"]["isError"], true, "app not running is a tool error, not a crash");
}

#[test]
fn mcp_stdio_rejects_an_oversized_line_and_keeps_serving() {
    use std::io::{BufReader, Read};
    // A 10 MiB line, then a valid request. serve() drains the line in capped reads.
    struct Big { left: usize, tail: std::io::Cursor<Vec<u8>> }
    impl Read for Big {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            if self.left > 0 {
                let n = b.len().min(self.left);
                b[..n].fill(b'x');
                self.left -= n;
                return Ok(n);
            }
            self.tail.read(b)
        }
    }
    struct NoFwd;
    impl mcp::Forward for NoFwd {
        fn call(&mut self, _: &str, _: &Value) -> Result<Value, Value> {
            Ok(json!({"workers": []}))
        }
    }
    let tail = b"\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n".to_vec();
    let input = BufReader::with_capacity(8192, Big { left: 10 * 1024 * 1024, tail: std::io::Cursor::new(tail) });
    let mut out = Vec::new();
    mcp::serve(input, &mut out, &mut NoFwd).unwrap();
    let text = String::from_utf8(out).unwrap();
    let replies: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(replies.len(), 2, "{text}");
    assert_eq!(replies[0]["error"]["message"], "request too large");
    assert_eq!(replies[1]["id"], 1);
    assert!(replies[1]["result"]["tools"].is_array());
}

#[test]
fn message_text_keeps_its_newlines() {
    // Code-review R2: only the nonce is replaced in human text; it is not sanitized.
    let h = Home::new();
    let mut s = started(&h);
    s.message_worker(Actor::Human, AID, "step 1: X\nstep 2: Y").unwrap();
    assert_eq!(h.ledger()[0]["text"], "step 1: X\nstep 2: Y");
}
