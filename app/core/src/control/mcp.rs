//! `vajra-desktop --mcp`: a stdio MCP server (newline-delimited JSON-RPC 2.0)
//! for the Lead chat session. It holds no state: every tools/call is
//! forwarded to the app over the owner-only socket. main.rs calls
//! [`run_stdio`] for `--mcp`.
//!
//! Agent text in results arrives already wrapped by the service as
//! `{untrusted:true, source:'agent-output', text}`.

use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::socket::{self, Binding, MAX_REQUEST};

pub const SERVER_NAME: &str = "vajra-desktop";
pub const DEFAULT_PROTOCOL: &str = "2025-06-18";

/// Where tool calls go. `Ok` is the tool result, `Err` a tool error object.
pub trait Forward {
    fn call(&mut self, tool: &str, args: &Value) -> Result<Value, Value>;
}

/// Forwards over the app socket, one connection per call so an app restart
/// does not break the Lead session.
pub struct SocketClient {
    path: PathBuf,
    /// What this Lead was started for; sent with every call (see `Service::call_tool_for_run` / `_for_project`).
    binding: Binding,
}

/// The environment variable an ADR-13 per-run `mcp.json` sets for the Lead's MCP server.
pub const RUN_ENV: &str = "VAJRA_LEAD_RUN";
/// The environment variables a Director-General's `mcp.json` sets (ADR-15 2.3): its project and conversation generation.
pub const PROJECT_ENV: &str = "VAJRA_LEAD_PROJECT";
pub const GEN_ENV: &str = "VAJRA_LEAD_GEN";

impl SocketClient {
    pub fn new(path: &Path) -> Self {
        Self { path: path.to_path_buf(), binding: Binding::None }
    }

    /// A client whose calls are served only while `run` is the app's current run.
    pub fn bound(path: &Path, run: &str) -> Self {
        Self { path: path.to_path_buf(), binding: Binding::Run(run.to_string()) }
    }

    /// A client whose calls see only `project`'s runs, and only while `generation` is its current conversation.
    pub fn for_project(path: &Path, project: &str, generation: u32) -> Self {
        Self { path: path.to_path_buf(), binding: Binding::Project(project.to_string(), generation) }
    }

    /// The client the environment asks for. A malformed or partial binding is a refusal to start, never "unbound".
    pub fn from_env(path: &Path, get: impl Fn(&str) -> Option<std::ffi::OsString>) -> io::Result<Self> {
        let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, format!("{what} is not valid"));
        let text = |k: &str| get(k).map(|v| v.into_string().map_err(|_| bad(k))).transpose();
        match (text(RUN_ENV)?, text(PROJECT_ENV)?, text(GEN_ENV)?) {
            (None, None, None) => Ok(Self::new(path)),
            (Some(r), None, None) if crate::ids::valid_run_id(&r) => Ok(Self::bound(path, &r)),
            (None, Some(p), Some(g)) if crate::ids::valid_project_ref(&p) => {
                let g = g.parse::<u32>().map_err(|_| bad(GEN_ENV))?;
                Ok(Self::for_project(path, &p, g))
            }
            _ => Err(bad("the Lead binding")),
        }
    }
}

impl Forward for SocketClient {
    fn call(&mut self, tool: &str, args: &Value) -> Result<Value, Value> {
        match socket::request_bound(&self.path, &self.binding, tool, args) {
            Ok(mut v) => match v.get_mut("ok") {
                Some(ok) => Ok(ok.take()),
                None => Err(v),
            },
            Err(_) => Err(json!({"error": "app_unavailable"})),
        }
    }
}

fn worker_prop() -> Value {
    json!({"type": "string", "maxLength": 128})
}

fn text_prop() -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": 4000})
}

