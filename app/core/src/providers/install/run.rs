//! The one-click run (ADR-17 rules 2-4, bhishma conditions 1, 2, 8 and 9), after consent. The verified tarball is
//! written fresh (O_EXCL|O_NOFOLLOW, 0600) into a new private folder just before the spawn, the install runs on its
//! own `setup-<id>` stream with exactly the Setup env, and its result is checked on every exit, not only 0:
//!
//! - the installed closure equals the approved one: Installed, and any Broken mark clears;
//! - a fresh install that isn't: uninstalled with the same absolute npm plus `--ignore-scripts`, and `npm ls` must show
//!   it gone, else Broken;
//! - an upgrade that changed the install but isn't the approved closure: Broken (there is no pinned old version to go
//!   back to); one that changed nothing: Failed.
//!
//! Each run appends one audit line through `append_line` (id, method, version, exit, result): never a token, a path or
//! the environment.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use super::brew::{installed_version, BrewRun};
use super::closure;
use super::env::SetupEnv;
use super::marks::Marks;
use super::npm::{installed_now, npm_out, NpmRun, REGISTRY_FLAG};
use super::plan::{Plan, TARBALL};
use super::script;
use crate::providers::catalog;
use crate::crew::redact::Redactor;
use crate::sessions::{Supervisor, SETUP_STREAM};

const HELPER_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Installed,
    /// Nothing changed; the harness is as it was.
    Failed,
    /// A fresh install that did not check out was taken back out.
    RolledBack,
    /// The install changed and could not be confirmed or undone: Set up shows Broken until a clean install.
    Broken,
}

impl Outcome {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Outcome::Installed => "installed",
            Outcome::Failed => "failed",
            Outcome::RolledBack => "rolled_back",
            Outcome::Broken => "broken",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub outcome: Outcome,
    pub reason: Option<&'static str>,
    /// The install's exit code; None when it timed out or never started.
    pub exit: Option<u32>,
    pub version: Option<String>,
}

impl Report {
    pub(crate) fn new(outcome: Outcome, reason: Option<&'static str>, exit: Option<u32>) -> Self {
        Self { outcome, reason, exit, version: None }
    }
}

pub struct Ctx<'a> {
    pub sup: &'a Supervisor,
    pub env: &'a SetupEnv,
    pub marks: &'a Marks,
    pub audit: &'a Path,
    pub timeout: Duration,
    /// The harness's binary and version as Set up detects them now, for a Broken mark.
    pub current: &'a dyn Fn() -> Option<(PathBuf, Option<String>)>,
}

/// The bus stream of `id`'s install.
pub fn stream(id: &str) -> String {
    format!("{SETUP_STREAM}{id}")
}

/// Write `bytes` as a new file in `dir`: never through a link, never over an existing file.
fn write_fresh(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::OpenOptionsExt;
    let p = dir.join(name);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&p)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(p)
}

/// Spawn on the setup stream and wait. `Ok(None)`: it ran past the timeout and was stopped.
pub(crate) fn spawn_wait(ctx: &Ctx, id: &str, argv: Vec<String>, cwd: &Path) -> Result<Option<u32>, &'static str> {
    let s = stream(id);
    ctx.sup.spawn_setup(&s, argv, cwd, ctx.env, Redactor::default()).map_err(|_| "spawn_failed")?;
    match ctx.sup.wait(&s, ctx.timeout) {
        Ok(Some(e)) => Ok(Some(if e.signal.is_some() && e.code == 0 { 1 } else { e.code })),
        Ok(None) => {
            let _ = ctx.sup.kill(&s);
            let _ = ctx.sup.wait(&s, Duration::from_secs(5));
            Ok(None)
        }
        Err(_) => Err("spawn_failed"),
    }
}

fn audit(ctx: &Ctx, plan: &Plan, r: &Report) {
    let exit = r.exit.map_or_else(|| "none".to_string(), |c| c.to_string());
    let reason = format!("setup id={} method={} version={} exit={exit} result={}", plan.id, plan.method, r.version.as_deref().unwrap_or(&plan.version), r.outcome.word());
    let _ = crate::control::audit::append_line(ctx.audit, "SETUP", &reason, SystemTime::now());
}

fn mark(ctx: &Ctx, id: &str, reason: &'static str) {
    if let Some((path, version)) = (ctx.current)() {
        let _ = ctx.marks.set(id, &path, version.as_deref(), reason);
    }
}

/// Run the npm install of `plan` (as shown and consented to) with what planning kept.
pub fn run_npm(ctx: &Ctx, plan: &Plan, run: NpmRun) -> Report {
    let r = match crate::providers::setup::private_dir("vajra-install") {
        Ok(dir) => {
            let r = npm_in(ctx, &dir, plan, &run);
            let _ = std::fs::remove_dir_all(&dir);
            r
        }
        Err(_) => Report::new(Outcome::Failed, Some("io"), None),
    };
    audit(ctx, plan, &r);
    r
}

