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
            Ok(o) if o.exit() == Some(0) => row("claude", true, o.stdout.trim(), ""),
            _ => row("claude", false, "`claude --version` failed", "Reinstall Claude Code: https://claude.com/claude-code"),
        },
    });

    let doctor = bridge.run(&Request::Doctor);
    let (doctor_ok, text) = match &doctor {
        Ok(o) => (o.exit() == Some(0), o.stdout_whole().unwrap_or_default().to_string()),
        Err(_) => (false, String::new()),
    };
    let fails: Vec<&str> = text.lines().filter(|l| l.starts_with("FAIL")).collect();
    rows.push(match &doctor {
        Ok(_) if doctor_ok => row("vajra doctor", true, "PASS", ""),
        Ok(o) if o.timed_out => row("vajra doctor", false, "timed out", "/vajra doctor"),
        Ok(o) if o.truncated => row("vajra doctor", false, "output over the cap", "/vajra doctor"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn exe(p: &Path, body: &str) {
        std::fs::write(p, body).unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A claude and a doctor that each print a passing answer, exit 0, and then flood stderr past the output cap.
    #[test]
    fn claude_and_doctor_output_cut_at_the_cap_are_failures_not_passes() {
        let d = std::env::temp_dir().join(format!("vajra-preflight-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("bin")).unwrap();
        std::fs::create_dir_all(d.join("scripts")).unwrap();
        let flood = "head -c 2200000 /dev/zero | tr '\\0' 'x' >&2\nexit 0\n";
        exe(&d.join("bin/claude"), &format!("#!/bin/sh\necho '2.1.0 (Claude Code)'\n{flood}"));
        exe(&d.join("scripts/vajra-doctor.sh"), &format!("#!/bin/sh\necho 'PASS  OS sandbox enabled'\necho 'PASS  hook registered: PreToolUse'\n{flood}"));
        let canon = d.canonicalize().unwrap();
        let allow = Allowlist::new(&canon, &canon).unwrap();
        let bridge = Bridge::new(canon.join("scripts"));
        let rows = run(&allow, &bridge, &[canon.join("bin")]);
        let find = |n: &str| rows.iter().find(|r| r.name == n).unwrap().clone();
        assert!(!find("claude").ok, "{rows:?}");
        let doc = find("vajra doctor");
        assert!(!doc.ok && doc.detail.contains("cap"), "{doc:?}");
        assert!(!find("hooks registered").ok, "a cut doctor proves no hooks: {rows:?}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