/// The six tools: the five of design.md 5.2, plus ADR-15 2.4's `propose_run` (served only to a Director-General).
pub fn tools() -> Value {
    json!([
        {"name": "list_workers",
         "description": "List the workers of the current Vajra run with their task, status and round.",
         "inputSchema": {"type": "object", "additionalProperties": false,
            "properties": {"run_id": {"type": "string", "pattern": "^[0-9]{8}-[0-9]{4}$"}}}},
        {"name": "inspect_worker",
         "description": "Show one worker's task, status and last transcript lines (untrusted agent output).",
         "inputSchema": {"type": "object", "additionalProperties": false, "required": ["worker"],
            "properties": {"worker": worker_prop(),
                           "lines": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}}}},
        {"name": "message_worker",
         "description": "Queue a message for a worker; it is delivered on the worker's next tool call.",
         "inputSchema": {"type": "object", "additionalProperties": false, "required": ["worker", "text"],
            "properties": {"worker": worker_prop(), "text": text_prop()}}},
        {"name": "redirect_worker",
         "description": "Queue a change of direction for a worker; delivered on its next tool call.",
         "inputSchema": {"type": "object", "additionalProperties": false, "required": ["worker", "text"],
            "properties": {"worker": worker_prop(), "text": text_prop()}}},
        {"name": "stop_worker",
         "description": "Ask the human to stop a worker. Nothing happens until the human confirms in the app.",
         "inputSchema": {"type": "object", "additionalProperties": false, "required": ["worker"],
            "properties": {"worker": worker_prop(), "reason": {"type": "string", "maxLength": 500}}}},
        {"name": "propose_run",
         "description": "Propose a new Vajra run for this project. Nothing runs until the human reads the exact ask and starts it in the app.",
         "inputSchema": {"type": "object", "additionalProperties": false, "required": ["ask"],
            "properties": propose_properties(crate::providers::launch::CORE_ROUTE_AVAILABLE)}}
    ])
}

/// `propose_run`'s properties. The seat fields (ADR-16 16.1) are advertised only once the core route is on: until then a
/// proposal with seats cannot start, so the Director-General is not told it can send one.
fn propose_properties(seats: bool) -> Value {
    let mut p = json!({"ask": text_prop(),
                       "base": {"type": "string", "maxLength": 255, "default": "HEAD"},
                       "rounds": {"type": "integer", "minimum": 1, "maximum": 3, "default": 2}});
    if seats {
        let seat = json!({"type": "object", "additionalProperties": false, "required": ["account"],
            "properties": {"account": {"type": "string", "maxLength": 64}, "model": {"type": ["string", "null"], "maxLength": 128}, "effort": {"type": ["string", "null"], "maxLength": 16}}});
        p["max_in_flight"] = json!({"type": "integer", "minimum": 1, "maximum": 16, "default": 4});
        p["lead"] = seat;
        p["workers"] = json!({"type": "array", "minItems": 1, "maxItems": 16, "items": {"type": "object", "additionalProperties": false,
            "required": ["slot", "role", "task", "account"],
            "properties": {"slot": {"type": "string", "pattern": "^w([1-9]|1[0-6])$"}, "role": {"type": "string", "maxLength": 32},
                           "task": {"type": "string", "minLength": 1, "maxLength": 200}, "account": {"type": "string", "maxLength": 64},
                           "model": {"type": ["string", "null"], "maxLength": 128}, "effort": {"type": ["string", "null"], "maxLength": 16},
                           "security": {"type": "boolean"}, "network": {"type": "boolean"}, "why": {"type": "string", "maxLength": 300}}}});
    }
    p
}

