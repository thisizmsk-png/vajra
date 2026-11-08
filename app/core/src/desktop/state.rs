//! Receipt-wrapped actions and the revisioned `project_state` projection (ADR-15 section 3).
//!
//! `act` is the one door for a UI action that must be safe to repeat: it asks the receipt store whether to run the
//! effect (`receipt::Store::begin`), runs it with no state lock held, then records the outcome. Every state change
//! bumps the project's revision; the UI asks `project_state(project_id, since_rev)` and gets `unchanged` or a fresh,
//! bounded projection. Until T1's project types land, a project is only the `project_id` string the caller passes
//! and the projection shows the one control session; swap the source, not the shape, when projects arrive.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};

use super::{App, Res, fail, lock};
use crate::control::receipt::{Begin, Caller, Receipt, Request, State, Store};
use crate::control::time::unix_ms;
use crate::sessions::argv::RunInput;
use std::sync::Mutex;

/// Receipts shown in one projection.
const RECENT: usize = 20;
/// Error codes that mean the effect was attempted and broke, not that it was refused up front.
const FAILED_CODES: [&str; 5] = ["io_error", "send_failed", "spawn_failed", "launch_failed", "kb_failed"];

pub(super) struct Hub {
    pub(super) store: Store,
    /// Revisions start here for every project. It is the start time in ms, or above every recovered receipt's
    /// revision, so a revision handed out before a restart is never handed out again for different content (a
    /// process bumps far less than once a millisecond).
    base: u64,
    /// project_id -> revision, from `base` when first seen.
    revs: HashMap<String, u64>,
    /// project_id -> runs started through a receipt (the project generation `start_run` is guarded by).
    run_gen: HashMap<String, u64>,
    /// Held from judging a start run to recording its outcome, so two starts are decided one after the other.
    start_gate: std::sync::Arc<Mutex<()>>,
    /// Fingerprint of the worker list `project_state` last showed, to notice a quiet change.
    workers_fp: u64,
}

/// Where `act` gets the target's current generation.
pub(super) enum Gen {
    /// Already read by the caller (a worker's attempt is rechecked under the control lock by the effect itself).
    Given(Option<u64>),
    /// The project's run count, read under the same lock as the receipt decision.
    ProjectRuns,
}

impl From<Option<u64>> for Gen {
    fn from(g: Option<u64>) -> Self {
        Gen::Given(g)
    }
}

impl Hub {
    pub(super) fn open(data: &Path) -> std::io::Result<Self> {
        let now = now_ms();
        let store = Store::open(&data.join("control/ledger.jsonl"), now)?;
        // Recovered state: the highest revision any receipt carries, and each project's run generation (a done
        // start run records its new generation in `current_generation`).
        let mut base = now;
        let mut run_gen: HashMap<String, u64> = store.generations().clone();
        for r in store.all() {
            base = base.max(r.rev.saturating_add(1));
            if r.action == "start_run" && r.state == State::Done {
                let g = run_gen.entry(r.project_id.clone()).or_insert(0);
                *g = (*g).max(r.current_generation.unwrap_or(0));
            }
        }
        Ok(Self { store, base, revs: HashMap::new(), run_gen, start_gate: Default::default(), workers_fp: 0 })
    }

    pub(super) fn rev(&mut self, project: &str) -> u64 {
        *self.revs.entry(project.to_string()).or_insert(self.base)
    }

    fn bump(&mut self, project: &str) -> u64 {
        let base = self.base;
        let r = self.revs.entry(project.to_string()).or_insert(base);
        *r += 1;
        *r
    }
}

pub(super) fn now_ms() -> u64 {
    unix_ms(std::time::SystemTime::now())
}

/// What the UI sends with every guarded action.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct Envelope {
    pub action_id: String,
    pub project_id: String,
    /// The generation of the target when the UI looked (the worker's attempt, the project's run count).
    pub expect_generation: Option<u64>,
}

/// A 64-bit generation for a plan, from the first 16 hex chars of its `plan_sha256` (None when not hex).
fn sha_generation(sha: &str) -> Option<u64> {
    u64::from_str_radix(sha.get(..16)?, 16).ok()
}

