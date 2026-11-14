use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::*;
use crate::providers::detect::{Out, Probe};

/// Binaries by name, each with the `--version` answer it gives (`None`: it does not finish).
#[derive(Default)]
struct Fake {
    bins: HashMap<String, Option<(i32, String)>>,
}

impl Fake {
    fn with(mut self, name: &str, code: i32, out: &str) -> Self {
        self.bins.insert(name.into(), Some((code, out.into())));
        self
    }
    fn hung(mut self, name: &str) -> Self {
        self.bins.insert(name.into(), None);
        self
    }
}

impl Probe for Fake {
    fn find(&self, name: &str) -> Option<PathBuf> {
        self.bins.contains_key(name).then(|| PathBuf::from(format!("/opt/homebrew/bin/{name}")))
    }
    fn run(&self, path: &Path, args: &[&str]) -> Option<Out> {
        assert_eq!(args, ["--version"], "detection only asks for the version");
        let name = path.file_name()?.to_str()?;
        let (code, out) = self.bins.get(name)?.clone()?;
        Some(Out { code: Some(code), stdout: out, stderr: String::new() })
    }
    fn env(&self, _key: &str) -> Option<OsString> {
        None
    }
}

fn row<'a>(l: &'a SetupList, id: &str) -> &'a SetupRow {
    l.rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
}

#[test]
fn every_catalog_entry_is_a_row_in_catalog_order() {
    let l = list(&Fake::default(), false);
    assert_eq!(l.schema, SETUP_SCHEMA);
    assert_eq!(l.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["claude", "codex", "gemini"]);
}

#[test]
fn a_missing_unverified_harness_is_install_it_yourself_with_the_vendor_commands_and_link() {
    let l = list(&Fake::default(), false);
    let r = row(&l, "codex");
    assert_eq!(r.state, State::Manual);
    assert_eq!(r.reason_code.as_deref(), Some("source_not_verified"));
    assert_eq!(r.docs_url, "https://github.com/openai/codex");
    let cmds: Vec<(&str, &str)> = r.commands.iter().map(|c| (c.tool.as_str(), c.command.as_str())).collect();
    assert_eq!(cmds, [("brew", "brew install --cask codex"), ("npm", "npm install -g @openai/codex")]);
    assert!(!r.can_sign_in);
}

#[test]
fn an_installed_harness_with_an_adapter_shows_its_version_and_can_sign_in() {
    let l = list(&Fake::default().with("claude", 0, "2.1.322 (Claude Code)"), false);
    let r = row(&l, "claude");
    assert_eq!(r.state, State::Installed);
    assert_eq!(r.version.as_deref(), Some("2.1.322"));
    assert_eq!(r.path.as_deref(), Some("/opt/homebrew/bin/claude"));
    assert!(r.can_sign_in);
    assert!(r.commands.is_empty(), "an installed row offers no install command");
}

#[test]
fn an_installed_harness_outside_its_tested_range_is_set_up_but_not_runnable() {
    // The same tested-range decision detect::one and launchable_with use: Codex is tested on 0.166 only.
    let r = |out: &str| {
        let l = list(&Fake::default().with("codex", 0, out), false);
        let r = row(&l, "codex");
        (r.state, r.version.clone(), r.can_sign_in)
    };
    assert_eq!(r("codex-cli 0.167.1"), (State::Unsupported, Some("0.167.1".into()), true));
    assert_eq!(r("codex-cli 0.166.0"), (State::Installed, Some("0.166.0".into()), true));
}

#[test]
fn an_installed_harness_without_an_adapter_is_set_up_but_not_runnable() {
    let l = list(&Fake::default().with("gemini", 0, "0.68.0"), false);
    let r = row(&l, "gemini");
    assert_eq!(r.state, State::Unsupported);
    assert_eq!(r.version.as_deref(), Some("0.68.0"));
    assert!(r.can_sign_in);
}

#[test]
fn a_binary_that_fails_its_version_check_is_broken_with_repair_steps() {
    for fake in [Fake::default().with("codex", 1, "boom"), Fake::default().with("codex", 0, "no version here"), Fake::default().hung("codex")] {
        let l = list(&fake, false);
        let r = row(&l, "codex");
        assert_eq!(r.state, State::Broken);
        assert_eq!(r.reason_code.as_deref(), Some("version_failed"));
        assert!(!r.can_sign_in);
        // Repair in this step is the vendor's own steps: the same commands and link as "Install it yourself".
        assert_eq!(r.commands.len(), 2);
        assert_eq!(r.docs_url, "https://github.com/openai/codex");
    }
}

#[test]
fn the_engine_scripts_row_is_detection_only_with_the_install_command() {
    let off = list(&Fake::default(), false);
    assert!(!off.engine.installed);
    assert_eq!(off.engine.command, "bash ~/.claude/skills/vajra/scripts/install.sh");
    assert!(list(&Fake::default(), true).engine.installed);
}

#[test]
fn the_list_carries_no_install_offer_in_this_step() {
    let l = list(&Fake::default(), false);
    let v = serde_json::to_value(&l).unwrap().to_string();
    for word in ["consent", "plan", "token", "install_now"] {
        assert!(!v.contains(word), "{word} in {v}");
    }
}

// ---- sign in ----------------------------------------------------------------------------------------------------------

#[test]
fn sign_in_is_refused_for_an_unknown_or_missing_harness() {
    assert_eq!(sign_in("nope", &Fake::default()).unwrap_err(), SetupError::Unknown);
    assert_eq!(sign_in("codex", &Fake::default()).unwrap_err(), SetupError::NotInstalled);
    assert_eq!(sign_in("codex", &Fake::default().with("codex", 1, "")).unwrap_err(), SetupError::NotInstalled);
}