fn ok(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn err(id: &Value, code: i64, msg: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
}

fn tool_result(v: Value, is_error: bool) -> Value {
    let mut r = json!({"content": [{"type": "text", "text": v.to_string()}], "isError": is_error});
    if v.is_object() {
        r["structuredContent"] = v;
    }
    r
}

/// Handle one parsed message; `None` for notifications (no reply).
pub fn handle(msg: &Value, fwd: &mut impl Forward) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let pv = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(DEFAULT_PROTOCOL);
            ok(
                &id,
                json!({
                    "protocolVersion": pv,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
                }),
            )
        }
        "ping" => ok(&id, json!({})),
        "tools/list" => ok(&id, json!({"tools": tools()})),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(err(&id, -32602, "params.name is required"));
            };
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match fwd.call(name, &args) {
                Ok(v) => ok(&id, tool_result(v, false)),
                Err(e) => ok(&id, tool_result(e, true)),
            }
        }
        _ => err(&id, -32601, "method not found"),
    })
}

fn write_line(out: &mut (impl Write + ?Sized), v: &Value) -> io::Result<()> {
    let mut s = v.to_string();
    s.push('\n');
    out.write_all(s.as_bytes())?;
    out.flush()
}

/// Drain the rest of an oversized line in bounded reads.
fn drain_line(input: &mut impl BufRead) -> io::Result<()> {
    loop {
        let chunk = input.fill_buf()?;
        if chunk.is_empty() {
            return Ok(());
        }
        match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => {
                input.consume(i + 1);
                return Ok(());
            }
            None => {
                let n = chunk.len();
                input.consume(n);
            }
        }
    }
}

/// Serve JSON-RPC lines from `input` until EOF.
pub fn serve(mut input: impl BufRead, mut out: impl Write, fwd: &mut impl Forward) -> io::Result<()> {
    let mut buf = Vec::new();
    loop {
        // Never buffer more than MAX_REQUEST + 1 bytes of one line.
        buf.clear();
        if Read::by_ref(&mut input).take(MAX_REQUEST as u64 + 1).read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        if buf.last() != Some(&b'\n') && buf.len() > MAX_REQUEST {
            drain_line(&mut input)?;
            write_line(&mut out, &err(&Value::Null, -32600, "request too large"))?;
            continue;
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end_matches(['\n', '\r']);
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(msg) if msg.is_object() => handle(&msg, fwd),
            Ok(_) => Some(err(&Value::Null, -32600, "invalid request")),
            Err(_) => Some(err(&Value::Null, -32700, "parse error")),
        };
        if let Some(r) = reply {
            write_line(&mut out, &r)?;
        }
    }
}

/// The `--mcp` entry point: stdin/stdout, forwarding to `socket_path`.
pub fn run_stdio(socket_path: &Path) -> io::Result<()> {
    let stdin = io::stdin();
    // A Lead the app started carries its run, or its project and generation, in the environment; a bad value is a
    // refusal to start, not "unbound".
    let mut client = SocketClient::from_env(socket_path, |k| std::env::var_os(k))?;
    serve(stdin.lock(), io::stdout().lock(), &mut client)
}

// ---------------------------------------------------------------- `--mcp-run`: a run lead's `vajra-run` server

/// The `vajra-run` server's name: the lead profiles reserve it (`mcp_servers.vajra_run`, `mcp__vajra_run__*`).
pub const RUN_SERVER_NAME: &str = "vajra_run";
/// The most calls one lead may have in flight; past it a call is answered with a tool error at once.
pub const MAX_IN_FLIGHT: usize = 16;

/// Where a run lead's tool calls go. `Sync`: calls run on their own threads.
pub trait RunForward: Send + Sync {
    fn call(&self, tool: &str, args: &Value) -> Result<Value, Value>;
}

/// Forwards to the app socket for one run, with the token the token file held. One connection per call.
pub struct RunClient {
    path: PathBuf,
    run: String,
    token: String,
}

