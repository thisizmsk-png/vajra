//! The worker guard keeps every writer except the app away from the Ask-the-Lead files
//! (`<data>/control/lead/<run_id>/{settings,mcp}.json`). Runs the real `hooks/pre-tool-use.sh` in a temp HOME.
#![allow(clippy::disallowed_methods)] // test code: the app's own spawns go through the spawn lock

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn have(cmd: &str) -> bool {
    Command::new("sh").args(["-c", &format!("command -v {cmd}")]).stdout(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn verdict(home: &PathBuf, hook: &PathBuf, payload: &serde_json::Value) -> Option<i32> {
    let mut child = Command::new("bash")
        .arg(hook)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
    child.wait().unwrap().code()
}

#[test]
fn a_worker_cannot_rewrite_the_lead_files_under_the_control_dir() {
    if !have("jq") || !have("bash") {
        eprintln!("SKIP: jq and bash required");
        return;
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let home = std::env::temp_dir().join(format!("vajra-lead-guard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let skill = home.join(".claude/skills/vajra");
    for d in ["hooks", "config", "scripts"] {
        std::fs::create_dir_all(skill.join(d)).unwrap();
    }
    std::fs::create_dir_all(home.join(".claude/vajra")).unwrap();
    let hook = skill.join("hooks/pre-tool-use.sh");
    std::fs::copy(repo.join("hooks/pre-tool-use.sh"), &hook).unwrap();
    std::fs::write(home.join(".claude/vajra/.hmac-key"), "k").unwrap();
    std::fs::write(home.join(".claude/settings.json"), "{}").unwrap();
    let proj = home.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let lead = home.join("Library/Application Support/VajraDesktop/control/lead/20261106-0911");
    let write = |tool: &str, file: &str, worker: bool| {
        let mut p = serde_json::json!({
            "session_id": "t", "cwd": proj, "hook_event_name": "PreToolUse", "tool_name": tool,
            "tool_input": {"file_path": lead.join(file)},
        });
        if worker {
            p["agent_id"] = "a1b2c3".into();
            p["agent_type"] = "general-purpose".into();
        }
        verdict(&home, &hook, &p)
    };
    let mut seen = Vec::new();
    for (tool, file, worker) in [("Write", "settings.json", true), ("Edit", "mcp.json", true), ("Write", "settings.json", false), ("MultiEdit", "mcp.json", false)] {
        seen.push((tool, file, worker, write(tool, file, worker)));
    }
    // A write somewhere ordinary is not blocked, so the verdicts above are the guard's doing.
    let ok = verdict(&home, &hook, &serde_json::json!({
        "session_id": "t", "cwd": proj, "hook_event_name": "PreToolUse", "tool_name": "Write",
        "tool_input": {"file_path": proj.join("x.txt")}, "agent_id": "a1b2c3", "agent_type": "general-purpose",
    }));
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(ok, Some(0), "control: an ordinary write is allowed");
    for (tool, file, worker, rc) in seen {
        assert_eq!(rc, Some(2), "{tool} {file} (worker={worker}) must be blocked");
    }
}
