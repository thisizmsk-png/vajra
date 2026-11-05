//! `vajra-desktop --mcp`: a stdio MCP server (newline-delimited JSON-RPC 2.0)
//! for the Lead chat session. It holds no state: every tools/call is
//! forwarded to the app over the owner-only socket. main.rs calls
//! [`run_stdio`] for `--mcp`.
//!
//! Agent text in results arrives already wrapped by the service as
//! `{untrusted:true, source:'agent-output', text}`.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::socket::{self, MAX_REQUEST};

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
}

impl SocketClient {
    pub fn new(path: &Path) -> Self {
        Self { path: path.to_path_buf() }
    }
}

impl Forward for SocketClient {
    fn call(&mut self, tool: &str, args: &Value) -> Result<Value, Value> {
        match socket::request(&self.path, tool, args) {
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

/// The five tools, schemas as in design.md 5.2.
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
            "properties": {"worker": worker_prop(), "reason": {"type": "string", "maxLength": 500}}}}
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
pub fn serve(input: impl BufRead, mut out: impl Write, fwd: &mut impl Forward) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_REQUEST {
            write_line(&mut out, &err(&Value::Null, -32600, "request too large"))?;
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) if msg.is_object() => handle(&msg, fwd),
            Ok(_) => Some(err(&Value::Null, -32600, "invalid request")),
            Err(_) => Some(err(&Value::Null, -32700, "parse error")),
        };
        if let Some(r) = reply {
            write_line(&mut out, &r)?;
        }
    }
    Ok(())
}

/// The `--mcp` entry point: stdin/stdout, forwarding to `socket_path`.
pub fn run_stdio(socket_path: &Path) -> io::Result<()> {
    let stdin = io::stdin();
    serve(stdin.lock(), io::stdout().lock(), &mut SocketClient::new(socket_path))
}