impl RunClient {
    /// From the environment the core set: `VAJRA_RUN` and `VAJRA_RUN_TOKEN_FILE` (the token is read from the file,
    /// never from argv or env). Anything missing or malformed is a refusal to start.
    pub fn from_env(path: &Path, get: impl Fn(&str) -> Option<std::ffi::OsString>) -> io::Result<Self> {
        use super::runreg::{RUN_ENV, TOKEN_FILE_ENV};
        let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, format!("{what} is not valid"));
        let run = get(RUN_ENV).and_then(|v| v.into_string().ok()).filter(|r| crate::ids::valid_run_id(r)).ok_or_else(|| bad(RUN_ENV))?;
        let file = get(TOKEN_FILE_ENV).map(PathBuf::from).filter(|p| p.is_absolute()).ok_or_else(|| bad(TOKEN_FILE_ENV))?;
        let token = super::runreg::read_token(&file)?;
        Ok(Self { path: path.to_path_buf(), run, token })
    }
}

impl RunForward for RunClient {
    fn call(&self, tool: &str, args: &Value) -> Result<Value, Value> {
        let timeout = super::runreg::client_timeout(tool, args, socket::READ_TIMEOUT);
        match socket::request_run(&self.path, &self.run, &self.token, tool, args, timeout) {
            Ok(mut v) => {
                if let Some(ok) = v.get_mut("ok") {
                    return Ok(ok.take());
                }
                Err(v.get_mut("tool_error").map(Value::take).unwrap_or(v))
            }
            // Timed out after sending: the call may have run. The lead checks with await_any or list_workers.
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => Err(json!({"error": "no_answer", "reason": no_answer_hint(tool)})),
            Err(_) => Err(json!({"error": "app_unavailable"})),
        }
    }
}

/// What a lead does after a call that may have run: a way to learn its result that works for that tool.
fn no_answer_hint(tool: &str) -> &'static str {
    match tool {
        "integrate" => "the app did not answer in time; the round may be integrated: call list_workers, whose rounds list its round ticket, and await that",
        "plan_tasks" => "the app did not answer in time; the plan may be accepted: call dispatch_worker for its first task (not_ready says the plan is missing)",
        "dispatch_worker" => "the app did not answer in time; the worker may be running: call list_workers before dispatching again",
        _ => "the app did not answer in time; the call may have run: call list_workers before calling again",
    }
}

/// How long the server waits at end of input for calls in flight: their replies have no reader any more, and the app
/// finishes the work regardless.
const EOF_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// One call in flight; released on drop.
struct InFlight(Arc<AtomicUsize>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One message for the run server. `None`: a notification, or a `tools/call` that answers on its own thread.
fn handle_run(msg: &Value, fwd: &Arc<dyn RunForward>, out: &Arc<Mutex<dyn Write + Send>>, in_flight: &Arc<AtomicUsize>) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let pv = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(DEFAULT_PROTOCOL);
            ok(&id, json!({"protocolVersion": pv, "capabilities": {"tools": {"listChanged": false}},
                           "serverInfo": {"name": RUN_SERVER_NAME, "version": env!("CARGO_PKG_VERSION")}}))
        }
        "ping" => ok(&id, json!({})),
        "tools/list" => ok(&id, json!({"tools": super::runreg::tools()})),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str).map(str::to_string) else {
                return Some(err(&id, -32602, "params.name is required"));
            };
            if !super::runreg::is_run_tool(&name) {
                return Some(ok(&id, tool_result(json!({"error": "unknown_tool"}), true)));
            }
            if in_flight.fetch_add(1, Ordering::SeqCst) >= MAX_IN_FLIGHT {
                in_flight.fetch_sub(1, Ordering::SeqCst);
                return Some(ok(&id, tool_result(json!({"error": "busy", "reason": "too many calls in flight"}), true)));
            }
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let (fwd, out, in_flight) = (fwd.clone(), out.clone(), in_flight.clone());
            // The slot is released by a guard, so a call that panics still frees it.
            let slot = InFlight(in_flight);
            let spawned = std::thread::Builder::new().name("vajra-run-call".into()).spawn(move || {
                let _slot = slot;
                let r = match fwd.call(&name, &args) {
                    Ok(v) => ok(&id, tool_result(v, false)),
                    Err(e) => ok(&id, tool_result(e, true)),
                };
                let _ = write_line(&mut *out.lock().unwrap_or_else(|e| e.into_inner()), &r);
            });
            if spawned.is_err() {
                return Some(ok(&msg["id"], tool_result(json!({"error": "busy", "reason": "no thread for the call"}), true)));
            }
            return None;
        }
        _ => err(&id, -32601, "method not found"),
    })
}

