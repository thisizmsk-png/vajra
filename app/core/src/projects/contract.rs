//! The versioned contract for project and session records (`vajra.project/1`, `vajra.session/1`; ADR-15 section 1).
//!
//! Types and rules only: no file is read or written here. The store (PR 1b), the receipts and the UI all build on
//! these. The rules every reader follows:
//! - Every record carries `schema: "vajra.<thing>/<major>"`. A reader accepts its own major and refuses another one
//!   with `UnsupportedVersion`, so a UI built for another version says "update needed" instead of misreading data.
//! - Adding a field is not a version bump: unknown fields are ignored. An unknown enum value is read as the
//!   catch-all (`Other` / `Unknown`) rather than failing the whole record.
//! - Ids are `<prefix>_<ULID>`: sortable, never reused, and safe in a file name and an argv (no `-`, `/` or `.`).
//! - A record read from disk is validated here, field by field, before anything uses it.

use serde::{Deserialize, Serialize};

use crate::control::time;
use crate::ids;

pub const PROJECT_SCHEMA: &str = "vajra.project/1";
pub const SESSION_SCHEMA: &str = "vajra.session/1";
pub const APP_INFO_SCHEMA: &str = "vajra.app_info/1";

/// The major versions this build reads and writes.
pub const PROJECT_MAJOR: u32 = 1;
pub const SESSION_MAJOR: u32 = 1;

/// A project or session record larger than this is refused unread: both are small by construction.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;
/// A project name longer than this (in characters) is refused.
pub const MAX_NAME_CHARS: usize = 200;
/// The largest `rev` a record may carry: 2^53 - 1, the most a JSON reader in JavaScript holds exactly. It is part of the
/// contract, so the core refuses what the window could not read. (`dev` and `ino` are strings on the wire instead.)
pub const MAX_JSON_INT: u64 = 9_007_199_254_740_991;
/// The longest path accepted in a record.
pub const MAX_PATH_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    /// Not JSON, not an object, or a required field is missing or has the wrong type.
    Malformed(String),
    /// The record's major version is not one this build reads. `found` is the schema string as written.
    UnsupportedVersion { found: String, supported: u32 },
    /// The record parsed, but a field breaks a rule (a bad id, a relative root, a long name).
    Invalid(&'static str),
    TooLarge,
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(m) => write!(f, "malformed record: {m}"),
            Self::UnsupportedVersion { found, supported } => write!(f, "record version {found} is not supported (this build reads major {supported})"),
            Self::Invalid(why) => write!(f, "invalid record: {why}"),
            Self::TooLarge => write!(f, "record too large"),
        }
    }
}

impl std::error::Error for ContractError {}

// ----------------------------------------------------------------------------------------------------------- ids

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

pub const PROJECT_PREFIX: &str = "p";
pub const SESSION_PREFIX: &str = "s";
pub const ACTION_PREFIX: &str = "a";

/// `<prefix>_<26-char ULID>` from a millisecond clock and 10 random bytes. Pure, so tests pin it.
pub fn make_id(prefix: &str, now_ms: u64, rand: [u8; 10]) -> String {
    // 48 bits of milliseconds above 80 random bits: 128 bits, written as 26 base32 characters.
    let low = rand.iter().fold(0u128, |acc, b| (acc << 8) | u128::from(*b));
    let v: u128 = (u128::from(now_ms & 0xFFFF_FFFF_FFFF) << 80) | low;
    let mut out = String::with_capacity(prefix.len() + 27);
    out.push_str(prefix);
    out.push('_');
    for i in (0..26).rev() {
        out.push(CROCKFORD[((v >> (5 * i)) & 31) as usize] as char);
    }
    out
}

/// A fresh id from the system clock and the one randomness source.
pub fn fresh_id(prefix: &str) -> std::io::Result<String> {
    let hex = crate::control::nonce::random_hex(10)?;
    let mut rand = [0u8; 10];
    for (i, b) in rand.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0);
    }
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    Ok(make_id(prefix, ms, rand))
}

