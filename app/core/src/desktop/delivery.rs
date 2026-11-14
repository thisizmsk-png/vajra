//! 6i: the evidence ticket and the delivery, the round ticket's last phase (ADR-16 16.5a "The round ticket"). When a
//! round's gate passes, `CoreGate` hands the verifier's `Verification` here, and the run is `verified` only if every
//! step below passes. Any failure ends the run `not_verified` with the step and its reason in plain words.
//!
//! 1. **The request.** The exact ordered list: the verifier's setup commands, then each criterion's check command in
//!    criteria order, then the suite command. Bound to the run, round, integration commit and verifier seat (`Ask`).
//! 2. **Approval by trust** (fixed at Start in `seats.json`). `guarded_by_approval`: the person approves the whole list
//!    in the app (N20), or the run ends not verified. `guarded` and `outside_guard` skip it; an `outside_guard` run
//!    keeps its label everywhere it's read.
//! 3. **The evidence run,** in a fresh worktree at the integration commit that no agent session ever works in, through
//!    the core's sandboxed evidence runner after its launch-time key probe. An approval run goes through
//!    `run_approved_evidence`, which uses the approval once and checks the worktree is exactly the approved commit.
//! 4. **Sign** (`--sign`, outside the sandbox, only the bytes the core recorded), then **`CheckEvidence`** for the
//!    integration commit.
//! 5. **The record,** each write through the core's ledger: every task's close on the evidence, the evidence archive,
//!    each criterion's pass, the delivered progress line and checkpoint, the done close and the delivery report.
//! 6. **`CampaignVerify` last,** with the ledger check: the record's signature holds and every entry is the core's.
//!
//! Between steps the run's life is checked: once a Stop, a timeout or a lost lead has ended the run, nothing more runs.
//! The evidence worktree is removed at the end, whatever happened.
//!
//! Every outside effect is a `Steps` method, so the sequence is tested with fakes; `CoreSteps` is the real one.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use super::evidence_approval::{ApprovalDesk, Ask, progress_line};
use super::plan::Criterion;
use super::proof::Trust;
use super::record::{CoreRecord, OUTSIDE_GUARD, RunRecord};
use super::runproto::{Delivered, RoundInput, RunLife};
use super::verify::Verification;
use crate::adapter::evidence_runner::{self as runner, EvidenceRun, Profile, RunOutcome, SignOutcome};
use crate::control::service::WorkerSeat;
use crate::vajra::bridge::{Bridge, CloseStatus, Request, fnv32};
use crate::vajra::bridge_record::{self as bridge_text, EVIDENCE_MAX, LABEL_MAX, MAX_SETUP, PROGRESS_MAX, REPORT_MAX, SETUP_MAX};
use crate::vajra::gitw::{Gitw, Ticket};

/// How often the evidence ticket looks for the person's answer.
const ANSWER_POLL: Duration = Duration::from_secs(1);

/// The run the delivery is for.
#[derive(Clone, Debug)]
pub struct DeliveryConfig {
    pub run: String,
    /// The run's campaign (`seats.json`'s `campaign_slug`); the evidence folder is named for it.
    pub slug: String,
    /// `seats.json`'s `evidence_trust`, fixed at Start.
    pub trust: Trust,
}

/// The evidence worktree, as the runner and the signing step use it. Every path is plain and resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    /// The worktree, detached at the integration commit; the commands' cwd.
    pub worktree: PathBuf,
    /// The run's own tmp dir: must not exist yet (the runner makes it fresh).
    pub tmp: PathBuf,
    /// The repo's common git dir.
    pub git_common: PathBuf,
    /// The walkthrough folder in the worktree: the bundle lands in `<base>/<slug>/`.
    pub base: PathBuf,
}

/// Every outside effect of the delivery.
pub trait Steps: Send + Sync {
    /// A fresh evidence worktree at the round's integration commit.
    fn tree(&self, round: &RoundInput) -> Result<Tree, String>;
    /// Asks the person and waits for the answer: `Ok` only once they approved this exact ask. Any other end (a
    /// decline, no answer within the hold, the run ended) is `Err` with the reason, and the ask is then void.
    fn approve(&self, ask: &Ask, life: &dyn RunLife) -> Result<(), String>;
    /// Runs the commands in the evidence runner. `approved`: through the approval, used once; its sha comes back.
    fn run(&self, ask: Ask, approved: bool, tree: &Tree) -> Result<(Option<String>, RunOutcome), String>;
    fn sign(&self, slug: &str, sha256: &str, tree: &Tree) -> SignOutcome;
    /// `check-evidence` for `head`: `Ok` only when green.
    fn check(&self, evidence: &Path, head: &str, tree: &Tree) -> Result<(), String>;
    /// One record write, through the core's ledger.
    fn write(&self, req: &Request) -> Result<(), String>;
    /// `CampaignVerify` and the ledger check.
    fn verify(&self) -> Result<(), String>;
    /// Removes the evidence worktree.
    fn remove(&self, tree: &Tree);
}

