//! The harness catalog (ADR-17 rule 1): which agent CLIs Settings > Providers > Set up knows, and each vendor's own
//! documented install methods. It is compiled into the signed app (`include_str!`) and never fetched at run time, so it
//! changes only with an app release.
//!
//! An entry whose source the security review has not verified (`source_verified_by: null`) is always "Install it
//! yourself": the vendor's link and command, and no button, whatever its methods say. Every entry ships unverified; a
//! verification is a reviewed catalog edit. The `command` texts are shown to the user to run themselves and are never
//! executed by Vajra; only `sign_in.argv` runs, as argv in a user terminal, never through a shell.

use std::sync::OnceLock;

use serde::Deserialize;

pub const CATALOG_SCHEMA: &str = "vajra.harness_catalog/1";

const RAW: &str = include_str!("harness_catalog.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema: String,
    pub harnesses: Vec<Harness>,
    pub engine_scripts: EngineScripts,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Harness {
    pub id: String,
    pub name: String,
    /// The program name looked up on the user's PATH.
    pub binary: String,
    pub detect_args: Vec<String>,
    /// The Vajra adapter that runs it as an agent; `None` is "Installed. Vajra can't run it as an agent yet" (ADR-16 N5).
    pub adapter: Option<String>,
    pub sign_in: SignIn,
    /// The vendor's own install-docs page, kept for review.
    pub docs_url: String,
    pub source_verified_by: Option<String>,
    pub source_verified_on: Option<String>,
    /// The review that verified the source (a PR link).
    pub verification_ref: Option<String>,
    /// The only program names this entry's install may put on the PATH.
    pub allowed_binaries: Vec<String>,
    pub methods: Vec<Method>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignIn {
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineScripts {
    pub manual_command: String,
    /// The same three verified fields as a harness (D7, bhishma C7), all or none. Until the security review signs off
    /// the engine install, they are null and the engine row stays "Install it yourself".
    #[serde(default)]
    pub source_verified_by: Option<String>,
    #[serde(default)]
    pub source_verified_on: Option<String>,
    #[serde(default)]
    pub verification_ref: Option<String>,
}

impl EngineScripts {
    /// The security review verified the bundled engine install, so a one-click install may be offered.
    pub fn verified(&self) -> bool {
        self.source_verified_by.is_some() && self.source_verified_on.is_some() && self.verification_ref.is_some()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Method {
    Npm(Npm),
    BrewCask(Brew),
    BrewFormula(Brew),
    VendorScript(VendorScript),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Npm {
    pub package: String,
    pub version: String,
    /// `sha512-<base64>` of the published tarball.
    pub integrity: String,
    pub command: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Brew {
    pub package: String,
    pub tap: Option<String>,
    /// The vendor's download hosts a cask's url must be on.
    #[serde(default)]
    pub cask_url_hosts: Vec<String>,
    pub command: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VendorScript {
    pub url: String,
    pub sha256: String,
    /// The vendor documents that the script checks its own download against a published checksum.
    pub self_verifying: bool,
    pub command: String,
}

impl Method {
    /// Whether the method names exact bytes (Homebrew cannot pin a version, N12).
    pub fn is_pinned(&self) -> bool {
        matches!(self, Method::Npm(_) | Method::VendorScript(_))
    }

    /// The vendor's command, for the user to run themselves.
    pub fn command(&self) -> &str {
        match self {
            Method::Npm(m) => &m.command,
            Method::BrewCask(m) | Method::BrewFormula(m) => &m.command,
            Method::VendorScript(m) => &m.command,
        }
    }

    /// The tool the method needs: `npm`, `brew` or none.
    pub fn tool(&self) -> Option<&'static str> {
        match self {
            Method::Npm(_) => Some("npm"),
            Method::BrewCask(_) | Method::BrewFormula(_) => Some("brew"),
            Method::VendorScript(_) => None,
        }
    }
}

impl Harness {
    /// The security review verified this entry's source, so a one-click install may be offered.
    pub fn verified(&self) -> bool {
        self.source_verified_by.is_some() && self.source_verified_on.is_some() && self.verification_ref.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError(pub String);

/// The compiled-in catalog, parsed and checked once. A bad catalog is a build-time test failure; at run time it reads as
/// an error (Settings shows no Set up rows), never a panic.
pub fn catalog() -> Result<&'static Catalog, CatalogError> {
    static PARSED: OnceLock<Result<Catalog, CatalogError>> = OnceLock::new();
    PARSED
        .get_or_init(|| {
            let c: Catalog = serde_json::from_str(RAW).map_err(|e| CatalogError(e.to_string()))?;
            validate(&c).map(|_| c)
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub fn find(id: &str) -> Option<&'static Harness> {
    catalog().ok()?.harnesses.iter().find(|h| h.id == id)
}

fn bad<T>(what: impl Into<String>) -> Result<T, CatalogError> {
    Err(CatalogError(what.into()))
}

/// A plain program or id name: never a path, never an option.
fn plain(s: &str) -> bool {
    s.len() <= 64 && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// One argv element with no shell meaning, so it reads the same however it is shown.
fn argv_word(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'=' | b'/' | b':' | b'@'))
}

/// `1.2.3` or `1.2.3-pre.1`: an exact version, never a range or a tag.
fn exact_version(s: &str) -> bool {
    let (core, pre) = s.split_once('-').unwrap_or((s, ""));
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
        && (s.find('-').is_none() || (!pre.is_empty() && pre.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')))
}

fn https(s: &str) -> bool {
    s.strip_prefix("https://").is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/') && s.bytes().all(|b| b.is_ascii_graphic()))
}

/// A one-line printable ASCII text for the sheet.
fn shown(s: &str) -> bool {
    !s.is_empty() && s.len() <= 300 && s.bytes().all(|b| (b' '..=b'~').contains(&b))
}

fn npm_package(s: &str) -> bool {
    let name = match s.strip_prefix('@') {
        Some(scoped) => match scoped.split_once('/') {
            Some((scope, n)) if plain(scope) => n,
            _ => return false,
        },
        None => s,
    };
    plain(name) && name.bytes().all(|b| !b.is_ascii_uppercase())
}

/// A `YYYY-MM-DD` date.
fn ymd(d: &str) -> bool {
    d.len() == 10 && d.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() })
}

pub fn validate(c: &Catalog) -> Result<(), CatalogError> {
    if c.schema != CATALOG_SCHEMA {
        return bad(format!("schema {:?}", c.schema));
    }
    let es = &c.engine_scripts;
    if !shown(&es.manual_command) {
        return bad("engine_scripts.manual_command");
    }
    // bhishma C7: the engine's verified fields are all set or all null.
    let set = [es.source_verified_by.is_some(), es.source_verified_on.is_some(), es.verification_ref.is_some()].iter().filter(|b| **b).count();
    if set != 0 && set != 3 {
        return bad("engine_scripts: the verified fields are all set or all null");
    }
    if es.verification_ref.as_deref().is_some_and(|r| !https(r)) || es.source_verified_on.as_deref().is_some_and(|d| !ymd(d)) || es.source_verified_by.as_deref().is_some_and(|b| !shown(b)) {
        return bad("engine_scripts: verified fields");
    }
    for (i, h) in c.harnesses.iter().enumerate() {
        let e = |what: &str| bad(format!("{}: {what}", h.id));
        if !plain(&h.id) || c.harnesses[..i].iter().any(|o| o.id == h.id) {
            return e("id is not a unique plain name");
        }
        // The engine's own id: `setup_*` treat it as the engine, so no harness may take it (bhishma C7).
        if h.id == crate::providers::install::engine::flow::ID {
            return e("the id engine is reserved for the engine scripts");
        }
        if !shown(&h.name) {
            return e("name");
        }
        if !plain(&h.binary) || !h.allowed_binaries.contains(&h.binary) || !h.allowed_binaries.iter().all(|b| plain(b)) {
            return e("binary must be a plain name listed in allowed_binaries");
        }
        if h.detect_args.is_empty() || !h.detect_args.iter().all(|a| argv_word(a)) {
            return e("detect_args");
        }
        if h.sign_in.argv.first() != Some(&h.binary) || !h.sign_in.argv.iter().all(|a| argv_word(a)) {
            return e("sign_in.argv must start with the binary and hold plain words only");
        }
        if !https(&h.docs_url) {
            return e("docs_url must be https");
        }
        if h.verification_ref.as_deref().is_some_and(|r| !https(r)) {
            return e("verification_ref must be https");
        }
        if h.source_verified_by.is_some() && !h.verified() {
            return e("a verified entry needs source_verified_on and verification_ref");
        }
        if h.source_verified_on.as_deref().is_some_and(|d| !ymd(d)) {
            return e("source_verified_on must be YYYY-MM-DD");
        }
        if h.methods.is_empty() {
            return e("no methods");
        }
        for m in &h.methods {
            if !shown(m.command()) {
                return e("method command");
            }
            match m {
                Method::Npm(n) => {
                    if !npm_package(&n.package) || !exact_version(&n.version) {
                        return e("npm needs a package and an exact version");
                    }
                    let b64 = n.integrity.strip_prefix("sha512-").unwrap_or("");
                    if b64.is_empty() || !b64.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')) {
                        return e("npm integrity must be sha512");
                    }
                }
                Method::BrewCask(b) | Method::BrewFormula(b) => {
                    let want = if matches!(m, Method::BrewCask(_)) { "homebrew/cask" } else { "homebrew/core" };
                    if !plain(&b.package) || b.tap.as_deref() != Some(want) || !b.cask_url_hosts.iter().all(|h| plain(h)) {
                        return e("brew needs a plain name on its official tap");
                    }
                }
                Method::VendorScript(v) => {
                    if !https(&v.url) || v.sha256.len() != 64 || !v.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return e("vendor_script needs an https url and a sha256");
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