/// `<prefix>_` followed by exactly 26 Crockford base32 characters whose first is at most `7` (a ULID fits 128 bits).
pub fn valid_id(prefix: &str, id: &str) -> bool {
    let Some(rest) = id.strip_prefix(prefix).and_then(|r| r.strip_prefix('_')) else { return false };
    rest.len() == 26 && rest.as_bytes()[0] <= b'7' && rest.bytes().all(|c| CROCKFORD.contains(&c))
}

pub fn valid_project_id(id: &str) -> bool {
    valid_id(PROJECT_PREFIX, id)
}

pub fn valid_session_id(id: &str) -> bool {
    valid_id(SESSION_PREFIX, id)
}

pub fn valid_action_id(id: &str) -> bool {
    valid_id(ACTION_PREFIX, id)
}

// ------------------------------------------------------------------------------------------------- the schema tag

/// The major version of a `vajra.<thing>/<major>` tag whose `<thing>` is `thing`, or `None` for any other shape.
pub fn schema_major(tag: &str, thing: &str) -> Option<u32> {
    let rest = tag.strip_prefix("vajra.")?.strip_prefix(thing)?.strip_prefix('/')?;
    if rest.is_empty() || rest.len() > 4 || !rest.bytes().all(|b| b.is_ascii_digit()) || (rest.len() > 1 && rest.starts_with('0')) {
        return None;
    }
    rest.parse().ok()
}

fn check_schema(tag: &str, thing: &str, supported: u32) -> Result<(), ContractError> {
    match schema_major(tag, thing) {
        Some(m) if m == supported => Ok(()),
        _ => Err(ContractError::UnsupportedVersion { found: tag.chars().take(64).collect(), supported }),
    }
}

// ---------------------------------------------------------------------------------------------------- project

/// What identifies a folder across renames, aliases and case: its device and inode, plus the git common dir (which a
/// worktree shares with its main repository).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootKey {
    /// Device and inode are u64 here and DECIMAL STRINGS on the wire (`"18446744073709551615"`: plain digits, no sign, no
    /// leading zero), because a JSON number loses precision above 2^53 in the window. Anything else reads as malformed.
    #[serde(with = "decimal_u64")]
    pub dev: u64,
    #[serde(with = "decimal_u64")]
    pub ino: u64,
    #[serde(default)]
    pub git_common_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub schema: String,
    pub id: String,
    /// Display only; the owner can rename it.
    pub name: String,
    /// The canonical realpath at add time.
    pub root: String,
    pub root_key: RootKey,
    pub created: String,
    /// When archived (hidden, nothing deleted); `None` while active.
    #[serde(default)]
    pub archived: Option<String>,
    /// Bumped on every change.
    pub rev: u64,
}

impl Project {
    /// A new record: schema set, rev 1, not archived. Callers still run `validate`.
    pub fn new(id: String, name: String, root: String, root_key: RootKey, created: String) -> Self {
        Self { schema: PROJECT_SCHEMA.into(), id, name, root, root_key, created, archived: None, rev: 1 }
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        check_schema(&self.schema, "project", PROJECT_MAJOR)?;
        if !valid_project_id(&self.id) {
            return Err(ContractError::Invalid("project id"));
        }
        valid_name(&self.name)?;
        valid_abs_path(&self.root)?;
        if let Some(g) = &self.root_key.git_common_dir {
            valid_abs_path(g)?;
        }
        valid_time(&self.created)?;
        if let Some(a) = &self.archived {
            valid_time(a)?;
        }
        if self.rev == 0 || self.rev > MAX_JSON_INT {
            return Err(ContractError::Invalid("rev"));
        }
        Ok(())
    }
}

