//! S4c: the Run view's timeline (AC-4.1.x) and task graph (AC-4.2.x), derived read-only from what the campaign already
//! wrote: `progress.md` (the lines `vajra-deliver.js` appends) and `tasks.json`. Nothing here runs a process.
//!
//! The workflow writes these `run <id> ...` progress lines (workflows/vajra-deliver.js):
//! - `run <id> round <n>: <k>/<m> criteria passing; ...`   a verification round finished
//! - `run <id> round <n>: waiting for the user on ...`     stopped, needs input
//! - `run <id> round <n>: review rejected the approach`    rejected
//! - `run <id>: no verified delivery after <r> round(s); ...`  gave up
//! - `run <id> delivered <branch> @ <sha>`                 done

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::crew::names::ui_name;

/// The phases in order. Fix is shown with its round when one is known.
pub const PHASES: [&str; 6] = ["Understand", "Plan", "Execute", "Verify", "Fix", "Deliver"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Still running (or at least not finished by any recorded line).
    Active,
    Done,
    NeedInput,
    Rejected,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Done => "done",
            Self::NeedInput => "need_input",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Timeline {
    /// Index into `PHASES` of the one current phase.
    pub current: usize,
    pub outcome: Outcome,
    /// The round the Fix phase is (or was last) in, and the round limit when a line named it.
    pub fix_round: Option<u32>,
    pub max_rounds: Option<u32>,
}

#[derive(Default)]
struct Lines {
    /// The last verification line: (round, passing, total).
    verified: Option<(u32, u32, u32)>,
    /// Highest round named by any `round N` line.
    round: u32,
    outcome: Option<(Outcome, u32)>,
    /// The run id of the latest progress line. A line from a different run (a resume registers a fresh id) starts the
    /// campaign's state over: nothing a previous run recorded (rounds, limit, outcome) carries into it.
    cur_run: String,
    max_rounds: Option<u32>,
    /// The stop is `no verified delivery after N round(s)` (node fix_loop), not a mid-round wait for the user.
    exhausted: bool,
}

impl Lines {
    fn enter(&mut self, run: &str) {
        if self.cur_run != run {
            *self = Lines { cur_run: run.to_string(), ..Lines::default() };
        }
    }
}

/// The label `vajra-deliver.js` puts before each progress line of an outside-guard campaign (`OG_MARK`).
const OUTSIDE_GUARD_MARK: &str = "[evidence ran outside the Vajra guard]";

fn scan(progress: &str) -> Lines {
    let mut out = Lines::default();
    for line in progress.lines() {
        // Only the shape `append_progress` writes: `- <timestamp> <text>`, with the text starting `run <id>` (what the workflow
        // writes) or `resumed by vajra-deliver run <id>`. Free text that merely contains "run " (a note, a reject reason) is not read.
        let Some(text) = line.strip_prefix("- ").and_then(|l| l.split_once(' ')).map(|x| x.1) else { continue };
        // A campaign whose evidence ran outside the Vajra guard has every line labelled first (`og()` in the workflow).
        let text = text.strip_prefix(OUTSIDE_GUARD_MARK).map_or(text, str::trim_start);
        if let Some(r) = text.strip_prefix("resumed by vajra-deliver run ") {
            // A resume is a new run: all run-scoped state starts over, even if the id were repeated.
            let id = r.split(|c: char| c.is_whitespace() || c == ':').next().unwrap_or("");
            out = Lines { cur_run: id.to_string(), ..Lines::default() };
            continue;
        }
        let Some(rest) = text.strip_prefix("run ") else { continue };
        // `<id> ...` or `<id>: ...`: the run id is one word of id characters.
        let Some((id, after)) = rest.split_once(|c: char| c.is_whitespace() || c == ':') else { continue };
        let id_ok = id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !id_ok {
            continue;
        }
        let after = after.trim_start_matches(|c: char| c.is_whitespace() || c == ':');
        out.enter(id);
        if let Some(r) = after.strip_prefix("round ") {
            let n: u32 = r.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0);
            out.round = out.round.max(n);
            let body = r.split_once(':').map(|x| x.1.trim_start()).unwrap_or("");
            if body.starts_with("waiting for the user") {
                out.outcome = Some((Outcome::NeedInput, n));
                out.exhausted = false;
            } else if body.starts_with("review rejected") {
                out.outcome = Some((Outcome::Rejected, n));
                out.exhausted = false;
            } else if let Some((k, m)) = body.split_once(" criteria passing").and_then(|(h, _)| h.split_once('/')) {
                if let (Ok(k), Ok(m)) = (k.trim().parse::<u32>(), m.trim().parse::<u32>()) {
                    // The workflow writes this line only when the round's gate FAILED (criteria, suite, review or blocking
                    // issues): a round that passes everything breaks out before writing it. So even `3/3` means rework.
                    out.verified = Some((n, k, m));
                    out.outcome = None; // a finished round supersedes an earlier stop
                    out.exhausted = false;
                }
            }
        } else if let Some(r) = after.strip_prefix("no verified delivery after ") {
            let n: u32 = r.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0);
            out.max_rounds = Some(n);
            out.round = out.round.max(n);
            // The workflow writes this line right before returning `{status: 'need_input', node: 'fix_loop'}`: the user can
            // continue, take over or drop the campaign. It is a stop that needs the user, at the Fix phase, not an error.
            out.outcome = Some((Outcome::NeedInput, n));
            out.exhausted = true;
        } else if after.starts_with("delivered ") && after.contains(" @ ") {
            out.outcome = Some((Outcome::Done, out.round));
        }
    }
    out
}

