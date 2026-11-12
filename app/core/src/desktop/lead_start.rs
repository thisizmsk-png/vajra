//! Start for a proposal with seats (ADR-16 16.5, 6d2c): the core run. The seats come from the guarded `seats.json`
//! `prepare_seats` just wrote; the run gets its own `RunProtocol` over the worker runtime (with the 6d1 stub gate, so
//! it ends at best `not_verified` until 6g), a registry entry, and a `RunLead` whose watch runs on its own thread. A
//! proposal without seats never comes here: it starts exactly as before.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::run_lead::{LeadEnv, LeadSeat, RunLead, Shutdown, READ_BACK_TIMEOUT, SHUTDOWN_WAIT};
use super::runproto::{CoreWorkers, Limits, RunConfig, RunProtocol, StubGate, SystemClock};
use super::{fail, lock, workers, App};
use crate::adapter::profiles::{self, RoleKind};
use crate::control::service::WorkerSeat;
use crate::crew::redact::Redactor;
use crate::sessions::argv::RunInput;

/// The live run leads, by run id. `closing` is set, under the map's lock, when the app starts to quit: from then on
/// no lead is added.
#[derive(Default)]
pub struct LeadRuns(std::sync::Mutex<Leads>);

#[derive(Default)]
pub struct Leads {
    runs: HashMap<String, RunLead>,
    closing: bool,
}

/// What `shutdown_leads` did: `ended` leads are gone (stopped within the wait, or already exited); `left` holds the
/// run ids whose lead was killed but still live at the deadline.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LeadShutdown {
    pub ended: usize,
    pub left: Vec<String>,
}

fn quitting() -> Value {
    fail("quitting", "the app is quitting; no new run starts")
}

/// The recorded lead seat and worker seats of a run, as `seats.json` holds them.
fn seats_of(doc: &Value) -> Result<(LeadSeat, Vec<WorkerSeat>), Value> {
    let bad = || fail("io", "the run's recorded seats are not readable");
    let text = |v: &Value, k: &str| v[k].as_str().map(String::from);
    let mut lead = None;
    let mut out = Vec::new();
    for s in doc["seats"].as_array().ok_or_else(bad)? {
        let account = text(s, "account").ok_or_else(bad)?;
        if s["seat"] == "lead" {
            lead = Some(LeadSeat { account, model: text(s, "model"), effort: text(s, "effort") });
            continue;
        }
        out.push(WorkerSeat {
            slot: text(s, "slot").ok_or_else(bad)?,
            role: text(s, "role").ok_or_else(bad)?,
            task: text(s, "task").unwrap_or_default(),
            account,
            model: text(s, "model"),
            effort: text(s, "effort"),
            security: s["security"].as_bool().unwrap_or(false),
            network: s["network"].as_bool().unwrap_or(false),
            why: text(s, "why").unwrap_or_default(),
            source: text(s, "source").unwrap_or_default(),
        });
    }
    Ok((lead.ok_or_else(bad)?, out))
}

/// The writer roles that have seats: the only owner roles a plan may name (the charter's roster).
fn writer_roles(seats: &[WorkerSeat]) -> Vec<String> {
    let mut r: Vec<String> = seats.iter().filter(|s| RoleKind::of_role(&s.role) == Some(RoleKind::Worker)).map(|s| s.role.clone()).collect();
    r.sort();
    r.dedup();
    r
}

impl App {
    /// The absolute codex binary on the user's PATH (or the test dirs).
    fn codex_bin(&self) -> PathBuf {
        let dirs = self.provider_dirs.clone().unwrap_or_else(crate::vajra::preflight::path_dirs);
        crate::vajra::preflight::find_on_path("codex", &dirs).unwrap_or_else(|| PathBuf::from("codex"))
    }