/// The command list, in order: setup, each criterion's check, the suite. `setup` is kept apart for the archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commands {
    pub setup: Vec<String>,
    pub all: Vec<String>,
    /// Each criterion's check command, as the verifier ran it: `(criterion id, command)`, in criteria order.
    pub checks: Vec<(String, String)>,
}

impl Commands {
    fn of(&self, criterion: &str) -> &str {
        self.checks.iter().find(|(id, _)| id == criterion).map_or("", |(_, c)| c.as_str())
    }
}

/// The ordered list from the verifier's answer, or why it can't be run as the evidence.
pub fn commands(v: &Verification, criteria: &[Criterion]) -> Result<Commands, String> {
    let push = |list: &mut Vec<String>, c: &str| {
        let c = c.trim();
        if !c.is_empty() && !list.iter().any(|x| x == c) {
            list.push(c.to_string());
        }
    };
    let mut setup = Vec::new();
    for c in &v.setup_cmds {
        push(&mut setup, c);
    }
    // The archive keeps the setup list whole (`vajra_tasks.py` takes at most 5, each at most 1000 characters): one it
    // can't keep is refused here, never cut, so the archive always names exactly what ran.
    if setup.len() > MAX_SETUP {
        return Err(format!("the verifier listed {} setup commands, and Vajra can keep at most {MAX_SETUP}", setup.len()));
    }
    if setup.iter().any(|c| c.chars().count() > SETUP_MAX) {
        return Err(format!("a setup command is longer than {SETUP_MAX} characters"));
    }
    let mut list = Vec::new();
    let mut per = Vec::new();
    for c in criteria {
        let r = v.results.iter().find(|r| r.criterion_id == c.id).ok_or_else(|| format!("criterion {} has no check in the verifier's answer", c.id))?;
        if r.command.trim().is_empty() {
            return Err(format!("criterion {} has no check command", c.id));
        }
        push(&mut list, &r.command);
        per.push((c.id.clone(), r.command.trim().to_string()));
    }
    push(&mut list, &v.suite_command);
    let mut all = setup.clone();
    all.extend(list);
    Ok(Commands { setup, all, checks: per })
}

/// The delivery phase for one run.
pub struct Delivery {
    cfg: DeliveryConfig,
    steps: Arc<dyn Steps>,
}

/// The steps run so far, and the end.
struct Log(Vec<Value>);

impl Log {
    fn ok(&mut self, step: &str) {
        self.0.push(json!({"step": step, "ok": true}));
    }

    fn fail(&mut self, step: &str, why: &str) -> Delivered {
        let why = bridge_text::one_line(why, 300);
        self.0.push(json!({"step": step, "ok": false, "why": why}));
        Delivered { verified: false, reason: format!("{step}: {why}"), steps: std::mem::take(&mut self.0) }
    }
}

/// The run is still going: a no-op through `life.start`, which refuses once the run has ended.
pub(crate) fn live(life: &dyn RunLife) -> bool {
    life.start(&mut || {})
}

const ENDED: &str = "the run ended before the evidence check finished";

impl Delivery {
    pub fn new(cfg: DeliveryConfig, steps: Arc<dyn Steps>) -> Self {
        Delivery { cfg, steps }
    }

