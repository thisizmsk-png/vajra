//! A run lead's life (ADR-16 16.5 "Lifetime", N9; 6d2c): the files under `<data>/runs/<run>/lead/`, the spawn through
//! the session supervisor, the read-back (16.4 rule 4), and the watch that turns the process's exit into
//! `RunProtocol::lead_exited`. Every restart runs in this order: the exit is seen, the old lead is cleared from the
//! registry, the token is rotated, the new lead is spawned, and only then is it set as the run's lead, so no call
//! from the old tree passes once its successor is set.
//!
//! What the lead runs as comes from its profile (`adapter::profiles`); what it may do comes from the run protocol.
//! The core never starts a worker under a lead's tree: workers go through the worker runtime, which the app owns.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::adapter::profiles::{self, ClaudeLead, CodexRoute, LeadLaunch, RunServer};
use crate::adapter::Engine;
use crate::control::runreg::{self, LeadProc, RunRegistry};
use crate::control::{fsx, sha256};
use crate::crew::redact::Redactor;
use crate::desktop::charter::{self, Brief};
use crate::desktop::runproto::{LeadAction, LeadExit, RunProtocol, RunStatus};
use crate::sessions::{SpawnSpec, Supervisor};

/// How long the read-back may wait for the harness's opening (its rollout or its init event).
pub const READ_BACK_TIMEOUT: Duration = Duration::from_secs(60);
/// How often the watch looks at the lead and ticks the run.
const POLL: Duration = Duration::from_millis(250);
/// The longest a shutdown waits for the lead's process group to go.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

/// The run-lead seat, as `seats.json` recorded it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeadSeat {
    pub account: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// What the app gives a lead launch, resolved once at Start.
#[derive(Clone, Debug)]
pub struct LeadEnv {
    pub data: PathBuf,
    pub home: PathBuf,
    /// The app's own binary (the `--mcp-run` server).
    pub exe: PathBuf,
    pub socket: PathBuf,
    pub codex: PathBuf,
    pub claude: PathBuf,
    /// The user's own Codex home: a login-route lead's rollout is there.
    pub user_codex_home: PathBuf,
    /// The project's folders the Claude lead's deny rules cover, the main checkout first.
    pub repos: Vec<PathBuf>,
    pub others: Vec<PathBuf>,
    pub read_back_timeout: Duration,
}

/// One run's lead. Cheap to clone; the watch thread holds one.
#[derive(Clone)]
pub struct RunLead(Arc<Inner>);

struct Inner {
    run: String,
    project_root: String,
    ask: String,
    max_rounds: u32,
    roles: Vec<String>,
    seat: LeadSeat,
    engine: Engine,
    env: LeadEnv,
    proto: RunProtocol,
    reg: Arc<RunRegistry>,
    sup: Arc<Supervisor>,
    bus: Arc<crate::bus::Bus>,
    /// 6k: the Vajra dirs a Codex lead's profile denies, worked out once, so every launch and every read-back of this
    /// run use the same list (bhishma L3 on #174).
    deny: Vec<PathBuf>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The supervisor session and process of the lead now running.
    current: Option<(String, LeadProc)>,
    launches: u32,
    /// Set by `shutdown`: the watch ends and no restart happens.
    stopping: bool,
    /// Why the run ended on the lead's side, when it did.
    ended: Option<String>,
}

/// Why a lead launch failed before or as it started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchFailed(pub String);

fn lead_dir(data: &Path, run: &str) -> PathBuf {
    data.join("runs").join(run).join("lead")
}