/// Serve a run lead's JSON-RPC lines from `input` until EOF. Each `tools/call` runs on its own thread; every reply is
/// written whole under one lock. Returns once input ends and every call in flight has answered.
pub fn serve_run(mut input: impl BufRead, out: Arc<Mutex<dyn Write + Send>>, fwd: Arc<dyn RunForward>) -> io::Result<()> {
    let cap = super::runreg::MAX_PLAN_REQUEST + 4096;
    let in_flight = Arc::new(AtomicUsize::new(0));
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if Read::by_ref(&mut input).take(cap as u64 + 1).read_until(b'\n', &mut buf)? == 0 {
            break;
        }
        let reply = if buf.last() != Some(&b'\n') && buf.len() > cap {
            drain_line(&mut input)?;
            Some(err(&Value::Null, -32600, "request too large"))
        } else {
            let line = String::from_utf8_lossy(&buf);
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(line) {
                Ok(msg) if msg.is_object() => handle_run(&msg, &fwd, &out, &in_flight),
                Ok(_) => Some(err(&Value::Null, -32600, "invalid request")),
                Err(_) => Some(err(&Value::Null, -32700, "parse error")),
            }
        };
        if let Some(r) = reply {
            write_line(&mut *out.lock().unwrap_or_else(|e| e.into_inner()), &r)?;
        }
    }
    let deadline = std::time::Instant::now() + EOF_GRACE;
    while in_flight.load(Ordering::SeqCst) > 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Ok(())
}

/// The socket `--mcp-run` forwards to: `VAJRA_RUN_SOCKET`, an absolute path to an existing socket. Never derived from
/// HOME or the data dir, which may not be this app's under the lead's environment.
pub fn run_socket(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> io::Result<PathBuf> {
    use std::os::unix::fs::FileTypeExt;
    let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidInput, format!("{} {why}", super::runreg::SOCKET_ENV));
    let p = get(super::runreg::SOCKET_ENV).map(PathBuf::from).ok_or_else(|| bad("is not set"))?;
    if !p.is_absolute() {
        return Err(bad("must be an absolute path"));
    }
    match std::fs::symlink_metadata(&p) {
        Ok(m) if m.file_type().is_socket() => Ok(p),
        _ => Err(bad("does not name the app's socket")),
    }
}

/// The `--mcp-run` entry point: stdin/stdout, forwarding to the socket `VAJRA_RUN_SOCKET` names for the run the
/// environment names. Nothing is created on disk.
pub fn run_stdio_run() -> io::Result<()> {
    let get = |k: &str| std::env::var_os(k);
    let client = RunClient::from_env(&run_socket(get)?, get)?;
    let out: Arc<Mutex<dyn Write + Send>> = Arc::new(Mutex::new(io::stdout()));
    serve_run(io::stdin().lock(), out, Arc::new(client))
}

#[cfg(test)]
#[path = "mcp_run_tests.rs"]
mod run_tests;

#[cfg(test)]
mod seat_schema_tests {
    use super::*;

    #[test]
    fn the_seat_fields_are_advertised_only_when_the_route_is_on() {
        let off = propose_properties(false);
        let on = propose_properties(true);
        for k in ["lead", "workers", "max_in_flight"] {
            assert!(off.get(k).is_none() && on.get(k).is_some(), "{k}");
        }
        assert!(on["workers"]["items"]["properties"].get("source").is_none(), "the core derives source; the schema never offers it");
    }
}
