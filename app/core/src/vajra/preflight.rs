//! Preflight rows for the Start screen (AC-1.3.1 .. 1.3.4). Read-only: it
//! runs `claude --version` and `vajra-doctor.sh`, and checks tools exist on
//! PATH. It never opens the HMAC key (the doctor script is the only thing
//! that looks at it, and it does so outside this process).

use std::path::{Path, PathBuf};

use super::allowlist::Allowlist;
use super::bridge::{Bridge, Request, run_process_on_path};

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

/// The user's PATH dirs (login shell, process PATH, install dirs; absolute only). See `crate::userpath`.
pub fn path_dirs() -> Vec<PathBuf> {
    crate::userpath::current()
}

/// The detail for a missing claude: names where it was looked for.
pub const CLAUDE_MISSING: &str =
    "not found on your login-shell PATH or in ~/.local/bin, ~/.claude/local, /opt/homebrew/bin, /usr/local/bin";

pub fn run(allow: &Allowlist, bridge: &Bridge, dirs: &[PathBuf]) -> Vec<Row> {
    let mut rows = Vec::new();

    rows.push(match find_on_path("claude", dirs) {
        None => row("claude", false, CLAUDE_MISSING, "Install Claude Code: https://claude.com/claude-code"),
        Some(p) => match run_process_on_path(&p, &["--version".to_string()], None, bridge.timeout, dirs) {
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

    fn claude_row(home: &Path, dirs: &[PathBuf]) -> Row {
        std::fs::create_dir_all(home.join("scripts")).unwrap();
        let allow = Allowlist::new(home, home).unwrap();
        let bridge = Bridge::new(home.join("scripts"));
        run(&allow, &bridge, dirs).into_iter().find(|r| r.name == "claude").unwrap()
    }

    /// A Finder launch: the process PATH is the minimal GUI one and claude lives in a dir only the login shell names.
    #[test]
    fn a_gui_launch_finds_claude_through_the_login_shell_path() {
        let home = std::env::temp_dir().join(format!("vajra-preflight-gui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("custom/bin")).unwrap();
        let home = home.canonicalize().unwrap();
        exe(&home.join("custom/bin/claude"), "#!/bin/sh\necho '9.9.9 (Claude Code)'\n");
        let shell = home.join("zsh");
        exe(&shell, &format!("#!/bin/sh\nprintf '%s' '__VAJRA_PATH_BEGIN__{}:/usr/bin:/bin__VAJRA_PATH_END__'\n", home.join("custom/bin").display()));
        let minimal: Vec<PathBuf> = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from).to_vec();
        let login = crate::userpath::detect(&shell, &home, std::time::Duration::from_secs(5));
        let dirs = crate::userpath::effective(login, &minimal, &home);
        let r = claude_row(&home, &dirs);
        assert!(r.ok && r.detail == "9.9.9 (Claude Code)", "{r:?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// An npm install: `claude` is `#!/usr/bin/env <node>` and that node exists only in a dir the resolved PATH names.
    #[test]
    fn an_npm_claude_whose_node_is_only_on_the_resolved_path_passes_version() {
        let home = std::env::temp_dir().join(format!("vajra-preflight-npm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in ["nvm/bin", "other"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        let home = home.canonicalize().unwrap();
        // A real `node` name, so it competes with any node in the system or Homebrew dirs.
        let node = "node";
        exe(&home.join("nvm/bin").join(node), "#!/bin/sh\necho '9.9.9 (Claude Code)'\n");
        exe(&home.join("nvm/bin/claude"), &format!("#!/usr/bin/env {node}\n"));
        let shell = home.join("zsh");
        exe(&shell, &format!("#!/bin/sh\nprintf '%s' '__VAJRA_PATH_BEGIN__{}:/usr/bin:/bin__VAJRA_PATH_END__'\n", home.join("nvm/bin").display()));
        let minimal: Vec<PathBuf> = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from).to_vec();
        let login = crate::userpath::detect(&shell, &home, std::time::Duration::from_secs(5));
        let dirs = crate::userpath::effective(login, &minimal, &home);
        let r = claude_row(&home, &dirs);
        assert!(r.ok && r.detail == "9.9.9 (Claude Code)", "{r:?}");

        // Control: a claude whose interpreter is on no dir of the PATH fails the version check. (Not `node`: a
        // system or Homebrew node would answer.)
        exe(&home.join("other/claude"), "#!/usr/bin/env vajra-missing-node\n");
        let r = claude_row(&home, &[home.join("other")]);
        assert!(!r.ok && r.detail.contains("--version"), "{r:?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn requests_get_the_fixed_system_path_and_the_probe_gets_the_user_order() {
        use super::super::bridge::{BASE_PATH, bridge_path, probe_path};
        assert_eq!(bridge_path(), std::ffi::OsString::from(BASE_PATH.join(":")));
        let user = [PathBuf::from("/x/nvm/bin"), PathBuf::from("."), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/bin")];
        let dirs: Vec<PathBuf> = std::env::split_paths(&probe_path(&user)).collect();
        assert_eq!(&dirs[..3], &[PathBuf::from("/x/nvm/bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/bin")], "the user order is kept: {dirs:?}");
        assert!(dirs.iter().all(|d| d.is_absolute()), "{dirs:?}");
        assert_eq!(dirs.iter().filter(|d| *d == Path::new("/usr/bin")).count(), 1);
        assert!(BASE_PATH.iter().all(|b| dirs.contains(&PathBuf::from(b))), "missing system dirs are appended: {dirs:?}");
    }

    #[test]
    fn a_missing_claude_says_where_it_looked_and_how_to_install() {
        let home = std::env::temp_dir().join(format!("vajra-preflight-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("empty")).unwrap();
        let home = home.canonicalize().unwrap();
        let r = claude_row(&home, &[home.join("empty")]);
        assert!(!r.ok, "{r:?}");
        assert_eq!(r.detail, CLAUDE_MISSING);
        assert!(r.detail.contains("login-shell PATH") && r.detail.contains("~/.local/bin"), "{r:?}");
        assert!(r.fix.contains("https://claude.com/claude-code"), "{r:?}");
        let _ = std::fs::remove_dir_all(&home);
    }
}
