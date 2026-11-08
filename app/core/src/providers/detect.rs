//! Provider detection and the read-only login check. For each adapter this finds the CLI on the resolved PATH, asks it
//! `--version`, and runs the CLI's OWN login status command (never reading a token, keychain item or config file
//! ourselves: the CLI owns its login). A status that does not answer in time, answers something unexpected, or does
//! not exist reads as `cannot_tell`, which never blocks a start. Only the exit code and the one boolean we need are
//! used; the status output (it can name an account) is dropped, not stored or sent anywhere.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::contract::{BillingInfo, CapabilityFlags, Limit, Login, LoginState, ProviderInfo, ProviderList, StopKind, Tested, PROVIDERS_SCHEMA};
use crate::adapter::{self, AgentAdapter, Engine};

/// The status commands answer in well under a second; a hung one is `cannot_tell`.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The result of one CLI run: only when it finished whole (not killed, not cut at the output cap).
pub struct Out {
    pub code: Option<i32>,
    pub stdout: String,
    /// Codex prints "Not logged in" here, not on stdout.
    pub stderr: String,
}

/// What detection needs from the machine; the tests supply a fake one.
pub trait Probe {
    fn find(&self, name: &str) -> Option<PathBuf>;
    /// Run `path args...` with the scrubbed environment and a timeout. `None` when it did not finish whole.
    fn run(&self, path: &Path, args: &[&str]) -> Option<Out>;
    fn env(&self, key: &str) -> Option<OsString>;
}

/// The real machine: the user's PATH dirs, the bridge's scrubbed-env runner.
pub struct System {
    dirs: Vec<PathBuf>,
}

impl System {
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        Self { dirs }
    }
}

impl System {
    /// Whether the CLI for `engine` is on the PATH; no process is run.
    pub fn installed(&self, engine: Engine) -> bool {
        self.find(engine.id()).is_some()
    }
}

impl Probe for System {
    fn find(&self, name: &str) -> Option<PathBuf> {
        crate::vajra::preflight::find_on_path(name, &self.dirs)
    }
    fn run(&self, path: &Path, args: &[&str]) -> Option<Out> {
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let o = crate::vajra::bridge::run_process_on_path(path, &argv, None, TIMEOUT, &self.dirs).ok()?;
        o.whole().then_some(Out { code: o.code, stdout: o.stdout, stderr: o.stderr })
    }
    fn env(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }
}

/// (major, minor) bounds the adapter's argv and status handling were checked against.
struct Range {
    min: (u32, u32),
    max: (u32, u32),
    text: &'static str,
}

struct Meta {
    engine: Engine,
    name: &'static str,
    route: &'static str,
    tested: Range,
    status: &'static [&'static str],
}

const METAS: [Meta; 2] = [
    Meta { engine: Engine::Claude, name: "Claude Code", route: "Uses your Claude Code login", tested: Range { min: (2, 0), max: (2, 999), text: "2.x" }, status: &["auth", "status", "--json"] },
    Meta { engine: Engine::Codex, name: "Codex CLI", route: "Uses your Codex login", tested: Range { min: (0, 166), max: (0, 166), text: "0.166" }, status: &["login", "status"] },
];

/// The first `x.y` or `x.y.z` token in a `--version` line ("2.1.296 (Claude Code)", "codex-cli 0.166.0").
fn version_of(text: &str) -> Option<(String, (u32, u32))> {
    for tok in text.split_whitespace() {
        let parts: Vec<&str> = tok.split('.').collect();
        if parts.len() >= 2 && parts.len() <= 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 6 && p.bytes().all(|b| b.is_ascii_digit())) {
            let n = |i: usize| parts[i].parse::<u32>().unwrap_or(0);
            return Some((tok.to_string(), (n(0), n(1))));
        }
    }
    None
}

fn flags(c: adapter::Capabilities) -> CapabilityFlags {
    CapabilityFlags {
        pty_attach: c.pty_attach,
        transcript: c.transcript,
        inspect: c.inspect,
        message: c.message,
        redirect: c.redirect,
        stop: match c.stop {
            adapter::StopKind::HookBlock => StopKind::HookBlock,
            adapter::StopKind::Terminate => StopKind::Terminate,
        },
        resume: c.resume,
        vajra_workflow: c.vajra_workflow,
    }
}

/// One reason per control that is off or behaves differently, with a stable code for the UI's own wording.
fn limits(name: &str, c: &CapabilityFlags, installed: bool) -> Vec<Limit> {
    let mut out = Vec::new();
    let mut add = |control: &str, code: &str, reason: String| out.push(Limit { control: control.into(), code: Some(code.into()), reason });
    for (on, control, code, what) in [
        (c.message, "message", "no_message_mid_run", "take a message mid-run"),
        (c.redirect, "redirect", "no_redirect_mid_run", "be redirected mid-run"),
        (c.inspect, "inspect", "no_inspect", "be inspected mid-run"),
        (c.transcript, "transcript", "no_transcript", "show a transcript"),
        (c.vajra_workflow, "vajra_workflow", "no_vajra_workflow", "run the Vajra workflow"),
    ] {
        if !on {
            add(control, code, format!("{name} cannot {what}"));
        }
    }
    if c.stop == StopKind::Terminate {
        add("stop", "stop_ends_process", format!("{name} has no hand-off: Stop ends its process"));
    }
    if installed && !c.resume {
        add("resume", "no_resume_session", format!("{name} cannot be resumed: Vajra does not get its session id"));
    }
    if !installed {
        add("start", "not_installed", format!("{name} is not installed"));
        add("resume", "not_installed", format!("{name} is not installed"));
    }
    out
}