fn npm_in(ctx: &Ctx, dir: &Path, plan: &Plan, run: &NpmRun) -> Report {
    let Ok(file) = write_fresh(dir, "pkg.tgz", &run.tgz) else { return Report::new(Outcome::Failed, Some("io"), None) };
    let Some(file_s) = file.to_str() else { return Report::new(Outcome::Failed, Some("io"), None) };
    let argv: Vec<String> = run.npm.argv(&["install", "-g", "--ignore-scripts", REGISTRY_FLAG]).iter().map(|a| a.to_string_lossy().into_owned()).chain([file_s.to_string()]).collect();
    // What runs is what the user saw, with the tarball's path in place of its placeholder.
    let shown: Vec<String> = plan.command.iter().map(|a| if a == TARBALL { file_s.to_string() } else { a.clone() }).collect();
    if argv != shown {
        return Report::new(Outcome::Failed, Some("plan_changed"), None);
    }
    let exit = match spawn_wait(ctx, &plan.id, argv, dir) {
        Ok(e) => e,
        Err(e) => return Report::new(Outcome::Failed, Some(e), None),
    };
    let now = installed_now(&run.npm, &run.package, dir, ctx.env);
    let matches = |c: &closure::Closure| matches!(&now, Ok(Some(n)) if closure::same(n, c));
    if exit == Some(0) && matches(&run.closure) {
        let _ = ctx.marks.clear(&plan.id);
        return Report { version: Some(plan.version.clone()), ..Report::new(Outcome::Installed, None, exit) };
    }
    let why = match exit {
        Some(0) => "closure_mismatch",
        Some(_) => "install_failed",
        None => "install_timeout",
    };
    match &run.snapshot {
        None => {
            let args = ["uninstall", "-g", "--ignore-scripts", REGISTRY_FLAG, &run.package];
            let un = npm_out(&run.npm, &args, dir, ctx.env, HELPER_TIMEOUT);
            if un.is_ok() && matches!(installed_now(&run.npm, &run.package, dir, ctx.env), Ok(None)) {
                return Report::new(Outcome::RolledBack, Some(why), exit);
            }
            mark(ctx, &plan.id, "rollback_failed");
            Report::new(Outcome::Broken, Some("rollback_failed"), exit)
        }
        Some(prev) if matches(prev) => Report::new(Outcome::Failed, Some(why), exit),
        Some(_) => {
            mark(ctx, &plan.id, why);
            Report::new(Outcome::Broken, Some(why), exit)
        }
    }
}

/// Run the Homebrew install of `plan`: exactly the shown command. The audit records the version brew reports
/// installed afterwards, not the one planned (brew installs the current version; it can't be pinned). Brew's record is
/// not enough: Installed (which clears a Broken mark) also needs Set up to find the program and get its version.
pub fn run_brew(ctx: &Ctx, plan: &Plan, run: &BrewRun) -> Report {
    let r = match spawn_wait(ctx, &plan.id, plan.command.clone(), Path::new("/")) {
        Err(e) => Report::new(Outcome::Failed, Some(e), None),
        Ok(exit) => match (exit, installed_version(run, ctx.env)) {
            (Some(0), Some(v)) => match (ctx.current)() {
                Some((_, Some(_))) => {
                    let _ = ctx.marks.clear(&plan.id);
                    Report { version: Some(v), ..Report::new(Outcome::Installed, None, exit) }
                }
                Some((path, None)) => {
                    let _ = ctx.marks.set(&plan.id, &path, None, "not_working");
                    Report { version: Some(v), ..Report::new(Outcome::Broken, Some("not_working"), exit) }
                }
                None => Report { version: Some(v), ..Report::new(Outcome::Failed, Some("not_detected"), exit) },
            },
            (Some(0), None) => Report::new(Outcome::Failed, Some("not_installed"), exit),
            (Some(_), _) => Report::new(Outcome::Failed, Some("install_failed"), exit),
            (None, _) => Report::new(Outcome::Failed, Some("install_timeout"), exit),
        },
    };
    audit(ctx, plan, &r);
    r
}

/// Run a self-verifying vendor script: fetched over https, kept only when its sha256 is the catalog's, run as
/// `/bin/sh <file>`, and removed afterwards.
pub fn run_script(ctx: &Ctx, plan: &Plan, m: &catalog::VendorScript, get: &dyn Fn(&str, &Path) -> Result<(), &'static str>) -> Report {
    let r = match script::fetch(m, get) {
        Err(e) => Report::new(Outcome::Failed, Some(e), None),
        Ok(file) => {
            let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
            let argv: Vec<String> = script::argv(&file).iter().map(|a| a.to_string_lossy().into_owned()).collect();
            let r = match spawn_wait(ctx, &plan.id, argv, &dir) {
                Err(e) => Report::new(Outcome::Failed, Some(e), None),
                Ok(Some(0)) => Report::new(Outcome::Installed, None, Some(0)),
                Ok(Some(c)) => Report::new(Outcome::Failed, Some("install_failed"), Some(c)),
                Ok(None) => Report::new(Outcome::Failed, Some("install_timeout"), None),
            };
            let _ = std::fs::remove_dir_all(&dir);
            r
        }
    };
    audit(ctx, plan, &r);
    r
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