    /// Runs the whole delivery for a passed round. `verifier` is the seat whose commands these are.
    pub fn deliver(&self, round: &RoundInput, v: &Verification, verifier: &WorkerSeat, life: &dyn RunLife) -> Delivered {
        let mut log = Log(Vec::new());
        let cmds = match commands(v, &round.criteria) {
            Ok(c) => c,
            Err(why) => return log.fail("The check commands", &why),
        };
        let ask = Ask {
            run_id: self.cfg.run.clone(), round: round.round, commit: round.head.clone(), seat: verifier.slot.clone(), account: verifier.account.clone(),
            commands: cmds.all.clone(), network: verifier.network,
        };
        if let Err(why) = ask.check() {
            return log.fail("The check commands", &why);
        }
        // Every task closes on the evidence: one with no commit to close on can't be delivered.
        if let Some(t) = round.tasks.iter().find(|t| !round.closes.iter().any(|c| c.id == format!("{}.{}", self.cfg.run, t.id))) {
            return log.fail("The task graph", &format!("task {} has no commit to close on the evidence", t.id));
        }
        log.ok("request");
        if !live(life) {
            return log.fail("The evidence check", ENDED);
        }
        let tree = match self.steps.tree(round) {
            Ok(t) => t,
            Err(why) => return log.fail("The evidence worktree", &why),
        };
        log.ok("worktree");
        let out = self.in_tree(round, ask, &cmds, &tree, life, &mut log);
        self.steps.remove(&tree);
        out
    }

    fn in_tree(&self, round: &RoundInput, ask: Ask, cmds: &Commands, tree: &Tree, life: &dyn RunLife, log: &mut Log) -> Delivered {
        let (slug, head) = (self.cfg.slug.as_str(), round.head.as_str());
        let approved = self.cfg.trust == Trust::GuardedByApproval;
        if approved {
            if let Err(why) = self.steps.approve(&ask, life) {
                return log.fail("Your approval", &why);
            }
            log.ok("approval");
        }
        if !live(life) {
            return log.fail("The evidence check", ENDED);
        }
        let (used, outcome) = match self.steps.run(ask, approved, tree) {
            Ok(x) => x,
            Err(why) => return log.fail("The evidence run", &why),
        };
        // A used approval is on the signed record whatever the run found (the proof needs the line, N20).
        if let Some(sha) = &used {
            if let Err(why) = self.steps.write(&Request::CampaignProgress { slug: slug.into(), text: progress_line(sha) }) {
                return log.fail("The record", &format!("the approval's progress line wasn't written: {why}"));
            }
        }
        let sha256 = match outcome {
            RunOutcome::Ran { passed: true, sha256 } => sha256,
            RunOutcome::Ran { passed: false, .. } => return log.fail("The evidence run", "a check command failed when Vajra ran it"),
            RunOutcome::Failed(why) => return log.fail("The evidence run", &why),
        };
        log.ok("run");
        match self.steps.sign(slug, &sha256, tree) {
            SignOutcome::Signed => log.ok("sign"),
            SignOutcome::Refused(why) => return log.fail("Signing the evidence", &format!("refused ({why})")),
            SignOutcome::NoKey => return log.fail("Signing the evidence", "Vajra's signing key isn't usable"),
            SignOutcome::Unknown(why) => return log.fail("Signing the evidence", &why),
        }
        let evidence = tree.base.join(slug).join("evidence.json");
        if let Err(why) = self.steps.check(&evidence, head, tree) {
            return log.fail("The evidence check", &why);
        }
        log.ok("check_evidence");
        // The record. A Stop between writes lets the current one finish, then ends the run (16.5a).
        for c in &round.closes {
            if !live(life) {
                return log.fail("The record", ENDED);
            }
            let req = Request::TasksClose {
                slug: slug.into(), id: c.id.clone(), evidence: evidence.clone(), head: head.into(), commit: c.commit.clone(), branch: c.branch.clone(),
                base_commit: round.base.clone(), changed: c.changed.clone(), repo: tree.worktree.clone(),
            };
            if let Err(why) = self.steps.write(&req) {
                return log.fail("Closing the tasks", &format!("task {} didn't close: {why}", c.id));
            }
        }
        log.ok("tasks_close");
        let archive = Request::CampaignArchiveEvidence { slug: slug.into(), dir: tree.base.join(slug), head: head.into(), setup: cmds.setup.clone() };
        if let Err(why) = self.steps.write(&archive) {
            return log.fail("Archiving the evidence", &why);
        }
        log.ok("archive");
        let marked = |s: String| if self.cfg.trust == Trust::OutsideGuard { format!("{OUTSIDE_GUARD} {s}") } else { s };
        for c in &round.criteria {
            let command = cmds.of(&c.id);
            let req = Request::CampaignPass { slug: slug.into(), feature: c.id.clone(), evidence: bridge_text::one_line(&marked(format!("{command} (exit 0)")), EVIDENCE_MAX) };
            if let Err(why) = self.steps.write(&req) {
                return log.fail("Passing the criteria", &format!("criterion {} wasn't marked passed: {why}", c.id));
            }
        }
        log.ok("pass");
        let line = bridge_text::one_line(&marked(format!("run {} delivered round {} at {head}", self.cfg.run, round.round)), PROGRESS_MAX);
        let label = bridge_text::one_line(&marked(format!("delivered {head}")), LABEL_MAX);
        for (req, what) in [
            (Request::CampaignProgress { slug: slug.into(), text: line }, "the delivered progress line"),
            (Request::CampaignCheckpoint { slug: slug.into(), label }, "the delivered checkpoint"),
            (Request::CampaignClose { slug: slug.into(), status: CloseStatus::Done }, "the done close"),
        ] {
            if let Err(why) = self.steps.write(&req) {
                return log.fail("The record", &format!("{what} wasn't written: {why}"));
            }
        }
        log.ok("close");
        let report = self.report(round, cmds);
        let b64 = crate::providers::install::plan::base64(report.as_bytes());
        let req = Request::CampaignReport { slug: slug.into(), bytes: report.len(), check: fnv32(&b64), base64: b64 };
        if let Err(why) = self.steps.write(&req) {
            return log.fail("The delivery report", &why);
        }
        log.ok("report");
        // Last: the report re-signs the state, so the check covers every write above.
        if let Err(why) = self.steps.verify() {
            return log.fail("The run record", &why);
        }
        log.ok("verify");
        let reason = match self.cfg.trust {
            Trust::OutsideGuard => format!("round {} passed and its evidence was checked; the evidence ran outside the Vajra guard", round.round),
            Trust::GuardedByApproval => format!("round {} passed and its evidence was checked; check commands approved by you", round.round),
            Trust::Guarded => format!("round {} passed and its evidence was checked", round.round),
        };
        Delivered { verified: true, reason, steps: std::mem::take(&mut log.0) }
    }

