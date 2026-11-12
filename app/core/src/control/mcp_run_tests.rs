//! `--mcp-run` (6d2): the run server lists exactly the `vajra-run` tools, answers calls concurrently with whole reply
//! lines, bounds the calls in flight, and starts only from a valid run and token file.

use super::*;
use std::io::Cursor;
use std::sync::Condvar;
use std::time::{Duration, Instant};

/// Sleeps `ms` from the args, then echoes the tool; `hold` keeps calls parked until released.
#[derive(Default)]
struct Slow {
    hold: Mutex<bool>,
    released: Condvar,
}

impl RunForward for Slow {
    fn call(&self, tool: &str, args: &Value) -> Result<Value, Value> {
        if tool == "cancel_worker" {
            return Err(json!({"error": "not_ready", "reason": "no"}));
        }
        let ms = args.get("ms").and_then(Value::as_u64).unwrap_or(0);
        std::thread::sleep(Duration::from_millis(ms));
        let g = crate::lock(&self.hold);
        drop(self.released.wait_while(g, |h| *h).unwrap());
        Ok(json!({"tool": tool}))
    }
}

/// A `Write` the test reads after the server ends.
#[derive(Clone, Default)]
struct Out(Arc<Mutex<Vec<u8>>>);

impl Write for Out {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        // Write one byte at a time: a reply written outside the lock would interleave.
        for x in b {
            crate::lock(&self.0).push(*x);
            std::thread::yield_now();
        }
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn lines(input: &[Value]) -> String {
    input.iter().map(|v| format!("{v}\n")).collect()
}

fn call(id: u64, name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

fn run(input: &[Value], fwd: Arc<dyn RunForward>) -> Vec<Value> {
    let out = Out::default();
    let w: Arc<Mutex<dyn Write + Send>> = Arc::new(Mutex::new(out.clone()));
    serve_run(Cursor::new(lines(input)), w, fwd).unwrap();
    let text = String::from_utf8(crate::lock(&out.0).clone()).unwrap();
    text.lines().map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("a torn line {l:?}: {e}"))).collect()
}

fn by_id(replies: &[Value], id: u64) -> &Value {
    replies.iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("no reply {id}: {replies:?}"))
}

#[test]
fn tools_list_returns_exactly_the_vajra_run_tools() {
    let r = run(&[json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
                  json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})], Arc::new(Slow::default()));
    assert_eq!(by_id(&r, 1)["result"]["serverInfo"]["name"], RUN_SERVER_NAME);
    let names: Vec<&str> = by_id(&r, 2)["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, super::super::runreg::TOOL_NAMES);
}

#[test]
fn calls_run_side_by_side_and_every_reply_is_one_whole_line() {
    let started = Instant::now();
    let input: Vec<Value> = (1..=6).map(|i| call(i, "await_any", json!({"ms": 600}))).collect();
    let r = run(&input, Arc::new(Slow::default()));
    assert!(started.elapsed() < Duration::from_millis(2000), "six 600 ms calls ran concurrently: {:?}", started.elapsed());
    for i in 1..=6 {
        assert_eq!(by_id(&r, i)["result"]["structuredContent"]["tool"], "await_any");
    }
}

#[test]
fn a_refusal_comes_back_as_a_tool_error_and_an_unknown_tool_is_refused_without_a_call() {
    let r = run(&[call(1, "cancel_worker", json!({})), call(2, "propose_run", json!({})), call(3, "message_worker", json!({}))], Arc::new(Slow::default()));
    assert_eq!(by_id(&r, 1)["result"]["isError"], true);
    assert_eq!(by_id(&r, 1)["result"]["structuredContent"]["error"], "not_ready");
    for id in [2, 3] {
        assert_eq!(by_id(&r, id)["result"]["structuredContent"]["error"], "unknown_tool");
    }
}

#[test]
fn past_the_in_flight_bound_a_call_is_answered_busy_at_once() {
    let slow = Arc::new(Slow::default());
    *crate::lock(&slow.hold) = true;
    let s = slow.clone();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        *crate::lock(&s.hold) = false;
        s.released.notify_all();
    });
    let n = MAX_IN_FLIGHT as u64 + 2;
    let input: Vec<Value> = (1..=n).map(|i| call(i, "await_any", json!({}))).collect();
    let r = run(&input, slow);
    release.join().unwrap();
    let busy = r.iter().filter(|x| x["result"]["structuredContent"]["error"] == "busy").count();
    assert_eq!(busy, 2);
    assert_eq!(r.len() as u64, n, "every call is answered");
}

#[test]
fn an_oversized_line_is_refused_and_the_next_one_still_served() {
    let big = format!("{}\n", "x".repeat(super::super::runreg::MAX_PLAN_REQUEST + 8192));
    let input = format!("{big}{}\n", json!({"jsonrpc": "2.0", "id": 9, "method": "ping"}));
    let out = Out::default();
    let w: Arc<Mutex<dyn Write + Send>> = Arc::new(Mutex::new(out.clone()));
    serve_run(Cursor::new(input), w, Arc::new(Slow::default())).unwrap();
    let text = String::from_utf8(crate::lock(&out.0).clone()).unwrap();
    assert!(text.contains("request too large"), "{text}");
    assert!(text.contains("\"id\":9"), "{text}");
}

#[test]
fn the_run_client_starts_only_from_a_valid_run_and_a_private_token_file() {
    let dir = std::env::temp_dir().join(format!("vajra-mcprun-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (path, _) = super::super::runreg::write_token(&dir).unwrap();
    let env = |run: &'static str, file: Option<PathBuf>| {
        move |k: &str| match k {
            "VAJRA_RUN" => Some(run.into()),
            "VAJRA_RUN_TOKEN_FILE" => file.clone().map(Into::into),
            _ => None,
        }
    };
    let sock = Path::new("/nonexistent/app.sock");
    assert!(RunClient::from_env(sock, env("20261112-0930", Some(path.clone()))).is_ok());
    assert!(RunClient::from_env(sock, env("not a run", Some(path.clone()))).is_err());
    assert!(RunClient::from_env(sock, env("20261112-0930", None)).is_err());
    assert!(RunClient::from_env(sock, env("20261112-0930", Some(PathBuf::from("relative/token")))).is_err());
    // With no app answering, a call is a tool error, never a panic or a hang.
    let c = RunClient::from_env(sock, env("20261112-0930", Some(path))).unwrap();
    assert_eq!(c.call("list_workers", &json!({})).unwrap_err()["error"], "app_unavailable");
    let _ = std::fs::remove_dir_all(&dir);
}
