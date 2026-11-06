//! Ledger vs audit.log reconciliation (AC-3.4.3, design.md 5.6):
//! - every message/redirect/stop that reached `delivered` has a
//!   desktop-control line with the same id; every resume has its resume line;
//! - every desktop-control line carrying an id has a ledger action;
//! - every guard `ALLOW mcp__vajra-desktop__<tool>` line has a ledger
//!   `via:"mcp"` action of that kind within 5 s, and the reverse.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::Path;

use serde_json::Value;

use super::ledger::Ledger;
use super::{fsx, time};

pub const MCP_WINDOW_SECS: i64 = 5;
const MCP_PREFIX: &str = "mcp__vajra-desktop__";

#[derive(Debug, Default)]
pub struct Report {
    pub checked: usize,
    /// Ledger action ids with no matching audit line.
    pub ledger_without_audit: Vec<String>,
    /// Audit action ids (or guard tool names) with no ledger action.
    pub audit_without_ledger: Vec<String>,
}

impl Report {
    pub fn is_clean(&self) -> bool {
        self.ledger_without_audit.is_empty() && self.audit_without_ledger.is_empty()
    }
}

fn kind_of_tool(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "list_workers" => "list",
        "inspect_worker" => "inspect",
        "message_worker" => "message",
        "redirect_worker" => "redirect",
        "stop_worker" => "stop",
        _ => return None,
    })
}

enum Line {
    /// desktop-control line with an action id.
    Control(String),
    /// Guard ALLOW line for a vajra-desktop MCP tool: (unix secs, kind, tool).
    Guard(i64, &'static str, String),
}

fn parse_line(l: &str) -> Option<Line> {
    let mut it = l.splitn(4, ' ');
    let ts = it.next()?;
    let verdict = it.next()?;
    let tool = it.next()?;
    let rest = it.next().unwrap_or("");
    if tool == "desktop-control" {
        let reason = rest.strip_prefix("reason=\"")?.strip_suffix('"')?;
        let mut words = reason.split(' ');
        if !matches!(words.next()?, "deliver" | "stop" | "resume") {
            return None; // nonce echo, steer tag warnings: not an action
        }
        let id = words.find_map(|w| w.strip_prefix("id="))?;
        return Some(Line::Control(id.to_string()));
    }
    let name = tool.strip_prefix(MCP_PREFIX)?;
    if verdict != "ALLOW" {
        return None; // a blocked call never reached the app
    }
    Some(Line::Guard(time::parse_iso(ts)?, kind_of_tool(name)?, tool.to_string()))
}

pub fn run(ledger_path: &Path, audit_log: &Path) -> io::Result<Report> {
    let records = Ledger::read_all(ledger_path)?;
    let audit = match fsx::read_nofollow(audit_log) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        r => r?,
    };
    let audit = String::from_utf8_lossy(&audit);

    // Per action: kind, first ts, last state, via.
    struct Act {
        kind: String,
        first: i64,
        last_state: String,
        mcp: bool,
    }
    let mut order: Vec<String> = Vec::new();
    let mut acts: HashMap<String, Act> = HashMap::new();
    for r in &records {
        let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let id = s("action_id");
        if id.is_empty() {
            continue;
        }
        let ts = time::parse_iso(&s("ts")).unwrap_or(0);
        let a = acts.entry(id.clone()).or_insert_with(|| {
            order.push(id.clone());
            Act { kind: s("kind"), first: ts, last_state: String::new(), mcp: s("via") == "mcp" }
        });
        a.last_state = s("state");
    }

    let mut rep = Report::default();
    let mut control_ids: HashSet<String> = HashSet::new();
    let mut used: HashSet<String> = HashSet::new();
    for l in audit.lines() {
        match parse_line(l) {
            Some(Line::Control(id)) => {
                rep.checked += 1;
                if !acts.contains_key(&id) && !control_ids.contains(&id) {
                    rep.audit_without_ledger.push(id.clone());
                }
                control_ids.insert(id);
            }
            Some(Line::Guard(ts, kind, tool)) => {
                rep.checked += 1;
                let hit = order.iter().find(|id| {
                    let a = &acts[*id];
                    a.mcp && a.kind == kind && !used.contains(*id) && (a.first - ts).abs() <= MCP_WINDOW_SECS
                });
                match hit {
                    Some(id) => {
                        used.insert(id.clone());
                    }
                    None => rep.audit_without_ledger.push(tool),
                }
            }
            None => {}
        }
    }
    for id in &order {
        let a = &acts[id];
        rep.checked += 1;
        let needs_control = (matches!(a.kind.as_str(), "message" | "redirect" | "stop") && a.last_state == "delivered")
            || a.kind == "resume";
        let missing_control = needs_control && !control_ids.contains(id);
        let missing_guard = a.mcp && !used.contains(id);
        if missing_control || missing_guard {
            rep.ledger_without_audit.push(id.clone());
        }
    }
    Ok(rep)
}
