//! The verify and review phases, run by the core (ADR-16 16.5a, PR 6g). On a clean `integrate{round}` the run protocol
//! hands the round to `CoreGate`, which starts, in parallel and with no lead call, a verify ticket on the `verifier`
//! seat, a design review on the `reviewer` seat and, on a security-flagged run, a security review on the seat the
//! 16.1 order picks (N15: any harness). Each is a fresh check session at the integration commit. Their last messages
//! are parsed into the workflow's `Verification` and `Review` shapes; a ticket with no valid result gets one retry; a
//! verification whose commands can't be re-run gets one redo; then the ported rejection check and gate decide the
//! round. The briefs are the workflow's `verifyPrompt` and `reviewPrompt`, ported verbatim (`vajra-deliver.js:644–676`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::plan::{Criterion, PlanTask, one_line};
use super::runproto::{ProtectedAction, RoundGate, RoundInput, RoundResult, RoundVerdict, data};
use super::verify::{self, Review, Verification};
use super::workers::{CheckKind, Outcome, State, TaskSpec, WorkerRuntime};
use crate::control::service::WorkerSeat;
use crate::crew::redact::Redactor;
use crate::crew::roles;
use crate::providers::launch;

// ---------------------------------------------------------------- the briefs

pub const VERIFY_BRIEF: &str = "vajra.verify_brief/1";
pub const REVIEW_BRIEF: &str = "vajra.review_brief/1";
/// The workflow's `lenses` (`:661`): the persona is the lens text, not a subagent, so any harness gets the same brief.
pub const DESIGN_LENS: &str = "correctness, design, maintainability";
pub const SECURITY_LENS: &str = "security: OWASP Top 10, authz, secrets, injection";

/// The workflow's `GUARD` (`:117`).
pub const GUARD: &str = "Never push, open a PR, deploy, run migrations against a shared database, delete data outside your worktree, \
read or write secrets, spend money or send external messages. If the work needs one of these, do not do it: \
list it in your output instead. Never run install.sh, install-skills.sh or generate-manifest.sh (from any copy, \
including your worktree), and never write under the real ~/.claude: tests set HOME to a temp dir.";

/// The workflow passes `S.Verification` as a schema; a session here answers in its last message instead.
const VERIFY_ANSWER: &str = "When you are done, your final message is only this JSON object, bare or in one ```json fence, with every \
field and no others: {\"results\": [{\"criterion_id\": \"ac-1\", \"status\": \"pass | fail | not_run\", \"command\": \"...\", \
\"notes\": \"...\", \"exit_code\": 0, \"evidence\": \"...\"}], \"suite_command\": \"...\", \"suite_passes\": true, \"regressions\": [\"...\"], \
\"setup_cmds\": [\"...\"]}";
const REVIEW_ANSWER: &str = "When you are done, your final message is only this JSON object, bare or in one ```json fence, with every \
field and no others: {\"verdict\": \"approve | revise | reject\", \"blocking_issues\": [{\"task\": \"t1\", \"issue\": \"...\"}], \
\"risks\": [\"...\"], \"rationale\": \"...\"}";

/// `vajra.verify_brief/1`: `verifyPrompt(problems)`, plus how to answer. `problems` is the redo's `fb` (`:683`).
pub fn verify_brief(head: &str, criteria: &[Criterion], problems: Option<&Value>) -> String {
    let ids = criteria.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(", ");
    let mut s = format!("Verify the integration commit {head} independently. In your worktree run `git checkout --detach {head}`.\n");
    s += "- Environment: build anything the checks need inside this worktree, and list each preparation command in setup_cmds, in order \
(for example `python3 -m venv .venv && .venv/bin/pip install -e '.[dev]'`, or `npm ci`); [] if none. Never use an interpreter, \
virtualenv, node_modules or any other path outside the repository (no scratchpad, /tmp or home-directory paths): these commands are \
re-run later in a different fresh worktree, so every path must be repo-relative. Do not list the git checkout in setup_cmds.\n";
    s += "- For EVERY acceptance criterion run a real check (its listed check, or a better one). command = the exact, literal, single shell \
command you ran from the repository root: it is re-run verbatim with `bash -c` after setup_cmds in a fresh worktree and shown to the \
user under \"How to run it\". No notes, parentheses or prose after it, no elisions (...), no `; echo $?` or `|| true` (its exit code is \
the result). Each command runs in a fresh shell with no virtualenv activated, so call `.venv/bin/python`, not a bare `python`. If you \
ran several variants, chain them with && into one command or keep the decisive one. notes = everything else (variants you also tried, \
which interpreter, caveats); \"\" if none. Record exit_code and a short excerpt of the output as evidence. ";
    s += &format!("Return exactly one result per criterion, with criterion_id set to its id ({ids}), not its text. ");
    s += "A criterion passes only if you ran a command and saw it pass.\n";
    s += "- Then run the project test suite and list regressions; suite_command is the exact command you ran, under the same rules. If the \
project has no test suite, set suite_command to '' and suite_passes to true.\n";
    s += "Do not edit tracked files; never mark a criterion pass without running it. ";
    s += &format!("{GUARD}\n{VERIFY_ANSWER}\n{}", data("acceptance_criteria", &json!(criteria)));
    if let Some(p) = problems {
        s += &format!("\n{}", data("commands_rejected_because", p));
    }
    s
}

