//! The npm plan (ADR-17 rule 2): `npm pack` the pinned version into a fresh private folder, check the bytes against the
//! catalog's sha512, read the tarball in memory, apply the closure rule, find the prefix and whether it needs admin,
//! and record what is installed now. Read-only apart from the pack folder, which is removed before this returns.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::admin::{self, Targets};
use super::closure::{self, Closure};
use super::env::SetupEnv;
use super::plan::{integrity, AdminRow, Pin, Plan, ToolRow, TARBALL};
use super::tar;
use super::tools;
use crate::providers::catalog;
use crate::vajra::bridge::{self, run_process_exact};

pub const REGISTRY_FLAG: &str = "--registry=https://registry.npmjs.org/";
const PACK_TIMEOUT: Duration = Duration::from_secs(120);
const HELPER_TIMEOUT: Duration = Duration::from_secs(20);
/// The largest tarball read into memory.
const MAX_TGZ: u64 = 512 << 20;
/// Entries the needs_admin walk checks before it gives up (and asks for the manual command).
const ADMIN_WALK: usize = 100_000;

/// What `run` needs beyond the plan the user saw. The tarball stays in memory; it is written fresh just before spawn.
#[derive(Debug, Clone)]
pub struct NpmRun {
    pub tgz: Vec<u8>,
    pub closure: Closure,
    /// What `npm ls -g` showed before the install; None when it was not installed.
    pub snapshot: Option<Closure>,
    pub npm: tools::Npm,
    pub package: String,
    pub prefix: PathBuf,
}

pub(crate) fn npm_out(npm: &tools::Npm, args: &[&str], cwd: &Path, env: &SetupEnv, timeout: Duration) -> Result<bridge::Output, &'static str> {
    let argv = npm.argv(args);
    run_process_exact(Path::new(&argv[0]), &argv[1..], cwd, timeout, env.vars()).map_err(|_| "npm_failed")
}

/// The installed closure of `package` now (`npm ls -g` exits non-zero when it is missing, so only its JSON counts).
pub(crate) fn installed_now(npm: &tools::Npm, package: &str, cwd: &Path, env: &SetupEnv) -> Result<Option<Closure>, &'static str> {
    let out = npm_out(npm, &["ls", "-g", package, "--all", "--json"], cwd, env, HELPER_TIMEOUT)?;
    if out.timed_out || out.truncated {
        return Err("npm_ls_failed");
    }
    closure::installed(&out.stdout, package)
}

/// Plan the npm install of `m` for harness `id` (whose program is `binary`), as user `uid`.
pub fn plan(id: &str, binary: &str, m: &catalog::Npm, npm: &tools::Npm, env: &SetupEnv, uid: u32) -> Result<(Plan, NpmRun), &'static str> {
    let dir = crate::providers::setup::private_dir("vajra-pack").map_err(|_| "io")?;
    let r = plan_in(&dir, id, binary, m, npm, env, uid);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

#[allow(clippy::too_many_arguments)]
fn plan_in(dir: &Path, id: &str, binary: &str, m: &catalog::Npm, npm: &tools::Npm, env: &SetupEnv, uid: u32) -> Result<(Plan, NpmRun), &'static str> {
    let spec = format!("{}@{}", m.package, m.version);
    let dest = dir.to_str().ok_or("io")?;
    let out = npm_out(npm, &["pack", &spec, "--ignore-scripts", REGISTRY_FLAG, "--pack-destination", dest, "--json"], dir, env, PACK_TIMEOUT)?;
    if out.code != Some(0) {
        return Err("pack_failed");
    }
    // Pack's JSON names a file; it is not trusted. The folder was empty and private: exactly one regular file is the tarball.
    let entries: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|_| "io")?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    let [file] = entries.as_slice() else { return Err("pack_output") };
    let meta = std::fs::symlink_metadata(file).map_err(|_| "pack_output")?;
    if !meta.is_file() {
        return Err("pack_output");
    }
    if meta.len() > MAX_TGZ {
        return Err("tar_too_big");
    }
    let bytes = std::fs::read(file).map_err(|_| "io")?;
    let sha = integrity(&bytes);
    if sha != m.integrity {
        return Err("integrity_mismatch");
    }
    let pack = tar::read_tgz(&bytes, tar::Limits::default()).map_err(tar::TarError::code)?;
    let closure = closure::check(&pack, &m.package, &m.version).map_err(closure::Refusal::code)?;

    let out = npm_out(npm, &["config", "get", "prefix"], dir, env, HELPER_TIMEOUT)?;
    let prefix = PathBuf::from(out.stdout.trim());
    if out.code != Some(0) || !prefix.is_absolute() {
        return Err("npm_prefix");
    }
    let scope_registry = match m.package.strip_prefix('@').and_then(|s| s.split_once('/')) {
        Some((scope, _)) => {
            let out = npm_out(npm, &["config", "get", &format!("@{scope}:registry")], dir, env, HELPER_TIMEOUT)?;
            Some(out.stdout.trim().to_string()).filter(|s| !s.is_empty() && s != "undefined")
        }
        None => None,
    };
    let snapshot = installed_now(npm, &m.package, dir, env)?;

    let targets = Targets {
        dirs: vec![prefix.clone(), prefix.join("lib/node_modules"), prefix.join("bin")],
        trees: vec![prefix.join("lib/node_modules").join(&m.package)],
        files: vec![prefix.join("bin").join(binary)],
    };
    let blocker = admin::check(&targets, uid, ADMIN_WALK);

    let command = npm.argv(&["install", "-g", "--ignore-scripts", REGISTRY_FLAG, TARBALL]).iter().map(|a| a.to_string_lossy().into_owned()).collect();
    let row = |role, t: &tools::Tool| ToolRow { role, path: t.path.display().to_string(), label: t.label, owned_by_you: t.owned_by_you };
    let tools = vec![row("node", &npm.node), row("npm", &npm.cli)];
    let mut p = Plan::new(id, "npm", command, tools, "registry.npmjs.org".into(), m.version.clone(), prefix.display().to_string(), Pin::Npm { tgz_sha512: sha });
    p.needs_admin = blocker.map(|b| AdminRow { path: b.path.display().to_string(), reason: b.reason });
    p.notes.push("npmrc_applies");
    if npm.node.owned_by_you || npm.cli.owned_by_you {
        p.notes.push("owned_by_you");
    }
    p.scope_registry = scope_registry;
    let p = p.rehash();
    Ok((p, NpmRun { tgz: bytes, closure, snapshot, npm: npm.clone(), package: m.package.clone(), prefix }))
}

#[cfg(test)]
#[path = "npm_tests.rs"]
mod tests;
