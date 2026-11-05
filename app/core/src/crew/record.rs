//! Claude Code transcript records -> StreamLines, with the schema
//! fingerprint: a record whose shape is not one we know returns `None`, and
//! the caller switches that file to the raw view.
//!
//! Also the workflow `journal.jsonl` and `agent-*.meta.json` readers.

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    User,
    Text,
    Tool,
    ToolResult,
    /// Hook context (`hook_additional_context`), where steer messages land.
    Hook,
    /// Unparsed line, shown when the format is not recognized.
    Raw,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamLine {
    pub kind: LineKind,
    pub text: String,
}

impl StreamLine {
    /// The text pushed to the bus: a short kind prefix, then the text.
    pub fn display(&self) -> String {
        let p = match self.kind {
            LineKind::User => "[user] ",
            LineKind::Tool => "[tool] ",
            LineKind::ToolResult => "[result] ",
            LineKind::Hook => "[hook] ",
            LineKind::Text | LineKind::Raw => "",
        };
        format!("{p}{}", self.text)
    }
}

/// Record types that carry no stream text but are a known shape.
const QUIET_TYPES: [&str; 8] = [
    "system",
    "summary",
    "file-history-snapshot",
    "progress",
    "queue-operation",
    "custom-title",
    "tag",
    "last-prompt",
];

const BLOCK_TYPES: [&str; 7] =
    ["text", "thinking", "redacted_thinking", "tool_use", "tool_result", "image", "document"];

/// Parse one record. `None` = fingerprint mismatch (unknown shape).
pub fn parse(v: &Value) -> Option<Vec<StreamLine>> {
    let obj = v.as_object()?;
    let ty = obj.get("type")?.as_str()?;
    match ty {
        "user" | "assistant" => {
            let content = v.get("message")?.as_object()?.get("content")?;
            if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
                // Harness-injected reminders: known shape, not shown.
                return (content.is_string() || content.is_array()).then(Vec::new);
            }
            content_lines(ty, content)
        }
        "attachment" => {
            let a = v.get("attachment")?.as_object()?;
            let aty = a.get("type")?.as_str()?;
            if aty != "hook_additional_context" {
                return Some(Vec::new());
            }
            let text = match a.get("content")? {
                Value::String(s) => s.clone(),
                Value::Array(xs) => xs.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"),
                _ => return None,
            };
            Some(vec![StreamLine { kind: LineKind::Hook, text }])
        }
        t if QUIET_TYPES.contains(&t) => Some(Vec::new()),
        _ => None,
    }
}

fn content_lines(ty: &str, content: &Value) -> Option<Vec<StreamLine>> {
    let text_kind = if ty == "user" { LineKind::User } else { LineKind::Text };
    match content {
        Value::String(s) => Some(vec![StreamLine { kind: text_kind, text: s.clone() }]),
        Value::Array(blocks) => {
            let mut out = Vec::new();
            for b in blocks {
                let bty = b.get("type")?.as_str()?;
                if !BLOCK_TYPES.contains(&bty) {
                    return None;
                }
                match bty {
                    "text" => out.push(StreamLine { kind: text_kind, text: b.get("text")?.as_str()?.to_string() }),
                    "tool_use" => out.push(StreamLine { kind: LineKind::Tool, text: tool_use_text(b)? }),
                    "tool_result" => out.push(StreamLine { kind: LineKind::ToolResult, text: result_text(b) }),
                    _ => {} // thinking and media are not shown in the stream
                }
            }
            Some(out)
        }
        _ => None,
    }
}

fn tool_use_text(b: &Value) -> Option<String> {
    let name = b.get("name")?.as_str()?;
    let input = b.get("input")?;
    let detail = input
        .get("command")
        .or_else(|| input.get("file_path"))
        .or_else(|| input.get("pattern"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| input.to_string());
    Some(format!("{name}: {detail}"))
}

fn result_text(b: &Value) -> String {
    match b.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(xs)) => xs
            .iter()
            .filter_map(|x| x.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The text of a non-meta user prompt record, if `v` is one.
pub fn user_prompt(v: &Value) -> Option<&str> {
    if v.get("type")?.as_str()? != "user" || v.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    match v.get("message")?.get("content")? {
        Value::String(s) => Some(s),
        Value::Array(xs) => xs.iter().find(|b| b.get("type").and_then(Value::as_str) == Some("text"))?.get("text")?.as_str(),
        _ => None,
    }
}

/// `agent-<id>.meta.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub agent_type: String,
    pub description: String,
}

pub fn parse_meta(s: &str) -> Option<Meta> {
    let v: Value = serde_json::from_str(s).ok()?;
    Some(Meta {
        agent_type: v.get("agentType")?.as_str()?.to_string(),
        description: v.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
    })
}

/// One workflow agent from `journal.jsonl`: `started` gives the label,
/// a later `result` for the same agent marks it finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub agent_id: String,
    pub label: String,
    pub phase: String,
    pub finished: bool,
}

pub fn parse_journal(s: &str) -> Vec<JournalEntry> {
    let mut out: Vec<JournalEntry> = Vec::new();
    for v in s.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        let Some(id) = v.get("agentId").and_then(Value::as_str) else { continue };
        match v.get("type").and_then(Value::as_str) {
            Some("started") => out.push(JournalEntry {
                agent_id: id.to_string(),
                label: v.get("label").and_then(Value::as_str).unwrap_or("").to_string(),
                phase: v.get("phase").and_then(Value::as_str).unwrap_or("").to_string(),
                finished: false,
            }),
            Some("result") => {
                if let Some(e) = out.iter_mut().rev().find(|e| e.agent_id == id) {
                    e.finished = true;
                }
            }
            _ => {}
        }
    }
    out
}
