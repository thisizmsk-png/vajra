use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::*;
use crate::providers::detect::Out;
use crate::providers::keychain::{Error, Key, Store};

#[derive(Default)]
struct Fake {
    bins: HashMap<&'static str, &'static str>,
    answers: HashMap<(&'static str, &'static str), (i32, &'static str)>,
}
impl Probe for Fake {
    fn find(&self, name: &str) -> Option<PathBuf> {
        self.bins.get(name).map(PathBuf::from)
    }
    fn run(&self, path: &Path, args: &[&str]) -> Option<Out> {
        let key = (path.to_str().unwrap(), args.join(" "));
        self.answers.iter().find(|((p, a), _)| *p == key.0 && *a == key.1).map(|(_, (c, s))| Out { code: Some(*c), stdout: (*s).into(), stderr: String::new() })
    }
    fn env(&self, _: &str) -> Option<OsString> {
        None
    }
}

fn machine(claude: &'static str, codex: &'static str) -> Fake {
    let mut f = Fake::default();
    f.bins.insert("claude", "/bin/claude");
    f.bins.insert("codex", "/bin/codex");
    f.answers.insert(("/bin/claude", "--version"), (0, claude));
    f.answers.insert(("/bin/codex", "--version"), (0, codex));
    f
}
fn good() -> Fake {
    machine("2.1.296 (Claude Code)\n", "codex-cli 0.166.0\n")
}

/// A Store that only answers `exists`: any other call, or any attempt to read a value, panics.
struct Exists(Result<bool, Error>);
impl Store for Exists {
    fn exists(&self, _: &str, _: &str) -> Result<bool, Error> {
        self.0
    }
    fn add(&self, _: &str, _: &str, _: &Key) -> Result<(), Error> {
        panic!("a launch check never writes a key")
    }
    fn delete(&self, _: &str, _: &str) -> Result<bool, Error> {
        panic!("a launch check never deletes a key")
    }
}
fn keys(r: Result<bool, Error>) -> Keys {
    Keys::new(Box::new(Exists(r)))
}

fn choice(account: &str) -> Choice {
    Choice { account: account.into(), model: None, effort: None }
}
fn agents(accounts: &[&str]) -> Agents {
    let mut a = Agents::from_legacy(crate::preferences::Agent::Claude);
    a.allowed = accounts.iter().map(|x| crate::preferences::Allowed { account: x.to_string(), model: None }).collect();
    a
}
fn check(route: bool, account: &str, kind: Kind, p: &Fake, k: &Keys) -> Result<(), Refused> {
    launchable_with(route, &choice(account), kind, &agents(&["claude", "codex", "openrouter"]), p, k)
}
fn code(r: Result<(), Refused>) -> &'static str {
    r.map_or_else(|e| e.code, |_| "ok")
}

#[test]
fn a_tested_installed_harness_passes_for_every_role_kind() {
    for kind in [Kind::Lead, Kind::Writer, Kind::Checker] {
        for a in ["claude", "codex"] {
            assert_eq!(code(check(true, a, kind, &good(), &keys(Ok(false)))), "ok", "{a}");
        }
    }
}

#[test]
fn the_route_is_off_in_this_build() {
    const { assert!(!CORE_ROUTE_AVAILABLE) };
    assert_eq!(code(launchable(&choice("claude"), Kind::Writer, &agents(&["claude"]), &good(), &keys(Ok(true)))), "core_route_unavailable");
}

#[test]
fn each_refusal_code_has_a_seat_that_causes_it() {
    let k = keys(Ok(false));
    // 1: outside the ceiling.
    let r = launchable_with(true, &choice("codex"), Kind::Writer, &agents(&["claude"]), &good(), &k);
    assert_eq!(code(r), "outside_allowed");
    // 2: not installed, and installed but silent about its version.
    let mut f = good();
    f.bins.remove("codex");
    assert_eq!(code(check(true, "codex", Kind::Writer, &f, &k)), "harness_missing");
    let mut f = good();
    f.answers.remove(&("/bin/codex", "--version"));
    assert_eq!(code(check(true, "codex", Kind::Writer, &f, &k)), "harness_broken");
    // 3: a gateway account with no saved key, and with a Keychain that fails.
    assert_eq!(code(check(true, "openrouter", Kind::Writer, &good(), &keys(Ok(false)))), "key_missing");
    assert_eq!(code(check(true, "openrouter", Kind::Writer, &good(), &keys(Err(Error::Unavailable)))), "key_unreadable");
    assert_eq!(code(check(true, "openrouter", Kind::Writer, &good(), &keys(Ok(true)))), "ok");
    // 4: an account no adapter runs, and a version outside the tested range.
    assert_eq!(code(launchable_with(true, &choice("gemini"), Kind::Writer, &agents(&["gemini"]), &good(), &k)), "harness_unsupported");
    let f = machine("2.1.296 (Claude Code)\n", "codex-cli 0.170.0\n");
    let e = check(true, "codex", Kind::Writer, &f, &k).unwrap_err();
    assert_eq!(e.code, "harness_unsupported");
    assert!(e.detail.contains("0.170.0") && e.detail.contains("tested: 0.166"), "{}", e.detail);
    // 5: the route.
    assert_eq!(code(check(false, "claude", Kind::Writer, &good(), &k)), "core_route_unavailable");
}

#[test]
fn engine_checks_need_the_scripts_and_a_tested_version() {
    let d = std::env::temp_dir().join(format!("vajra-launch-engine-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(engine_ready(&d).unwrap_err().code, "engine_missing");
    let root = d.join(".claude/skills/vajra");
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    for f in ENGINE_FILES {
        std::fs::write(root.join(f), "").unwrap();
    }
    assert!(engine_ready(&d).unwrap_err().detail.contains("install.sh"));
    assert_eq!(engine_ready(&d).unwrap_err().code, "engine_unsupported", "no version file");
    std::fs::write(root.join("package.json"), r#"{"version": "0.6.2"}"#).unwrap();
    assert!(engine_ready(&d).is_ok());
    std::fs::write(root.join("package.json"), r#"{"version": "0.5.9"}"#).unwrap();
    assert_eq!(engine_ready(&d).unwrap_err().code, "engine_unsupported");
    let _ = std::fs::remove_dir_all(&d);
}
