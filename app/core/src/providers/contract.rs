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
    /// A stable code for this reason (`no_message_mid_run`, `stop_ends_process`, `not_installed`, `not_supported`), so a
    /// UI can show its own approved wording. `reason` stays as the fallback text for a code the UI does not know.
    #[serde(default)]
    pub code: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    /// Plain words for the route, e.g. "Uses your Claude Code login". Never a credential.
    pub route: String,
    /// Stable code for the route (`uses_claude_login`, `uses_codex_login`); `route` is the fallback text.
    #[serde(default)]
    pub route_code: Option<String>,
    pub state: LoginState,
    /// `cli_status` when the CLI's own status command answered, `none` when the CLI has no such command.
    pub checked_by: String,
    #[serde(default)]
    pub detail: Option<String>,
    /// Stable code for `detail` (`check_unfinished`, `check_not_understood`); `detail` is the fallback text.
    #[serde(default)]
    pub detail_code: Option<String>,
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
    /// Display name ("Codex CLI") and whether the version is tested, so Crew and Run need no second `provider_list` read.
    pub name: String,
    pub installed: bool,
    pub tested: Tested,
    /// From the LAST `provider_list` check; reading `project_state` never runs a CLI.
    pub login_state: LoginState,
    pub capabilities: CapabilityFlags,
    #[serde(default)]
    pub limits: Vec<Limit>,
}

fn text_ok(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= MAX_TEXT_CHARS && !s.chars().any(|c| c.is_control() || crate::control::lead::invisible(c))
}

