//! What the install sheet shows and the user consents to (ADR-17 rule 4): the exact command, where it comes from, the
//! version, the absolute tool paths and the folder it installs into. The hash binds a consent token to this plan.

use serde::Serialize;
use sha2::{Digest, Sha256, Sha512};

/// Stands in for the verified tarball's path in the shown command: the file is written fresh for each run.
pub const TARBALL: &str = "<verified tarball>";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolRow {
    pub role: &'static str,
    pub path: String,
    pub label: Option<&'static str>,
    pub owned_by_you: bool,
}

/// What the plan pins, beyond the command. Part of the hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pin {
    Npm { tgz_sha512: String },
    /// Homebrew can't pin a version: the hash covers the version, the token and the cask's sha256 and url (or the
    /// bottle's sha256 for a formula), so a change between plan and run needs a new consent.
    Brew { token: String, sha256: String, url: String },
    Script { sha256: String },
    /// The engine bundled with the app (D7): its tgz sha256, whether the build is a release, and every fact about the
    /// target the plan relied on (bhishma C7), so any of them changing between plan and run needs a new consent.
    Engine {
        tgz_sha256: String,
        build: &'static str,
        case: &'static str,
        old_version: Option<String>,
        old_root: Option<(u64, u64)>,
        workflows: Vec<String>,
        home: String,
        /// For an update or Repair of the app's copy: how many of the bundle's files the installed copy lacks or has
        /// different bytes for, read in memory (no script runs at plan time). `None` for a fresh install.
        differs: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdminRow {
    pub path: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub id: String,
    pub method: &'static str,
    pub command: Vec<String>,
    pub tools: Vec<ToolRow>,
    pub source: String,
    pub version: String,
    pub target: String,
    pub pin: Pin,
    /// Set when the install would need administrator rights: the sheet shows the command and why, nothing runs.
    pub needs_admin: Option<AdminRow>,
    /// Plain-word notes for the sheet (`npmrc_applies`, `brew_not_pinned`, `brew_env_applies`, `owned_by_you`).
    pub notes: Vec<&'static str>,
    /// npm: the effective `@scope:registry` for a scoped package (bhishma condition 6).
    pub scope_registry: Option<String>,
    /// Homebrew: the dependencies it will install or upgrade.
    pub deps: Vec<String>,
    pub hash: String,
}

impl Plan {
    #[allow(clippy::too_many_arguments)]
    pub fn new(id: &str, method: &'static str, command: Vec<String>, tools: Vec<ToolRow>, source: String, version: String, target: String, pin: Pin) -> Self {
        Plan { id: id.into(), method, command, tools, source, version, target, pin, needs_admin: None, notes: Vec::new(), scope_registry: None, deps: Vec::new(), hash: String::new() }.rehash()
    }

    /// Recompute the hash after a field changed. Everything the sheet shows that could change what runs is covered.
    pub fn rehash(mut self) -> Self {
        #[derive(Serialize)]
        struct Bound<'a> {
            id: &'a str,
            method: &'a str,
            command: &'a [String],
            tools: Vec<&'a str>,
            source: &'a str,
            version: &'a str,
            target: &'a str,
            pin: &'a Pin,
            scope_registry: &'a Option<String>,
            deps: &'a [String],
            needs_admin: &'a Option<AdminRow>,
            notes: &'a [&'static str],
        }
        let b = Bound {
            id: &self.id,
            method: self.method,
            command: &self.command,
            tools: self.tools.iter().map(|t| t.path.as_str()).collect(),
            source: &self.source,
            version: &self.version,
            target: &self.target,
            pin: &self.pin,
            scope_registry: &self.scope_registry,
            deps: &self.deps,
            needs_admin: &self.needs_admin,
            notes: &self.notes,
        };
        let json = serde_json::to_vec(&b).unwrap_or_default();
        self.hash = hex(&Sha256::digest(&json));
        self
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// npm's `integrity` for these bytes: `sha512-` and the standard base64 of the digest.
pub fn integrity(bytes: &[u8]) -> String {
    format!("sha512-{}", base64(&Sha512::digest(bytes)))
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
