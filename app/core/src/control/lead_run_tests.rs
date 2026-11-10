use super::*;
use crate::control::lead::frame;
use crate::crew::redact::Redactor;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicUsize;

const ASK: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const RUN: &str = "20261106-0911";
static N: AtomicUsize = AtomicUsize::new(0);

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vajra-leadrun-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn framed(text: &str) -> FramedAsk {
    frame(ASK, RUN, text, &Redactor::default()).unwrap()
}

/// Write an executable `sh` script and return its path. It is run once (`--warm`, which exits at once) before it is
/// returned: macOS scans a new executable on its first run, which takes long enough to blow a 400 ms deadline.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n[ \"$1\" = --warm ] && exit 0\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _ = Command::new(&p).arg("--warm").env_clear().status();
    p
}

/// Wait (up to 10 s) for a file a fake claude writes once it is really running.
fn wait_for(p: &Path) {
    let t = Instant::now();
    while !p.exists() && t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(p.exists(), "{} never appeared", p.display());
}

pub(crate) fn init_event(tools: &[String], mode: &str) -> String {
    json!({"type": "system", "subtype": "init", "session_id": "0f8fad5b-d9cb-469f-a165-70867728950e", "tools": tools, "permissionMode": mode}).to_string()
}

fn good_init() -> String {
    init_event(&expected_tools(), "dontAsk")
}

const HOOKS: &str = r#"{"type":"system","subtype":"hook_started","hook_name":"SessionStart:startup"}
{"type":"system","subtype":"hook_started","hook_name":"SessionStart:startup"}
{"type":"system","subtype":"hook_response","hook_name":"SessionStart:startup"}
{"type":"system","subtype":"hook_response","hook_name":"SessionStart:startup"}
{"type":"system","subtype":"session_state_changed","state":"running"}"#;

