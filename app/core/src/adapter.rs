//! Agent adapters: what the app can do with each stock agent CLI.
//!
//! The app runs the user's own `claude` or `codex` binary in a PTY under the
//! user's login (`Supervisor::spawn`, scrubbed env). It never reads, stores or
//! proxies a credential: no `--bare`, no SDK, no key forwarding. Spawning,
//! streaming, PTY attach and kill are engine-neutral and live in `sessions`;
//! an adapter supplies argv and says honestly which of the rest it supports,
//! so the UI greys out what an engine cannot do.

use std::ffi::OsString;

pub mod claude_selftest;
pub mod profiles;

use crate::sessions::argv::{self, InvalidInput, RunInput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Claude,
    Codex,
}

impl Engine {
    /// Engine id, shown only in the inspector (AC-2.1.3).
    pub fn id(self) -> &'static str {
        match self {
            Engine::Claude => "claude",
            Engine::Codex => "codex",
        }
    }

    pub fn from_id(id: &str) -> Option<Engine> {
        [Engine::Claude, Engine::Codex].into_iter().find(|e| e.id() == id)
    }
}

/// How a harness is authenticated and paid for. Not an engine: Codex under its own login and Codex through a gateway are
/// one engine on two routes with different billing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The CLI owns its login.
    CliLogin,
    /// A key the user pasted, held in the Keychain.
    ApiKey,
    /// A non-secret base URL plus a Keychain key.
    Gateway,
}

/// One way of running and paying for agents: an id the preferences name, the harness that runs the worker, and the route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub engine: Engine,
    pub route: Route,
}

impl Account {
    /// The built-in account for an engine under its own login; its id is the engine id.
    pub fn login(engine: Engine) -> Account {
        Account { id: engine.id().into(), engine, route: Route::CliLogin }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopKind {
    /// Sticky block through the desktop-control hook; the worker hands back.
    HookBlock,
    /// No hook channel: a stop terminates the PTY session.
    Terminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub pty_attach: bool,
    /// Transcript files the crew tailer can read.
    pub transcript: bool,
    pub inspect: bool,
    pub message: bool,
    pub redirect: bool,
    pub stop: StopKind,
    pub resume: bool,
    /// Runs `/vajra-deliver` (crew tree, tasks, verification).
    pub vajra_workflow: bool,
}

/// Where usage is billed. `source` is always "login": keys are never forwarded
/// (`sessions::ENV_ALLOW`), so the CLI bills through its own login or config.
/// `key_in_app_env` names (never values) a key the app saw but did not pass on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Billing {
    pub source: &'static str,
    pub key_in_app_env: Option<&'static str>,
}

pub trait AgentAdapter: Send + Sync {
    fn engine(&self) -> Engine;
    fn capabilities(&self) -> Capabilities;
    fn run_argv(&self, input: &RunInput, steer_nonce: &str) -> Result<Vec<String>, InvalidInput>;
    /// `session_id` is the session being resumed; for an engine with a steer
    /// channel the rotated `steer_nonce` goes into the resumed prompt.
    fn resume_argv(&self, session_id: &str, run_id: &str, campaign: &str, steer_nonce: &str) -> Result<Vec<String>, InvalidInput>;
    fn billing(&self, env: &dyn Fn(&str) -> Option<OsString>) -> Billing;
}

/// The launch argv for a Codex-harness account on the `ApiKey` or `Gateway` route. A login account, or a Claude one, is
/// refused here: it launches through `adapter_for(..).run_argv` and never gets a generated config. Not called by
/// `start_run` yet; a later PR wires it with `providers::codex_home::write` and account and model selection.
pub fn gateway_run_argv(account: &Account, ask: &str, model: &str, effort: Option<&str>) -> Result<Vec<String>, InvalidInput> {
    if account.engine != Engine::Codex || account.route == Route::CliLogin {
        return Err(InvalidInput { field: "provider", reason: "is not a Codex gateway account" });
    }
    argv::build_gateway_argv(ask, model, effort)
}

pub fn adapter_for(engine: Engine) -> &'static dyn AgentAdapter {
    match engine {
        Engine::Claude => &Claude,
        Engine::Codex => &Codex,
    }
}

struct Claude;
struct Codex;

pub(crate) const CLAUDE_KEYS: [&str; 2] = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];
pub(crate) const CODEX_KEYS: [&str; 2] = ["CODEX_API_KEY", "OPENAI_API_KEY"];

fn billing(keys: &[&'static str], env: &dyn Fn(&str) -> Option<OsString>) -> Billing {
    let key_in_app_env = keys.iter().copied().find(|k| env(k).is_some_and(|v| !v.is_empty()));
    Billing { source: "login", key_in_app_env }
}


impl AgentAdapter for Claude {
    fn engine(&self) -> Engine {
        Engine::Claude
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            pty_attach: true,
            transcript: true,
            inspect: true,
            message: true,
            redirect: true,
            stop: StopKind::HookBlock,
            resume: true,
            vajra_workflow: true,
        }
    }
    fn run_argv(&self, input: &RunInput, steer_nonce: &str) -> Result<Vec<String>, InvalidInput> {
        argv::build_run_argv(input, steer_nonce)
    }
    fn resume_argv(&self, session_id: &str, run_id: &str, campaign: &str, steer_nonce: &str) -> Result<Vec<String>, InvalidInput> {
        argv::build_resume_argv(session_id, run_id, campaign, steer_nonce)
    }
    fn billing(&self, env: &dyn Fn(&str) -> Option<OsString>) -> Billing {
        billing(&CLAUDE_KEYS, env)
    }
}

impl AgentAdapter for Codex {
    fn engine(&self) -> Engine {
        Engine::Codex
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            pty_attach: true,
            transcript: false,
            inspect: false,
            message: false,
            redirect: false,
            stop: StopKind::Terminate,
            // An app-started Codex run is never told the app's session id and Vajra does not read Codex's own, so there
            // is no id to resume. Offering one would hand `codex resume` an id Codex never saw.
            resume: false,
            vajra_workflow: false,
        }
    }
    /// `codex -- <ask>`: `--` keeps an ask like "exec ..." or "--flag" one positional
    /// prompt. No hook channel, so the nonce is not used.
    fn run_argv(&self, input: &RunInput, _steer_nonce: &str) -> Result<Vec<String>, InvalidInput> {
        argv::check_ask(&input.ask)?;
        Ok(vec!["codex".into(), "--".into(), input.ask.clone()])
    }
    fn resume_argv(&self, session_id: &str, _run_id: &str, _campaign: &str, _steer_nonce: &str) -> Result<Vec<String>, InvalidInput> {
        if !crate::ids::valid_session_id(session_id) {
            return Err(InvalidInput { field: "session_id", reason: "must be a canonical uuid" });
        }
        Ok(vec!["codex".into(), "resume".into(), session_id.into()])
    }
    fn billing(&self, env: &dyn Fn(&str) -> Option<OsString>) -> Billing {
        billing(&CODEX_KEYS, env)
    }
}
