//! The deliver workflow's verify and review rules, ported (ADR-16 16.5a, PR 6g): the `Verification` and `Review`
//! shapes, `resultsByCriterion`, `commandProblem`, `setupOf`, `verificationProblems`, the redo feedback, the gate, the
//! per-task feedback and the `SECURITY` rule, from `workflows/vajra-deliver.js`. A straight port: the same order of
//! checks and the same messages. There is no regex crate, so each JS pattern is a small scan, written next to the
//! pattern it replaces (both are ASCII patterns over `oneLine` text, whose white space is single spaces).
//!
//! One deliberate difference: the workflow looks blocking issues' tasks up in a plain object, so a task named
//! `constructor` or `toString` counts as known. Here only the plan's task ids do.
//!
//! The shared cases in `tests/fixtures/verify/cases.json` are run by both this port and the JS itself; the file pins
//! the hash of the ported JS blocks, which is `GATE_VERSION`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::plan::{self, Criterion, PlanTask, js_space, one_line};
use crate::control::lead::sanitise;
use crate::crew::redact::Redactor;
use crate::crew::security;

/// sha256 of the ported JS blocks (the report's `gate_version`, 16.5a); a test keeps it equal to the fixtures' pin.
pub const GATE_VERSION: &str = "c6cddbfda163c57d059a0b9f60938c021ac2265536c96cd4e639271a89afea80";
/// The longest last message a check may answer with (one bus line).
pub const MAX_MESSAGE: usize = 64 * 1024;
/// The longest text field in a result.
pub const MAX_TEXT: usize = 4000;
/// The most entries in any list of a result.
pub const MAX_ITEMS: usize = 32;