/// `vajra.review_brief/1`: `reviewPrompt(lens)`, plus how to answer. `base` is the workflow's `ORIGIN`.
pub fn review_brief(base: &str, head: &str, lens: &str, criteria: &[Criterion], tasks: &[PlanTask]) -> String {
    let mut s = format!("In your worktree run `git checkout --detach {head}`, then review `git diff {base}..{head}` ({lens}). ");
    s += "This review is read-only: do not modify, stage or commit any tracked file, create branches, or write outside your worktree. \
You may run the project's tests or linters to confirm a finding (build any environment inside the worktree). \
Approve only if the change meets the acceptance criteria without unrequested scope. Name the task id (from the plan) for each blocking issue. \
Reject only if the approach is wrong in a way more rounds cannot fix. A separate verifier runs every acceptance check, so do not list \
\"tests not run\" as a risk.\n";
    s += &format!("{GUARD}\n{REVIEW_ANSWER}\n{}\n{}", data("acceptance_criteria", &json!(criteria)), data("plan", &json!(tasks)));
    s
}

// ---------------------------------------------------------------- the planner's knowledge and the protected actions

/// How many knowledge lines the charter carries, and how long each may be.
pub const KB_LINES: usize = 20;
pub const KB_LINE: usize = 300;

/// `KB_DATA`: the `KbSearch` hits over the ask, for the lead's charter, wrapped as data (16.5a). Each line is one line
/// of plain text, redacted; at most `KB_LINES`. No hits: `None`, and the charter says nothing.
pub fn knowledge_data(lines: &[String], red: &Redactor) -> Option<String> {
    let lines: Vec<String> = lines.iter().map(|l| one_line(&red.redact(l.clone()), KB_LINE)).filter(|l| !l.is_empty()).take(KB_LINES).collect();
    (!lines.is_empty()).then(|| data("project_knowledge", &json!(lines)))
}

/// The report's "Waiting for your approval" section: the plan's `protected_actions` (`:748–756`). Drafting a deploy plan
/// is not ported (a D1 non-goal): the section says to ask for one. None: `None`.
pub fn approvals_section(actions: &[ProtectedAction]) -> Option<String> {
    if actions.is_empty() {
        return None;
    }
    let mut s = String::from("Waiting for your approval\n");
    for a in actions {
        s += &format!("- {}: {}\n", one_line(&a.action, 40), one_line(&a.why, 300));
    }
    s += "Vajra didn't do these. To get a deploy plan, ask for one separately.\n";
    Some(s)
}

// ---------------------------------------------------------------- the seam over the check tickets

/// How a check ticket ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckEnd {
    pub outcome: Option<Outcome>,
    pub reason: Option<String>,
}

/// The check tickets, as the gate uses them. `WorkerRuntime` is the real one.
pub trait Checks: Send + Sync {
    /// Starts attempt `n` of a check (0 first, then `<kind>-a<n>`), fresh at `at`.
    fn start(&self, kind: CheckKind, n: u32, spec: TaskSpec, seat: WorkerSeat, at: &str) -> Result<String, String>;
    /// Parks until the ticket ends. `None`: no such ticket.
    fn wait(&self, ticket: &str) -> Option<CheckEnd>;
    /// The ended check's final message.
    fn answer(&self, ticket: &str) -> Option<String>;
    /// Removes the ended check's worktree.
    fn discard(&self, ticket: &str);
}