/// A chosen provider id this build has no adapter for (the preference names something removed or from a newer build):
/// present, not installed, and unable to start, rather than absent (absent means "none chosen").
pub fn unsupported(id: &str) -> super::contract::ProjectProvider {
    let off = CapabilityFlags { pty_attach: false, transcript: false, inspect: false, message: false, redirect: false, stop: StopKind::Other, resume: false, vajra_workflow: false };
    let limit = |control: &str| Limit { control: control.into(), code: Some("not_supported".into()), reason: "This version of Vajra does not support that provider".into() };
    super::contract::ProjectProvider {
        id: id.into(),
        name: id.into(),
        installed: false,
        tested: Tested::Unknown,
        login_state: LoginState::NotInstalled,
        capabilities: off,
        limits: vec![limit("start"), limit("resume")],
    }
}

/// Read the CLI's own status answer. Claude prints JSON with `loggedIn`; Codex answers by exit code and prints
/// "Not logged in" when signed out. Anything else (a CLI too old to have the command exits with a usage error, a
/// timeout, a cut answer, garbage) is `cannot_tell`.
fn login_state(engine: Engine, out: Option<Out>) -> (LoginState, Option<(&'static str, &'static str)>) {
    let unsure = |code, why| (LoginState::CannotTell, Some((code, why)));
    let Some(o) = out else { return unsure("check_unfinished", "The login check did not finish") };
    match engine {
        // Asymmetric on purpose: `signed_in` needs BOTH `loggedIn: true` and exit 0 (the one state that says things are fine,
        // so it must be earned twice), while `loggedIn: false` is signed out whatever the exit code (the CLI said so
        // itself, and signed out only shows a hint, it never blocks a start). Anything else is `cannot_tell`.
        Engine::Claude => match serde_json::from_str::<serde_json::Value>(&o.stdout).ok().and_then(|v| v.get("loggedIn").and_then(|b| b.as_bool())) {
            Some(true) if o.code == Some(0) => (LoginState::SignedIn, None),
            Some(false) => (LoginState::SignedOut, None),
            _ => unsure("check_not_understood", "The login check gave an answer Vajra does not understand"),
        },
        Engine::Codex => match o.code {
            Some(0) => (LoginState::SignedIn, None),
            Some(1) if format!("{}\n{}", o.stdout, o.stderr).to_ascii_lowercase().contains("not logged in") => (LoginState::SignedOut, None),
            _ => unsure("check_not_understood", "The login check gave an answer Vajra does not understand"),
        },
    }
}

fn describe(m: &Meta, p: &dyn Probe) -> ProviderInfo {
    let adapter: &dyn AgentAdapter = adapter::adapter_for(m.engine);
    let caps = flags(adapter.capabilities());
    let bill = adapter.billing(&|k| p.env(k));
    let billing = BillingInfo { source: bill.source.into(), key_in_app_env: bill.key_in_app_env.map(str::to_string) };
    let path = p.find(m.engine.id());
    let login = |state, checked_by: &str, detail: Option<(&str, &str)>| Login {
        route: m.route.into(),
        route_code: Some(format!("uses_{}_login", m.engine.id())),
        state,
        checked_by: checked_by.into(),
        detail: detail.map(|d| d.1.to_string()),
        detail_code: detail.map(|d| d.0.to_string()),
    };
    let Some(path) = path else {
        return ProviderInfo {
            id: m.engine.id().into(),
            name: m.name.into(),
            installed: false,
            path: None,
            version: None,
            tested: Tested::Unknown,
            tested_range: Some(m.tested.text.into()),
            limits: limits(m.name.split(' ').next().unwrap_or(m.name), &caps, false),
            capabilities: caps,
            login: login(LoginState::NotInstalled, "none", None),
            billing,
        };
    };
    let ver = p.run(&path, &["--version"]).filter(|o| o.code == Some(0)).and_then(|o| version_of(&o.stdout));
    let tested = match &ver {
        None => Tested::Unknown,
        Some((_, mm)) if *mm >= m.tested.min && *mm <= m.tested.max => Tested::Tested,
        Some(_) => Tested::Untested,
    };
    let (state, detail) = login_state(m.engine, p.run(&path, m.status));
    ProviderInfo {
        id: m.engine.id().into(),
        name: m.name.into(),
        installed: true,
        path: Some(path.to_string_lossy().into_owned()),
        version: ver.map(|(v, _)| v),
        tested,
        tested_range: Some(m.tested.text.into()),
        limits: limits(m.name.split(' ').next().unwrap_or(m.name), &caps, true),
        capabilities: caps,
        login: login(state, "cli_status", detail),
        billing,
    }
}

/// Every adapter, in a fixed order. Always answers (a missing or broken CLI is a row, not an error).
pub fn list(p: &dyn Probe) -> ProviderList {
    ProviderList { schema: PROVIDERS_SCHEMA.into(), providers: METAS.iter().map(|m| describe(m, p)).collect() }
}

/// One provider by id, for start and resume.
pub fn one(p: &dyn Probe, id: &str) -> Option<ProviderInfo> {
    METAS.iter().find(|m| m.engine.id() == id).map(|m| describe(m, p))
}

#[cfg(test)]
#[path = "detect_tests.rs"]
mod tests;