const TAIL: &str = r#"{"type":"rate_limit_event"}
{"type":"assistant","message":{"content":[{"type":"text","text":"line one\nline two"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"line one\nline two","permission_denials":[]}
{"type":"system","subtype":"session_state_changed","state":"idle"}"#;

/// A fake claude that prints `lines` and exits.
fn fake_printing(dir: &Path, lines: &str) -> PathBuf {
    let f = dir.join("events.jsonl");
    std::fs::write(&f, format!("{lines}\n")).unwrap();
    script(dir, "claude", &format!("cat > /dev/null &\ncat '{}'", f.display()))
}

#[derive(Default)]
struct Collect {
    raw: Vec<String>,
    events: Vec<(Tag, String)>,
    inits: Vec<InitInfo>,
}
impl Sink for Collect {
    fn raw(&mut self, l: &str) {
        self.raw.push(l.to_string());
    }
    fn init(&mut self, i: &InitInfo) {
        self.inits.push(i.clone());
    }
    fn event(&mut self, t: Tag, s: &str) {
        self.events.push((t, s.to_string()));
    }
}

fn run(prog: &Path, limits: Limits) -> (Result<Done, LeadError>, Collect, Arc<Handle>) {
    let h = Handle::new();
    let mut sink = Collect::default();
    let child = spawn(&[prog.to_str().unwrap().to_string()], Path::new("/"), &env(), &framed("hi"), &h).unwrap();
    let r = drive(child, &h, limits, &mut sink);
    (r, sink, h)
}

/// What a scrubbed Lead env would hold: a real PATH, so the fake `sh` scripts find `sleep` and `cat`.
fn env() -> Vec<(&'static str, OsString)> {
    vec![("PATH", OsString::from("/usr/bin:/bin"))]
}

fn quick() -> Limits {
    Limits { init: Duration::from_secs(5), total: Duration::from_secs(20) }
}

/// Is any process in group `pgid` still alive?
fn group_alive(pgid: u32) -> bool {
    // SAFETY: signal 0 only checks existence.
    unsafe { libc::kill(-(pgid as i32), 0) == 0 }
}

// ------------------------------------------------------------ the init scan

#[test]
fn init_found_after_hook_events() {
    let d = tmp("order");
    let p = fake_printing(&d, &format!("{HOOKS}\n{}\n{TAIL}", good_init()));
    let (r, sink, _) = run(&p, quick());
    assert_eq!(r.unwrap(), Done::default());
    assert_eq!(sink.inits.len(), 1);
    assert_eq!(sink.inits[0].session_id, "0f8fad5b-d9cb-469f-a165-70867728950e");
    // Hook events are never shown; the reply is, one line each.
    assert_eq!(sink.events, vec![(Tag::Lead, "line one".to_string()), (Tag::Lead, "line two".to_string())]);
}

#[test]
fn init_tools_mismatch_kills() {
    let d = tmp("tools");
    // The spike's 31-tool list, and the process keeps running afterwards: it must be killed.
    let mut all = expected_tools();
    all.extend(["Bash", "Write", "Edit", "Workflow", "WebFetch"].map(String::from));
    let body = format!("sleep 30 &\necho $! > '{}'\necho '{}'\nwait", d.join("child.pid").display(), init_event(&all, "dontAsk"));
    let p = script(&d, "claude", &body);
    let t = Instant::now();
    let (r, sink, h) = run(&p, quick());
    assert_eq!(r.unwrap_err(), LeadError::ToolsMismatch);
    assert!(sink.inits.is_empty() && sink.events.is_empty());
    assert!(t.elapsed() < Duration::from_secs(5), "killed, not waited out");
    let pid: i32 = std::fs::read_to_string(d.join("child.pid")).unwrap().trim().parse().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    // SAFETY: signal 0 only checks existence.
    assert!(unsafe { libc::kill(pid, 0) } != 0, "the whole group is gone");
    assert!(!group_alive(h.pid.load(Ordering::SeqCst)));
    // A missing or malformed tool list is a mismatch too.
    for bad in [r#"{"type":"system","subtype":"init","permissionMode":"dontAsk"}"#, r#"{"type":"system","subtype":"init","tools":[1],"permissionMode":"dontAsk"}"#] {
        let mut s = InitScan::default();
        assert_eq!(s.feed(&serde_json::from_str(bad).unwrap()).unwrap_err(), LeadError::ToolsMismatch);
    }
    // A subset is a mismatch as well: the set must be exact.
    let mut s = InitScan::default();
    let subset = init_event(&["Read".to_string()], "dontAsk");
    assert_eq!(s.feed(&serde_json::from_str(&subset).unwrap()).unwrap_err(), LeadError::ToolsMismatch);
}

#[test]
fn init_mode_mismatch_kills() {
    let d = tmp("mode");
    for mode in ["auto", "default", "bypassPermissions", "acceptEdits", ""] {
        let p = fake_printing(&d, &format!("{HOOKS}\n{}\n{TAIL}", init_event(&expected_tools(), mode)));
        let (r, sink, _) = run(&p, quick());
        assert_eq!(r.unwrap_err(), LeadError::ModeMismatch, "{mode}");
        assert!(sink.events.is_empty(), "nothing is shown from a Lead that failed the check");
    }
}

#[test]
fn no_init_before_assistant_kills() {
    let d = tmp("noinit");
    for first in [
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#,
        r#"{"type":"user","message":{"content":[]}}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"x"}"#,
    ] {
        // The init that follows is too late: the answer came first.
        let p = fake_printing(&d, &format!("{HOOKS}\n{first}\n{}\n{TAIL}", good_init()));
        let (r, sink, _) = run(&p, quick());
        assert_eq!(r.unwrap_err(), LeadError::NoInit, "{first}");
        assert!(sink.events.is_empty() && sink.inits.is_empty());
    }
}

#[test]
fn no_init_within_bound_kills() {
    let d = tmp("bound");
    // 64 events is allowed; the 65th is too late, even when it is the init.
    let noise = |n: usize| (0..n).map(|_| r#"{"type":"system","subtype":"hook_started"}"#).collect::<Vec<_>>().join("\n");
    let p = fake_printing(&d, &format!("{}\n{}\n{TAIL}", noise(63), good_init()));
    assert!(run(&p, quick()).0.is_ok(), "init as event 64");
    let p = fake_printing(&d, &format!("{}\n{}\n{TAIL}", noise(64), good_init()));
    assert_eq!(run(&p, quick()).0.unwrap_err(), LeadError::NoInit, "init as event 65");
    // Lines that are not events still count.
    let junk = (0..70).map(|_| "not json").collect::<Vec<_>>().join("\n");
    let p = fake_printing(&d, &format!("{junk}\n{}\n{TAIL}", good_init()));
    assert_eq!(run(&p, quick()).0.unwrap_err(), LeadError::NoInit);
    // Time bound: a process that never prints is killed at the init deadline.
    let p = script(&d, "claude", "sleep 30");
    let t = Instant::now();
    let (r, _, _) = run(&p, Limits { init: Duration::from_millis(300), total: Duration::from_secs(20) });
    assert_eq!(r.unwrap_err(), LeadError::NoInit);
    assert!(t.elapsed() < Duration::from_secs(5));
    // And a stream that ends with no init at all.
    let p = fake_printing(&d, HOOKS);
    assert_eq!(run(&p, quick()).0.unwrap_err(), LeadError::NoInit);
}

// ------------------------------------------------------------ the process

#[test]
fn ask_timeout_marks_failed() {
    let d = tmp("timeout");
    let p = script(&d, "claude", &format!("echo '{}'\nsleep 30", good_init()));
    let t = Instant::now();
    let (r, sink, h) = run(&p, Limits { init: Duration::from_secs(5), total: Duration::from_millis(400) });
    assert_eq!(r.unwrap_err(), LeadError::Timeout);
    assert_eq!(sink.inits.len(), 1, "it had started: raw={:?}", sink.raw);
    assert!(t.elapsed() < Duration::from_secs(5));
    assert!(!group_alive(h.pid.load(Ordering::SeqCst)));
}

#[test]
fn cancel_ask_kills_group() {
    let d = tmp("cancel");
    // A CLI that has a child of its own (its MCP server and hooks): both must die.
    let body = format!("sleep 60 &\necho $! > '{}'\necho '{}'\nwait", d.join("child.pid").display(), good_init());
    let p = script(&d, "claude", &body);
    let h = Handle::new();
    let child = spawn(&[p.to_str().unwrap().to_string()], Path::new("/"), &env(), &framed("hi"), &h).unwrap();
    let canceller = {
        let (h, pidfile) = (h.clone(), d.join("child.pid"));
        std::thread::spawn(move || {
            wait_for(&pidfile);
            h.cancel();
        })
    };
    let t = Instant::now();
    let r = drive(child, &h, quick(), &mut Collect::default());
    canceller.join().unwrap();
    assert_eq!(r.unwrap_err(), LeadError::Cancelled);
    assert!(t.elapsed() < Duration::from_secs(5));
    let pid: i32 = std::fs::read_to_string(d.join("child.pid")).unwrap().trim().parse().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    // SAFETY: signal 0 only checks existence.
    assert!(unsafe { libc::kill(pid, 0) } != 0, "the child of the CLI is gone with it");
    assert!(!group_alive(h.pid.load(Ordering::SeqCst)));
    // A second cancel after the reap reaches nobody (no recycled pid is signalled).
    h.cancel();
}

#[test]
fn a_lead_that_exits_normally_leaves_no_group_member_behind() {
    let d = tmp("normal-exit");
    let f = d.join("events.jsonl");
    std::fs::write(&f, format!("{}\n{HOOKS}\n{TAIL}\n", good_init())).unwrap();
    // The CLI starts a background helper (its MCP server or an async hook), prints a complete turn and exits.
    let body = format!("sleep 60 >/dev/null 2>&1 &\necho $! > '{}'\ncat > /dev/null &\ncat '{}'", d.join("helper.pid").display(), f.display());
    let p = script(&d, "claude", &body);
    let (r, _, h) = run(&p, quick());
    assert!(r.is_ok(), "{r:?}");
    let pid: i32 = std::fs::read_to_string(d.join("helper.pid")).unwrap().trim().parse().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    // SAFETY: signal 0 only checks existence.
    assert!(unsafe { libc::kill(pid, 0) } != 0, "the helper the CLI left behind is killed with its group");
    assert!(!group_alive(h.pid.load(Ordering::SeqCst)));
}

#[test]
fn cancel_before_spawn_spawns_nothing() {
    let d = tmp("precancel");
    let marker = d.join("ran");
    let p = script(&d, "claude", &format!("touch '{}'", marker.display()));
    let h = Handle::new();
    h.cancel();
    let e = spawn(&[p.to_str().unwrap().to_string()], Path::new("/"), &[], &framed("hi"), &h).unwrap_err();
    assert_eq!(e, LeadError::Cancelled);
    std::thread::sleep(Duration::from_millis(100));
    assert!(!marker.exists());
}

#[test]
fn stdin_is_exactly_the_framed_ask_and_env_is_scrubbed() {
    let d = tmp("stdin");
    let body = format!("cat > '{0}/stdin.bin'\nenv > '{0}/env.txt'\npwd > '{0}/cwd.txt'\nprintf '%s\\n' \"$@\" > '{0}/args.txt'\necho '{1}'\necho '{2}'", d.display(), good_init(), r#"{"type":"result","is_error":false,"result":"ok"}"#);
    let p = script(&d, "claude", &body);
    let h = Handle::new();
    let ask = framed("/clear\n!echo hi\n-p");
    let env = [("HOME", OsString::from("/tmp/h")), ("PATH", OsString::from("/usr/bin:/bin"))];
    let argv = vec![p.to_str().unwrap().to_string(), "-p".into(), "--resume".into()];
    let child = spawn(&argv, &d, &env, &ask, &h).unwrap();
    let r = drive(child, &h, quick(), &mut Collect::default());
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(std::fs::read(d.join("stdin.bin")).unwrap(), ask.as_bytes());
    assert!(std::fs::read(d.join("stdin.bin")).unwrap().starts_with(b"[Vajra Desktop: question from the human, ask "));
    assert_eq!(std::fs::read_to_string(d.join("args.txt")).unwrap(), "-p\n--resume\n");
    let envtxt = std::fs::read_to_string(d.join("env.txt")).unwrap();
    let mut names: Vec<&str> = envtxt.lines().filter_map(|l| l.split_once('=').map(|x| x.0)).filter(|n| !matches!(*n, "PWD" | "SHLVL" | "_" | "OLDPWD")).collect();
    names.sort_unstable();
    assert_eq!(names, ["HOME", "PATH"], "{envtxt}");
    assert_eq!(std::fs::canonicalize(std::fs::read_to_string(d.join("cwd.txt")).unwrap().trim()).unwrap(), std::fs::canonicalize(&d).unwrap());
}

#[test]
fn tool_use_results_and_errors_are_tagged() {
    let d = tmp("tags");
    let lines = format!(
        "{HOOKS}\n{}\n{}\n{}\n{}\n{}",
        good_init(),
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/x"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":"File is in a directory that is denied by your permission settings."}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"SECRET FILE BODY"}]}}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done","permission_denials":[{"tool_name":"Read"}]}"#,
    );
    let (r, sink, _) = run(&fake_printing(&d, &lines), quick());
    let done = r.unwrap();
    assert!(done.denied && done.tool_attempted && !done.is_error);
    let tags: Vec<Tag> = sink.events.iter().map(|e| e.0).collect();
    assert_eq!(tags, [Tag::Tool, Tag::Error, Tag::Tool, Tag::Lead], "{:?}", sink.events);
    assert!(sink.events[0].1.starts_with("Read "));
    assert!(sink.events.iter().all(|e| !e.1.contains("SECRET FILE BODY")), "tool results are not echoed");
}