    /// The delivery report, built in code: no worker's prose, only the run's facts and the commands that ran.
    fn report(&self, round: &RoundInput, cmds: &Commands) -> String {
        let trust = match self.cfg.trust {
            Trust::Guarded => "guarded",
            Trust::GuardedByApproval => "guarded, check commands approved by you",
            Trust::OutsideGuard => "outside the Vajra guard",
        };
        let mut s = format!("# Delivery report\n\nRun {} delivered round {} at commit {}.\n\nEvidence: {trust}.\n\n## Criteria\n", self.cfg.run, round.round, round.head);
        for c in &round.criteria {
            s += &format!("- {}: {}\n  check: {}\n", c.id, bridge_text::one_line(&c.criterion, 300), bridge_text::one_line(cmds.of(&c.id), 1000));
        }
        s += "\n## Tasks closed on the evidence\n";
        for c in &round.closes {
            s += &format!("- {} at {}\n", c.id, c.commit);
        }
        if !cmds.setup.is_empty() {
            s += "\n## Setup commands\n";
            for c in &cmds.setup {
                s += &format!("- {}\n", bridge_text::one_line(c, 1000));
            }
        }
        if s.len() > REPORT_MAX {
            let mut end = REPORT_MAX;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
        }
        s
    }
}

// ---------------------------------------------------------------- the real steps

/// What the real steps need.
pub struct CoreStepsConfig {
    pub desk: ApprovalDesk,
    pub record: Arc<CoreRecord>,
    /// The app's bridge: `check-evidence` runs through it.
    pub bridge: Bridge,
    pub gw: Arc<Gitw>,
    pub repo: PathBuf,
    pub project: String,
    pub run: String,
    /// The engine's `vajra-verify-work.sh` (6i-e: `--run-only` and `--sign`).
    pub script: PathBuf,
    pub home: PathBuf,
    /// The app's data dir (denied to the commands).
    pub data: PathBuf,
    /// The Vajra state dir (its key must be out of the commands' reach, proven by the probe).
    pub state_dir: PathBuf,
    /// The user's resolved PATH, for the check commands' tools.
    pub tool_path: Vec<PathBuf>,
}

/// The real steps: the core's git writer, the evidence runner, the bridge and the run's record.
pub struct CoreSteps {
    cfg: CoreStepsConfig,
    tickets: Mutex<Vec<Ticket>>,
}