impl Checks for WorkerRuntime {
    fn start(&self, kind: CheckKind, n: u32, spec: TaskSpec, seat: WorkerSeat, at: &str) -> Result<String, String> {
        self.dispatch_check_attempt(kind, n, spec, seat, at).map_err(|e| format!("{e:?}"))
    }
    fn wait(&self, ticket: &str) -> Option<CheckEnd> {
        let id = [ticket.to_string()];
        loop {
            // Every ticket ends: by its outcome, its worker timeout, or a Stop.
            if let Some(v) = self.await_any(&id, Duration::from_secs(60)) {
                return Some(CheckEnd { outcome: v.outcome, reason: v.reason });
            }
            if self.view(ticket).is_none_or(|v| v.state == State::Ended) {
                return self.view(ticket).map(|v| CheckEnd { outcome: v.outcome, reason: v.reason });
            }
        }
    }
    fn answer(&self, ticket: &str) -> Option<String> {
        self.last_message(ticket)
    }
    fn discard(&self, ticket: &str) {
        let _ = WorkerRuntime::discard(self, ticket);
    }
}

// ---------------------------------------------------------------- seats (16.1, N15)

/// The seats a round's checks run on.
#[derive(Clone, Debug, PartialEq)]
pub struct Picked {
    pub verifier: WorkerSeat,
    pub reviewer: WorkerSeat,
    /// On a flagged run: the security reviewer, and whether it shares a security writer's harness and account.
    pub security: Option<(WorkerSeat, bool)>,
}

fn harness(s: &WorkerSeat) -> Option<&'static str> {
    launch::harness_of(&s.account).map(|e| e.id())
}

/// The run's seats for `role`, then the role defaults the app resolved (`agents.default`, `source: "role_default"`).
fn first_of<'a>(seats: &'a [WorkerSeat], defaults: &'a [WorkerSeat], role: &str) -> Option<&'a WorkerSeat> {
    seats.iter().chain(defaults).find(|s| s.role == role)
}

/// The security reviewer, in the 16.1 order: a `security_reviewer` seat marked `security: true`; any other
/// `security_reviewer` seat; one from the role default. Within each step, a seat whose harness and model both differ
/// from every security writer's is preferred. The run never holds for want of a marked seat (N15, revision 5).
pub fn security_reviewer(seats: &[WorkerSeat], defaults: &[WorkerSeat], writers: &[WorkerSeat]) -> Option<(WorkerSeat, bool)> {
    let sr = |s: &&WorkerSeat| s.role == roles::SECURITY_REVIEWER;
    let marked: Vec<&WorkerSeat> = seats.iter().filter(sr).filter(|s| s.security).collect();
    let other: Vec<&WorkerSeat> = seats.iter().filter(sr).filter(|s| !s.security && s.source != "role_default").collect();
    let default: Vec<&WorkerSeat> = seats.iter().chain(defaults).filter(sr).filter(|s| !s.security && s.source == "role_default").collect();
    // An unset model is the harness's default: on another harness, another model.
    let differs = |s: &WorkerSeat| writers.iter().all(|w| harness(w) != harness(s) && (w.model.is_none() || w.model != s.model));
    let step = [marked, other, default].into_iter().find(|step| !step.is_empty())?;
    let seat = step.iter().find(|s| differs(s)).unwrap_or(&step[0]);
    let shared = writers.iter().any(|w| harness(w) == harness(seat) && w.account == seat.account);
    Some(((*seat).clone(), shared))
}

/// Every seat the round needs, or why the round can't be checked.
pub fn pick(seats: &[WorkerSeat], defaults: &[WorkerSeat], security: bool, writers: &[WorkerSeat]) -> Result<Picked, String> {
    let verifier = first_of(seats, defaults, roles::VERIFIER).ok_or("no verifier seat: the round can't be verified")?;
    let reviewer = first_of(seats, defaults, roles::REVIEWER).ok_or("no reviewer seat: the round can't be reviewed")?;
    let security = match security {
        false => None,
        true => Some(security_reviewer(seats, defaults, writers).ok_or("no security reviewer seat: the security-flagged round can't be reviewed")?),
    };
    Ok(Picked { verifier: verifier.clone(), reviewer: reviewer.clone(), security })
}

