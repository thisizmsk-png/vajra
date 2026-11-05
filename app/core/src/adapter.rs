//! Agent adapters: what the app can do with each stock agent CLI.
//!
//! The app runs the user's own `claude` or `codex` binary in a PTY under the
//! user's login (`Supervisor::spawn`, scrubbed env). It never reads, stores or
//! proxies a credential: no `--bare`, no SDK, no key forwarding. Spawning,
//! streaming, PTY attach and kill are engine-neutral and live in `sessions`;
//! an adapter supplies argv and says honestly which of the rest it supports,
//! so the UI greys out what an engine cannot do.

use std::ffi::OsString;

use crate::sessions::argv::{build_run_argv, InvalidInput, RunInput, MAX_ASK_BYTES};

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
    fn resume_argv(&self, session_id: &str) -> Result<Vec<String>, InvalidInput>;
    fn billing(&self, env: &dyn Fn(&str) -> Option<OsString>) -> Billing;
}

pub fn adapter_for(engine: Engine) -> &'static dyn AgentAdapter {
    match engine {
        Engine::Claude => &Claude,
        Engine::Codex => &Codex,
    }
}

struct Claude;
struct Codex;

const CLAUDE_KEYS: [&str; 2] = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];
const CODEX_KEYS: [&str; 2] = ["CODEX_API_KEY", "OPENAI_API_KEY"];

fn billing(keys: &[&'static str], env: &dyn Fn(&str) -> Option<OsString>) -> Billing {
    let key_in_app_env = keys.iter().copied().find(|k| env(k).is_some_and(|v| !v.is_empty()));
    Billing { source: "login", key_in_app_env }
}

/// Canonical lowercase/uppercase 8-4-4-4-12 hex uuid only.
fn check_uuid(id: &str) -> Result<(), InvalidInput> {
    let ok = id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        });
    if ok {
        Ok(())
    } else {
        Err(InvalidInput { field: "session_id", reason: "must be a session uuid" })
    }
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
        build_run_argv(input, steer_nonce)
    }
    fn resume_argv(&self, session_id: &str) -> Result<Vec<String>, InvalidInput> {
        check_uuid(session_id)?;
        Ok(vec!["claude".into(), "--resume".into(), session_id.into(), "/vajra continue".into()])
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
            resume: true,
            vajra_workflow: false,
        }
    }
    /// `codex -- <ask>`: `--` keeps an ask like "exec ..." or "--flag" one positional
    /// prompt. No hook channel, so the nonce is not used.
    fn run_argv(&self, input: &RunInput, _steer_nonce: &str) -> Result<Vec<String>, InvalidInput> {
        if input.ask.trim().is_empty() {
            return Err(InvalidInput { field: "ask", reason: "must not be empty" });
        }
        if input.ask.len() > MAX_ASK_BYTES {
            return Err(InvalidInput { field: "ask", reason: "is longer than 32 KiB" });
        }
        Ok(vec!["codex".into(), "--".into(), input.ask.clone()])
    }
    fn resume_argv(&self, session_id: &str) -> Result<Vec<String>, InvalidInput> {
        check_uuid(session_id)?;
        Ok(vec!["codex".into(), "resume".into(), session_id.into()])
    }
    fn billing(&self, env: &dyn Fn(&str) -> Option<OsString>) -> Billing {
        billing(&CODEX_KEYS, env)
    }
}
