//! The deliver workflow's plan rules, ported (ADR-16 16.5, PR 6d1): `validateUnderstanding`'s criteria part,
//! `validatePlan`, `topo`, `waves` and the path helpers they use, from `workflows/vajra-deliver.js`. A straight port:
//! the same order of checks and the same messages, so the lead hears what today's planner hears. One difference by
//! design: `validatePlan`'s writer roster is the run's writer seat roles, so "owner has a writer" reads "owner_role
//! has a seat". Decisions are checked by the run protocol (16.5a allows 8), not here.
//!
//! The shared cases in `tests/fixtures/plan/cases.json` are run by both this port and the JS itself; the file pins
//! the hash of the JS block, so a rule change there fails CI until the fixtures and this port follow.

use serde::{Deserialize, Serialize};

use crate::crew::roles::{self, Kind};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub id: String,
    pub criterion: String,
    pub check: String,
}

/// One task as the lead plans it (`plan_tasks`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanTask {
    pub id: String,
    pub title: String,
    pub owner_role: String,
    pub files: Vec<String>,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub criteria: Vec<String>,
    pub brief: String,
    #[serde(default)]
    pub touches_security: bool,
}

// ---------------------------------------------------------------- text (`clean`, `oneLine`)

/// JS `\s` and `String.prototype.trim`: Unicode white space plus the BOM.
pub(super) fn js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

/// `clean(s, n)`: C0 controls but tab and newline, and DEL, become spaces; trimmed; cut to `n`.
fn clean(s: &str, n: usize) -> String {
    let s: String = s.chars().map(|c| if matches!(c, '\u{0}'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}') { ' ' } else { c }).collect();
    s.trim_matches(js_space).chars().take(n).collect()
}

/// `oneLine(s, n)`: `clean`, every run of white space one space, cut to `n`.
pub(super) fn one_line(s: &str, n: usize) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in clean(s, 4000).chars() {
        if js_space(c) {
            gap = true;
            continue;
        }
        if gap {
            out.push(' ');
            gap = false;
        }
        out.push(c);
    }
    if gap {
        out.push(' ');
    }
    out.chars().take(n).collect()
}

fn blank(s: &str) -> bool {
    s.trim_matches(js_space).is_empty()
}

// ---------------------------------------------------------------- paths

/// `normPath`: no leading `./`, no trailing `/*` or `/**`, no trailing `/`; `.` is the repo root, `""`.
fn norm_path(p: &str) -> String {
    let mut x = p;
    while let Some(r) = x.strip_prefix("./") {
        x = r;
    }
    let x = x.strip_suffix("/**").or_else(|| x.strip_suffix("/*")).unwrap_or(x);
    let x = x.trim_end_matches('/');
    if x == "." { String::new() } else { x.to_string() }
}

/// `SAFE_PATH`: relative, no `~`, only `[A-Za-z0-9._@+-/*]`.
fn safe_path(p: &str) -> bool {
    !p.is_empty() && !p.starts_with('/') && !p.starts_with('~') && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"._@+-/*".contains(&b))
}

/// `GLOB_CH`.
fn glob_ch(c: char) -> bool {
    matches!(c, '*' | '?' | '[')
}

fn static_prefix(x: &str) -> &str {
    x.find(glob_ch).map_or(x, |i| &x[..i])
}

/// `overlaps(a, b)`: the two paths or globs can name a common file.
pub fn overlaps(a: &str, b: &str) -> bool {
    let (x, y) = (norm_path(a), norm_path(b));
    if x.is_empty() || y.is_empty() {
        return true;
    }
    if x.contains(glob_ch) || y.contains(glob_ch) {
        let (px, py) = (static_prefix(&x), static_prefix(&y));
        return px.starts_with(py) || py.starts_with(px);
    }
    x == y || y.starts_with(&format!("{x}/")) || x.starts_with(&format!("{y}/"))
}

// ---------------------------------------------------------------- the rules