// ---------------------------------------------------------------- the shapes (`S.Verification`, `S.Review`)

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Fail,
    NotRun,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VResult {
    pub criterion_id: String,
    pub status: Status,
    pub command: String,
    pub notes: String,
    pub exit_code: i64,
    pub evidence: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub results: Vec<VResult>,
    pub suite_command: String,
    pub suite_passes: bool,
    pub regressions: Vec<String>,
    pub setup_cmds: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Approve,
    Revise,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blocking {
    pub task: String,
    pub issue: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub verdict: Verdict,
    pub blocking_issues: Vec<Blocking>,
    pub risks: Vec<String>,
    pub rationale: String,
}

// ---------------------------------------------------------------- parsing a check's last message

/// The message alone, or the body of one code fence that is the whole message.
fn unfence(msg: &str) -> Result<&str, String> {
    if msg.len() > MAX_MESSAGE {
        return Err(format!("the answer is over {MAX_MESSAGE} bytes"));
    }
    let t = msg.trim();
    let Some(rest) = t.strip_prefix("```") else { return Ok(t) };
    let (lang, body) = rest.split_once('\n').ok_or("an unclosed code fence")?;
    if !matches!(lang.trim(), "" | "json") {
        return Err("a code fence that is not json".into());
    }
    body.trim_end().strip_suffix("```").map(str::trim).ok_or_else(|| "an unclosed code fence".into())
}

fn bounded(lists: &[usize], texts: &[&str]) -> Result<(), String> {
    if lists.iter().any(|n| *n > MAX_ITEMS) || texts.iter().any(|t| t.len() > MAX_TEXT) {
        return Err(format!("at most {MAX_ITEMS} entries per list and {MAX_TEXT} bytes per text"));
    }
    Ok(())
}

/// A verify ticket's answer: exactly the workflow's `Verification`, bounded; every text sanitised and redacted.
pub fn parse_verification(msg: &str, red: &Redactor) -> Result<Verification, String> {
    let mut v: Verification = serde_json::from_str(unfence(msg)?).map_err(|e| format!("not a Verification: {e}"))?;
    let mut texts: Vec<&str> = vec![&v.suite_command];
    texts.extend(v.regressions.iter().map(String::as_str));
    texts.extend(v.setup_cmds.iter().map(String::as_str));
    for r in &v.results {
        texts.extend([r.criterion_id.as_str(), &r.command, &r.notes, &r.evidence]);
    }
    bounded(&[v.results.len(), v.regressions.len(), v.setup_cmds.len()], &texts)?;
    let s = |t: &mut String| *t = sanitise(t, red);
    s(&mut v.suite_command);
    v.regressions.iter_mut().chain(v.setup_cmds.iter_mut()).for_each(s);
    for r in &mut v.results {
        [&mut r.criterion_id, &mut r.command, &mut r.notes, &mut r.evidence].into_iter().for_each(s);
    }
    Ok(v)
}

/// A review ticket's answer: exactly the workflow's `Review`, bounded; every text sanitised and redacted.
pub fn parse_review(msg: &str, red: &Redactor) -> Result<Review, String> {
    let mut r: Review = serde_json::from_str(unfence(msg)?).map_err(|e| format!("not a Review: {e}"))?;
    let mut texts: Vec<&str> = vec![&r.rationale];
    texts.extend(r.risks.iter().map(String::as_str));
    for b in &r.blocking_issues {
        texts.extend([b.task.as_str(), &b.issue]);
    }
    bounded(&[r.blocking_issues.len(), r.risks.len()], &texts)?;
    let s = |t: &mut String| *t = sanitise(t, red);
    s(&mut r.rationale);
    r.risks.iter_mut().for_each(s);
    for b in &mut r.blocking_issues {
        s(&mut b.task);
        s(&mut b.issue);
    }
    Ok(r)
}

// ---------------------------------------------------------------- `resultsByCriterion`

/// JS `\w` (no `u` flag): ASCII letters, digits and `_`.
fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `normText`: one line, lower case, every run of non-`[a-z0-9]` one space, trimmed.
fn norm_text(x: &str) -> String {
    let mut out = String::new();
    for c in one_line(x, 1000).to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim_matches(' ').to_string()
}

/// `/^(ac-\d{1,2})\b/`: the id that starts the text.
fn leading_id(lower: &str) -> Option<&str> {
    let digits = lower.strip_prefix("ac-")?.bytes().take_while(u8::is_ascii_digit).count();
    let end = 3 + digits;
    ((1..=2).contains(&digits) && !lower[end..].chars().next().is_some_and(word)).then(|| &lower[..end])
}

/// Each criterion id to the index of its first result. The verifier is told to answer with the id; an echoed
/// criterion text, or "ac-1: text", still reaches the gate.
pub fn results_by_criterion(results: &[VResult], criteria: &[Criterion]) -> BTreeMap<String, usize> {
    let has = |id: &str| criteria.iter().any(|c| c.id == id);
    let mut out = BTreeMap::new();
    for (i, r) in results.iter().enumerate() {
        let lower = one_line(&r.criterion_id, 1000).to_lowercase();
        let mut id = has(&lower).then(|| lower.clone());
        if id.is_none() {
            id = leading_id(&lower).filter(|m| has(m)).map(str::to_string);
        }
        if id.is_none() {
            let t = norm_text(&r.criterion_id);
            let exact: Vec<&Criterion> = criteria.iter().filter(|c| norm_text(&c.criterion) == t).collect();
            let loose: Vec<&Criterion> = if t.len() >= 8 {
                criteria.iter().filter(|c| {
                    let n = norm_text(&c.criterion);
                    n.contains(&t) || t.contains(&n)
                }).collect()
            } else {
                Vec::new()
            };
            id = match (exact.as_slice(), loose.as_slice()) {
                ([one], _) | (_, [one]) => Some(one.id.clone()),
                _ => None,
            };
        }
        if let Some(id) = id {
            out.entry(id).or_insert(i);
        }
    }
    out
}

fn by_criterion<'a>(results: &'a [VResult], criteria: &[Criterion]) -> BTreeMap<String, &'a VResult> {
    results_by_criterion(results, criteria).into_iter().map(|(id, i)| (id, &results[i])).collect()
}

// ---------------------------------------------------------------- `commandProblem`

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmdKind {
    Check,
    Setup,
}

const OUTSIDE: &str = "uses a path outside the repository (home, temp, scratchpad or an absolute venv/node_modules path); build the environment \
inside the worktree with setup_cmds and use repo-relative paths such as .venv/bin/python";
const MASKS: &str = "masks the exit code (echo $? or || true); the exit code itself is the result";
const BARE_PY: &str = "calls a bare python/pip/pytest, but each command runs in a fresh shell with no virtualenv activated; call the one \
setup_cmds built (.venv/bin/python)";

/// The byte offsets of `c` where a token may start: the start, or after a char of `[\s=:'"(]`.
fn token_starts(c: &str) -> impl Iterator<Item = usize> + '_ {
    let mut prev: Option<char> = None;
    c.char_indices().filter_map(move |(i, ch)| {
        let ok = prev.is_none_or(|p| js_space(p) || "=:'\"(".contains(p));
        prev = Some(ch);
        ok.then_some(i)
    })
}

