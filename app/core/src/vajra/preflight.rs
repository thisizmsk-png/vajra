//! Preflight rows for the Start screen (AC-1.3.1 .. 1.3.4). Read-only: it
//! runs `claude --version` and `vajra-doctor.sh`, and checks tools exist on
//! PATH. It never opens the HMAC key (the doctor script is the only thing
//! that looks at it, and it does so outside this process).

use std::path::{Path, PathBuf};

use super::allowlist::Allowlist;
use super::bridge::{Bridge, Request, run_process};

pub const INSTALL_CMD: &str = "bash ~/.claude/skills/vajra/scripts/install.sh";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    /// One command or link; empty when the row is fine.
    pub fix: String,
}

fn row(name: &str, ok: bool, detail: impl Into<String>, fix: &str) -> Row {
    Row { name: name.into(), ok, detail: detail.into(), fix: if ok { String::new() } else { fix.into() } }
}

/// "Start run" is enabled only when every row is ok (AC-1.3.2).
pub fn start_enabled(rows: &[Row]) -> bool {
    rows.iter().all(|r| r.ok)
}

pub fn find_on_path(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    dirs.iter().map(|d| d.join(name)).find(|p| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

/// `$PATH` split into dirs.
pub fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default()
}

pub fn run(allow: &Allowlist, bridge: &Bridge, dirs: &[PathBuf]) -> Vec<Row> {
    let mut rows = Vec::new();

    rows.push(match find_on_path("claude", dirs) {
        None => row("claude", false, "not found on PATH", "Install Claude Code: https://claude.com/claude-code"),
        Some(p) => match run_process(&p, &["--version".to_string()], None, bridge.timeout) {
            Ok(o) if o.code == Some(0) => row("claude", true, o.stdout.trim(), ""),
            _ => row("claude", false, "`claude --version` failed", "Reinstall Claude Code: https://claude.com/claude-code"),
        },
    });

    let doctor = bridge.run(&Request::Doctor);
    let (doctor_ok, text) = match &doctor {
        Ok(o) => (o.code == Some(0) && !o.timed_out, o.stdout.clone()),
        Err(_) => (false, String::new()),
    };
    let fails: Vec<&str> = text.lines().filter(|l| l.starts_with("FAIL")).collect();
    rows.push(match &doctor {
        Ok(_) if doctor_ok => row("vajra doctor", true, "PASS", ""),
        Ok(o) if o.timed_out => row("vajra doctor", false, "timed out", "/vajra doctor"),
        Ok(_) => row("vajra doctor", false, format!("FAIL: {}", fails.join("; ")), "/vajra doctor"),
        Err(e) => row("vajra doctor", false, e.to_string(), INSTALL_CMD),
    });

    let hooks: Vec<&str> = text.lines().filter(|l| l.contains("hook registered:")).collect();
    let missing = hooks.iter().filter(|l| !l.starts_with("PASS")).count();
    rows.push(if !hooks.is_empty() && missing == 0 {
        row("hooks registered", true, format!("yes ({} events)", hooks.len()), "")
    } else {
        row("hooks registered", false, "no", INSTALL_CMD)
    });

    // Informational: never blocks Start.
    let sandbox = match text.lines().find(|l| l.contains("OS sandbox")) {
        Some(l) if l.starts_with("PASS") => "on",
        Some(_) => "off",
        None => "unknown",
    };
    rows.push(row("sandbox", true, sandbox, ""));

    for tool in ["git", "python3", "jq", "openssl"] {
        rows.push(match find_on_path(tool, dirs) {
            Some(p) => row(tool, true, p.display().to_string(), ""),
            None => row(tool, false, "not found on PATH", &format!("brew install {tool}")),
        });
    }

    let wf: &Path = &allow.workflow_path();
    rows.push(if allow.resolve(wf).is_ok() {
        row("workflow", true, "vajra-deliver.js installed", "")
    } else {
        row(
            "workflow",
            false,
            "~/.claude/workflows/vajra-deliver.js is missing",
            &format!("{INSTALL_CMD} then /vajra doctor"),
        )
    });
    rows
}