    /// The worker runtime for core runs, built once (the first core run's project gives the account caps).
    fn core_runtime(&self, project: &str) -> Result<workers::WorkerRuntime, Value> {
        use crate::control::service::SeatPolicy;
        let (data, home) = (self.data.clone(), self.home.clone());
        let codex = self.codex_bin();
        let claude = self.claude.clone();
        let caps = self.seat_env.agents(project).map(|a| a.account_caps).unwrap_or_default();
        let rt = self.worker_runtime(move || workers::Config {
            tools: profiles::WriterTools::load(&home.join(".claude/skills/vajra/bundled-agents")),
            codex_home: home.join(".codex"),
            home,
            data,
            codex,
            claude,
            account_caps: caps,
            worker_timeout: workers::WORKER_TIMEOUT,
            poll: Duration::from_millis(500),
            clock: None,
            redact: Redactor::default(),
            steer: None,
            #[cfg(test)]
            claude_unverified_ok: false,
        })?;
        Ok(rt.clone())
    }

    /// Every other project's folder, denied wholesale in a Claude lead's flag file.
    fn other_roots(&self, own: &str) -> Vec<PathBuf> {
        self.projects.list(true).map(|l| l.projects.into_iter().filter(|p| p.id != own).map(|p| PathBuf::from(p.root)).collect()).unwrap_or_default()
    }

    /// Start a seated proposal's run: `run` is the id `prepare_seats` took, its seats already recorded.
    pub(super) fn start_lead_run(&self, project: &str, input: RunInput, cwd: &str, run: &str) -> Result<Value, Value> {
        if lock(&self.lead_runs.0).closing {
            return Err(quitting());
        }
        self.projects_usable()?;
        let repo = super::repo_dir(cwd)?;
        let base = self.resolve_base(&repo, &input.base)?;
        let doc = self.read_run_seats(run).ok().flatten().ok_or_else(|| fail("io", "the run's seats were not recorded"))?;
        let (lead_seat, seats) = seats_of(&doc)?;
        let engine = crate::providers::launch::harness_of(&lead_seat.account).ok_or_else(|| fail("harness_unsupported", "the lead's account has no harness"))?;
        if let Err(u) = profiles::profile(engine, RoleKind::Lead).supported {
            return Err(json!({"error": "harness_unsupported", "reason": u.reason, "detail": u.detail}));
        }
        let rt = self.core_runtime(project)?;
        let gw = rt.gitw().clone();
        // Before the lead exists: what it was started on, so a crash reconcile and the resume check can read it back.
        super::recover::record_started(&self.data, run, project, &repo, &lead_seat.account).map_err(|e| fail("io", e))?;
        let roles = writer_roles(&seats);
        let cfg = RunConfig {
            project: project.into(), run: run.into(), repo: repo.clone(), base, ask: input.ask.clone(), seats, max_in_flight: input.max_in_flight,
            limits: Limits { rounds: input.max_rounds, ..Limits::default() }, redact: Redactor::default(),
        };
        let proto = RunProtocol::new(cfg, Arc::new(CoreWorkers { rt, gw }), Arc::new(StubGate), Arc::new(SystemClock::default()));
        // The first launch writes the run's token and rotates this placeholder, which no client holds.
        self.run_leads.register(run, proto.clone(), crate::control::sha256::digest(crate::control::nonce::random_hex(32).map_err(|e| fail("io", e))?.as_bytes()));
        let env = LeadEnv {
            data: self.data.clone(), home: self.home.clone(), exe: std::env::current_exe().map_err(|e| fail("io", e))?,
            socket: crate::control::socket_path(&self.data), codex: self.codex_bin(),
            claude: self.claude.clone(), user_codex_home: self.home.join(".codex"), repos: vec![repo.clone()], others: self.other_roots(project),
            read_back_timeout: READ_BACK_TIMEOUT,
        };
        let root = repo.to_str().ok_or_else(|| fail("not_found", "the project's folder is not a UTF-8 path"))?.to_string();
        let lead = RunLead::new(run, &root, &input.ask, input.max_rounds, roles, lead_seat, env, proto, self.run_leads.clone(), self.sup.clone(), self.bus.clone())
            .map_err(|e| fail("harness_unsupported", e.0))?;
        if let Err(e) = lead.start() {
            self.run_leads.remove(run);
            return Err(fail("lead_failed", e.0));
        }
        {
            let mut l = lock(&self.lead_runs.0);
            if l.closing {
                // The quit began while this lead launched: it never joins the map, so it is stopped here.
                drop(l);
                lead.shutdown(SHUTDOWN_WAIT);
                self.run_leads.remove(run);
                return Err(quitting());
            }
            l.runs.insert(run.to_string(), lead.clone());
        }
        let (reg, id, runs, w, data) = (self.run_leads.clone(), run.to_string(), self.lead_runs_handle(), lead.clone(), self.data.clone());
        let watching = std::thread::Builder::new().name(format!("lead-{run}")).spawn(move || {
            w.watch();
            // How it ended, so the next start does not take it for a run a crash cut short.
            let (status, why) = w.ended();
            let status = status.map(|s| serde_json::to_value(s).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default());
            // The status is the code; the lead's own words stay in the log, never in the record the IPC reads.
            let _ = why;
            let _ = super::recover::record_ended(&data, &id, status.as_deref().unwrap_or("stopped"), "");
            reg.remove(&id);
            if let Some(r) = runs.upgrade() {
                lock(&r.0).runs.remove(&id);
            }
        });
        if let Err(e) = watching {
            // No watch means no read-back and no N9: the lead does not run unwatched.
            lead.shutdown(SHUTDOWN_WAIT);
            lock(&self.lead_runs.0).runs.remove(run);
            self.run_leads.remove(run);
            return Err(fail("io", e));
        }
        Ok(json!({"run_id": run, "core": true}))
    }

