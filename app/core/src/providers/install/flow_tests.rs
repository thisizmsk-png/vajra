use std::sync::Arc;
use std::time::Instant;

use super::super::plan::integrity;
use super::super::testkit::{tgz, FakeNpm};
use super::*;
use crate::bus::{Bus, Config};
use crate::providers::catalog::{self, Method};

const NOT_INSTALLED: &str = r#"{"dependencies":{}}"#;

fn bytes() -> Vec<u8> {
    tgz(&[("package/package.json", r#"{"name":"x","version":"1.0.0","bin":{"x":"cli.js"}}"#), ("package/cli.js", "1")])
}

fn harness(verified: bool, methods: Vec<Method>) -> catalog::Harness {
    let mut h = catalog::catalog().unwrap().harnesses[0].clone();
    h.id = "x".into();
    h.binary = "x".into();
    h.source_verified_by = verified.then(|| "owner".into());
    h.source_verified_on = verified.then(|| "2026-11-11".into());
    h.verification_ref = verified.then(|| "ref".into());
    h.methods = methods;
    h
}

fn npm_method() -> Method {
    Method::Npm(catalog::Npm { package: "x".into(), version: "1.0.0".into(), integrity: integrity(&bytes()), command: "npm install -g x".into() })
}

fn me() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

fn world(f: &FakeNpm) -> Machine {
    Machine { tools: Tools { npm: Some(f.npm()), ..Tools::default() }, uid: me(), tag: Some("arm64_sequoia".into()), get: Box::new(|_| None) }
}

#[test]
fn an_unverified_source_is_always_install_it_yourself() {
    let f = FakeNpm::new("flow-unverified", &bytes(), NOT_INSTALLED, None);
    assert_eq!(offer(&harness(false, vec![npm_method()]), &world(&f)).err(), Some("source_not_verified"));
    assert!(!f.log().contains("pack"), "nothing runs for an unverified source");
}

#[test]
fn a_verified_npm_entry_gets_a_plan_from_the_resolved_npm() {
    let f = FakeNpm::new("flow-npm", &bytes(), NOT_INSTALLED, None);
    let o = offer(&harness(true, vec![npm_method()]), &world(&f)).unwrap();
    assert_eq!(o.plan.method, "npm");
    assert!(o.env.vars().iter().any(|(k, v)| *k == "PATH" && v.to_string_lossy().starts_with(&f.dir.join("tool/bin").display().to_string())), "node's folder leads the Setup PATH");
}

#[test]
fn a_missing_tool_falls_through_to_the_next_method_and_the_first_reason_is_kept() {
    let f = FakeNpm::new("flow-none", &bytes(), NOT_INSTALLED, None);
    let mut m = world(&f);
    m.tools.npm = None;
    let brew = Method::BrewCask(catalog::Brew { package: "x".into(), tap: Some("homebrew/cask".into()), cask_url_hosts: vec![], command: "brew install --cask x".into() });
    assert_eq!(offer(&harness(true, vec![npm_method(), brew]), &m).err(), Some("no_npm"));
}

#[test]
fn the_run_needs_a_token_for_the_plan_it_recomputes() {
    let f = FakeNpm::new("flow-run", &bytes(), NOT_INSTALLED, None);
    std::fs::write(f.dir.join("install.ls"), r#"{"dependencies":{"x":{"version":"1.0.0"}}}"#).unwrap();
    let data = f.dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let bus = Arc::new(Bus::open(&data.join("spill"), Config::default()).unwrap());
    let sup = Supervisor::new(bus, None);
    let h = harness(true, vec![npm_method()]);
    let m = world(&f);
    let consents = Consents::default();
    let shown = offer(&h, &m).unwrap().plan;
    let token = consents.mint("x", &shown.hash, Instant::now());
    let paths = Paths { data: &data, audit: &data.join("audit.log") };
    let current = || None;
    // A token for another plan is refused, and nothing installs.
    let other = consents.mint("x", &"0".repeat(64), Instant::now());
    assert_eq!(run(&h, &m, &consents, &other, &sup, paths, &current).err(), Some("consent_required"));
    assert!(!f.log().contains("install -g"));
    let r = run(&h, &m, &consents, &token, &sup, paths, &current).unwrap();
    assert_eq!(r.outcome, super::super::run::Outcome::Installed);
    assert_eq!(run(&h, &m, &consents, &token, &sup, paths, &current).err(), Some("consent_required"), "a token works once");
}