fn run_of(t: &Value) -> &str {
    t.get("id").and_then(Value::as_str).and_then(|id| id.split_once('.').map(|x| x.0)).unwrap_or("")
}

/// The run the campaign is in now and its tasks. `tasks.json` accumulates every run's tasks (`<run>.tN`; a resume adds a
/// fresh set), so the current run is the latest one named by a progress line, else the run of the newest task.
pub fn scope(progress: &str, tasks: Option<&[Value]>) -> (String, Option<Vec<Value>>) {
    let cur = scan(progress).cur_run;
    let Some(ts) = tasks else { return (cur, None) };
    let run = if cur.is_empty() { ts.iter().rev().map(run_of).find(|r| !r.is_empty()).unwrap_or("").to_string() } else { cur };
    let scoped = if run.is_empty() { ts.to_vec() } else { ts.iter().filter(|t| run_of(t) == run).cloned().collect() };
    (run, Some(scoped))
}

/// Where the run is. `tasks` is `tasks.json`'s task list (`None` when the file is absent), `progress` is `progress.md`.
/// Only the current run's tasks and lines count. Exactly one phase is current, always.
pub fn timeline(progress: &str, tasks: Option<&[Value]>) -> Timeline {
    let l = scan(progress);
    let (_, scoped) = scope(progress, tasks);
    let tasks = scoped.as_deref();
    // A worker that finishes runs `report <id> done`, which keeps the task `in_progress` (only the Deliver write-back closes it),
    // so "finished" is closed OR reported done. Execute lasts until every task is finished; then the verifier and reviewers run.
    let open = tasks.is_some_and(|t| t.iter().any(|x| !finished(x)));
    let have_tasks = tasks.is_some_and(|t| !t.is_empty());
    // A recorded round line means that round's gate failed, so the next round is the Fix round.
    let next_fix = l.verified.map(|(n, _, _)| n + 1);
    let (current, outcome, fix_round) = match l.outcome {
        Some((Outcome::Done, _)) => (5, Outcome::Done, next_fix),
        Some((Outcome::Rejected, _)) => (3, Outcome::Rejected, None),
        Some((Outcome::NeedInput, n)) if l.exhausted => (4, Outcome::NeedInput, Some(n.max(1))),
        Some((Outcome::NeedInput, _)) => (2, Outcome::NeedInput, None),
        _ => match (tasks, have_tasks, open, next_fix) {
            (None, _, _, _) => (0, Outcome::Active, None),
            (Some(_), false, _, _) => (1, Outcome::Active, None),
            (_, true, _, Some(n)) => (4, Outcome::Active, Some(n)),
            (_, true, true, None) => (2, Outcome::Active, None),
            // Every task finished (closed or reported done) and no failed round: verifying (and, once the gate passes, delivering: the workflow records
            // nothing between the gate and the `delivered` line, so that stretch reads as Verify).
            (_, true, false, None) => (3, Outcome::Active, None),
        },
    };
    Timeline { current, outcome, fix_round, max_rounds: l.max_rounds }
}

