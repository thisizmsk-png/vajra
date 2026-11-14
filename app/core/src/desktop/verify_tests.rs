//! Parity with the workflow (16.5a): the shared cases in `tests/fixtures/verify/cases.json` hold today's JS results;
//! `tests/workflows/vajra-verify-parity.test.mjs` checks the JS gives them, and this checks the port does.

use super::*;
use serde_json::{Value, json};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/verify/cases.json");
const WORKFLOW: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../workflows/vajra-deliver.js");

fn between<'a>(src: &'a str, a: &str, b: &str) -> &'a str {
    let i = src.find(a).unwrap_or_else(|| panic!("marker {a}"));
    let j = i + src[i..].find(b).unwrap_or_else(|| panic!("marker {b}"));
    &src[i..j]
}

/// A result from a fixture, which may name only some fields.
fn result(r: &Value) -> VResult {
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    VResult {
        criterion_id: s("criterion_id"),
        status: serde_json::from_value(r.get("status").cloned().unwrap_or(json!("pass"))).unwrap(),
        command: s("command"),
        notes: s("notes"),
        exit_code: r.get("exit_code").and_then(Value::as_i64).unwrap_or(0),
        evidence: s("evidence"),
    }
}

fn run(c: &Value, d: &Value) -> Value {
    let criteria: Vec<Criterion> = serde_json::from_value(c.get("criteria").unwrap_or(&d["criteria"]).clone()).unwrap();
    let tasks: Vec<PlanTask> = serde_json::from_value(c.get("tasks").unwrap_or(&d["tasks"]).clone()).unwrap();
    let verification = || -> Verification { serde_json::from_value(c["verification"].clone()).unwrap() };
    let strs = |k: &str| -> Vec<String> { c.get(k).map(|v| serde_json::from_value(v.clone()).unwrap()).unwrap_or_default() };
    match c["kind"].as_str().unwrap() {
        "results_by_criterion" => {
            let rs: Vec<VResult> = c["results"].as_array().unwrap().iter().map(result).collect();
            let out = results_by_criterion(&rs, &criteria);
            Value::Object(out.into_iter().map(|(id, i)| (id, json!(i))).collect())
        }
        "command_problem" => {
            let kind = if c.get("kind_of").and_then(Value::as_str) == Some("setup") { CmdKind::Setup } else { CmdKind::Check };
            json!({"problem": command_problem(c["cmd"].as_str().unwrap(), &strs("setup"), kind)})
        }
        "setup_of" => json!({"setup": setup_of(&strs("setup_cmds"))}),
        "verification_problems" => verification_problems(&verification(), &criteria).to_json(),
        "redo_feedback" => {
            let v = verification();
            redo_feedback(&v, &verification_problems(&v, &criteria), &criteria)
        }
        "gate" => {
            let v = verification();
            let reviews: Vec<Review> = serde_json::from_value(c.get("reviews").cloned().unwrap_or(json!([]))).unwrap();
            let g = gate(&v, &verification_problems(&v, &criteria), &criteria, &tasks, &reviews);
            json!({"passed": g.passed, "failing": g.failing, "suite_ok": g.suite_ok, "feedback": g.feedback, "rejected": rejected(&reviews)})
        }
        "security" => {
            let crit: Vec<String> = strs("criteria_text");
            json!({"flagged": security_flag(c["ask"].as_str().unwrap(), crit.iter().map(String::as_str), c["touches_security"].as_bool().unwrap())})
        }
        k => panic!("unknown kind {k}"),
    }
}

#[test]
fn parity_with_the_workflow() {
    let f: Value = serde_json::from_str(&std::fs::read_to_string(FIXTURES).unwrap()).unwrap();
    let cases = f["cases"].as_array().unwrap();
    assert!(cases.len() >= 70, "the shared cases are all there");
    for c in cases {
        assert_eq!(run(c, &f["defaults"]), c["expect"], "{}", c["name"]);
    }
}