impl CoreSteps {
    pub fn new(cfg: CoreStepsConfig) -> Self {
        CoreSteps { cfg, tickets: Mutex::new(Vec::new()) }
    }

    fn profile(&self, tree: &Tree, network: bool) -> Profile {
        Profile { worktree: tree.worktree.clone(), tmp: tree.tmp.clone(), deny: runner::deny_dirs(&self.cfg.home, &self.cfg.data), network }
    }
}

impl Steps for CoreSteps {
    fn tree(&self, round: &RoundInput) -> Result<Tree, String> {
        let c = &self.cfg;
        let t = c.gw.worktree_add(&c.repo, &c.project, &c.run, "evidence", round.round, &round.head).map_err(|e| format!("it couldn't be made ({})", e.code()))?;
        let made = (|| {
            let worktree = t.wt.canonicalize().map_err(|_| "its folder can't be resolved")?;
            let base = worktree.join(super::VERIFY_EXCLUDES[0]);
            std::fs::create_dir_all(&base).map_err(|_| "its walkthrough folder can't be made")?;
            let base = runner::canonical_base(&base).ok_or("its walkthrough folder isn't a plain folder")?;
            let tmp = t.tmp.canonicalize().map_err(|_| "its temp folder can't be resolved")?.join("evidence-run");
            let git_common = t.common_dir().canonicalize().map_err(|_| "the repository's git folder can't be resolved")?;
            Ok::<Tree, &str>(Tree { worktree, tmp, git_common, base })
        })();
        match made {
            Ok(tree) => {
                crate::lock(&self.tickets).push(t);
                Ok(tree)
            }
            Err(why) => {
                let _ = c.gw.discard(&t);
                Err(why.into())
            }
        }
    }

    fn approve(&self, ask: &Ask, life: &dyn RunLife) -> Result<(), String> {
        self.cfg.desk.await_answer(ask, &|| live(life), ANSWER_POLL)
    }

    fn run(&self, ask: Ask, approved: bool, tree: &Tree) -> Result<(Option<String>, RunOutcome), String> {
        let c = &self.cfg;
        let profile = self.profile(tree, ask.network);
        if !runner::key_probe(&profile, &c.state_dir) {
            return Err("Vajra couldn't prove the check commands can't reach its signing key, so it didn't run them".into());
        }
        let slug = c.record.card().campaign;
        let run = EvidenceRun {
            script: &c.script, slug: &slug, commands: &ask.commands, base: &tree.base, git_common: &tree.git_common, home: &c.home, state_dir: &c.state_dir,
            tool_path: &c.tool_path, limit: runner::RUN_MAX, at_spawn: None,
        };
        if approved {
            let (sha, out) = c.desk.run_approved_evidence(ask.clone(), &profile, &run, SystemTime::now())?;
            Ok((Some(sha), out))
        } else {
            Ok((None, runner::run(&profile, &run)))
        }
    }

    fn sign(&self, slug: &str, sha256: &str, tree: &Tree) -> SignOutcome {
        runner::sign(&self.cfg.script, slug, sha256, &tree.base, &tree.worktree, &self.cfg.home)
    }

    fn check(&self, evidence: &Path, head: &str, tree: &Tree) -> Result<(), String> {
        let req = Request::CheckEvidence { path: evidence.to_path_buf(), head: Some(head.into()), archived: None, repo: Some(tree.worktree.clone()) };
        let o = self.cfg.bridge.run(&req).map_err(|e| format!("check-evidence didn't run: {e}"))?;
        let v: Value = serde_json::from_str(o.stdout.trim()).unwrap_or(Value::Null);
        if o.exit() == Some(0) && v["green"] == true {
            return Ok(());
        }
        let reasons: Vec<&str> = v["reasons"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        Err(if reasons.is_empty() { "the evidence isn't green for the integration commit".into() } else { format!("not green: {}", reasons.join("; ")) })
    }

    fn write(&self, req: &Request) -> Result<(), String> {
        self.cfg.record.flush();
        self.cfg.record.write(req)
    }

    fn verify(&self) -> Result<(), String> {
        self.cfg.record.verify_now()
    }

    fn remove(&self, tree: &Tree) {
        let mut held = crate::lock(&self.tickets);
        if let Some(i) = held.iter().position(|t| t.wt.canonicalize().ok().as_deref() == Some(tree.worktree.as_path())) {
            let t = held.remove(i);
            drop(held);
            let _ = self.cfg.gw.discard(&t);
        }
    }
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
