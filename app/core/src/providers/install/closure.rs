//! The npm closure rule (ADR-17 rule 2): one-click is offered only when every package that will be installed is pinned,
//! by the tarball's own hash (bundled) or by a complete shrinkwrap. The same closure is then compared with the
//! installed tree (`npm ls -g <pkg> --all --json`) after the install.

use std::collections::BTreeSet;

use serde_json::Value;

use super::tar::Pack;

/// Every (name, version) the install may put on disk.
pub type Closure = BTreeSet<(String, String)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `package.json` is not the catalog's package and version (or does not parse).
    ManifestMismatch,
    NeedsInstallScripts,
    /// A dependency, peer or optional dependency that is neither bundled nor in a complete shrinkwrap.
    UnpinnedDependency,
    /// A shrinkwrap with a range, a missing or weak integrity, a non-registry `resolved`, a link, or lockfile v1.
    BadShrinkwrap,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Refusal::ManifestMismatch => "manifest_mismatch",
            Refusal::NeedsInstallScripts => "needs_install_scripts",
            Refusal::UnpinnedDependency => "unpinned_dependency",
            Refusal::BadShrinkwrap => "bad_shrinkwrap",
        }
    }
}

const REGISTRY: &str = "https://registry.npmjs.org/";

/// `1.2.3`, `1.2.3-beta.1` or `1.2.3+build`: an exact version, never a range.
fn exact(v: &str) -> bool {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && v.bytes().all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
}

fn name_version(v: &Value) -> Option<(String, String)> {
    Some((v.get("name")?.as_str()?.to_string(), v.get("version")?.as_str()?.to_string()))
}

/// The package name a shrinkwrap `packages` key installs (`node_modules/a/node_modules/@s/b` is `@s/b`).
fn key_name(key: &str) -> Option<&str> {
    key.rsplit_once("node_modules/").map(|(_, n)| n).filter(|n| !n.is_empty())
}

