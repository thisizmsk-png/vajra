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

/// Receipts shown in one projection.
const RECENT: usize = 20;
/// Error codes that mean the effect was attempted and broke, not that it was refused up front.
const FAILED_CODES: [&str; 5] = ["io_error", "send_failed", "spawn_failed", "launch_failed", "kb_failed"];

pub(super) struct Hub {
    pub(super) store: Store,
    /// project_id -> revision. Starts at 1 when first seen.
    revs: HashMap<String, u64>,
    /// project_id -> runs started through a receipt (the project generation `start_run` is guarded by).
    run_gen: HashMap<String, u64>,
}

impl Hub {
    pub(super) fn open(data: &Path) -> std::io::Result<Self> {
        let store = Store::open(&data.join("control/ledger.jsonl"), now_ms())?;
        Ok(Self { store, revs: HashMap::new(), run_gen: HashMap::new() })
    }

    pub(super) fn rev(&mut self, project: &str) -> u64 {
        *self.revs.entry(project.to_string()).or_insert(1)
    }

    fn bump(&mut self, project: &str) -> u64 {
        let r = self.revs.entry(project.to_string()).or_insert(1);
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
    pub(super) fn act(&self, env: &Envelope, caller: Caller, human_only: bool, action: &str, target: &str, current_generation: Option<u64>, effect: impl FnOnce() -> Res) -> Value {
        let req = Request { action_id: &env.action_id, project_id: &env.project_id, action, target, expect_generation: env.expect_generation, caller, human_only };
        let begun = {
            let mut h = lock(&self.acts);
            let rev = h.rev(&env.project_id);
            h.store.begin(&req, current_generation, rev, now_ms())
        };
        let (rc, outcome) = match begun {
            // The ledger would not take the receipt: nothing was done, and nothing is claimed.
            Err(e) => return json!({"receipt": null, "error": fail("io_error", e)}),
            Ok(Begin::Done(rc)) => {
                if rc.state == State::Rejected {
                    lock(&self.acts).bump(&env.project_id);
                }
                return json!({"receipt": rc, "replayed": true});
            }
            Ok(Begin::Run(_)) => {
                let out = effect();
                let (state, reason) = match &out {
                    Ok(_) => (State::Done, None),
                    Err(v) => {
                        let code = v["error"].as_str().unwrap_or("failed");
                        (if FAILED_CODES.contains(&code) { State::Failed } else { State::Rejected }, Some(code.to_string()))
                    }
                };
                let detail = out.as_ref().err().and_then(|v| v["detail"].as_str().map(str::to_string));
                let mut h = lock(&self.acts);
                if state == State::Done && action == "start_run" {
                    *h.run_gen.entry(env.project_id.clone()).or_insert(0) += 1;
                }
                let rev = h.bump(&env.project_id);
                let rc = match h.store.finish(&env.action_id, state, reason.as_deref(), detail.as_deref(), rev, now_ms()) {
                    Ok(rc) => rc,
                    // The effect ran but its outcome could not be written: say so, never claim done.
                    Err(_) => return json!({"receipt": null, "error": fail("io_error", "outcome not recorded"), "result": out.ok()}),
                };
                (rc, out)
            }
        };
        match outcome {
            Ok(v) => json!({"receipt": rc, "result": v}),
            Err(e) => json!({"receipt": rc, "error": e}),
        }
    }

    pub fn act_start_run(&self, env: &Envelope, input: RunInput, cwd: &str) -> Value {
        let cur = lock(&self.acts).run_gen.get(&env.project_id).copied().unwrap_or(0);
        self.act(env, Caller::Ui, false, "start_run", &env.project_id, Some(cur), || self.start_run(input, cwd))
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
            "generations": {"project": run_gen},
            "workers": workers,
            "receipts": recent,
        }))
    }
}
