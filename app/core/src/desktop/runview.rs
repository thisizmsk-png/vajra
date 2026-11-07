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
    Error,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Done => "done",
            Self::NeedInput => "need_input",
            Self::Rejected => "rejected",
            Self::Error => "error",
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
    /// The run id that wrote `outcome`; a line from a different (later, resumed) run supersedes it.
    outcome_run: String,
    max_rounds: Option<u32>,
}

fn scan(progress: &str) -> Lines {
    let mut out = Lines::default();
    for line in progress.lines() {
        let Some(rest) = line.split_once("run ").map(|x| x.1) else { continue };
        // `<id> ...`: the run id is one word.
        let Some((id, after)) = rest.split_once(|c: char| c.is_whitespace() || c == ':') else { continue };
        let after = after.trim_start_matches(|c: char| c.is_whitespace() || c == ':');
        if out.outcome.is_some() && id != out.outcome_run {
            out.outcome = None; // a resumed run (new id) starts the campaign again
        }
        if let Some(r) = after.strip_prefix("round ") {
            let n: u32 = r.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0);
            out.round = out.round.max(n);
            let body = r.split_once(':').map(|x| x.1.trim_start()).unwrap_or("");
            if body.starts_with("waiting for the user") {
                out.outcome = Some((Outcome::NeedInput, n));
                out.outcome_run = id.to_string();
            } else if body.starts_with("review rejected") {
                out.outcome = Some((Outcome::Rejected, n));
                out.outcome_run = id.to_string();
            } else if let Some((k, m)) = body.split_once(" criteria passing").and_then(|(h, _)| h.split_once('/')) {
                if let (Ok(k), Ok(m)) = (k.trim().parse::<u32>(), m.trim().parse::<u32>()) {
                    out.verified = Some((n, k, m));
                    out.outcome = None; // a finished round supersedes an earlier stop
                }
            }
        } else if let Some(r) = after.strip_prefix("no verified delivery after ") {
            let n: u32 = r.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0);
            out.max_rounds = Some(n);
            out.round = out.round.max(n);
            out.outcome = Some((Outcome::Error, n));
            out.outcome_run = id.to_string();
        } else if after.starts_with("delivered ") && after.contains(" @ ") {
            out.outcome = Some((Outcome::Done, out.round));
            out.outcome_run = id.to_string();
        }
    }
    out
}

/// Where the run is. `tasks` is `tasks.json`'s task list (`None` when the file is absent), `progress` is `progress.md`.
/// Exactly one phase is current, always.
pub fn timeline(progress: &str, tasks: Option<&[Value]>) -> Timeline {
    let l = scan(progress);
    let open = tasks.is_some_and(|t| t.iter().any(|x| !matches!(status(x), "done" | "closed")));
    let have_tasks = tasks.is_some_and(|t| !t.is_empty());
    // The Fix round after the last verification that did not pass fully.
    let next_fix = l.verified.filter(|(_, k, m)| k < m).map(|(n, _, _)| n + 1);
    let (current, outcome, fix_round) = match l.outcome {
        Some((Outcome::Done, _)) => (5, Outcome::Done, l.verified.map(|v| v.0).filter(|n| *n > 1)),
        Some((Outcome::Rejected, _)) => (3, Outcome::Rejected, None),
        Some((Outcome::NeedInput, _)) => (2, Outcome::NeedInput, None),
        Some((Outcome::Error, n)) => (4, Outcome::Error, Some(n.max(1))),
        _ => match (tasks, have_tasks, open, next_fix) {
            (None, _, _, _) => (0, Outcome::Active, None),
            (Some(_), false, _, _) => (1, Outcome::Active, None),
            (_, true, true, Some(n)) => (4, Outcome::Active, Some(n)),
            (_, true, true, None) => (2, Outcome::Active, None),
            (_, true, false, Some(n)) => (4, Outcome::Active, Some(n)),
            (_, true, false, None) => {
                // All tasks closed: verifying until a round reports everything passing, then delivering.
                if l.verified.is_some_and(|(_, k, m)| k == m) { (5, Outcome::Active, None) } else { (3, Outcome::Active, None) }
            }
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
        // Everything passing: delivering.
        let all = "run r round 1: 3/3 criteria passing; x\n";
        assert_eq!(cur(all, Some(&done)), ("Deliver", Outcome::Active));
    }

    #[test]
    fn the_four_ways_a_run_ends_each_show_status_and_the_node_reached() {
        let t = [task("r.t1", "closed", &[])];
        assert_eq!(cur("run r round 1: 3/3 criteria passing\nrun r delivered b @ abc\n", Some(&t)), ("Deliver", Outcome::Done));
        assert_eq!(cur("run r round 1: waiting for the user on t1; x\n", Some(&t)), ("Execute", Outcome::NeedInput));
        assert_eq!(cur("run r round 1: review rejected the approach\n", Some(&t)), ("Verify", Outcome::Rejected));
        let t = timeline("run r round 1: 1/3 criteria passing\nrun r: no verified delivery after 3 round(s); x\n", Some(&t));
        assert_eq!((PHASES[t.current], t.outcome, t.fix_round, t.max_rounds), ("Fix", Outcome::Error, Some(3), Some(3)));
    }

    #[test]
    fn a_resume_after_a_stop_starts_again() {
        let t = [task("r.t1", "in_progress", &[])];
        let p = "run r round 1: waiting for the user on t1\nrun r2 round 1: 0/3 criteria passing\n";
        assert_eq!(cur(p, Some(&t)), ("Fix", Outcome::Active), "the stop no longer applies once a later run writes lines");
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
}
