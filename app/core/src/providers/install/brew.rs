//! The Homebrew plan (ADR-17 rule 2, bhishma condition 5). Homebrew installs its current version and checks its own
//! checksums; it can't pin one. So the plan reads `brew info --json=v2` with the absolute brew, refuses anything that
//! isn't a plain official package, and hashes the version and the cask's sha256 and url (or the bottle's), so a change
//! between plan and run needs a new consent.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::admin::{self, Targets};
use super::env::SetupEnv;
use super::plan::{AdminRow, Pin, Plan, ToolRow};
use super::tools::Tool;
use crate::providers::catalog;
use crate::vajra::bridge::run_process_exact;

const INFO_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Cask,
    Formula,
}

/// What `run` needs beyond the plan: the fully qualified token and the brew to run.
#[derive(Debug, Clone)]
pub struct BrewRun {
    pub token: String,
    pub kind: Kind,
    pub brew: Tool,
}

/// Cask artifacts that only place files: anything else (pkg, installer, script, pre/postflight...) can need sudo or run
/// code, so the cask is "Install it yourself".
const CASK_ARTIFACTS: [&str; 5] = ["app", "binary", "zap", "generate_completions_from_executable", "manpage"];
/// Uninstall directives that run code or touch the system.
const BAD_UNINSTALL: [&str; 4] = ["script", "pkgutil", "launchctl", "delete"];

/// The bottle tag for this Mac: `arm64_sequoia`, `sonoma`... None for a release this build doesn't know.
pub fn platform_tag(arch: &str, macos_major: u32) -> Option<String> {
    let name = match macos_major {
        12 => "monterey",
        13 => "ventura",
        14 => "sonoma",
        15 => "sequoia",
        26 => "tahoe",
        _ => return None,
    };
    Some(if arch == "aarch64" { format!("arm64_{name}") } else { name.to_string() })
}

/// This Mac's bottle tag.
pub fn current_tag() -> Option<String> {
    let mut buf = [0u8; 32];
    let mut len = buf.len();
    // SAFETY: a NUL-terminated name, a buffer and its length.
    let rc = unsafe { libc::sysctlbyname(c"kern.osproductversion".as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) };
    if rc != 0 {
        return None;
    }
    let v = std::str::from_utf8(&buf[..len]).ok()?.trim_end_matches('\0');
    platform_tag(std::env::consts::ARCH, v.split('.').next()?.parse().ok()?)
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn https_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    (!authority.contains('@')).then(|| authority.split(':').next().unwrap_or(authority))
}

/// Check a cask's artifacts. `Ok` when every artifact only places files and every binary is an allowed name.
fn cask_artifacts(entry: &Value, allowed: &[String]) -> Result<(), &'static str> {
    for art in entry.get("artifacts").and_then(Value::as_array).into_iter().flatten() {
        let Some(obj) = art.as_object() else { return Err("cask_runs_installer") };
        for (k, v) in obj {
            match k.as_str() {
                "uninstall" => {
                    for d in v.as_array().into_iter().flatten() {
                        if d.as_object().is_some_and(|o| BAD_UNINSTALL.iter().any(|b| o.contains_key(*b))) {
                            return Err("cask_uninstall");
                        }
                    }
                }
                "uninstall_preflight" | "uninstall_postflight" => return Err("cask_uninstall"),
                "binary" => {
                    let a = v.as_array().ok_or("cask_binary")?;
                    let source = a.first().and_then(Value::as_str).ok_or("cask_binary")?;
                    let target = a.iter().skip(1).find_map(|o| s(o, "target")).unwrap_or_else(|| Path::new(source).file_name().and_then(|n| n.to_str()).unwrap_or(""));
                    if !allowed.iter().any(|b| b == target) {
                        return Err("cask_binary");
                    }
                }
                k if CASK_ARTIFACTS.contains(&k) => {}
                _ => return Err("cask_runs_installer"),
            }
        }
    }
    Ok(())
}