#[test]
fn an_erroring_result_names_a_typed_provider_failure() {
    use crate::providers::errors::Kind;
    for (text, kind) in [("API Error: 429 rate_limit_error", Some(Kind::RateLimit)), ("API Error: 401 authentication_error", Some(Kind::Auth)), ("it went wrong somehow", None)] {
        let d = tmp("provider-err");
        let lines = format!("{}\n{}", good_init(), json!({"type": "result", "subtype": "error", "is_error": true, "result": text}));
        let done = run(&fake_printing(&d, &lines), quick()).0.unwrap();
        assert!(done.is_error);
        assert_eq!(done.provider_error.map(|c| c.kind), kind, "{text}");
    }
}

#[test]
fn a_slash_command_reply_with_no_model_call_is_shown() {
    let d = tmp("slash");
    let lines = format!("{}\n{}", good_init(), r#"{"type":"result","subtype":"success","is_error":false,"result":"/help isn't available","num_turns":0}"#);
    let (r, sink, _) = run(&fake_printing(&d, &lines), quick());
    assert!(r.is_ok());
    assert_eq!(sink.events, vec![(Tag::Lead, "/help isn't available".to_string())]);
}

#[test]
fn an_oversized_line_and_an_early_end_fail() {
    let d = tmp("big");
    let p = script(&d, "claude", &format!("echo '{}'\nhead -c 5000000 /dev/zero | tr '\\0' 'a'\necho", good_init()));
    let (r, _, h) = run(&p, quick());
    assert_eq!(r.unwrap_err().code(), "io_error");
    assert!(!group_alive(h.pid.load(Ordering::SeqCst)));
    let (r, _, _) = run(&fake_printing(&d, &good_init()), quick());
    assert_eq!(r.unwrap_err().code(), "io_error", "init but no result");
}

#[test]
fn a_missing_binary_is_an_error_not_a_panic() {
    let h = Handle::new();
    let e = spawn(&["/nonexistent/claude".to_string()], Path::new("/"), &[], &framed("hi"), &h).unwrap_err();
    assert_eq!(e.code(), "io_error");
    assert_eq!(spawn(&[], Path::new("/"), &[], &framed("hi"), &h).unwrap_err().code(), "invalid_args");
}
