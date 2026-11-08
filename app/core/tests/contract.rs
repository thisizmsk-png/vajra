//! The shared contract fixtures in tests/contract/ are parsed here and by the UI tests (app/ui/tests/projectContract.test.ts),
//! so the Rust types and the TypeScript mirror cannot drift apart.

use std::path::PathBuf;

use vajra_desktop::projects::contract::*;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/contract").join(name)).unwrap()
}

#[test]
fn the_project_fixture_parses_to_the_documented_record() {
    let p = parse_project(&fixture("project.v1.json")).unwrap();
    assert_eq!((p.id.as_str(), p.name.as_str(), p.root.as_str(), p.rev), ("p_01JC5X8Z3A9K2M4N6P7Q8R9S0T", "vajra", "/Users/u/src/vajra", 7));
    assert_eq!((p.root_key.dev, p.root_key.ino), (16_777_230, 9123));
    assert_eq!(p.root_key.git_common_dir.as_deref(), Some("/Users/u/src/vajra/.git"));
    assert!(p.archived.is_none());
}

#[test]
fn a_record_with_fields_this_build_does_not_know_still_parses() {
    let p = parse_project(&fixture("project.v1.extra-fields.json")).unwrap();
    assert_eq!((p.rev, p.root_key.git_common_dir), (1, None));
}

#[test]
fn another_major_version_is_reported_not_misread() {
    assert_eq!(parse_project(&fixture("project.v2.json")), Err(ContractError::UnsupportedVersion { found: "vajra.project/2".into(), supported: 1 }));
}

#[test]
fn the_session_fixtures_parse() {
    let s = parse_session(&fixture("session.v1.json")).unwrap();
    assert_eq!((s.kind, s.status, s.generation, s.attempt), (SessionKind::Worker, SessionStatus::Running, 3, Some(2)));
    assert_eq!(s.parent.as_deref(), Some("s_01JC5X8Z3C1K2M4N6P7Q8R9S0T"));
    let u = parse_session(&fixture("session.v1.unknown-enums.json")).unwrap();
    assert_eq!((u.kind, u.status), (SessionKind::Other, SessionStatus::Unknown));
}

#[test]
fn the_jsonl_fixture_keeps_the_newest_line_per_session_and_skips_bad_ones() {
    let bytes = fixture("sessions.v1.jsonl");
    let lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()).collect();
    let got = latest_sessions(&lines);
    let ids: Vec<(&str, SessionStatus)> = got.iter().map(|s| (s.id.as_str(), s.status)).collect();
    assert_eq!(ids, vec![("s_01JC5X8Z3C1K2M4N6P7Q8R9S0T", SessionStatus::Done), ("s_01JC5X8Z3B0K2M4N6P7Q8R9S0T", SessionStatus::Running)]);
}