/// A stable code: lowercase letters, digits and underscores, 1 to 64.
fn valid_code(c: &str) -> bool {
    !c.is_empty() && c.len() <= 64 && c.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn limits_ok(l: &[Limit]) -> Result<(), ContractError> {
    if l.len() > MAX_LIMITS {
        return Err(ContractError::Invalid("limits"));
    }
    for x in l {
        if !valid_label(&x.control) || !text_ok(&x.reason) || x.code.as_deref().is_some_and(|c| !valid_code(c)) {
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
        for (c, what) in [(&self.login.route_code, "route_code"), (&self.login.detail_code, "detail_code")] {
            if c.as_deref().is_some_and(|c| !valid_code(c)) {
                return Err(ContractError::Invalid(what));
            }
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
        if !text_ok(&self.name) || !valid_provider(&self.id) || (self.login_state == LoginState::NotInstalled) == self.installed {
            return Err(ContractError::Invalid("provider"));
        }
        limits_ok(&self.limits)
    }
}

// ---- `vajra.provider_accounts/1`: one record per way of running and paying for agents ------------------------------------
//
// Additive to `vajra.providers/1`. Same rules: unknown fields ignored, another major reported, an unknown state reads as
// `cannot_tell`, an unknown billing kind as `unknown`. There is no field that can hold a key, token or secret: a stored key
// is only the boolean `credential.saved`. There is no rotation field and none will be added (H7).

pub const ACCOUNTS_SCHEMA: &str = "vajra.provider_accounts/1";
pub const ACCOUNTS_MAJOR: u32 = 1;
const MAX_ACCOUNTS: usize = 16;
const MAX_MODELS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSource {
    /// The CLI owns its login; Vajra reads only the CLI's own status.
    CliLogin,
    /// A key the user pasted, held in the Keychain (written by a later change).
    ApiKey,
    /// A non-secret base URL plus a Keychain key.
    Gateway,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    pub source: CredentialSource,
    /// `api_key` and `gateway` only: whether a key is stored. Never the key, a prefix or a suffix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingKind {
    Subscription,
    ApiKey,
    Gateway,
    #[serde(other)]
    Unknown,
}

/// An API key that would bill instead of the subscription. Holds the NAME of the setting and where it was found, never a
/// value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverrideWarning {
    /// An environment variable or setting name (`OPENAI_API_KEY`, `apiKeyHelper`).
    pub name: String,
    /// `cli_env_block`, `cli_config` or `cli_api_key_helper`.
    pub found_in: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountBilling {
    pub kind: BillingKind,
    /// Only what the CLI itself prints (a plan, a masked email). Never guessed, never a key.
    #[serde(default)]
    pub account_label: Option<String>,
    #[serde(default)]
    pub override_warning: Option<OverrideWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    SignedIn,
    SignedOut,
    NotInstalled,
    KeySaved,
    KeyMissing,
    /// Also the reading of any state this version does not know. Never blocks a start.
    #[serde(other)]
    CannotTell,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountModel {
    /// Opaque; passed straight through to the harness.
    pub id: String,
    /// Picks the icon (`claude`, `gpt`, `grok`, `deepseek`, ...).
    pub family: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub provider: String,
    /// The unmodified CLI that runs the worker (`claude`, `codex`). A gateway is a route, not a harness.
    pub harness: String,
    pub credential: Credential,
    pub billing: AccountBilling,
    pub label: String,
    pub state: AccountState,
    #[serde(default)]
    pub models: Vec<AccountModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountList {
    pub schema: String,
    pub accounts: Vec<Account>,
}

/// A model id: opaque, bounded, no whitespace or control characters.
pub(crate) fn valid_model_id(m: &str) -> bool {
    !m.is_empty() && m.len() <= 80 && m.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._/:".contains(&b))
}

/// A setting or environment variable name.
fn valid_setting_name(k: &str) -> bool {
    !k.is_empty() && k.len() <= 64 && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl Account {
    pub fn validate(&self) -> Result<(), ContractError> {
        if !valid_provider(&self.id) || !valid_provider(&self.provider) || !valid_provider(&self.harness) {
            return Err(ContractError::Invalid("account id"));
        }
        if !text_ok(&self.label) || self.billing.account_label.as_deref().is_some_and(|l| !text_ok(l)) {
            return Err(ContractError::Invalid("text"));
        }
        // `saved` belongs to a stored key and to nothing else.
        if (self.credential.source == CredentialSource::CliLogin) != self.credential.saved.is_none() {
            return Err(ContractError::Invalid("credential"));
        }
        if let Some(w) = &self.billing.override_warning {
            if !valid_setting_name(&w.name) || !["cli_env_block", "cli_config", "cli_api_key_helper"].contains(&w.found_in.as_str()) {
                return Err(ContractError::Invalid("override_warning"));
            }
        }
        if self.models.len() > MAX_MODELS || self.models.iter().any(|m| !valid_model_id(&m.id) || !valid_provider(&m.family)) {
            return Err(ContractError::Invalid("models"));
        }
        Ok(())
    }
}

/// Parse and validate a `provider_accounts` answer. The schema tag is read first, so another major reads as unsupported.
pub fn parse_account_list(bytes: &[u8]) -> Result<AccountList, ContractError> {
    if bytes.len() > MAX_BYTES {
        return Err(ContractError::TooLarge);
    }
    #[derive(Deserialize)]
    struct Tag {
        schema: String,
    }
    let t: Tag = serde_json::from_slice(bytes).map_err(|e| ContractError::Malformed(e.to_string()))?;
    if schema_major(&t.schema, "provider_accounts") != Some(ACCOUNTS_MAJOR) {
        return Err(ContractError::UnsupportedVersion { found: t.schema.chars().take(64).collect(), supported: ACCOUNTS_MAJOR });
    }
    let l: AccountList = serde_json::from_slice(bytes).map_err(|e| ContractError::Malformed(e.to_string()))?;
    if l.accounts.len() > MAX_ACCOUNTS {
        return Err(ContractError::Invalid("accounts"));
    }
    let mut seen = std::collections::HashSet::new();
    for a in &l.accounts {
        a.validate()?;
        if !seen.insert(a.id.as_str()) {
            return Err(ContractError::Invalid("account id"));
        }
    }
    Ok(l)
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
