//! `vajra-desktop --mcp`: a stdio MCP server (newline-delimited JSON-RPC 2.0)
//! for the Lead chat session. It holds no state: every tools/call is
//! forwarded to the app over the owner-only socket. main.rs calls
//! [`run_stdio`] for `--mcp`.
//!
//! Agent text in results arrives already wrapped by the service as
//! `{untrusted:true, source:'agent-output', text}`.

use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};

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
            "properties": {"ask": text_prop(),
                           "base": {"type": "string", "maxLength": 255, "default": "HEAD"},
                           "rounds": {"type": "integer", "minimum": 1, "maximum": 3, "default": 2}}}}
    ])
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

fn write_line(out: &mut impl Write, v: &Value) -> io::Result<()> {
    let mut s = v.to_string();
    s.push('\n');
    out.write_all(s.as_bytes())?;
    out.flush()
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
            // Drain the rest of the oversized line in bounded reads.
            loop {
                let chunk = input.fill_buf()?;
                if chunk.is_empty() {
                    break;
                }
                match chunk.iter().position(|&b| b == b'\n') {
                    Some(i) => {
                        input.consume(i + 1);
                        break;
                    }
                    None => {
                        let n = chunk.len();
                        input.consume(n);
                    }
                }
            }
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
