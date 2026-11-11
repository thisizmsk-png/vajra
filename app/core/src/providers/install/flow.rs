//! Offer, consent and run, end to end (ADR-17 rules 2-4). `offer` picks the first catalog method this machine can do in
//! one click: only for an entry whose source the owner verified, and only with a tool resolved under the tool roots. The
//! run recomputes the offer and needs a consent token for exactly that plan's hash, one run per harness at a time.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::brew::{self, BrewRun, Kind};
use super::consent::{ConsentError, Consents};
use super::env::{setup_env, SetupEnv};
use super::marks::Marks;
use super::npm::{self, NpmRun};
use super::plan::Plan;
use super::run::{run_brew, run_npm, run_script, Ctx, Report};
use super::script;
use super::tools::{self, Roots, Tools};
use crate::providers::catalog::{self, Harness, Method};
use crate::sessions::Supervisor;

/// The longest an install may run before it is stopped (and checked and rolled back like any other exit).
pub const RUN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);

/// Reads one of the app's own environment variables.
pub type EnvGet = Box<dyn Fn(&str) -> Option<OsString> + Send + Sync>;

/// What this machine offers: resolved tools, the user, the bottle tag and the app's own environment.
pub struct Machine {
    pub tools: Tools,
    pub uid: u32,
    pub tag: Option<String>,
    pub get: EnvGet,
}

impl Machine {
    /// This Mac, for the user whose home is `home`, with the tools found on the login PATH.
    pub fn this(home: &Path) -> Self {
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { libc::geteuid() };
        Machine { tools: tools::resolve(&Roots::of(home), &crate::userpath::current()), uid, tag: brew::current_tag(), get: Box::new(|k| std::env::var_os(k)) }
    }
}

/// What a run needs beyond the plan, kept in the core: the UI only ever names a harness id.
pub enum Pending {
    Npm(NpmRun),
    Brew(BrewRun),
    Script(catalog::VendorScript),
}

pub struct Offer {
    pub plan: Plan,
    pub env: SetupEnv,
    pub pending: Pending,
}

fn dir_of(p: &Path) -> Vec<PathBuf> {
    p.parent().map(|d| vec![d.to_path_buf()]).unwrap_or_default()
}

fn try_one(h: &Harness, m: &Machine, method: &Method) -> Result<Offer, &'static str> {
    let get = |k: &str| (m.get)(k);
    match method {
        Method::Npm(n) => {
            let tool = m.tools.npm.as_ref().ok_or("no_npm")?;
            let env = setup_env(&dir_of(&tool.node.path), get);
            let (plan, run) = npm::plan(&h.id, &h.binary, n, tool, &env, m.uid)?;
            Ok(Offer { plan, env, pending: Pending::Npm(run) })
        }
        Method::BrewCask(b) | Method::BrewFormula(b) => {
            let kind = if matches!(method, Method::BrewCask(_)) { Kind::Cask } else { Kind::Formula };
            let tool = m.tools.brew.as_ref().ok_or("no_brew")?;
            let env = setup_env(&dir_of(&tool.path), get);
            let (plan, run) = brew::plan(&h.id, &h.allowed_binaries, kind, b, tool, &env, m.tag.as_deref().unwrap_or(""), m.uid)?;
            Ok(Offer { plan, env, pending: Pending::Brew(run) })
        }
        Method::VendorScript(s) => {
            let plan = script::plan(&h.id, s)?;
            Ok(Offer { plan, env: setup_env(&[], get), pending: Pending::Script(s.clone()) })
        }
    }
}

/// The one-click offer for `h`, or why it is "Install it yourself" (the first method's reason).
pub fn offer(h: &Harness, m: &Machine) -> Result<Offer, &'static str> {
    if !h.verified() {
        return Err("source_not_verified");
    }
    let mut first = None;
    for method in &h.methods {
        match try_one(h, m, method) {
            Ok(o) => return Ok(o),
            Err(e) => {
                first.get_or_insert(e);
            }
        }
    }
    Err(first.unwrap_or("no_method"))
}

/// Where a run keeps its Broken marks and writes its audit line.
#[derive(Clone, Copy)]
pub struct Paths<'a> {
    pub data: &'a Path,
    pub audit: &'a Path,
}

fn curl_get(env: &SetupEnv) -> impl Fn(&str, &Path) -> Result<(), &'static str> + '_ {
    move |url, dest| {
        let argv = script::curl_argv(url, dest);
        let cwd = dest.parent().unwrap_or(Path::new("/"));
        match crate::vajra::bridge::run_process_exact(Path::new(script::CURL), &argv, cwd, FETCH_TIMEOUT, env.vars()) {
            Ok(o) if o.code == Some(0) && !o.timed_out => Ok(()),
            _ => Err("fetch"),
        }
    }
}

/// Install `h` with the consent `token`: recompute the offer, spend the token on its hash, run.
pub fn run(h: &Harness, m: &Machine, consents: &Consents, token: &str, sup: &Supervisor, paths: Paths, current: &dyn Fn() -> Option<(PathBuf, Option<String>)>) -> Result<Report, &'static str> {
    let _running = consents.begin(&h.id).map_err(ConsentError::code)?;
    let o = offer(h, m)?;
    // ADR-17 rule 5, in the core and not only in the sheet: a plan that needs admin rights is never spawned.
    if o.plan.needs_admin.is_some() {
        return Err("needs_admin");
    }
    consents.take(token, &h.id, &o.plan.hash, Instant::now()).map_err(ConsentError::code)?;
    let marks = Marks::at(paths.data);
    let ctx = Ctx { sup, env: &o.env, marks: &marks, audit: paths.audit, timeout: RUN_TIMEOUT, current };
    Ok(match o.pending {
        Pending::Npm(r) => run_npm(&ctx, &o.plan, r),
        Pending::Brew(r) => run_brew(&ctx, &o.plan, &r),
        Pending::Script(s) => run_script(&ctx, &o.plan, &s, &curl_get(&o.env)),
    })
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