// ---------------------------------------------------------------- the gate

/// What the gate needs from the run, beyond the round.
pub struct GateConfig {
    pub project: String,
    pub repo: PathBuf,
    pub seats: Vec<WorkerSeat>,
    /// The role defaults the app resolved, for a role with no seat on the proposal.
    pub defaults: Vec<WorkerSeat>,
    pub redact: Redactor,
}

/// The core's round gate over real check tickets.
pub struct CoreGate {
    cfg: GateConfig,
    checks: Arc<dyn Checks>,
}

/// One check of the round, as it runs.
struct Job {
    kind: CheckKind,
    seat: WorkerSeat,
    brief: String,
    lens: Option<&'static str>,
    shared: Option<bool>,
    /// The attempts so far, and the ticket of each.
    tickets: Vec<String>,
    /// The last attempt's final message, or why it has none.
    answer: Result<String, String>,
    stopped: bool,
}

enum Parsed {
    Verify(Verification),
    Review(Review),
}

impl CoreGate {
    pub fn new(cfg: GateConfig, checks: Arc<dyn Checks>) -> Self {
        CoreGate { cfg, checks }
    }

    fn spec(&self, round: &RoundInput, brief: &str) -> TaskSpec {
        TaskSpec {
            project: self.cfg.project.clone(), run: round.run.clone(), repo: self.cfg.repo.clone(), task: String::new(), round: round.round,
            from: round.head.clone(), deps: Vec::new(), globs: Vec::new(), named: Vec::new(), brief: brief.to_string(),
        }
    }

    /// Starts the next attempt of every job in `which`, all before any wait (they run in parallel), then waits for each.
    fn attempt(&self, round: &RoundInput, jobs: &mut [Job], which: &[usize]) {
        let mut started = Vec::new();
        for &i in which {
            let j = &mut jobs[i];
            let n = j.tickets.len() as u32;
            match self.checks.start(j.kind, n, self.spec(round, &j.brief), j.seat.clone(), &round.head) {
                Ok(t) => {
                    j.tickets.push(t.clone());
                    started.push((i, t));
                }
                Err(e) => {
                    j.tickets.push(String::new());
                    j.answer = Err(format!("the check could not start ({e})"));
                }
            }
        }
        for (i, t) in started {
            let j = &mut jobs[i];
            j.answer = match self.checks.wait(&t) {
                None => Err("the check ticket is gone".into()),
                Some(end) if end.outcome == Some(Outcome::Done) => self.checks.answer(&t).ok_or_else(|| "the check gave no final message".into()),
                Some(end) => {
                    j.stopped = end.outcome == Some(Outcome::Stopped);
                    let o = end.outcome.map_or("no outcome".into(), |o| format!("{o:?}").to_lowercase());
                    Err(format!("the check ended {o}{}", end.reason.map(|r| format!(" ({r})")).unwrap_or_default()))
                }
            };
        }
    }

    fn parse(&self, j: &Job) -> Result<Parsed, String> {
        let msg = j.answer.as_ref().map_err(Clone::clone)?;
        match j.kind {
            CheckKind::Verify => verify::parse_verification(msg, &self.cfg.redact).map(Parsed::Verify),
            _ => verify::parse_review(msg, &self.cfg.redact).map(Parsed::Review),
        }
    }

    /// Who ran a check, for the round result, the card and the report.
    fn record(j: &Job, error: Option<&str>) -> Value {
        let mut v = json!({
            "kind": j.kind, "ticket": j.tickets.last(), "tickets": j.tickets, "slot": j.seat.slot, "role": j.seat.role,
            "harness": harness(&j.seat), "account": j.seat.account, "model": j.seat.model, "source": j.seat.source,
            "brief": if j.kind == CheckKind::Verify { VERIFY_BRIEF } else { REVIEW_BRIEF }, "valid": error.is_none(),
        });
        if let Some(l) = j.lens {
            v["lens"] = json!(l);
        }
        if let Some(s) = j.shared {
            v["shared_with_writer"] = json!(s);
        }
        if let Some(e) = error {
            v["error"] = json!(one_line(e, 300));
        }
        v
    }