/// Parse and validate one `project.json`.
pub fn parse_project(bytes: &[u8]) -> Result<Project, ContractError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(ContractError::TooLarge);
    }
    check_tag(bytes, "project", PROJECT_MAJOR)?;
    let p: Project = serde_json::from_slice(bytes).map_err(|e| ContractError::Malformed(e.to_string()))?;
    p.validate()?;
    Ok(p)
}

mod decimal_u64 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(v)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        let t = String::deserialize(d)?;
        let plain = !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) && (t == "0" || !t.starts_with('0'));
        match t.parse::<u64>() {
            Ok(v) if plain => Ok(v),
            _ => Err(serde::de::Error::custom("expected a decimal u64 string without sign or leading zeros")),
        }
    }
}

// ---------------------------------------------------------------------------------------------------- session

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Orchestrator,
    Run,
    Worker,
    TerminalNote,
    /// A kind written by a newer minor revision: kept, shown plainly, never acted on.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    Stopped,
    Done,
    Failed,
    /// Also what an unrecognised status reads as: "may or may not have finished", never "done".
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub schema: String,
    pub id: String,
    pub project_id: String,
    pub kind: SessionKind,
    /// A run's parent is the orchestrator; a worker's parent is its run.
    #[serde(default)]
    pub parent: Option<String>,
    /// The adapter id (`claude`, `codex`, ...).
    pub provider: String,
    /// What `--resume` needs; `None` for workers.
    #[serde(default)]
    pub native_session_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub campaign: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub attempt: Option<u32>,
    /// Bumped on every resume or restart of this session.
    pub generation: u32,
    pub status: SessionStatus,
    pub started: String,
    #[serde(default)]
    pub ended: Option<String>,
}

impl Session {
    pub fn validate(&self) -> Result<(), ContractError> {
        check_schema(&self.schema, "session", SESSION_MAJOR)?;
        if !valid_session_id(&self.id) {
            return Err(ContractError::Invalid("session id"));
        }
        if !valid_project_id(&self.project_id) {
            return Err(ContractError::Invalid("project_id"));
        }
        if self.parent.as_deref().is_some_and(|p| !valid_session_id(p) || p == self.id) {
            return Err(ContractError::Invalid("parent"));
        }
        // The orchestrator is the top of a project's tree: a run's parent is the orchestrator, a worker's is its run.
        if self.kind == SessionKind::Orchestrator && self.parent.is_some() {
            return Err(ContractError::Invalid("parent"));
        }
        if !valid_provider(&self.provider) {
            return Err(ContractError::Invalid("provider"));
        }
        if self.native_session_id.as_deref().is_some_and(|n| !ids::valid_session_id(n)) {
            return Err(ContractError::Invalid("native_session_id"));
        }
        if self.run_id.as_deref().is_some_and(|r| !ids::valid_run_id(r)) {
            return Err(ContractError::Invalid("run_id"));
        }
        for (v, what) in [(&self.campaign, "campaign"), (&self.task_id, "task_id")] {
            if v.as_deref().is_some_and(|s| !valid_label(s)) {
                return Err(ContractError::Invalid(what));
            }
        }
        valid_time(&self.started)?;
        if let Some(e) = &self.ended {
            valid_time(e)?;
        }
        Ok(())
    }
}

/// Parse and validate one `sessions.jsonl` line.
pub fn parse_session(line: &[u8]) -> Result<Session, ContractError> {
    if line.len() > MAX_RECORD_BYTES {
        return Err(ContractError::TooLarge);
    }
    check_tag(line, "session", SESSION_MAJOR)?;
    let s: Session = serde_json::from_slice(line).map_err(|e| ContractError::Malformed(e.to_string()))?;
    s.validate()?;
    Ok(s)
}

/// What `latest_sessions` read: the sessions, and what it could not use. A caller that shows sessions must also show that
/// something was skipped, and `unsupported` means a newer build wrote records this one cannot read ("update needed"):
/// the session list may then be out of date, for example a session shown as running that a newer build marked done.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionLog {
    pub sessions: Vec<Session>,
    /// Lines that did not parse or validate (unsupported-major lines included).
    pub skipped: usize,
    /// At least one skipped line was another major version of the session record.
    pub unsupported: bool,
}