pub fn timeline_json(t: &Timeline) -> Value {
    let phases: Vec<Value> = PHASES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let state = match i.cmp(&t.current) {
                std::cmp::Ordering::Less => "done",
                std::cmp::Ordering::Equal => "current",
                std::cmp::Ordering::Greater => "pending",
            };
            let mut p = json!({"name": name, "state": state});
            if *name == "Fix" {
                p["round"] = json!(t.fix_round);
                p["max_rounds"] = json!(t.max_rounds);
            }
            p
        })
        .collect();
    json!({"phases": phases, "current": PHASES[t.current], "status": t.outcome.as_str()})
}

fn finished(t: &Value) -> bool {
    matches!(status(t), "done" | "closed") || (status(t) == "in_progress" && t.get("report").and_then(|r| r.get("status")).and_then(Value::as_str) == Some("done"))
}

fn status(t: &Value) -> &str {
    t.get("status").and_then(Value::as_str).unwrap_or("")
}

/// The task graph: one node per task with its owner's role name, status, dependencies and wave (0 = no dependencies,
/// otherwise one more than its deepest dependency). `run_id` strips the `<run>.` prefix to the id the crew tree uses.
/// Unknown dependencies are listed but do not place a task; a cycle cannot loop (waves stop growing at the task count).
pub fn graph(run_id: &str, tasks: &[Value]) -> Value {
    let ids: Vec<&str> = tasks.iter().filter_map(|t| t.get("id").and_then(Value::as_str)).collect();
    let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let deps: Vec<Vec<usize>> = tasks
        .iter()
        .map(|t| {
            t.get("deps")
                .and_then(Value::as_array)
                .map(|d| d.iter().filter_map(Value::as_str).filter_map(|d| index.get(d).copied()).collect())
                .unwrap_or_default()
        })
        .collect();
    let n = ids.len();
    let mut wave = vec![0usize; n];
    for _ in 0..n {
        let mut changed = false;
        for i in 0..n {
            let w = deps[i].iter().map(|&d| wave[d] + 1).max().unwrap_or(0).min(n);
            if w > wave[i] {
                wave[i] = w;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let prefix = format!("{run_id}.");
    let mut nodes: Vec<Value> = tasks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let id = t.get("id").and_then(Value::as_str)?;
            let owner = t.get("owner_agent").and_then(Value::as_str).map(ui_name);
            let all_deps: Vec<&str> = t.get("deps").and_then(Value::as_array).map(|d| d.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            Some(json!({
                "id": id,
                "local": id.strip_prefix(&prefix).unwrap_or(id),
                "title": t.get("title"),
                "owner": owner,
                "status": status(t),
                "deps": all_deps,
                "wave": wave[i],
            }))
        })
        .collect();
    nodes.sort_by(|a, b| (a["wave"].as_u64(), a["id"].as_str()).cmp(&(b["wave"].as_u64(), b["id"].as_str())));
    let waves = wave.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    json!({"tasks": nodes, "waves": waves})
}

#[cfg(test)]
mod tests {
    use super::*;

    /// progress.md lines as `append_progress` writes them: `- <timestamp> <text>`.
    fn stamp(p: &str) -> String {
        p.lines().map(|l| format!("- 2026-11-07T00:00:00Z {l}\n")).collect()
    }
    fn timeline(p: &str, tasks: Option<&[Value]>) -> Timeline {
        super::timeline(&stamp(p), tasks)
    }

    fn task(id: &str, st: &str, deps: &[&str]) -> Value {
        json!({"id": id, "title": format!("T {id}"), "owner_agent": "bhima", "status": st, "deps": deps})
    }
    fn cur(p: &str, tasks: Option<&[Value]>) -> (&'static str, Outcome) {
        let t = timeline(p, tasks);
        (PHASES[t.current], t.outcome)
    }

    #[test]
    fn a_new_campaign_is_understanding_then_planning_then_executing() {
        assert_eq!(cur("", None), ("Understand", Outcome::Active));
        assert_eq!(cur("", Some(&[])), ("Plan", Outcome::Active));
        let t = [task("r.t1", "in_progress", &[])];
        assert_eq!(cur("", Some(&t)), ("Execute", Outcome::Active));
    }

    #[test]
    fn closed_tasks_are_verified_then_a_failing_round_means_fix_round_n_plus_one() {
        let done = [task("r.t1", "closed", &[]), task("r.t2", "closed", &["r.t1"])];
        assert_eq!(cur("", Some(&done)), ("Verify", Outcome::Active));
        let p = "run r round 1: 2/3 criteria passing; integration b @ x from y\n";
        let t = timeline(p, Some(&done));
        assert_eq!((PHASES[t.current], t.fix_round), ("Fix", Some(2)));
        // Fixing reopens a task: still Fix round 2, not Execute.
        let reopened = [task("r.t1", "closed", &[]), task("r.t2", "in_progress", &["r.t1"])];
        let t = timeline(p, Some(&reopened));
        assert_eq!((PHASES[t.current], t.fix_round), ("Fix", Some(2)));
        // The line is written only when the gate FAILED, so even `3/3` (the suite failed, a review asked for changes, or
        // blocking issues remain) is rework: Fix round 2, never Deliver.
        let all = "run r round 1: 3/3 criteria passing; x\n";
        assert_eq!(cur(all, Some(&done)), ("Fix", Outcome::Active));
        // A successful round writes no such line: closed tasks with no failed round read as Verify.
        assert_eq!(cur("", Some(&done)), ("Verify", Outcome::Active));
    }

    #[test]
    fn tasks_that_reported_done_are_verifying_even_though_their_status_stays_in_progress() {
        let mut reported = task("r.t1", "in_progress", &[]);
        reported["report"] = json!({"status": "done", "summary": "built"});
        let mut working = task("r.t2", "in_progress", &["r.t1"]);
        working["report"] = json!({"status": "progress", "summary": "part"});
        // All reported done: the verifier and reviewers are running.
        assert_eq!(cur("", Some(&[reported.clone()])), ("Verify", Outcome::Active));
        // One still working: Execute.
        assert_eq!(cur("", Some(&[reported.clone(), working])), ("Execute", Outcome::Active));
        // A task with no report yet, or one that failed, is not finished.
        assert_eq!(cur("", Some(&[reported.clone(), task("r.t3", "in_progress", &[])])), ("Execute", Outcome::Active));
        assert_eq!(cur("", Some(&[reported, task("r.t3", "failed", &[])])), ("Execute", Outcome::Active));
    }

    #[test]
    fn delivery_after_a_fix_shows_the_round_that_delivered() {
        let t = [task("r.t1", "closed", &[])];
        let p = "run r round 1: 3/3 criteria passing; the suite failed\nrun r delivered b @ abc\n";
        let tl = timeline(p, Some(&t));
        assert_eq!((PHASES[tl.current], tl.outcome, tl.fix_round), ("Deliver", Outcome::Done, Some(2)));
    }

    #[test]
    fn a_resume_after_an_exhausted_run_starts_from_scratch_before_any_new_verification_line() {
        // r failed rounds 1..3 and stopped; r2 is resumed and has planned its tasks, nothing verified yet.
        let tasks = [task("r.t1", "closed", &[]), task("r2.t1", "in_progress", &[])];
        let p = "run r round 1: 1/3 criteria passing\nrun r round 3: 1/3 criteria passing\nrun r: no verified delivery after 3 round(s)\nresumed by vajra-deliver run r2\n";
        let tl = timeline(p, Some(&tasks));
        assert_eq!((PHASES[tl.current], tl.outcome, tl.fix_round, tl.max_rounds), ("Execute", Outcome::Active, None, None));
        // Same when the new run's first line is a run line rather than the resume marker.
        let p = "run r round 3: 1/3 criteria passing\nrun r: no verified delivery after 3 round(s)\nrun r2 round 1: waiting for the user on t1\n";
        let tl = timeline(p, Some(&tasks));
        assert_eq!((tl.outcome, tl.fix_round, tl.max_rounds), (Outcome::NeedInput, None, None));
        // Resumed but not yet planned: Plan, with the old run's tasks not counted.
        let tl = timeline("run r: no verified delivery after 3 round(s)\nresumed by vajra-deliver run r2\n", Some(&[task("r.t1", "closed", &[])]));
        assert_eq!(PHASES[tl.current], "Plan");
    }

    #[test]
    fn a_campaign_with_two_runs_scopes_to_the_current_one() {
        let tasks = vec![task("r1.t1", "closed", &[]), task("r2.t1", "in_progress", &[]), task("r2.t2", "pending", &["r2.t1"])];
        let (run, scoped) = scope("run r1 round 1: 1/1 criteria passing\nresumed by vajra-deliver run r2\n", Some(&tasks));
        assert_eq!(run, "r2");
        let g = graph(&run, scoped.as_deref().unwrap());
        let ids: Vec<&str> = g["tasks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["r2.t1", "r2.t2"]);
        assert_eq!(g["tasks"][0]["local"], "t1");
        // No progress lines yet: the run of the newest task.
        assert_eq!(scope("", Some(&tasks)).0, "r2");
    }

    #[test]
    fn the_four_ways_a_run_ends_each_show_status_and_the_node_reached() {
        let t = [task("r.t1", "closed", &[])];
        assert_eq!(cur("run r round 1: 3/3 criteria passing\nrun r delivered b @ abc\n", Some(&t)), ("Deliver", Outcome::Done));
        assert_eq!(cur("run r round 1: waiting for the user on t1; x\n", Some(&t)), ("Execute", Outcome::NeedInput));
        assert_eq!(cur("run r round 1: review rejected the approach\n", Some(&t)), ("Verify", Outcome::Rejected));
        let t = timeline("run r round 1: 1/3 criteria passing\nrun r: no verified delivery after 3 round(s); x\n", Some(&t));
        assert_eq!((PHASES[t.current], t.outcome, t.fix_round, t.max_rounds), ("Fix", Outcome::NeedInput, Some(3), Some(3)));
    }

    #[test]
    fn a_resume_after_a_stop_starts_again() {
        let t = [task("r.t1", "in_progress", &[]), task("r2.t1", "in_progress", &[])];
        let p = "run r round 1: waiting for the user on t1\nrun r2 round 1: 0/3 criteria passing\n";
        assert_eq!(cur(p, Some(&t)), ("Fix", Outcome::Active), "the stop no longer applies once a later run writes lines");
    }

    #[test]
    fn free_text_containing_run_does_not_change_the_outcome() {
        let t = [task("r.t1", "closed", &[])];
        let stop = "run r round 1: waiting for the user on t1";
        // A note or a reject reason typed by the user, appended after the stop.
        for note in ["please rerun t2 after the fix", "note: run x delivered a @ b", "rejected: run x: no verified delivery after 3 round(s)", "see run r2 round 1: 3/3 criteria passing"] {
            assert_eq!(cur(&format!("{stop}\n{note}"), Some(&t)), ("Execute", Outcome::NeedInput), "{note}");
        }
        // And a forged ending in free text is ignored with nothing before it.
        assert_eq!(cur("note: run x delivered a @ b", Some(&t)).1, Outcome::Active);
        // Lines without the `- <timestamp> ` prefix are not progress lines at all.
        let raw = super::timeline("run r delivered b @ c\n", Some(&t));
        assert_eq!(raw.outcome, Outcome::Active);
    }

    #[test]
    fn exactly_one_phase_is_current_whatever_the_input() {
        let open = [task("r.t1", "in_progress", &[])];
        let closed = [task("r.t1", "closed", &[])];
        let progress = [
            "", "garbage line", "run r round 2: 1/2 criteria passing", "run r delivered b @ c", "run r round x: 1/2 criteria passing",
            "run r round 1: review rejected the approach", "run : round 1: 1/1 criteria passing", "run r: no verified delivery after 0 round(s)",
        ];
        for p in progress {
            for tasks in [None, Some(&[][..]), Some(&open[..]), Some(&closed[..])] {
                let v = timeline_json(&timeline(p, tasks));
                let n = v["phases"].as_array().unwrap().iter().filter(|x| x["state"] == "current").count();
                assert_eq!(n, 1, "{p:?} {tasks:?}: {v}");
            }
        }
    }

    #[test]
    fn the_timeline_json_marks_done_before_current_and_pending_after_with_the_fix_round() {
        let done = [task("r.t1", "closed", &[])];
        let v = timeline_json(&timeline("run r round 1: 1/2 criteria passing\n", Some(&done)));
        let states: Vec<&str> = v["phases"].as_array().unwrap().iter().map(|p| p["state"].as_str().unwrap()).collect();
        assert_eq!(states, ["done", "done", "done", "done", "current", "pending"]);
        assert_eq!(v["phases"][4]["round"], 2);
    }

    #[test]
    fn the_graph_places_tasks_in_waves_by_their_dependencies_and_names_owners_as_roles() {
        let tasks = vec![
            task("r.t3", "pending", &["r.t1", "r.t2"]),
            task("r.t1", "closed", &[]),
            task("r.t2", "in_progress", &["r.t1"]),
            task("r.t4", "pending", &["r.t9"]), // unknown dependency: listed, does not place the task
        ];
        let g = graph("r", &tasks);
        assert_eq!(g["waves"], 3);
        let by: HashMap<&str, &Value> = g["tasks"].as_array().unwrap().iter().map(|t| (t["id"].as_str().unwrap(), t)).collect();
        assert_eq!(by["r.t1"]["wave"], 0);
        assert_eq!(by["r.t2"]["wave"], 1);
        assert_eq!(by["r.t3"]["wave"], 2);
        assert_eq!(by["r.t4"]["wave"], 0);
        assert_eq!(by["r.t4"]["deps"][0], "r.t9");
        assert_eq!(by["r.t1"]["local"], "t1", "the run prefix is stripped to the crew tree's id");
        assert_eq!(by["r.t1"]["owner"], "Backend");
        let order: Vec<&str> = g["tasks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(order, ["r.t1", "r.t4", "r.t2", "r.t3"], "sorted by wave, then id");
    }

    #[test]
    fn a_dependency_cycle_terminates() {
        let g = graph("r", &[task("r.a", "pending", &["r.b"]), task("r.b", "pending", &["r.a"])]);
        assert_eq!(g["tasks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn two_hundred_tasks_build_quickly() {
        let tasks: Vec<Value> = (0..200).map(|i| task(&format!("r.t{i}"), "pending", &if i == 0 { vec![] } else { vec![format!("r.t{}", i - 1).leak() as &str] })).collect();
        let start = std::time::Instant::now();
        let g = graph("r", &tasks);
        assert_eq!(g["tasks"].as_array().unwrap().len(), 200);
        assert_eq!(g["waves"], 200);
        assert!(start.elapsed() < std::time::Duration::from_millis(500), "{:?}", start.elapsed());
    }

    /// Contract with the writer: the progress lines are taken from `workflows/vajra-deliver.js` itself, so rewording one
    /// there breaks this test instead of silently blanking the timeline.
    mod contract {
        use super::*;

        const WORKFLOW: &str = include_str!("../../../../workflows/vajra-deliver.js");

        /// Every `sq(`run ${RUN}...`)` template in the workflow, also when wrapped in the outside-guard label
        /// (`sq(og(`run ${RUN}...`))`), with its placeholders filled in.
        fn written_lines() -> Vec<String> {
            WORKFLOW
                .lines()
                .filter_map(|l| {
                    let start = l.find("sq(`run ${RUN}").map(|i| i + "sq(`".len()).or_else(|| l.find("sq(og(`run ${RUN}").map(|i| i + "sq(og(`".len()))?;
                    let end = start + l[start..].find('`')?;
                    Some(l[start..end].to_string())
                })
                .map(|t| {
                    t.replace("${RUN}", "r1")
                        .replace("${round}", "2")
                        .replace("${CRITERIA.length - failing.length}", "1")
                        .replace("${CRITERIA.length}", "3")
                        .replace("${MAX_ROUNDS}", "3")
                        .replace("${merge.branch}", "work/b")
                        .replace("${merge.commit}", "abc1234")
                        .replace("${asks.map((a) => a.id).join(', ')}", "t1")
                        .replace("${integrationNote()}", "no integration yet")
                })
                .collect()
        }

        fn line_with(needle: &str) -> String {
            let l = written_lines().into_iter().find(|l| l.contains(needle));
            l.unwrap_or_else(|| panic!("vajra-deliver.js no longer writes a progress line containing {needle:?}; update runview.rs"))
        }

        #[test]
        fn every_written_line_has_its_placeholders_filled_in() {
            let all = written_lines();
            assert!(all.len() >= 5, "expected the five run lines, found {all:?}");
            for l in &all {
                assert!(!l.contains("${"), "unhandled placeholder in {l:?}: teach this test (and the parser) the new field");
            }
        }

        #[test]
        fn exhausted_rounds_are_the_need_input_the_workflow_returns_at_the_fix_loop() {
            // The line is written immediately before this return, so the timeline must show that result and node.
            let at = WORKFLOW.find("no verified delivery after ${MAX_ROUNDS}").expect("the exhausted-rounds line");
            let ret = &WORKFLOW[at..];
            let ret = &ret[..ret.find("return {").map(|i| i + 120).expect("a return after the line").min(ret.len())];
            assert!(ret.contains("status: 'need_input', node: 'fix_loop'"), "the workflow's result for exhausted rounds changed: {ret}");
            let t = timeline(&format!("{}\n", line_with("no verified delivery")), Some(&[task("r1.t1", "closed", &[])]));
            assert_eq!((PHASES[t.current], t.outcome.as_str()), ("Fix", "need_input"));
        }

        #[test]
        fn the_resume_line_the_parser_keys_on_is_still_written() {
            assert!(WORKFLOW.contains("`resumed by vajra-deliver run ${RUN}`"), "the resume progress line changed; update runview.rs");
        }

        #[test]
        fn the_workflows_own_lines_drive_the_timeline_to_each_outcome() {
            let open = [task("r1.t1", "closed", &[])];
            let run = |lines: &[String]| {
                let t = timeline(&lines.join("\n"), Some(&open));
                (PHASES[t.current], t.outcome, t.fix_round, t.max_rounds)
            };
            let verified = line_with("criteria passing");
            // 1/3 in the fixture, but the writer emits this line only after a failed gate, whatever the count.
            assert_eq!(run(std::slice::from_ref(&verified)), ("Fix", Outcome::Active, Some(3), None), "{verified}");
            assert_eq!(run(&[line_with("waiting for the user")]).1, Outcome::NeedInput);
            assert_eq!(run(&[line_with("review rejected")]).1, Outcome::Rejected);
            assert_eq!(run(&[verified.clone(), line_with("no verified delivery")]), ("Fix", Outcome::NeedInput, Some(3), Some(3)));
            let done = run(&[verified, line_with(" delivered ")]);
            assert_eq!((done.0, done.1), ("Deliver", Outcome::Done));
        }

        /// An outside-guard campaign's lines carry the workflow's label first (#183); the timeline still reads them.
        #[test]
        fn an_outside_guard_campaigns_labelled_lines_still_drive_the_timeline() {
            let mark = WORKFLOW.lines().find_map(|l| l.strip_prefix("const OG_MARK = '")?.strip_suffix('\'')).expect("the workflow's OG_MARK");
            assert_eq!(mark, OUTSIDE_GUARD_MARK, "the workflow's outside-guard label changed; update runview.rs");
            // `timeline` (the test helper) adds the `- <timestamp> ` each line is written with.
            let progress = |lines: &[String]| lines.iter().map(|l| format!("{mark} {l}")).collect::<Vec<_>>().join("\n");
            let open = [task("r1.t1", "closed", &[])];
            let t = timeline(&progress(&[line_with("criteria passing"), line_with(" delivered ")]), Some(&open));
            assert_eq!((PHASES[t.current], t.outcome), ("Deliver", Outcome::Done));
            let t = timeline(&progress(&[line_with("waiting for the user")]), Some(&open));
            assert_eq!(t.outcome, Outcome::NeedInput);
        }
    }
}