#[test]
fn gate_version_is_the_hash_of_the_ported_blocks_the_fixtures_pin() {
    // The same slices, in the same order, as the node test hashes.
    let src = std::fs::read_to_string(WORKFLOW).unwrap();
    let block = [
        between(&src, "const SEC_RE", "const CAMPAIGN_SH"),
        between(&src, "const normText", "// Reviewer risks"),
        between(&src, "const suitePasses", "const flatten"),
        between(&src, "    const fb = {", "    const v2 ="),
        between(&src, "  const rejected = reviews.find", "  if (rejected)"),
        between(&src, "  const byC = resultsByCriterion(verification.results, CRITERIA)", "  log(`round ${round}: ${failing.length}"),
        between(&src, "const SECURITY = ", "\n"),
    ]
    .concat();
    let f: Value = serde_json::from_str(&std::fs::read_to_string(FIXTURES).unwrap()).unwrap();
    let sha = crate::control::sha256::hex_digest(block.as_bytes());
    assert_eq!(f["js_gate_sha256"].as_str().unwrap(), sha, "regenerate tests/fixtures/verify/cases.json (node tests/workflows/vajra-verify-parity.test.mjs --write)");
    assert_eq!(GATE_VERSION, sha, "the workflow's verify rules changed: port the change here and set GATE_VERSION");
}

// ---------------------------------------------------------------- parsing the last message

const GOOD_VERIFY: &str = r#"{"results":[{"criterion_id":"ac-1","status":"pass","command":"npm test","notes":"","exit_code":0,"evidence":"ok"}],"suite_command":"npm test","suite_passes":true,"regressions":[],"setup_cmds":[]}"#;

#[test]
fn a_result_is_parsed_from_the_last_message_alone_or_in_one_fence() {
    let red = Redactor::default();
    assert_eq!(parse_verification(GOOD_VERIFY, &red).unwrap().results.len(), 1);
    assert!(parse_verification(&format!("```json\n{GOOD_VERIFY}\n```"), &red).is_ok());
    assert!(parse_verification(&format!("  \n```\n{GOOD_VERIFY}\n```  "), &red).is_ok());
    let r = parse_review(r#"{"verdict":"revise","blocking_issues":[{"task":"t1","issue":"x"}],"risks":[],"rationale":"r"}"#, &red).unwrap();
    assert_eq!((r.verdict, r.blocking_issues.len()), (Verdict::Revise, 1));
}

#[test]
fn no_valid_result_is_none() {
    let red = Redactor::default();
    for bad in [
        "",
        "Here you go:\n{}",
        &format!("{GOOD_VERIFY} and some prose"),
        &format!("text before\n```json\n{GOOD_VERIFY}\n```"),
        &GOOD_VERIFY.replace(r#""setup_cmds":[]"#, r#""setup_cmds":[],"extra":1"#),
        &GOOD_VERIFY.replace(r#","regressions":[]"#, ""),
        &GOOD_VERIFY.replace(r#""status":"pass""#, r#""status":"passed""#),
        &GOOD_VERIFY.replace(r#""exit_code":0"#, r#""exit_code":"0""#),
        &GOOD_VERIFY.replace(r#""evidence":"ok""#, &format!(r#""evidence":"{}""#, "x".repeat(MAX_TEXT + 1))),
    ] {
        assert!(parse_verification(bad, &red).is_err(), "{bad:.80}");
    }
    assert!(parse_review(r#"{"verdict":"approve","blocking_issues":[],"risks":[],"rationale":"","lens":"x"}"#, &red).is_err(), "unknown field");
    let many = format!(r#"{{"verdict":"revise","blocking_issues":[{}],"risks":[],"rationale":""}}"#, vec![r#"{"task":"t1","issue":"x"}"#; MAX_ITEMS + 1].join(","));
    assert!(parse_review(&many, &red).is_err(), "bounded");
}

#[test]
fn parsed_text_is_sanitised_and_redacted() {
    const NONCE: &str = "abcdef0123456789abcdef0123456789";
    let red = Redactor::new(Some(NONCE));
    let msg = GOOD_VERIFY.replace(r#""evidence":"ok""#, &format!(r#""evidence":"ok\u001b[31m {NONCE}""#));
    let v = parse_verification(&msg, &red).unwrap();
    assert!(!v.results[0].evidence.contains(NONCE) && !v.results[0].evidence.contains('\u{1b}'), "{:?}", v.results[0].evidence);
}