    fn checked(&self, round: &RoundInput) -> RoundResult {
        let picked = match pick(&self.cfg.seats, &self.cfg.defaults, round.security, &round.security_writers) {
            Ok(p) => p,
            Err(why) => return RoundResult { end: Some(why), ..RoundResult::default() },
        };
        let job = |kind, seat: WorkerSeat, brief, lens, shared| Job { kind, seat, brief, lens, shared, tickets: Vec::new(), answer: Err(String::new()), stopped: false };
        let review = |lens| review_brief(&round.base, &round.head, lens, &round.criteria, &round.tasks);
        let mut jobs = vec![
            job(CheckKind::Verify, picked.verifier, verify_brief(&round.head, &round.criteria, None), None, None),
            job(CheckKind::Review, picked.reviewer, review(DESIGN_LENS), Some(DESIGN_LENS), None),
        ];
        if let Some((seat, shared)) = picked.security {
            jobs.push(job(CheckKind::SecurityReview, seat, review(SECURITY_LENS), Some(SECURITY_LENS), Some(shared)));
        }
        let all: Vec<usize> = (0..jobs.len()).collect();
        self.attempt(round, &mut jobs, &all);
        // A ticket with no valid result gets one retry (16.5a), unless the run was stopped under it.
        let invalid: Vec<usize> = all.iter().copied().filter(|&i| !jobs[i].stopped && self.parse(&jobs[i]).is_err()).collect();
        if !jobs.iter().any(|j| j.stopped) {
            self.attempt(round, &mut jobs, &invalid);
        }
        let parsed: Vec<Result<Parsed, String>> = jobs.iter().map(|j| self.parse(j)).collect();
        let mut checks: Vec<Value> = jobs.iter().zip(&parsed).map(|(j, p)| Self::record(j, p.as_ref().err().map(String::as_str))).collect();
        let result = if let Some(i) = parsed.iter().position(Result::is_err) {
            let why = format!("the {} check gave no valid result: {}", jobs[i].kind_str(), parsed[i].as_ref().err().map_or("", String::as_str));
            let note = json!({"checks_failed": one_line(&why, 300)});
            RoundResult { feedback: round.tasks.iter().map(|t| (t.id.clone(), note.clone())).collect(), ..RoundResult::default() }
        } else {
            let mut it = parsed.into_iter().map(Result::ok).map(Option::unwrap);
            let Some(Parsed::Verify(mut v)) = it.next() else { unreachable!("the first job is the verify") };
            let reviews: Vec<Review> = it.filter_map(|p| if let Parsed::Review(r) = p { Some(r) } else { None }).collect();
            let mut problems = verify::verification_problems(&v, &round.criteria);
            if problems.any() {
                // One redo, told exactly which commands can't be re-run as written (`:680`); its failure keeps the first.
                let fb = verify::redo_feedback(&v, &problems, &round.criteria);
                jobs[0].brief = verify_brief(&round.head, &round.criteria, Some(&fb));
                self.attempt(round, &mut jobs, &[0]);
                let redo = self.parse(&jobs[0]);
                let mut rec = Self::record(&jobs[0], redo.as_ref().err().map(String::as_str));
                rec["redo"] = json!(true);
                if let Ok(Parsed::Verify(v2)) = redo {
                    v = v2;
                    problems = verify::verification_problems(&v, &round.criteria);
                }
                checks[0]["redo"] = rec;
            }
            if verify::rejected(&reviews) {
                RoundResult { rejected: true, ..RoundResult::default() }
            } else {
                let g = verify::gate(&v, &problems, &round.criteria, &round.tasks, &reviews);
                let blocking = g.blocking.iter().map(|b| json!(b)).collect();
                RoundResult { passed: g.passed, failing: g.failing, blocking, feedback: g.feedback, ..RoundResult::default() }
            }
        };
        for t in jobs.iter().flat_map(|j| &j.tickets).filter(|t| !t.is_empty()) {
            self.checks.discard(t);
        }
        RoundResult { checks, ..result }
    }
}

impl Job {
    fn kind_str(&self) -> &'static str {
        match self.kind {
            CheckKind::Verify => "verify",
            CheckKind::Review => "design review",
            CheckKind::SecurityReview => "security review",
            CheckKind::Evidence => "evidence",
        }
    }
}

impl RoundGate for CoreGate {
    fn check(&self, round: &RoundInput) -> RoundVerdict {
        RoundVerdict::Checked(self.checked(round))
    }
}

#[cfg(test)]
#[path = "phases_tests.rs"]
mod tests;
