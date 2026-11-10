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
/// Inbox items in one project's projection (the count is the true number waiting in that project).
const INBOX_MAX: usize = 50;
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
    /// project_id -> fingerprint of the waiting items last read, so an item that arrives or leaves moves the revision.
    inbox_fp: HashMap<String, u64>,
    /// Whether the last inbox read worked (it starts true).
    inbox_ok: bool,
    /// project_id -> the fields `project_state` shows that change without a bump of their own, as it last saw them: the
    /// Director-General generation (a conversation created or renewed by any path: plain `new_conversation`,
    /// `lead_status`) and the count of active runs (a PTY that closes with nothing on the bus). One that moved bumps the
    /// revision before the `since_rev` check (#105 review).
    quiet: HashMap<String, (Option<u32>, usize)>,
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
        Ok(Self { store, base, revs: HashMap::new(), run_gen, start_gate: Default::default(), workers_fp: 0, inbox_fp: HashMap::new(), inbox_ok: true, quiet: HashMap::new() })
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
        self.act_bound(env, None, caller, human_only, action, target, current_generation, effect)
    }

    /// `act` for a target bound to a project (`bind`: the project the target resolved to, or why it has none). The binding
    /// is enforced under the same lock as the replay decision, after the same expiry `begin` applies: only an `action_id`
    /// that `begin` will answer from a stored receipt (a replay, or `action_id_reuse`) skips it. Anything else that names
    /// another project (`project_mismatch`) or a target with no project is refused with nothing stored (#105 review).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn act_bound(&self, env: &Envelope, bind: Option<Result<String, Value>>, caller: Caller, human_only: bool, action: &str, target: &str, current_generation: impl Into<Gen>, effect: impl FnOnce() -> Res) -> Value {
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
            let now = now_ms();
            // Expired exactly as `begin` will: a receipt past the dedupe window is not a replay, so it neither skips the
            // binding nor counts as "already there".
            let existed = h.store.live(&env.action_id, now);
            if !existed {
                match bind {
                    Some(Ok(p)) if p != env.project_id => return json!({"receipt": null, "error": {"error": "project_mismatch", "detail": "this action names a different project than its target; refresh and try again"}}),
                    Some(Err(e)) => return json!({"receipt": null, "error": e}),
                    _ => {}
                }
            }
            let rev = h.rev(&env.project_id);
            (h.store.begin(&req, current_generation, rev, now), existed)
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
                        // An effect that may have half run (a panic) or whose change landed but could not be confirmed
                        // (`outcome_unknown`) is unknown, never a refusal: the caller must reload and review.
                        let state = if code == "effect_panicked" || code == "outcome_unknown" {
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

    /// Reject plan `id` with a reason, through a receipt. Pass the `plan_sha256` the human read (as `act_approve_plan` does,
    /// as a string so no precision is lost crossing JSON): required: a missing or malformed hash is refused as invalid, and a plan that is not that exact one now is refused as stale, before
    /// any ledger line or progress call. The generation the receipt guards on is derived from the hash here, as an exact u64.
    pub fn act_reject_plan(&self, env: &Envelope, id: &str, reason: &str, plan_sha256: &str) -> Value {
        let shown = lock(&self.shown).get(id).and_then(|s| sha_generation(s));
        let mut env = env.clone();
        env.expect_generation = env.expect_generation.or_else(|| sha_generation(plan_sha256));
        self.act(&env, Caller::Ui, true, "reject_plan", id, shown, || {
            // Required: a reject without the hash the human read would reject whichever plan is current, unseen.
            if plan_sha256.len() != 64 || !plan_sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(super::invalid("plan_sha256", "the plan hash you read is required (64 hex chars)"));
            }
            self.reject_plan_expecting(id, reason, Some(plan_sha256))
        })
    }

    /// Answer a waiting task (a question or a block) from the inbox: the text goes to that task's worker as a control
    /// message, like `control_message`. Never a shell: the control service queues it for the worker to read. A run-level
    /// question (`question:<run>`) has no worker and is answered in the Director-General conversation.
    pub fn act_answer_question(&self, env: &Envelope, id: &str, text: &str) -> Value {
        let target = self.answer_target(id);
        let cur = target.as_ref().ok().and_then(|w| lock(&self.control).worker_generation(w));
        self.act(env, Caller::Ui, true, "answer_question", id, cur, || {
            let worker = target?;
            if text.trim().is_empty() {
                return Err(super::invalid("text", "an answer cannot be empty"));
            }
            self.control_message_expecting(&worker, text, env.expect_generation)
        })
    }

    /// The worker argument for an inbox item the human may answer, or why not.
    fn answer_target(&self, id: &str) -> Result<String, Value> {
        if id.starts_with("question:") {
            return Err(super::fail("not_answerable", "this question belongs to the whole run; answer it in the Director-General conversation"));
        }
        let inbox = self.inbox_all()?;
        let item = inbox.iter().find(|i| i["id"] == id && i["kind"] == "task").ok_or_else(|| super::fail("not_found", "that item is no longer waiting"))?;
        let campaign = item["campaign"].as_str().unwrap_or_default();
        let task = id.strip_prefix("task:").and_then(|r| r.strip_prefix(campaign)).and_then(|r| r.strip_prefix('.')).filter(|t| !t.is_empty()).ok_or_else(|| super::invalid("id", "not a task item"))?;
        // Bound to THIS campaign: only a worker whose task id is `<one of its runs>.<task>` can be the target. A live `t1`
        // of another campaign is never found (and two campaigns sharing `t1` never collide).
        let runs: Vec<String> = super::runindex::entries(&self.data.join("runs.jsonl")).into_iter().filter(|e| e.campaign == campaign).map(|e| e.run_id).collect();
        match lock(&self.control).task_worker(&runs, task) {
            Ok(Some(worker)) => Ok(worker),
            Ok(None) => Err(super::fail("not_found", "no worker of this campaign is on that task")),
            Err(e) => Err(e.to_json()),
        }
    }

    /// An inbox read happened: a project whose waiting items differ from the last read moves its revision, so
    /// `project_state` shows the change with no poll. Called after the read, with no lock held.
    pub(super) fn inbox_read(&self, fps: HashMap<String, u64>) {
        let moved: Vec<Value> = {
            let mut h = lock(&self.acts);
            let mut ids: Vec<String> = h.inbox_fp.keys().chain(fps.keys()).cloned().collect();
            ids.sort();
            ids.dedup();
            let mut moved = Vec::new();
            for p in ids {
                let (before, now) = (h.inbox_fp.get(&p).copied().unwrap_or(0), fps.get(&p).copied().unwrap_or(0));
                if before != now {
                    moved.push(json!({"project_id": p, "rev": h.bump(&p)}));
                }
            }
            h.inbox_fp = fps;
            moved
        };
        if !moved.is_empty() {
            self.emit_projects(moved);
        }
    }

    /// The inbox read worked or failed. A change of availability moves every known project's revision (and this
    /// one's), so a projection built from a failed read is never answered `unchanged` from a cached good one, and the
    /// recovery is seen too.
    fn inbox_availability(&self, ok: bool, project_id: &str) {
        let moved: Vec<Value> = {
            let mut h = lock(&self.acts);
            if h.inbox_ok == ok {
                return;
            }
            h.inbox_ok = ok;
            let mut ids: Vec<String> = h.revs.keys().cloned().collect();
            if !ids.iter().any(|p| p == project_id) {
                ids.push(project_id.to_string());
            }
            ids.into_iter().map(|p| json!({"project_id": p, "rev": h.bump(&p)})).collect()
        };
        self.emit_projects(moved);
    }

    /// The shell hands in how to emit named events (`inbox_new`), next to the `changed` sink.
    pub fn set_event_sink(&self, sink: super::EventSink) {
        self.events.set_sink(sink);
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
        // Read the inbox first: a project whose waiting items changed bumps its revision, and the revision below must be
        // the one that already includes it.
        let read = self.inbox_all();
        self.inbox_availability(read.is_ok(), project_id);
        let inbox = match read {
            Ok(v) => {
                // Counted and picked from the whole list: the global 200-item cut belongs to `inbox`, not to a project.
                let mine: Vec<&Value> = v.iter().filter(|i| i["project_id"] == project_id).collect();
                json!({"ok": true, "count": mine.len(), "campaign_count": mine.iter().filter(|i| i["kind"] != "run_proposal").count(), "items": mine.into_iter().take(INBOX_MAX).collect::<Vec<_>>()})
            }
            // Never "nothing waiting" on a failed read.
            Err(e) => json!({"ok": false, "error": e["error"]}),
        };
        // Read before the revision: a field that moved since the last read without bumping it (`Hub::quiet`) bumps it now,
        // so `since_rev` never answers `unchanged` across a new or renewed conversation or a run that ended quietly.
        let orch_gen = self.orchestrator_generation(project_id);
        let active_runs = self.active_run_count();
        let mut moved: Vec<Value> = Vec::new();
        let (rev, run_gen, recent) = {
            let mut h = lock(&self.acts);
            let seen = h.revs.contains_key(project_id);
            let now = (orch_gen, active_runs);
            if h.quiet.insert(project_id.to_string(), now) != Some(now) && seen {
                moved.push(json!({"project_id": project_id, "rev": h.bump(project_id)}));
            }
            let rev = h.rev(project_id);
            let rg = h.run_gen.get(project_id).copied().unwrap_or(0);
            let recent: Vec<Receipt> = h.store.recent(project_id, RECENT).into_iter().cloned().collect();
            (rev, rg, recent)
        };
        self.emit_projects(moved);
        if since_rev == Some(rev) {
            return Ok(json!({"schema": "vajra.project_state/1", "project_id": project_id, "rev": rev, "unchanged": true}));
        }
        let workers = lock(&self.control).workers_view();
        Ok(json!({
            "schema": "vajra.project_state/1",
            "project_id": project_id,
            "rev": rev,
            "unchanged": false,
            "active_runs": active_runs,
            "generations": {"project": run_gen, "orchestrator": orch_gen},
            "workers": workers,
            "receipts": recent,
            // ADR-15 4.2: the board's cards, from cached verdicts only (see `board`).
            "board": self.board(project_id),
            "inbox": inbox,
        }))
    }
}