/// The newest line per session id wins (`sessions.jsonl` is append-only). A line that fails to parse is skipped, not
/// fatal: one bad line must not hide a project's history. But it is counted, and a line of another major is flagged,
/// never silently dropped. Order of first appearance is kept.
pub fn latest_sessions(lines: &[&[u8]]) -> SessionLog {
    latest_sessions_of(None, lines)
}

/// Like `latest_sessions`, but for one project's log: a line whose `project_id` is another project's is skipped and counted
/// BEFORE the newest-per-id choice, so a line copied from project B can neither appear in A's list nor replace A's
/// session that shares its id.
pub fn latest_sessions_of(project_id: Option<&str>, lines: &[&[u8]]) -> SessionLog {
    let mut order: Vec<String> = Vec::new();
    let mut by_id: std::collections::HashMap<String, Session> = std::collections::HashMap::new();
    let mut log = SessionLog::default();
    for l in lines {
        match parse_session(l) {
            Ok(s) if project_id.is_some_and(|p| s.project_id != p) => log.skipped += 1,
            Ok(s) => {
                if !by_id.contains_key(&s.id) {
                    order.push(s.id.clone());
                }
                by_id.insert(s.id.clone(), s);
            }
            Err(e) => {
                log.skipped += 1;
                log.unsupported |= matches!(e, ContractError::UnsupportedVersion { .. });
            }
        }
    }
    log.sessions = order.into_iter().filter_map(|id| by_id.remove(&id)).collect();
    log
}

// ----------------------------------------------------------------------------------------------------- app_info

/// The answer to the read-only `app_info` command: which contract majors this build supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub schema: String,
    pub contracts: Contracts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contracts {
    pub project: u32,
    pub session: u32,
}

pub fn app_info() -> AppInfo {
    AppInfo { schema: APP_INFO_SCHEMA.into(), contracts: Contracts { project: PROJECT_MAJOR, session: SESSION_MAJOR } }
}

// ------------------------------------------------------------------------------------------------------ helpers

/// Read the `schema` tag alone first, so a record from another major version is reported as such even if its other
/// fields no longer fit this build's types.
fn check_tag(bytes: &[u8], thing: &str, supported: u32) -> Result<(), ContractError> {
    #[derive(Deserialize)]
    struct Tag {
        schema: String,
    }
    let t: Tag = serde_json::from_slice(bytes).map_err(|e| ContractError::Malformed(e.to_string()))?;
    check_schema(&t.schema, thing, supported)
}

fn valid_name(n: &str) -> Result<(), ContractError> {
    let t = n.trim();
    if t.is_empty() || t.chars().count() > MAX_NAME_CHARS || n.chars().any(|c| c.is_control() || crate::control::lead::invisible(c)) {
        return Err(ContractError::Invalid("name"));
    }
    Ok(())
}

fn valid_abs_path(p: &str) -> Result<(), ContractError> {
    // Absolute and already in canonical form: no `..`, `.` or empty component (`//`, a trailing `/`) anywhere.
    let canonical = p == "/" || p.strip_prefix('/').is_some_and(|r| r.split('/').all(|c| !c.is_empty() && c != "." && c != ".."));
    if !canonical || p.len() > MAX_PATH_BYTES || p.contains('\0') {
        return Err(ContractError::Invalid("path"));
    }
    Ok(())
}

fn valid_time(t: &str) -> Result<(), ContractError> {
    time::parse_iso(t).map(|_| ()).ok_or(ContractError::Invalid("timestamp"))
}

fn valid_provider(p: &str) -> bool {
    !p.is_empty() && p.len() <= 32 && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// A campaign slug or task id: short, printable, no separators.
fn valid_label(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.') && !s.starts_with('.')
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
