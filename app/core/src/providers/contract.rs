//! The wire contract for providers (`vajra.providers/1`) and the provider facts `project_state` carries. Types, serde and
//! validation only: no detection, no subprocess, no file access. The shared fixtures in `tests/contract/` are parsed by
//! the core (`contract_tests.rs`) and by the UI (`app/ui/tests/providerContract.test.ts`).
//!
//! Rules, as for the project contract: unknown fields are ignored; another major version is reported as such, never
//! misread; an unknown login state reads as `cannot_tell` (never as signed in); an unknown "tested" value reads as
//! `unknown`. Nothing here can carry a token: there is no field for one.

use serde::{Deserialize, Serialize};

use crate::projects::contract::{ContractError, schema_major, valid_label, valid_provider};

pub const PROVIDERS_SCHEMA: &str = "vajra.providers/1";
pub const PROVIDERS_MAJOR: u32 = 1;
/// A providers answer larger than this is refused unread.
pub const MAX_BYTES: usize = 64 * 1024;
const MAX_PROVIDERS: usize = 8;
const MAX_LIMITS: usize = 16;
const MAX_TEXT_CHARS: usize = 200;

/// The result of the read-only login check. `cannot_tell` never blocks a start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginState {
    SignedIn,
    SignedOut,
    NotInstalled,
    /// The CLI has no status command, it timed out, or its answer was not understood.
    #[serde(other)]
    CannotTell,
}

/// Whether this version of the CLI is inside the range the adapter was tested with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tested {
    Tested,
    Untested,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopKind {
    HookBlock,
    Terminate,
    #[serde(other)]
    Other,
}

/// What the adapter can do (mirrors `adapter::Capabilities`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityFlags {
    pub pty_attach: bool,
    pub transcript: bool,
    pub inspect: bool,
    pub message: bool,
    pub redirect: bool,
    pub stop: StopKind,
    pub resume: bool,
    pub vajra_workflow: bool,
}

/// One control that is off, with the words to show beside it (never hidden).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limit {
    /// A capability name: `message`, `redirect`, `inspect`, `transcript`, `vajra_workflow`, `resume` or `start`.
    pub control: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    /// Plain words for the route, e.g. "Uses your Claude Code login". Never a credential.
    pub route: String,
    pub state: LoginState,
    /// `cli_status` when the CLI's own status command answered, `none` when the CLI has no such command.
    pub checked_by: String,
    #[serde(default)]
    pub detail: Option<String>,
}

/// Where usage is billed: always the CLI's own login. `key_in_app_env` names (never holds) a key the app saw and did
/// not pass on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillingInfo {
    pub source: String,
    #[serde(default)]
    pub key_in_app_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// The adapter id: `claude`, `codex`.
    pub id: String,
    pub name: String,
    pub installed: bool,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    pub tested: Tested,
    #[serde(default)]
    pub tested_range: Option<String>,
    pub capabilities: CapabilityFlags,
    #[serde(default)]
    pub limits: Vec<Limit>,
    pub login: Login,
    pub billing: BillingInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderList {
    pub schema: String,
    pub providers: Vec<ProviderInfo>,
}

/// The provider facts in `project_state` (additive to `vajra.project_state/1`): the project's chosen provider, whether it
/// is usable, and what its controls can do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectProvider {
    pub id: String,
    pub installed: bool,
    pub login_state: LoginState,
    pub capabilities: CapabilityFlags,
    #[serde(default)]
    pub limits: Vec<Limit>,
}

fn text_ok(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= MAX_TEXT_CHARS && !s.chars().any(|c| c.is_control() || crate::control::lead::invisible(c))
}

fn limits_ok(l: &[Limit]) -> Result<(), ContractError> {
    if l.len() > MAX_LIMITS {
        return Err(ContractError::Invalid("limits"));
    }
    for x in l {
        if !valid_label(&x.control) || !text_ok(&x.reason) {
            return Err(ContractError::Invalid("limits"));
        }
    }
    Ok(())
}

impl ProviderInfo {
    pub fn validate(&self) -> Result<(), ContractError> {
        if !valid_provider(&self.id) {
            return Err(ContractError::Invalid("provider id"));
        }
        if !text_ok(&self.name) || !text_ok(&self.login.route) || !valid_label(&self.login.checked_by) {
            return Err(ContractError::Invalid("text"));
        }
        for (v, what) in [(&self.version, "version"), (&self.tested_range, "tested_range"), (&self.login.detail, "detail")] {
            if v.as_deref().is_some_and(|s| !text_ok(s)) {
                return Err(ContractError::Invalid(what));
            }
        }
        if self.path.as_deref().is_some_and(|p| !p.starts_with('/') || p.len() > 4096 || p.contains('\0')) {
            return Err(ContractError::Invalid("path"));
        }
        if self.installed != self.path.is_some() || (self.login.state == LoginState::NotInstalled) == self.installed {
            return Err(ContractError::Invalid("installed"));
        }
        if self.billing.source != "login" {
            return Err(ContractError::Invalid("billing"));
        }
        // A key NAME only (an environment variable name), never a value.
        if self.billing.key_in_app_env.as_deref().is_some_and(|k| k.is_empty() || k.len() > 64 || !k.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')) {
            return Err(ContractError::Invalid("billing"));
        }
        limits_ok(&self.limits)
    }
}

impl ProjectProvider {
    pub fn validate(&self) -> Result<(), ContractError> {
        if !valid_provider(&self.id) || (self.login_state == LoginState::NotInstalled) == self.installed {
            return Err(ContractError::Invalid("provider"));
        }
        limits_ok(&self.limits)
    }
}

/// Parse and validate a `provider_list` answer. The schema tag is read first, so another major reads as unsupported.
pub fn parse_provider_list(bytes: &[u8]) -> Result<ProviderList, ContractError> {
    if bytes.len() > MAX_BYTES {
        return Err(ContractError::TooLarge);
    }
    #[derive(Deserialize)]
    struct Tag {
        schema: String,
    }
    let t: Tag = serde_json::from_slice(bytes).map_err(|e| ContractError::Malformed(e.to_string()))?;
    if schema_major(&t.schema, "providers") != Some(PROVIDERS_MAJOR) {
        return Err(ContractError::UnsupportedVersion { found: t.schema.chars().take(64).collect(), supported: PROVIDERS_MAJOR });
    }
    let l: ProviderList = serde_json::from_slice(bytes).map_err(|e| ContractError::Malformed(e.to_string()))?;
    if l.providers.len() > MAX_PROVIDERS {
        return Err(ContractError::Invalid("providers"));
    }
    let mut seen = std::collections::HashSet::new();
    for p in &l.providers {
        p.validate()?;
        if !seen.insert(p.id.as_str()) {
            return Err(ContractError::Invalid("provider id"));
        }
    }
    Ok(l)
}

/// Parse and validate the `provider` object of a `project_state` answer.
pub fn parse_project_provider(bytes: &[u8]) -> Result<ProjectProvider, ContractError> {
    if bytes.len() > MAX_BYTES {
        return Err(ContractError::TooLarge);
    }
    let p: ProjectProvider = serde_json::from_slice(bytes).map_err(|e| ContractError::Malformed(e.to_string()))?;
    p.validate()?;
    Ok(p)
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