impl App {
    /// Run `effect` at most once for `env.action_id`. Always returns the receipt; `result`/`error` carry what the
    /// effect returned when it ran. `current_generation` is the target's generation now.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn act(&self, env: &Envelope, caller: Caller, human_only: bool, action: &str, target: &str, current_generation: impl Into<Gen>, effect: impl FnOnce() -> Res) -> Value {
        let current_generation = current_generation.into();
        let req = Request { action_id: &env.action_id, project_id: &env.project_id, action, target, expect_generation: env.expect_generation, caller, human_only };
        // Start runs are judged and finished one at a time: a second start waits here, then sees the generation the
        // first one left behind.
        let gate = (action == "start_run").then(|| lock(&self.acts).start_gate.clone());
        let _serial = gate.as_ref().map(|m| lock(m));
        let (begun, existed) = {
            let mut h = lock(&self.acts);
            let current_generation = match current_generation {
                Gen::Given(g) => g,
                Gen::ProjectRuns => Some(h.run_gen.get(&env.project_id).copied().unwrap_or(0)),
            };
            let rev = h.rev(&env.project_id);
            let existed = h.store.get(&env.action_id).is_some();
            (h.store.begin(&req, current_generation, rev, now_ms()), existed)
        };
        let (rc, outcome) = match begun {
            // The ledger would not take the receipt: nothing was done, and nothing is claimed.
            Err(e) => return json!({"receipt": null, "error": fail("io_error", e)}),
            Ok(Begin::Done(rc)) => {
                // A refusal stored just now changes the projection; a replay of an old receipt does not.
                if !existed && lock(&self.acts).store.get(&env.action_id).is_some() {
                    self.invalidate(&env.project_id);
                }
                return json!({"receipt": rc, "replayed": true});
            }
            Ok(Begin::Run(_)) => {
                // `accepted` is now in the projection.
                self.invalidate(&env.project_id);
                // A panicking effect may have half run: the receipt says unknown, not accepted for ever.
                let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(effect)).unwrap_or_else(|_| Err(fail("effect_panicked", "the action stopped unexpectedly")));
                let (state, reason) = match &out {
                    Ok(_) => (State::Done, None),
                    Err(v) => {
                        let code = v["error"].as_str().unwrap_or("failed");
                        let state = if code == "effect_panicked" {
                            State::Unknown
                        } else if FAILED_CODES.contains(&code) {
                            State::Failed
                        } else {
                            State::Rejected
                        };
                        (state, Some(code.to_string()))
                    }
                };
                let detail = out.as_ref().err().and_then(|v| v["detail"].as_str().map(str::to_string));
                let mut h = lock(&self.acts);
                // The generation to put in the receipt: a start's new project generation, or the one the effect
                // itself found when it refused as stale.
                let mut new_gen = out.as_ref().err().filter(|_| state == State::Rejected).and_then(|v| v["current_generation"].as_u64());
                let mut recorded = Ok(());
                if state == State::Done && action == "start_run" {
                    let g = h.run_gen.get(&env.project_id).copied().unwrap_or(0) + 1;
                    // Durable first: the high-water mark must not be lost with the receipt that reports it.
                    recorded = h.store.record_generation(&env.project_id, g);
                    h.run_gen.insert(env.project_id.clone(), g);
                    new_gen = Some(g);
                }
                let rev = h.bump(&env.project_id);
                let finished = match recorded {
                    Ok(()) => h.store.finish_with_generation(&env.action_id, state, reason.as_deref(), detail.as_deref(), new_gen, rev, now_ms()),
                    // The generation could not be made durable: do not claim done.
                    Err(_) => h.store.finish_with_generation(&env.action_id, State::Unknown, Some("outcome_not_recorded"), Some("the action may have happened but its outcome could not be saved; check the project before acting again"), None, rev, now_ms()),
                };
                let rc = match finished {
                    Ok(rc) => rc,
                    // The effect ran but its outcome could not be written: say so, never claim done.
                    Err(_) => {
                        drop(h);
                        self.emit_projects(vec![json!({"project_id": env.project_id, "rev": rev})]);
                        return json!({"receipt": null, "error": fail("io_error", "outcome not recorded"), "result": out.ok()});
                    }
                };
                drop(h);
                self.emit_projects(vec![json!({"project_id": env.project_id, "rev": rev})]);
                (rc, out)
            }
        };
        match outcome {
            Ok(v) => json!({"receipt": rc, "result": v}),
            Err(e) => json!({"receipt": rc, "error": e}),
        }
    }

    /// Called after the control service's worker list was replaced. If what `project_state` shows changed (even with
    /// no transcript output on the bus), advance every known project's revision and notify, after the change.
    pub(super) fn workers_published(&self) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for w in lock(&self.control).workers_view() {
            w.to_string().hash(&mut hasher);
        }
        let fp = hasher.finish();
        let projects = {
            let mut h = lock(&self.acts);
            if h.workers_fp == fp {
                return;
            }
            h.workers_fp = fp;
            let ids: Vec<String> = h.revs.keys().cloned().collect();
            ids.into_iter().map(|p| json!({"project_id": p, "rev": h.bump(&p)})).collect::<Vec<_>>()
        };
        if !projects.is_empty() {
            self.emit_projects(projects);
        }
    }

    /// Bump a project's revision and tell subscribers, whether or not anything reached the bus.
    fn invalidate(&self, project_id: &str) {
        let rev = lock(&self.acts).bump(project_id);
        self.emit_projects(vec![json!({"project_id": project_id, "rev": rev})]);
    }

    /// Send a `changed` payload naming the projects whose revision moved. The sink is installed by the shell;
    /// without one (tests, the socket server) nothing is sent.
    fn emit_projects(&self, projects: Vec<Value>) {
        if let Some(sink) = lock(&self.sink).as_ref() {
            sink(json!({"streams": [], "snapshot_needed": false, "projects": projects}));
        }
    }

    /// The shell hands in how to emit the `changed` event (`handle.emit`), so receipts can notify on their own.
    pub fn set_change_sink(&self, sink: super::ChangeSink) {
        *lock(&self.sink) = Some(sink);
    }

    pub fn act_start_run(&self, env: &Envelope, input: RunInput, cwd: &str) -> Value {
        self.act(env, Caller::Ui, false, "start_run", &env.project_id, Gen::ProjectRuns, || self.start_run(input, cwd))
    }

    pub fn act_stop_worker(&self, env: &Envelope, worker: &str, reason: Option<&str>) -> Value {
        let cur = lock(&self.control).worker_generation(worker);
        self.act(env, Caller::Ui, true, "stop_worker", worker, cur, || self.control_stop_expecting(worker, reason, env.expect_generation))
    }

    pub fn act_approve_plan(&self, env: &Envelope, id: &str, plan_sha256: &str) -> Value {
        // The plan's generation is the hash the inbox last showed; the UI's expect is the hash it holds.
        let shown = lock(&self.shown).get(id).and_then(|s| sha_generation(s));
        let mut env = env.clone();
        env.expect_generation = env.expect_generation.or_else(|| sha_generation(plan_sha256));
        self.act(&env, Caller::Ui, true, "approve_plan", id, shown, || self.approve_plan(id, plan_sha256))
    }

    /// Bump every known project's revision (the bus changed: workers, runs or plans may differ). Returns
    /// `[{project_id, rev}]` for the `changed` payload.
    pub fn bump_all_projects(&self) -> Vec<Value> {
        let mut h = lock(&self.acts);
        let ids: Vec<String> = h.revs.keys().cloned().collect();
        ids.into_iter().map(|p| json!({"project_id": p, "rev": h.bump(&p)})).collect()
    }

    /// The revisioned projection the UI subscribes to. `since_rev == rev` answers `unchanged`. Bounded: at most
    /// `RECENT` receipts and the worker list the control service holds.
    pub fn project_state(&self, project_id: &str, since_rev: Option<u64>) -> Res {
        if project_id.is_empty() || project_id.len() > 128 {
            return Err(super::invalid("project_id", "must be 1..=128 chars"));
        }
        let (rev, run_gen, recent) = {
            let mut h = lock(&self.acts);
            let rev = h.rev(project_id);
            let rg = h.run_gen.get(project_id).copied().unwrap_or(0);
            let recent: Vec<Receipt> = h.store.recent(project_id, RECENT).into_iter().cloned().collect();
            (rev, rg, recent)
        };
        if since_rev == Some(rev) {
            return Ok(json!({"schema": "vajra.project_state/1", "project_id": project_id, "rev": rev, "unchanged": true}));
        }
        let workers = lock(&self.control).workers_view();
        Ok(json!({
            "schema": "vajra.project_state/1",
            "project_id": project_id,
            "rev": rev,
            "unchanged": false,
            "active_runs": self.active_run_count(),
            "generations": {"project": run_gen, "orchestrator": self.orchestrator_generation(project_id)},
            "workers": workers,
            "receipts": recent,
            // ADR-15 4.2: the board's cards, from cached verdicts only (see `board`).
            "board": self.board(project_id),
        }))
    }
}