fn numbered(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix).is_some_and(|n| (1..=2).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `validateUnderstanding`'s criteria rules (a fresh run: no resumed campaign).
pub fn validate_criteria(c: &[Criterion]) -> Result<(), String> {
    if c.is_empty() || c.len() > 12 {
        return Err("acceptance_criteria must have 1-12 entries".into());
    }
    let mut ids: Vec<&str> = Vec::new();
    for x in c {
        if !numbered(&x.id, "ac-") {
            return Err(format!("criterion id \"{}\" must look like ac-1", one_line(&x.id, 40)));
        }
        if ids.contains(&x.id.as_str()) {
            return Err(format!("duplicate criterion id {}", x.id));
        }
        ids.push(&x.id);
        if blank(&x.criterion) || blank(&x.check) {
            return Err(format!("criterion {} needs text and a check", x.id));
        }
    }
    Ok(())
}

/// `validatePlan`, with the run's seat roles for the writer roster (only writer roles count, in seat order).
pub fn validate_plan(t: &[PlanTask], criteria: &[String], seat_roles: &[String]) -> Result<(), String> {
    if t.is_empty() || t.len() > 8 {
        return Err("plan must have 1-8 tasks".into());
    }
    let mut writers: Vec<&str> = Vec::new();
    for r in seat_roles {
        if roles::kind_of(r) == Some(Kind::Writer) && !writers.contains(&r.as_str()) {
            writers.push(r);
        }
    }
    let mut ids: Vec<&str> = Vec::new();
    let mut covered: Vec<&str> = Vec::new();
    for x in t {
        if !numbered(&x.id, "t") {
            return Err(format!("task id \"{}\" must look like t1", one_line(&x.id, 40)));
        }
        if ids.contains(&x.id.as_str()) {
            return Err(format!("duplicate task id {}", x.id));
        }
        ids.push(&x.id);
        if !writers.contains(&x.owner_role.as_str()) {
            return Err(format!("task {}: owner must be one of {}", x.id, writers.join(", ")));
        }
        if blank(&x.title) || blank(&x.brief) {
            return Err(format!("task {} needs a title and a brief", x.id));
        }
        if x.files.is_empty() {
            return Err(format!("task {} must own at least one path", x.id));
        }
        if let Some(f) = x.files.iter().find(|f| !safe_path(f) || f.split('/').any(|p| p == "..")) {
            return Err(format!("task {}: unsafe path \"{}\"", x.id, one_line(f, 80)));
        }
        for c in &x.criteria {
            if !criteria.contains(c) {
                return Err(format!("task {}: unknown criterion {}", x.id, one_line(c, 40)));
            }
            covered.push(c);
        }
    }
    if let Some(c) = criteria.iter().find(|c| !covered.contains(&c.as_str())) {
        return Err(format!("criterion {c} is not covered by any task"));
    }
    for x in t {
        if let Some(d) = x.deps.iter().find(|d| !ids.contains(&d.as_str()) || **d == x.id) {
            return Err(format!("task {}: bad dependency {}", x.id, one_line(d, 40)));
        }
    }
    for i in 0..t.len() {
        for j in i + 1..t.len() {
            for a in &t[i].files {
                if t[j].files.iter().any(|b| overlaps(a, b)) {
                    return Err(format!("tasks {} and {} both own {}; files must be disjoint", t[i].id, t[j].id, one_line(a, 80)));
                }
            }
        }
    }
    // Depth-first, in plan order: 1 = on the path, 2 = done.
    fn cyclic(i: usize, t: &[PlanTask], state: &mut [u8]) -> bool {
        match state[i] {
            1 => return true,
            2 => return false,
            _ => state[i] = 1,
        }
        for d in &t[i].deps {
            if t.iter().position(|x| &x.id == d).is_some_and(|k| cyclic(k, t, state)) {
                return true;
            }
        }
        state[i] = 2;
        false
    }
    let mut state = vec![0u8; t.len()];
    if (0..t.len()).any(|i| cyclic(i, t, &mut state)) {
        return Err("task dependencies contain a cycle".into());
    }
    Ok(())
}

/// `topo`: depth-first post-order over deps, in plan order.
pub fn topo(t: &[PlanTask]) -> Vec<String> {
    fn visit(id: &str, t: &[PlanTask], out: &mut Vec<String>, seen: &mut Vec<String>) {
        let Some(x) = t.iter().find(|x| x.id == id) else { return };
        if seen.iter().any(|s| s == id) {
            return;
        }
        seen.push(id.to_string());
        for d in &x.deps {
            visit(d, t, out, seen);
        }
        out.push(id.to_string());
    }
    let (mut out, mut seen) = (Vec::new(), Vec::new());
    for x in t {
        visit(&x.id, t, &mut out, &mut seen);
    }
    out
}

/// `waves`: in `topo` order, each task goes into the first wave after its deps' waves that holds no task with
/// overlapping files; each wave is then cut into batches of at most `max_in_flight` (none: whole waves).
pub fn waves(t: &[PlanTask], max_in_flight: Option<usize>) -> Vec<Vec<String>> {
    let by = |id: &str| t.iter().find(|x| x.id == id);
    let mut wave_of: Vec<(String, usize)> = Vec::new();
    let mut ws: Vec<Vec<String>> = Vec::new();
    for id in topo(t) {
        let Some(x) = by(&id) else { continue };
        let mut w = 0;
        for d in &x.deps {
            if let Some((_, k)) = wave_of.iter().find(|(i, _)| i == d) {
                w = w.max(k + 1);
            }
        }
        let clash = |wave: &[String]| wave.iter().filter_map(|o| by(o)).any(|o| x.files.iter().any(|f| o.files.iter().any(|g| overlaps(f, g))));
        while w < ws.len() && clash(&ws[w]) {
            w += 1;
        }
        if w == ws.len() {
            ws.push(Vec::new());
        }
        ws[w].push(id.clone());
        wave_of.push((id, w));
    }
    match max_in_flight {
        None | Some(0) => ws,
        Some(n) => ws.iter().flat_map(|w| w.chunks(n).map(<[String]>::to_vec)).collect(),
    }
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