/// Check the verified tarball of `pkg@version`. `Ok` is the approved closure.
pub fn check(p: &Pack, pkg: &str, version: &str) -> Result<Closure, Refusal> {
    let m: Value = serde_json::from_slice(&p.manifest).map_err(|_| Refusal::ManifestMismatch)?;
    if name_version(&m) != Some((pkg.to_string(), version.to_string())) {
        return Err(Refusal::ManifestMismatch);
    }
    if let Some(s) = m.get("scripts").and_then(Value::as_object) {
        if ["preinstall", "install", "postinstall"].iter().any(|k| s.contains_key(*k)) {
            return Err(Refusal::NeedsInstallScripts);
        }
    }
    let mut closure: Closure = [(pkg.to_string(), version.to_string())].into();

    // Bundled packages are covered by the tarball's sha512: every nested manifest joins the closure.
    for (_, man) in &p.nested {
        let v: Value = serde_json::from_slice(man).map_err(|_| Refusal::UnpinnedDependency)?;
        closure.insert(name_version(&v).ok_or(Refusal::UnpinnedDependency)?);
    }
    let top_bundled: BTreeSet<String> = p.nested.iter().filter_map(|(dir, _)| dir.strip_prefix("node_modules/").filter(|n| !n.contains("/node_modules/")).map(str::to_string)).collect();
    let bundle_all = m.get("bundleDependencies").or_else(|| m.get("bundledDependencies")).and_then(Value::as_bool) == Some(true);
    let bundle_named: BTreeSet<String> = m
        .get("bundleDependencies")
        .or_else(|| m.get("bundledDependencies"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    // The shrinkwrap, when there is one, must be complete before anything in it counts.
    // Every location the shrinkwrap pins (`node_modules/a`, `node_modules/a/node_modules/b`), and its entry.
    let mut wrapped = BTreeSet::new();
    let mut locked: Vec<(String, Value)> = Vec::new();
    if let Some(sw) = &p.shrinkwrap {
        let v: Value = serde_json::from_slice(sw).map_err(|_| Refusal::BadShrinkwrap)?;
        if v.get("lockfileVersion").and_then(Value::as_u64).unwrap_or(0) < 2 {
            return Err(Refusal::BadShrinkwrap);
        }
        let pkgs = v.get("packages").and_then(Value::as_object).ok_or(Refusal::BadShrinkwrap)?;
        for (key, e) in pkgs {
            if key.is_empty() {
                continue;
            }
            let name = key_name(key).ok_or(Refusal::BadShrinkwrap)?;
            if e.get("link").and_then(Value::as_bool) == Some(true) {
                return Err(Refusal::BadShrinkwrap);
            }
            let ver = e.get("version").and_then(Value::as_str).filter(|v| exact(v)).ok_or(Refusal::BadShrinkwrap)?;
            if e.get("inBundle").and_then(Value::as_bool) == Some(true) {
                // Inside the tarball already, so covered by its hash; it must really be there.
                if !p.nested.iter().any(|(d, _)| d == key) {
                    return Err(Refusal::BadShrinkwrap);
                }
            } else {
                let resolved = e.get("resolved").and_then(Value::as_str).ok_or(Refusal::BadShrinkwrap)?;
                let integrity = e.get("integrity").and_then(Value::as_str).ok_or(Refusal::BadShrinkwrap)?;
                if !resolved.starts_with(REGISTRY) || !integrity.starts_with("sha512-") {
                    return Err(Refusal::BadShrinkwrap);
                }
            }
            wrapped.insert(key.clone());
            locked.push((key.clone(), e.clone()));
            closure.insert((name.to_string(), ver.to_string()));
        }
    }

    // Where a package can be found: a shrinkwrap entry (checked above), or a bundled folder under a top-level package
    // the root bundles (only those are covered by the tarball's hash as the root's own choice).
    let top_ok = |name: &str| (bundle_all || bundle_named.contains(name)) && top_bundled.contains(name);
    let available = |loc: &str| {
        wrapped.contains(loc) || (p.nested.iter().any(|(d, _)| d == loc) && top_name(loc).is_some_and(top_ok))
    };

    // Every dependency edge of every package that will be installed, of all three kinds, must resolve the way Node
    // resolves it (the nearest node_modules, then each parent's) to a pinned or bundled package. Otherwise npm would
    // fetch whatever satisfies the range and the closure check would only catch it after it was on disk.
    let mut owners: Vec<(String, Value)> = vec![(String::new(), m.clone())];
    for (dir, man) in &p.nested {
        owners.push((dir.clone(), serde_json::from_slice(man).map_err(|_| Refusal::UnpinnedDependency)?));
    }
    owners.extend(locked);
    for (loc, man) in &owners {
        for kind in ["dependencies", "peerDependencies", "optionalDependencies"] {
            let Some(deps) = man.get(kind) else { continue };
            let deps = deps.as_object().ok_or(Refusal::UnpinnedDependency)?;
            for name in deps.keys() {
                if !resolves(loc, name, &available) {
                    return Err(Refusal::UnpinnedDependency);
                }
            }
        }
    }
    Ok(closure)
}

/// The top-level package a location sits under (`node_modules/@s/a/node_modules/b` is `@s/a`).
fn top_name(loc: &str) -> Option<&str> {
    let rest = loc.strip_prefix("node_modules/")?;
    let end = if rest.starts_with('@') { rest.match_indices('/').nth(1).map(|(i, _)| i) } else { rest.find('/') };
    Some(&rest[..end.unwrap_or(rest.len())])
}

/// Node's lookup of `name` required from the package at `from` ("" is the root): `from/node_modules/name`, then the
/// same from each enclosing package, up to the root's `node_modules/name`.
fn resolves(from: &str, name: &str, available: &dyn Fn(&str) -> bool) -> bool {
    let mut base = from;
    loop {
        let at = if base.is_empty() { format!("node_modules/{name}") } else { format!("{base}/node_modules/{name}") };
        if available(&at) {
            return true;
        }
        if base.is_empty() {
            return false;
        }
        base = base.rsplit_once("/node_modules/").map_or("", |(parent, _)| parent);
    }
}

/// The two closures hold exactly the same packages and versions.
pub fn same(a: &Closure, b: &Closure) -> bool {
    a == b
}

/// The installed closure of `pkg` from `npm ls -g <pkg> --all --json`; `Ok(None)` only when npm answered and `pkg` is
/// not there. Output that isn't JSON, isn't an object, or carries an `error` without the package is `npm_ls_failed`:
/// a failed read must never pass for "not installed" (that would uninstall an existing install, or call a rollback done).
pub fn installed(ls_json: &str, pkg: &str) -> Result<Option<Closure>, &'static str> {
    fn walk(name: &str, node: &Value, out: &mut Closure) {
        // A missing or invalid entry has no version: it can never match an approved closure.
        let ver = node.get("version").and_then(Value::as_str).unwrap_or("missing");
        out.insert((name.to_string(), ver.to_string()));
        if let Some(deps) = node.get("dependencies").and_then(Value::as_object) {
            for (n, d) in deps {
                walk(n, d, out);
            }
        }
    }
    const FAILED: &str = "npm_ls_failed";
    let v: Value = serde_json::from_str(ls_json).map_err(|_| FAILED)?;
    let top = v.as_object().ok_or(FAILED)?;
    let root = match top.get("dependencies") {
        None => None,
        Some(Value::Object(deps)) => deps.get(pkg),
        Some(_) => return Err(FAILED),
    };
    match root {
        Some(node @ Value::Object(_)) => {
            let mut out = Closure::new();
            walk(pkg, node, &mut out);
            Ok(Some(out))
        }
        Some(_) => Err(FAILED),
        None if top.contains_key("error") => Err(FAILED),
        None => Ok(None),
    }
}

#[cfg(test)]
#[path = "closure_tests.rs"]
mod tests;