#[test]
fn sign_in_runs_the_catalog_argv_on_the_detected_binary_in_a_fresh_private_dir() {
    use std::os::unix::fs::PermissionsExt;
    let d = std::env::temp_dir().join(format!("vajra-signin-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let bin = d.join("claude");
    std::fs::write(&bin, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    struct At(PathBuf);
    impl Probe for At {
        fn find(&self, n: &str) -> Option<PathBuf> {
            (n == "claude").then(|| self.0.clone())
        }
        fn run(&self, _: &Path, _: &[&str]) -> Option<Out> {
            Some(Out { code: Some(0), stdout: "2.1.322 (Claude Code)".into(), stderr: String::new() })
        }
        fn env(&self, _: &str) -> Option<OsString> {
            None
        }
    }
    let s = sign_in("claude", &At(bin.clone())).unwrap();
    assert_eq!(s.program(), bin);
    assert_eq!(s.args(), ["auth", "login"]);
    let cwd = s.cwd();
    assert!(cwd.is_dir());
    assert_eq!(std::fs::read_dir(cwd).unwrap().count(), 0, "the dir is empty");
    assert_eq!(std::fs::metadata(cwd).unwrap().permissions().mode() & 0o777, 0o700);
    assert_ne!(Some(cwd.to_path_buf()), std::env::var_os("HOME").map(PathBuf::from));
    let other = sign_in("claude", &At(bin)).unwrap();
    assert_ne!(other.cwd(), cwd, "each sign-in gets its own dir");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn no_mcp_or_control_tool_can_reach_set_up_or_sign_in() {
    let tools = crate::control::mcp::tools().to_string().to_lowercase();
    for word in ["setup", "set_up", "sign_in", "signin", "install", "login"] {
        assert!(!tools.contains(word), "{word} in {tools}");
    }
    // No file under src/control (the socket, the MCP server and every control-service tool, at any depth) names set up,
    // sign-in or the PATH refresh.
    // Nor the one-click install (bhishma condition 7): its consent and run are main-window commands only, and the CLI
    // example drives no install either.
    let banned = [
        "providers::setup", "open_sign_in", "SignInCommand", "setup_sign_in_command", "setup_list", "userpath::refresh",
        "providers::install", "setup_plan", "setup_consent", "setup_run", "spawn_setup", "Consents",
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut dirs = vec![root.join("src/control"), root.join("examples")];
    let mut scanned = 0;
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|e| e == "rs") {
                let code = std::fs::read_to_string(&p).unwrap();
                for word in banned {
                    assert!(!code.contains(word), "{word} in {}", p.display());
                }
                scanned += 1;
            }
        }
    }
    assert!(scanned > 0);
}

#[test]
fn a_broken_mark_holds_even_when_the_version_check_passes() {
    let d = std::env::temp_dir().join(format!("vajra-setup-marks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let marks = crate::providers::install::marks::Marks::at(&d);
    let fake = Fake::default().with("claude", 0, "2.1.322 (Claude Code)");
    let path = list(&fake, false).rows.iter().find(|r| r.id == "claude").unwrap().path.clone().unwrap();
    marks.set("claude", Path::new(&path), Some("2.1.322"), "closure_mismatch").unwrap();
    let r = list_marked(&fake, false, &marks).rows.into_iter().find(|r| r.id == "claude").unwrap();
    assert_eq!((r.state, r.reason_code.as_deref()), (State::Broken, Some("closure_mismatch")));
    assert!(!r.can_sign_in);
    let newer = Fake::default().with("claude", 0, "2.1.323 (Claude Code)");
    let r = list_marked(&newer, false, &marks).rows.into_iter().find(|r| r.id == "claude").unwrap();
    assert_eq!(r.state, State::Installed, "a new version clears it");
}

#[test]
fn current_names_the_binary_and_its_version_or_none() {
    let h = catalog::find("claude").unwrap();
    let (path, v) = current(h, &Fake::default().with("claude", 0, "2.1.322 (Claude Code)")).unwrap();
    assert_eq!((path.to_str(), v.as_deref()), (Some("/opt/homebrew/bin/claude"), Some("2.1.322")));
    assert_eq!(current(h, &Fake::default().with("claude", 1, "")).unwrap().1, None, "broken: a path, no version");
    assert!(current(h, &Fake::default()).is_none());
}

#[test]
fn one_click_is_offered_only_for_a_verified_source_that_is_missing_or_broken() {
    let mut h = catalog::find("claude").unwrap().clone();
    let none = crate::providers::install::marks::Marks::none();
    let r = super::row(&h, &Fake::default(), &none);
    assert!(!r.one_click, "unverified: install it yourself");
    assert_eq!(r.reason_code.as_deref(), Some("source_not_verified"));
    h.source_verified_by = Some("owner".into());
    h.source_verified_on = Some("2026-11-11".into());
    h.verification_ref = Some("ref".into());
    let r = super::row(&h, &Fake::default(), &none);
    assert!(r.one_click);
    assert_eq!(r.reason_code.as_deref(), Some("not_installed"));
    assert!(super::row(&h, &Fake::default().with("claude", 1, ""), &none).one_click, "a broken one can be repaired");
    assert!(!super::row(&h, &Fake::default().with("claude", 0, "2.1.322 (Claude Code)"), &none).one_click, "installed: nothing to do");
}