/// `\b<w>` ending at `i`: the char before `i` is no word char.
fn word_start(c: &str, i: usize) -> bool {
    !c[..i].chars().next_back().is_some_and(word)
}

/// `<w>\b`: `rest` starts with `w` and the char after it is no word char.
fn word_then_end(rest: &str, w: &str) -> bool {
    rest.strip_prefix(w).is_some_and(|r| !r.chars().next().is_some_and(word))
}

/// `OUTSIDE_RE`: `(^|[\s=:'"(])(~(?=[/\s'"]|$)|\$\{?(?:HOME|TMPDIR)\b|\/(?:Users|home|root|tmp|private|var\/folders|var\/tmp)\/)`.
fn outside(c: &str) -> bool {
    token_starts(c).any(|i| {
        let rest = &c[i..];
        if let Some(r) = rest.strip_prefix('~') {
            return r.chars().next().is_none_or(|n| n == '/' || js_space(n) || n == '\'' || n == '"');
        }
        if let Some(r) = rest.strip_prefix('$') {
            let r = r.strip_prefix('{').unwrap_or(r);
            return word_then_end(r, "HOME") || word_then_end(r, "TMPDIR");
        }
        ["Users", "home", "root", "tmp", "private", "var/folders", "var/tmp"].iter().any(|d| rest.strip_prefix('/').and_then(|r| r.strip_prefix(d)).is_some_and(|r| r.starts_with('/')))
    })
}

/// `ENV_ABS_RE`: `(^|[\s=:'"(])\/[^\s'"]*\/(?:\.?venv|virtualenv|site-packages|node_modules)\/`.
fn env_abs(c: &str) -> bool {
    token_starts(c).any(|i| {
        let rest = &c[i..];
        if !rest.starts_with('/') {
            return false;
        }
        let token = &rest[..rest.find(|ch: char| js_space(ch) || ch == '\'' || ch == '"').unwrap_or(rest.len())];
        (1..token.len()).filter(|j| token.is_char_boundary(*j)).any(|j| {
            ["/.venv/", "/venv/", "/virtualenv/", "/site-packages/", "/node_modules/"].iter().any(|n| token[j..].starts_with(n))
        })
    })
}

/// `MAKES_VENV`: `-m\s+venv\b|\bvirtualenv\b|\buv\s+venv\b`.
fn makes_venv(s: &str) -> bool {
    let spaced_venv = |r: &str| {
        let t = r.trim_start_matches(js_space);
        t.len() < r.len() && word_then_end(t, "venv")
    };
    s.match_indices("-m").any(|(i, _)| spaced_venv(&s[i + 2..]))
        || s.match_indices("virtualenv").any(|(i, _)| word_start(s, i) && word_then_end(&s[i..], "virtualenv"))
        || s.match_indices("uv").any(|(i, _)| word_start(s, i) && spaced_venv(&s[i + 2..]))
}

/// `(^|[<set>]\s*)` before `i`: `i` is 0, or the text before it, less trailing spaces, ends with a char of `set`.
fn after_separator(c: &str, i: usize, set: &str) -> bool {
    i == 0 || c[..i].trim_end_matches(js_space).chars().next_back().is_some_and(|p| set.contains(p))
}

/// `/(^|[;&|]\s*)echo\s+[^;&|]*\$\?/` or `/\|\|\s*(true|:)\s*$/`.
fn masks_exit(c: &str) -> bool {
    let echo = c.match_indices("echo").any(|(i, _)| {
        let after = &c[i + 4..];
        let seg = &after[..after.find([';', '&', '|']).unwrap_or(after.len())];
        after_separator(c, i, ";&|") && after.starts_with(js_space) && seg.contains("$?")
    });
    let or_true = [c.strip_suffix("true"), c.strip_suffix(':')].into_iter().flatten().any(|s| s.trim_end_matches(js_space).ends_with("||"));
    echo || or_true
}

/// `/(^|[;&|(]\s*)(python|pip|pytest)(?=\s|$)/`.
fn bare_python(c: &str) -> bool {
    c.char_indices().any(|(i, _)| {
        after_separator(c, i, ";&|(")
            && ["python", "pip", "pytest"].iter().any(|w| c[i..].strip_prefix(w).is_some_and(|r| r.chars().next().is_none_or(js_space)))
    })
}

/// `/(^|[^\s&|;(])\s+\(([^()]*)\)\s*$/`: the text of a note in parentheses that ends the command.
fn trailing_note(c: &str) -> Option<&str> {
    let body = c.strip_suffix(')')?;
    let k = body.rfind('(')?;
    let inner = &body[k + 1..];
    let before = &body[..k];
    let kept = before.trim_end_matches(js_space);
    let ok = !inner.contains(')') && kept.len() < before.len() && kept.chars().next_back().is_some_and(|p| !"&|;(".contains(p));
    ok.then_some(inner)
}