impl RunLead {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run: &str, project_root: &str, ask: &str, max_rounds: u32, roles: Vec<String>, seat: LeadSeat, env: LeadEnv, proto: RunProtocol,
        reg: Arc<RunRegistry>, sup: Arc<Supervisor>, bus: Arc<crate::bus::Bus>,
    ) -> Result<Self, LaunchFailed> {
        let engine = crate::providers::launch::harness_of(&seat.account).ok_or_else(|| LaunchFailed("the lead's account has no harness".into()))?;
        let deny = profiles::vajra_deny_dirs(&env.home, &env.data);
        Ok(RunLead(Arc::new(Inner {
            run: run.into(), project_root: project_root.into(), ask: ask.into(), max_rounds, roles, seat, engine, env, proto, reg, sup, bus, deny,
            state: Mutex::new(State::default()),
        })))
    }

    pub fn proto(&self) -> &RunProtocol {
        &self.0.proto
    }

    /// The bounded state a restarted lead reads: tasks without their briefs, ticket and round statuses.
    fn resume_state(&self) -> Value {
        let plan = self.0.proto.plan_record();
        let tasks: Vec<Value> = plan["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| json!({"id": t["task"]["id"], "title": t["task"]["title"], "owner_role": t["task"]["owner_role"], "files": t["task"]["files"], "deps": t["task"]["deps"]}))
            .collect();
        json!({"round": plan["round"], "criteria": plan["criteria"], "tasks": tasks, "tickets": self.0.proto.workers(None), "rounds": self.0.proto.rounds()})
    }

    fn prompt(&self, resume: bool) -> String {
        let b = Brief { ask: &self.0.ask, max_rounds: self.0.max_rounds, project_root: &self.0.project_root, roles: &self.0.roles };
        if resume { charter::resume_charter(&b, self.resume_state()) } else { charter::charter(&b) }
    }

    /// Write the token and the lead's files, build the argv, and spawn. Returns the supervisor session and process.
    /// The caller sets the lead in the registry (`start`, `restart`).
    fn spawn(&self, resume: bool) -> Result<(String, LeadProc), LaunchFailed> {
        let i = &self.0;
        let f = |e: &dyn std::fmt::Display| LaunchFailed(e.to_string());
        let run_dir = i.env.data.join("runs").join(&i.run);
        let dir = lead_dir(&i.env.data, &i.run);
        fsx::private_dir(&i.env.data.join("runs")).map_err(|e| f(&e))?;
        fsx::private_dir(&run_dir).map_err(|e| f(&e))?;
        fsx::private_dir(&dir).map_err(|e| f(&e))?;
        // bhishma L2 on #174: the lead dir is read back in under the denied data dir, so its real path must stay inside
        // that dir; a link anywhere on the way out would open a dir the profile meant to keep shut.
        match (dir.canonicalize(), i.env.data.canonicalize()) {
            (Ok(real), Ok(data)) if real.starts_with(&data) && real != data => {}
            _ => return Err(LaunchFailed("the lead dir is not inside the app data dir".into())),
        }
        // A fresh token for every launch: a restart's old tree, if any of it lingers, holds a dead one.
        let (token_file, hash) = runreg::write_token(&run_dir).map_err(|e| f(&e))?;
        if !i.reg.rotate_token(&i.run, hash) {
            return Err(LaunchFailed("the run is no longer registered".into()));
        }
        let prompt = self.prompt(resume);
        let server = RunServer { exe: &i.env.exe, run: &i.run, token_file: &token_file, socket: &i.env.socket };
        let project = PathBuf::from(&i.project_root);
        // A fresh profile name for every launch (6k); the Claude lead has its own deny rules in its flag file.
        let profile = profiles::codex_profile_name().map_err(|e| f(&e))?;
        let launch = LeadLaunch {
            prompt: &prompt, model: i.seat.model.as_deref(), effort: i.seat.effort.as_deref(), lead_dir: &dir, project: &project, server, deny: &i.deny,
            profile: &profile,
        };
        let (mut argv, extra_env) = match i.engine {
            Engine::Codex => {
                let gateway_home;
                let route = if i.seat.account == "codex" {
                    CodexRoute::Login
                } else {
                    // Rewritten just before the spawn, as for a worker: N3 on the gateway route rests on this file.
                    gateway_home = crate::providers::codex_home::write(&i.env.data, &i.seat.account, crate::providers::codex_home::DEFAULT_BASE_URL)
                        .map_err(|_| LaunchFailed("the gateway account's Codex home could not be written".into()))?;
                    CodexRoute::Gateway { home: &gateway_home }
                };
                let c = profiles::codex_lead_launch(&launch, route).map_err(|e| LaunchFailed(format!("{}: {}", e.field, e.reason)))?;
                (c.argv, c.env)
            }
            Engine::Claude => {
                let flag = profiles::claude_lead_flag(&i.env.home, &i.env.repos, &i.env.data, &i.env.others).map_err(|e| f(&e))?;
                let flag_path = fsx::write_atomic(&dir, "claude-lead.json", flag.as_bytes()).map_err(|e| f(&e))?;
                let mcp = profiles::claude_lead_mcp_config(&server).map_err(|e| LaunchFailed(format!("{}: {}", e.field, e.reason)))?;
                let mcp_path = fsx::write_atomic(&dir, "vajra-run.json", mcp.as_bytes()).map_err(|e| f(&e))?;
                // A fresh session id for every launch: Claude Code refuses a reused one.
                let sid = crate::ids::uuid_v4().map_err(|e| f(&e))?;
                let c = ClaudeLead { claude: &i.env.claude, session_id: &sid, settings: &flag_path, mcp_config: &mcp_path };
                let argv = profiles::claude_lead_argv(&launch, &c).map_err(|e| LaunchFailed(format!("{}: {}", e.field, e.reason)))?;
                // The flag file is exactly what the core wrote, read back just before the spawn.
                let on_disk = fsx::read_nofollow(&flag_path).map_err(|e| f(&e))?;
                if sha256::hex_digest(&on_disk) != sha256::hex_digest(flag.as_bytes()) {
                    return Err(LaunchFailed(profiles::PROFILE_NOT_APPLIED.into()));
                }
                (argv, profiles::claude_lead_env())
            }
        };
        // `sh -c 'exec "$@"' sh codex ...`: the harness word is the absolute binary the app resolved.
        if i.engine == Engine::Codex && argv.get(4).map(String::as_str) == Some("codex") {
            argv[4] = i.env.codex.to_str().ok_or_else(|| LaunchFailed("the codex path is not UTF-8".into()))?.to_string();
        }
        let n = {
            let mut s = crate::lock(&i.state);
            s.launches += 1;
            s.launches
        };
        let sid = format!("lead-{}-{n}", i.run);
        let spec = SpawnSpec { argv, cwd: dir, rows: 40, cols: 160, redact: Redactor::default(), extra_env };
        i.sup.spawn(&sid, spec).map_err(|e| LaunchFailed(format!("spawn_failed: {e}")))?;
        let pid = i.sup.pid(&sid).map_err(|e| f(&e))?;
        let Some(stamp) = crate::procid::start_stamp(pid) else {
            let _ = i.sup.kill(&sid);
            return Err(LaunchFailed("the lead exited as it started".into()));
        };
        Ok((sid, LeadProc { pid, stamp }))
    }

    /// Record the new lead as the current one and as starting in the registry (its calls pass once its read-back
    /// does). Under the state lock, so a shutdown either finds it to kill or this finds the shutdown: never a lead
    /// started after the quit step reported every lead gone, nor one spawned into a run that already ended.
    fn set_current(&self, sid: String, lead: LeadProc) -> Result<(), LaunchFailed> {
        let mut s = crate::lock(&self.0.state);
        let refused = if s.stopping {
            Some("the lead is shutting down".to_string())
        } else if self.0.proto.card().status.is_some() {
            Some("the run has ended".to_string())
        } else {
            self.0.reg.start_lead(&self.0.run, lead).err().map(|e| format!("{e:?}"))
        };
        if let Some(why) = refused {
            drop(s);
            let _ = self.0.sup.kill(&sid);
            let _ = self.0.sup.wait(&sid, SHUTDOWN_WAIT);
            kill_group(lead);
            return Err(LaunchFailed(why));
        }
        s.current = Some((sid, lead));
        Ok(())
    }

    /// The first launch. On a failure the run ends `lead_lost` with the reason and nothing is left running.
    pub fn start(&self) -> Result<(), LaunchFailed> {
        let r = self.spawn(false).and_then(|(sid, lead)| self.set_current(sid, lead));
        if let Err(e) = &r {
            self.end(&format!("the lead could not start: {}", e.0));
        }
        r
    }

    fn end(&self, why: &str) {
        self.0.proto.lead_refused(why);
        let mut s = crate::lock(&self.0.state);
        s.ended.get_or_insert_with(|| why.to_string());
    }

    /// The harness's events so far (the rollout pointer or the init event).
    fn events(&self, sid: &str) -> String {
        self.0.bus.recent(sid, 1 << 20).iter().map(|l| l.text.as_ref()).collect::<Vec<_>>().join("\n")
    }

    /// The read-back for the current lead: `None` while there is nothing to read yet.
    fn read_back(&self, sid: &str) -> Option<Result<(), String>> {
        let events = self.events(sid);
        match self.0.engine {
            Engine::Codex => {
                let home = if self.0.seat.account == "codex" {
                    self.0.env.user_codex_home.clone()
                } else {
                    crate::providers::codex_home::home_for(&self.0.env.data, &self.0.seat.account).ok()?
                };
                let rollout = crate::providers::effective::codex_rollout(&events, &home)?;
                if !rollout.lines().any(|l| l.contains("\"turn_context\"")) {
                    return None;
                }
                // The same deny list the launch used (bhishma L3 on #174).
                Some(profiles::check_codex_lead_applied(&rollout, &lead_dir(&self.0.env.data, &self.0.run), &self.0.deny).map_err(|e| e.0))
            }
            Engine::Claude => {
                if !events.contains("\"init\"") {
                    return None;
                }
                Some(profiles::check_claude_lead_applied(&events).map_err(|e| e.0))
            }
        }
    }

    /// Watch the lead until the run ends: the read-back, the run's tick, and its exit with N9's restart. Blocks; the
    /// app runs it on its own thread.
    pub fn watch(&self) {
        loop {
            let Some((sid, lead)) = crate::lock(&self.0.state).current.clone() else { return };
            match self.watch_one(&sid, lead) {
                Watched::Stopped => {
                    self.forget(lead);
                    return;
                }
                Watched::NotApplied(why) => {
                    self.forget(lead);
                    self.end(&format!("{}: {why}", profiles::PROFILE_NOT_APPLIED));
                    return;
                }
                Watched::Exited(exit) => {
                    // Exit seen; clear first, so no call from the old tree passes while the run decides.
                    self.forget(lead);
                    match self.0.proto.lead_exited(exit) {
                        LeadAction::Ended(_) => return,
                        LeadAction::Restart { backoff } => {
                            if !self.pause(backoff) {
                                return;
                            }
                            // Rotate (in `spawn`), spawn, then set.
                            if let Err(e) = self.spawn(true).and_then(|(sid, lead)| self.set_current(sid, lead)) {
                                self.end(&format!("the lead could not restart: {}", e.0));
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    /// The lead `lead` is gone: out of the registry, and no longer the current one (unless a successor already is).
    fn forget(&self, lead: LeadProc) {
        self.0.reg.clear_lead(&self.0.run, lead);
        kill_group(lead);
        let mut s = crate::lock(&self.0.state);
        if s.current.as_ref().is_some_and(|(_, l)| *l == lead) {
            s.current = None;
        }
    }

    /// Wait out a backoff; false when a shutdown came first.
    fn pause(&self, d: Duration) -> bool {
        let until = Instant::now() + d;
        while Instant::now() < until {
            if crate::lock(&self.0.state).stopping {
                return false;
            }
            std::thread::sleep(POLL.min(until.saturating_duration_since(Instant::now())));
        }
        !crate::lock(&self.0.state).stopping && self.0.proto.card().status.is_none()
    }

    fn watch_one(&self, sid: &str, lead: LeadProc) -> Watched {
        let start = Instant::now();
        let mut applied = false;
        loop {
            if let Ok(Some(x)) = self.0.sup.wait(sid, POLL) {
                // A lead the shutdown killed is stopped, not an exit the run decides on.
                if crate::lock(&self.0.state).stopping {
                    return Watched::Stopped;
                }
                let tail = self.0.sup.last_lines(sid).unwrap_or_default();
                // A lead that exits before its read-back passed ends as not applied, never restarted: it was never
                // confirmed confined.
                if !applied {
                    match self.read_back(sid) {
                        Some(Ok(())) => {}
                        Some(Err(why)) => return Watched::NotApplied(why),
                        None => return Watched::NotApplied("it exited before its read-back".into()),
                    }
                }
                let (c, _) = crate::providers::errors::classify_exit(Some(x.code), x.signal.as_deref(), &tail);
                let retry = matches!(c.kind, crate::providers::errors::Kind::RateLimit | crate::providers::errors::Kind::Upstream | crate::providers::errors::Kind::Timeout);
                return Watched::Exited(if retry { LeadExit::Retryable } else { LeadExit::Exited });
            }
            if crate::lock(&self.0.state).stopping {
                let _ = self.0.sup.kill(sid);
                return Watched::Stopped;
            }
            if self.0.proto.card().status.is_some() {
                // The run ended (finish_run, a stop, a timeout): the lead has nothing left to do.
                let _ = self.0.sup.kill(sid);
                let _ = self.0.sup.wait(sid, SHUTDOWN_WAIT);
                return Watched::Stopped;
            }
            if !applied {
                match self.read_back(sid) {
                    Some(Ok(())) => {
                        applied = true;
                        // Confirmed confined: from now on its calls pass.
                        self.0.reg.confirm_lead(&self.0.run, lead);
                    }
                    Some(Err(why)) => {
                        let _ = self.0.sup.kill(sid);
                        let _ = self.0.sup.wait(sid, SHUTDOWN_WAIT);
                        return Watched::NotApplied(why);
                    }
                    None if start.elapsed() >= self.0.env.read_back_timeout => {
                        let _ = self.0.sup.kill(sid);
                        let _ = self.0.sup.wait(sid, SHUTDOWN_WAIT);
                        return Watched::NotApplied("no read-back within the time allowed".into());
                    }
                    None => {}
                }
            }
            self.0.proto.tick();
        }
    }

    /// Stop the lead now (app quit, a human Stop). Kills its process group, waits at most `wait`, and clears it from
    /// the registry. Never restarts it. Returns whether the lead was gone within the wait.
    pub fn shutdown(&self, wait: Duration) -> Shutdown {
        let cur = {
            let mut s = crate::lock(&self.0.state);
            s.stopping = true;
            s.current.clone()
        };
        let Some((sid, lead)) = cur else { return Shutdown::NotRunning };
        let _ = self.0.sup.kill(&sid);
        let gone = matches!(self.0.sup.wait(&sid, wait), Ok(Some(_)));
        self.forget(lead);
        if gone { Shutdown::Stopped } else { Shutdown::StillRunning }
    }

    /// The run's status once it ended, and why on the lead's side.
    pub fn ended(&self) -> (Option<RunStatus>, Option<String>) {
        (self.0.proto.card().status, crate::lock(&self.0.state).ended.clone())
    }

    pub fn launches(&self) -> u32 {
        crate::lock(&self.0.state).launches
    }
}

/// Kill whatever is left of a gone lead's process group (a child that outlived it). The group id is the leader's pid
/// (it is a session leader); a pid is not reused while its group has members, and a live process with that pid but
/// another start stamp is someone else, so it is left alone.
fn kill_group(lead: LeadProc) {
    let Ok(pgid) = i32::try_from(lead.pid) else { return };
    if pgid <= 1 || crate::procid::start_stamp(lead.pid).is_some_and(|s| s != lead.stamp) {
        return;
    }
    // SAFETY: kill has no memory preconditions; a negative pid names the process group, and ESRCH is fine.
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
}

enum Watched {
    Exited(LeadExit),
    NotApplied(String),
    Stopped,
}

/// What `shutdown` found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shutdown {
    /// No lead was running.
    NotRunning,
    /// The lead's process exited within the wait.
    Stopped,
    /// It was killed but had not exited when the wait ended.
    StillRunning,
}

#[cfg(test)]
#[path = "run_lead_tests.rs"]
mod tests;
