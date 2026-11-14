//! Parity with the workflow (16.5): the shared cases in `tests/fixtures/plan/cases.json` hold today's JS results;
//! `tests/workflows/vajra-plan-parity.test.mjs` checks the JS gives them, and this checks the port does.

use super::*;
use serde_json::{Value, json};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/plan/cases.json");
const WORKFLOW: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../workflows/vajra-deliver.js");

fn between<'a>(src: &'a str, a: &str, b: &str) -> &'a str {
    let i = src.find(a).unwrap_or_else(|| panic!("marker {a}"));
    let j = i + src[i..].find(b).unwrap_or_else(|| panic!("marker {b}"));
    &src[i..j]
}

fn tasks(c: &Value) -> Vec<PlanTask> {
    c["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let list = |k: &str| t.get(k).and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect()).unwrap_or_default();
            let text = |k: &str, d: &str| t.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
            PlanTask {
                id: text("id", ""),
                title: text("title", "title"),
                owner_role: text("owner_role", "backend"),
                files: list("files"),
                deps: list("deps"),
                criteria: list("criteria"),
                brief: text("brief", "brief"),
                touches_security: false,
            }
        })
        .collect()
}

fn run(c: &Value, d: &Value) -> Value {
    let seats: Vec<String> = c.get("seats").unwrap_or(&d["seats"]).as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect();
    let cap = match c.get("max_in_flight") {
        Some(v) => v.as_u64().map(|n| n as usize),
        None => d["max_in_flight"].as_u64().map(|n| n as usize),
    };
    let criteria: Vec<Criterion> = serde_json::from_value(c.get("criteria").unwrap_or(&d["criteria"]).clone()).unwrap();
    match c["kind"].as_str().unwrap() {
        "criteria" => json!({"error": validate_criteria(&criteria).err()}),
        "waves" => {
            let t = tasks(c);
            json!({"order": topo(&t), "waves": waves(&t, cap)})
        }
        _ => {
            let t = tasks(c);
            let ids: Vec<String> = criteria.iter().map(|c| c.id.clone()).collect();
            match validate_plan(&t, &ids, &seats) {
                Err(e) => json!({"error": e}),
                Ok(()) => json!({"error": null, "order": topo(&t), "waves": waves(&t, cap)}),
            }
        }
    }
}

#[test]
fn parity_with_the_workflow() {
    let f: Value = serde_json::from_str(&std::fs::read_to_string(FIXTURES).unwrap()).unwrap();
    let cases = f["cases"].as_array().unwrap();
    assert!(cases.len() >= 40, "the shared cases are all there");
    for c in cases {
        assert_eq!(run(c, &f["defaults"]), c["expect"], "{}", c["name"]);
    }
}

#[test]
fn the_ported_js_block_is_the_one_the_fixtures_pin() {
    // The same three slices, in the same order, as the node test hashes.
    let src = std::fs::read_to_string(WORKFLOW).unwrap();
    let block = [between(&src, "const clean = ", "function fnv("), between(&src, "const normPath", "const globRe"), between(&src, "function validateUnderstanding(", "// Verifier results")].concat();
    let f: Value = serde_json::from_str(&std::fs::read_to_string(FIXTURES).unwrap()).unwrap();
    assert_eq!(
        f["js_rules_sha256"].as_str().unwrap(),
        crate::control::sha256::hex_digest(block.as_bytes()),
        "the workflow's plan rules changed: regenerate tests/fixtures/plan/cases.json (node tests/workflows/vajra-plan-parity.test.mjs --write) and port the change here"
    );
}

#[test]
fn overlaps_is_symmetric_on_the_workflow_samples() {
    // The samples of tests/workflows/vajra-waves.test.mjs.
    for (a, b) in [("src/", "src/a.ts"), ("src/*.ts", "src/a.ts"), ("docs/**", "docs/x.md"), (".", "anything"), ("src/b", "src/b/c.ts")] {
        assert!(overlaps(a, b) && overlaps(b, a), "{a} ~ {b}");
    }
    for (a, b) in [("lib/x", "lib/xy"), ("src/a.ts", "src/b.ts"), ("docs/*.md", "src/a.md"), ("test/", "tests/")] {
        assert!(!overlaps(a, b), "{a} !~ {b}");
    }
}