/// What makes a verifier's command unfit to re-run as written, or `None`.
pub fn command_problem(cmd: &str, setup: &[String], kind: CmdKind) -> Option<String> {
    let c = one_line(cmd, 1000);
    if c.is_empty() {
        return Some("no command".into());
    }
    if let Some(note) = trailing_note(&c).filter(|n| !(n.contains("&&") || n.contains("||") || n.chars().any(|ch| ";|<>$`=".contains(ch)))) {
        return Some(format!("ends with a note in parentheses \"({})\"; the command must be only what ran, notes go in notes", one_line(note, 80)));
    }
    if c.contains("\"...") || c.contains("'...") || c.contains('…') {
        return Some("elides part of the command (...); give the literal command that ran".into());
    }
    if outside(&c) || env_abs(&c) {
        return Some(OUTSIDE.into());
    }
    if kind == CmdKind::Setup {
        return None;
    }
    if masks_exit(&c) {
        return Some(MASKS.into());
    }
    let activates = c.match_indices("activate").any(|(i, _)| word_start(&c, i) && word_then_end(&c[i..], "activate"));
    if setup.iter().any(|s| makes_venv(s)) && !activates && bare_python(&c) {
        return Some(BARE_PY.into());
    }
    None
}

/// `/^git\s+(checkout|switch)\b/`.
fn git_checkout(x: &str) -> bool {
    x.strip_prefix("git").is_some_and(|r| {
        let t = r.trim_start_matches(js_space);
        t.len() < r.len() && (word_then_end(t, "checkout") || word_then_end(t, "switch"))
    })
}