    fn lead_runs_handle(&self) -> std::sync::Weak<LeadRuns> {
        Arc::downgrade(&self.lead_runs)
    }

    /// The quit sequence's lead step (agreed with the quit owner): refuse new runs, then kill every run lead's
    /// process group at once and wait at most `wait` in total for them to go. Each is cleared from the registry and
    /// never restarted. Workers are not touched here; the worker runtime's own quit ends them.
    pub fn shutdown_leads(&self, wait: Duration) -> LeadShutdown {
        let leads: Vec<(String, RunLead)> = {
            let mut l = lock(&self.lead_runs.0);
            l.closing = true;
            l.runs.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };
        // Side by side, so the whole step takes at most `wait`, however many leads there are.
        let results: Vec<(String, Shutdown)> = std::thread::scope(|s| {
            let waits: Vec<_> = leads.iter().map(|(run, l)| (run.clone(), s.spawn(move || l.shutdown(wait)))).collect();
            waits.into_iter().map(|(run, h)| (run, h.join().unwrap_or(Shutdown::StillRunning))).collect()
        });
        let mut out = LeadShutdown::default();
        for (run, r) in results {
            match r {
                Shutdown::StillRunning => out.left.push(run),
                Shutdown::Stopped | Shutdown::NotRunning => out.ended += 1,
            }
        }
        out.left.sort();
        out
    }

    /// Every ended core run, newest first, each with whether it can be resumed and, if not, why (6d2d, item e2).
    pub fn core_runs(&self) -> Result<Value, Value> {
        let installed = |account: &str| match crate::providers::launch::harness_of(account) {
            Some(crate::adapter::Engine::Codex) => self.codex_bin().is_absolute(),
            Some(crate::adapter::Engine::Claude) => self.lead.claude_found(),
            None => false,
        };
        let lead_ok = |account: &str| crate::providers::launch::harness_of(account).is_some_and(|e| profiles::profile(e, RoleKind::Lead).supported.is_ok());
        let seats = |run: &str| self.read_run_seats(run);
        let env = super::recover::ResumeEnv { seats: &seats, installed: &installed, lead_ok: &lead_ok };
        Ok(json!({"runs": super::recover::core_runs(&self.data, &env)}))
    }

    /// Test hook: `lead` joins the live run leads as `start_lead_run` adds one, so the quit sequence can be tested
    /// against a rig's lead.
    #[cfg(test)]
    pub(super) fn adopt_lead_run(&self, run: &str, lead: RunLead) {
        lock(&self.lead_runs.0).runs.insert(run.to_string(), lead);
    }

    /// The default wait for `shutdown_leads`.
    pub const LEAD_SHUTDOWN_WAIT: Duration = SHUTDOWN_WAIT;
}