/// Plan the Homebrew install of `m` for harness `id`, whose install may only add `allowed` programs to the PATH.
#[allow(clippy::too_many_arguments)]
pub fn plan(id: &str, allowed: &[String], kind: Kind, m: &catalog::Brew, brew: &Tool, env: &SetupEnv, tag: &str, uid: u32) -> Result<(Plan, BrewRun), &'static str> {
    let tap = m.tap.as_deref().filter(|t| *t == "homebrew/core" || *t == "homebrew/cask").ok_or("brew_tap")?;
    let token = format!("{tap}/{}", m.package);
    let mut args: Vec<std::ffi::OsString> = vec!["info".into(), "--json=v2".into()];
    if kind == Kind::Cask {
        args.push("--cask".into());
    }
    args.push(token.clone().into());
    let out = run_process_exact(&brew.path, &args, Path::new("/"), INFO_TIMEOUT, env.vars()).map_err(|_| "brew_failed")?;
    if out.code != Some(0) || out.truncated {
        return Err("brew_info");
    }
    let info: Value = serde_json::from_str(&out.stdout).map_err(|_| "brew_info")?;
    let list = info.get(if kind == Kind::Cask { "casks" } else { "formulae" }).and_then(Value::as_array).ok_or("brew_info")?;
    let [entry] = list.as_slice() else { return Err("brew_info") };
    if s(entry, "tap") != Some(tap) {
        return Err("brew_tap");
    }
    if s(entry, if kind == Kind::Cask { "full_token" } else { "full_name" }) != Some(m.package.as_str()) {
        return Err("brew_token");
    }
    if [entry.get("deprecated"), entry.get("disabled")].iter().any(|v| v.and_then(|v| v.as_bool()) == Some(true)) {
        return Err("brew_deprecated");
    }
    let (version, sha256, url, deps, store) = match kind {
        Kind::Cask => {
            let sha = s(entry, "sha256").ok_or("cask_sha")?;
            if sha == "no_check" {
                return Err("cask_no_check");
            }
            if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("cask_sha");
            }
            let url = s(entry, "url").ok_or("cask_url_host")?;
            if !https_host(url).is_some_and(|h| m.cask_url_hosts.iter().any(|a| a == h)) {
                return Err("cask_url_host");
            }
            cask_artifacts(entry, allowed)?;
            (s(entry, "version").ok_or("brew_info")?.to_string(), sha.to_string(), url.to_string(), Vec::new(), "Caskroom")
        }
        Kind::Formula => {
            let files = entry.get("bottle").and_then(|b| b.get("stable")).and_then(|b| b.get("files")).and_then(Value::as_object).ok_or("no_bottle")?;
            let file = files.get(tag).or_else(|| files.get("all")).ok_or("no_bottle")?;
            let deps = entry.get("dependencies").and_then(Value::as_array).map(|a| a.iter().filter_map(|d| d.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let version = entry.get("versions").and_then(|v| s(v, "stable")).ok_or("brew_info")?;
            (version.to_string(), s(file, "sha256").ok_or("no_bottle")?.to_string(), s(file, "url").ok_or("no_bottle")?.to_string(), deps, "Cellar")
        }
    };

    let out = run_process_exact(&brew.path, &["--prefix".into()], Path::new("/"), INFO_TIMEOUT, env.vars()).map_err(|_| "brew_failed")?;
    let prefix = PathBuf::from(out.stdout.trim());
    if out.code != Some(0) || !prefix.is_absolute() {
        return Err("brew_prefix");
    }
    let targets = Targets { dirs: vec![prefix.clone(), prefix.join(store), prefix.join("bin")], trees: vec![], files: allowed.iter().map(|b| prefix.join("bin").join(b)).collect() };
    let blocker = admin::check(&targets, uid, 1);

    // Already registered (a Repair): `install` would skip it and replace nothing, so the plan reinstalls. The verb is
    // in the command, so it is in the hash the user consents to.
    let verb = if installed_of(entry, kind).is_some() { "reinstall" } else { "install" };
    let mut command = vec![brew.path.display().to_string(), verb.into()];
    if kind == Kind::Cask {
        command.push("--cask".into());
    }
    command.push(token.clone());
    let tools = vec![ToolRow { role: "brew", path: brew.path.display().to_string(), label: None, owned_by_you: brew.owned_by_you }];
    let method = if kind == Kind::Cask { "brew_cask" } else { "brew_formula" };
    let mut p = Plan::new(id, method, command, tools, tap.into(), version, prefix.display().to_string(), Pin::Brew { token: token.clone(), sha256, url });
    p.needs_admin = blocker.map(|b| AdminRow { path: b.path.display().to_string(), reason: b.reason });
    p.notes.extend(["brew_not_pinned", "brew_env_applies"]);
    if brew.owned_by_you {
        p.notes.push("owned_by_you");
    }
    p.deps = deps;
    Ok((p.rehash(), BrewRun { token, kind, brew: brew.clone() }))
}

/// The version brew reports as installed for `run`'s token, read after the install (bhishma condition 5).
pub fn installed_version(run: &BrewRun, env: &SetupEnv) -> Option<String> {
    let mut args: Vec<std::ffi::OsString> = vec!["info".into(), "--json=v2".into()];
    if run.kind == Kind::Cask {
        args.push("--cask".into());
    }
    args.push(run.token.clone().into());
    let out = run_process_exact(&run.brew.path, &args, Path::new("/"), INFO_TIMEOUT, env.vars()).ok()?;
    let info: Value = serde_json::from_str(&out.stdout).ok().filter(|_| out.code == Some(0) && !out.truncated)?;
    installed_of(info.get(if run.kind == Kind::Cask { "casks" } else { "formulae" })?.get(0)?, run.kind)
}

/// The version a `brew info --json=v2` entry records as installed; None when it is not installed.
fn installed_of(entry: &Value, kind: Kind) -> Option<String> {
    let installed = entry.get("installed")?;
    match kind {
        Kind::Cask => installed.as_str().map(str::to_string),
        Kind::Formula => installed.as_array()?.last()?.get("version")?.as_str().map(str::to_string),
    }
}

#[cfg(test)]
#[path = "brew_tests.rs"]
mod tests;