/// `setupOf`: one line each, no blanks or repeats; a git checkout or switch is dropped (the core checks out itself).
pub fn setup_of(cmds: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in cmds.iter().map(|c| one_line(c, 1000)) {
        if !c.is_empty() && !git_checkout(&c) && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------- `verificationProblems` and the redo

/// What can't be re-run as written: a passing criterion's command, the suite command, a setup command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Problems {
    /// (criterion id, problem), in criteria order.
    pub criteria: Vec<(String, String)>,
    pub suite: Option<String>,
    /// (command, problem).
    pub setup: Vec<(String, String)>,
}

impl Problems {
    pub fn any(&self) -> bool {
        !self.criteria.is_empty() || self.suite.is_some() || !self.setup.is_empty()
    }
    fn of(&self, id: &str) -> Option<&str> {
        self.criteria.iter().find(|(c, _)| c == id).map(|(_, p)| p.as_str())
    }
    fn setup_json(&self) -> Value {
        self.setup.iter().map(|(command, problem)| json!({"command": command, "problem": problem})).collect()
    }
    /// The workflow's `problems` object.
    pub fn to_json(&self) -> Value {
        let criteria: Map<String, Value> = self.criteria.iter().map(|(id, p)| (id.clone(), json!(p))).collect();
        json!({"criteria": criteria, "suite": self.suite, "setup": self.setup_json(), "any": self.any()})
    }
}

fn js_trim(s: &str) -> &str {
    s.trim_matches(js_space)
}

/// `verificationProblems`.
pub fn verification_problems(v: &Verification, criteria: &[Criterion]) -> Problems {
    let setup = setup_of(&v.setup_cmds);
    let by = by_criterion(&v.results, criteria);
    let mut out = Problems::default();
    for c in criteria {
        if let Some(p) = by.get(&c.id).filter(|r| r.status == Status::Pass).and_then(|r| command_problem(&r.command, &setup, CmdKind::Check)) {
            out.criteria.push((c.id.clone(), p));
        }
    }
    if !js_trim(&v.suite_command).is_empty() {
        out.suite = command_problem(&v.suite_command, &setup, CmdKind::Check);
    }
    for s in &setup {
        if let Some(p) = command_problem(s, &setup, CmdKind::Setup) {
            out.setup.push((s.clone(), p));
        }
    }
    out
}

/// What the one redo ticket is told: each command that can't be re-run, and why (`fb`, `:683`).
pub fn redo_feedback(v: &Verification, p: &Problems, criteria: &[Criterion]) -> Value {
    let by = by_criterion(&v.results, criteria);
    let crit: Vec<Value> = p.criteria.iter().map(|(id, problem)| {
        let command = by.get(id).map_or(String::new(), |r| one_line(&r.command, 300));
        json!({"criterion_id": id, "command": command, "problem": problem})
    }).collect();
    let suite = p.suite.as_ref().map(|problem| json!({"command": one_line(&v.suite_command, 300), "problem": problem}));
    json!({"criteria": crit, "suite": suite, "setup_cmds": p.setup_json()})
}

// ---------------------------------------------------------------- the gate and the feedback

/// The gate's answer for one round.
#[derive(Clone, Debug, PartialEq)]
pub struct Gate {
    pub passed: bool,
    /// Criterion ids that did not pass, in criteria order.
    pub failing: Vec<String>,
    pub suite_ok: bool,
    pub blocking: Vec<Blocking>,
    /// Per task id: `{failing_criteria, suite_command_problem, blocking_issues, suite_regressions}` lists.
    pub feedback: BTreeMap<String, Value>,
}

/// A review rejected the approach: the run ends `rejected` (`:697`).
pub fn rejected(reviews: &[Review]) -> bool {
    reviews.iter().any(|r| r.verdict == Verdict::Reject)
}

/// The gate (`:704–710`) and the per-task feedback (`:712–728`).
pub fn gate(v: &Verification, problems: &Problems, criteria: &[Criterion], tasks: &[PlanTask], reviews: &[Review]) -> Gate {
    let by = by_criterion(&v.results, criteria);
    let suite_ok = (js_trim(&v.suite_command).is_empty() || v.suite_passes) && problems.suite.is_none();
    let passing = |c: &Criterion| {
        by.get(&c.id).is_some_and(|r| {
            r.status == Status::Pass && r.exit_code == 0 && !js_trim(&r.command).is_empty() && !js_trim(&r.evidence).is_empty() && problems.of(&c.id).is_none()
        })
    };
    let failing: Vec<&Criterion> = criteria.iter().filter(|c| !passing(c)).collect();
    let blocking: Vec<Blocking> = reviews.iter().flat_map(|r| r.blocking_issues.iter().cloned()).collect();
    let ids = failing.iter().map(|c| c.id.clone()).collect();
    if failing.is_empty() && suite_ok && reviews.iter().all(|r| r.verdict == Verdict::Approve) && blocking.is_empty() {
        return Gate { passed: true, failing: ids, suite_ok, blocking, feedback: BTreeMap::new() };
    }
    let mut fb: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    fn add(fb: &mut BTreeMap<String, Map<String, Value>>, id: &str, key: &str, value: Value) {
        let list = fb.entry(id.to_string()).or_default().entry(key).or_insert_with(|| json!([]));
        if let Value::Array(a) = list {
            a.push(value);
        }
    }
    for c in &failing {
        for t in tasks.iter().filter(|t| t.criteria.contains(&c.id)) {
            let mut item = json!({"criterion": c, "result": by.get(&c.id).map_or(json!("not checked"), |r| json!(r))});
            if let Some(p) = problems.of(&c.id) {
                item["verifier_command_problem"] = json!(format!("{p} (the check could not be re-run as written; make sure the criterion has a runnable check)"));
            }
            add(&mut fb, &t.id, "failing_criteria", item);
        }
    }
    let order = plan::topo(tasks);
    if let Some(p) = &problems.suite {
        if fb.is_empty() {
            for id in &order {
                add(&mut fb, id, "suite_command_problem", json!(p));
            }
        }
    }
    let mut unassigned = Vec::new();
    for b in &blocking {
        if tasks.iter().any(|t| t.id == b.task) {
            add(&mut fb, &b.task, "blocking_issues", json!(b.issue));
        } else {
            unassigned.push(b.issue.clone());
        }
    }
    if !unassigned.is_empty() || (!suite_ok && fb.is_empty()) {
        for id in &order {
            for i in &unassigned {
                add(&mut fb, id, "blocking_issues", json!(i));
            }
            if !suite_ok {
                add(&mut fb, id, "suite_regressions", json!(v.regressions));
            }
        }
    }
    let feedback = fb.into_iter().map(|(k, m)| (k, Value::Object(m))).collect();
    Gate { passed: false, failing: ids, suite_ok, blocking, feedback }
}

/// `SECURITY` (`:444`): the lead's flag, or `SEC_RE` over the ask or any criterion.
pub fn security_flag<'a>(ask: &str, mut criteria: impl Iterator<Item = &'a str>, touches_security: bool) -> bool {
    touches_security || security::matches(ask) || criteria.any(security::matches)
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
