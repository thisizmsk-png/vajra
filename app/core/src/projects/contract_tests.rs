use super::*;

const P: &str = "p_01JC5X8Z3A9K2M4N6P7Q8R9S0T";
const S1: &str = "s_01JC5X8Z3B0K2M4N6P7Q8R9S0T";
const S2: &str = "s_01JC5X8Z3C1K2M4N6P7Q8R9S0T";

type Mutate<T> = Box<dyn Fn(&mut T)>;

fn project() -> Project {
    Project::new(P.into(), "vajra".into(), "/Users/u/src/vajra".into(), RootKey { dev: 1, ino: 2, git_common_dir: Some("/Users/u/src/vajra/.git".into()) }, "2026-11-08T09:00:00Z".into())
}

fn session() -> Session {
    Session {
        schema: SESSION_SCHEMA.into(),
        id: S1.into(),
        project_id: P.into(),
        kind: SessionKind::Worker,
        parent: Some(S2.into()),
        provider: "claude".into(),
        native_session_id: None,
        run_id: Some("20261108-0930".into()),
        campaign: Some("deliver-20261108-0930".into()),
        task_id: Some("t3".into()),
        attempt: Some(2),
        generation: 3,
        status: SessionStatus::Running,
        started: "2026-11-08T09:30:00Z".into(),
        ended: None,
    }
}

// ------------------------------------------------------------------------------------------------------- ids

#[test]
fn an_id_is_a_prefixed_26_char_ulid_that_sorts_by_time_and_validates() {
    let a = make_id("p", 1_793_872_800_000, [0; 10]);
    let b = make_id("p", 1_793_872_800_001, [0; 10]);
    let c = make_id("p", 1_793_872_800_001, [0xFF; 10]);
    assert_eq!(a.len(), 28, "{a}");
    assert!(a < b && b < c, "{a} {b} {c}");
    for id in [&a, &b, &c] {
        assert!(valid_project_id(id), "{id}");
    }
    assert!(valid_project_id(P) && valid_session_id(S1));
    // The largest possible value still fits the 128 bits: the first character stays at most 7.
    assert!(valid_project_id(&make_id("p", u64::MAX, [0xFF; 10])));
}

#[test]
fn fresh_ids_are_valid_and_differ() {
    let a = fresh_id("s").unwrap();
    let b = fresh_id("s").unwrap();
    assert!(valid_session_id(&a) && valid_session_id(&b));
    assert_ne!(a, b);
    assert!(valid_action_id(&fresh_id("a").unwrap()));
}

#[test]
fn ids_with_the_wrong_prefix_length_alphabet_or_shape_are_refused() {
    for bad in [
        "",
        "p_",
        "p_01JC5X8Z3A9K2M4N6P7Q8R9S0",    // 25 characters
        "p_01JC5X8Z3A9K2M4N6P7Q8R9S0TT",  // 27 characters
        "s_01JC5X8Z3A9K2M4N6P7Q8R9S0T",   // a session id is not a project id
        "p01JC5X8Z3A9K2M4N6P7Q8R9S0T",    // no separator
        "p_01JC5X8Z3A9K2M4N6P7Q8R9S0I",   // I is not in the alphabet
        "p_01JC5X8Z3A9K2M4N6P7Q8R9S0U",   // neither is U
        "p_81JC5X8Z3A9K2M4N6P7Q8R9S0T",   // first character above 7 overflows 128 bits
        "p_01jc5x8z3a9k2m4n6p7q8r9s0t",   // lower case
        "p_../../../etc/passwd000000000", // a path
        "p_01JC5X8Z3A9K2M4N6P7Q8R9S0\n",
        "-p_01JC5X8Z3A9K2M4N6P7Q8R9S0T",
    ] {
        assert!(!valid_project_id(bad), "{bad:?}");
    }
    assert!(!valid_session_id(P) && !valid_project_id(S1) && !valid_action_id(P));
}

// ----------------------------------------------------------------------------------------------- schema tag

#[test]
fn the_schema_tag_names_a_thing_and_a_plain_major() {
    assert_eq!(schema_major("vajra.project/1", "project"), Some(1));
    assert_eq!(schema_major("vajra.project/12", "project"), Some(12));
    for bad in ["vajra.project/", "vajra.project/01", "vajra.project/1.0", "vajra.session/1", "vajra.project", "project/1", "vajra.project/-1", "vajra.project/99999", ""] {
        assert_eq!(schema_major(bad, "project"), None, "{bad}");
    }
}

// --------------------------------------------------------------------------------------------------- project

#[test]
fn a_project_round_trips_through_json_unchanged() {
    let p = project();
    let bytes = serde_json::to_vec(&p).unwrap();
    assert_eq!(parse_project(&bytes).unwrap(), p);
    assert_eq!(p.schema, "vajra.project/1");
    assert_eq!(p.rev, 1);
    assert!(p.archived.is_none());
}

#[test]
fn a_project_from_another_major_version_is_reported_as_unsupported_even_if_its_fields_changed() {
    let v2 = br#"{"schema":"vajra.project/2","id":5,"name":["x"],"root":null}"#;
    assert_eq!(parse_project(v2), Err(ContractError::UnsupportedVersion { found: "vajra.project/2".into(), supported: 1 }));
    let other = br#"{"schema":"vajra.session/1","id":"x"}"#;
    assert!(matches!(parse_project(other), Err(ContractError::UnsupportedVersion { .. })));
}

#[test]
fn a_project_with_no_schema_or_garbage_is_malformed() {
    for bad in [&b""[..], b"{", b"[]", b"null", br#"{"id":"x"}"#, br#"{"schema":1}"#] {
        assert!(matches!(parse_project(bad), Err(ContractError::Malformed(_))), "{}", String::from_utf8_lossy(bad));
    }
}

#[test]
fn a_project_record_over_the_size_cap_is_refused_unread() {
    let big = vec![b' '; MAX_RECORD_BYTES + 1];
    assert_eq!(parse_project(&big), Err(ContractError::TooLarge));
    assert_eq!(parse_session(&big), Err(ContractError::TooLarge));
}

#[test]
fn unknown_fields_are_ignored_and_a_missing_optional_field_reads_as_none() {
    let json = br#"{"schema":"vajra.project/1","id":"p_01JC5X8Z3A9K2M4N6P7Q8R9S0T","name":"n","root":"/r","root_key":{"dev":1,"ino":2,"volume":"x"},"created":"2026-11-08T09:00:00Z","rev":3,"later":{"a":1}}"#;
    let p = parse_project(json).unwrap();
    assert_eq!((p.rev, p.archived, p.root_key.git_common_dir), (3, None, None));
}

#[test]
fn project_fields_are_validated_one_by_one() {
    let cases: Vec<(&str, Mutate<Project>)> = vec![
        ("project id", Box::new(|p| p.id = "p_short".into())),
        ("project id", Box::new(|p| p.id = S1.into())),
        ("name", Box::new(|p| p.name = "   ".into())),
        ("name", Box::new(|p| p.name = "a\nb".into())),
        ("name", Box::new(|p| p.name = "x".repeat(MAX_NAME_CHARS + 1))),
        ("path", Box::new(|p| p.root = "relative/dir".into())),
        ("path", Box::new(|p| p.root = "/a/../b".into())),
        ("path", Box::new(|p| p.root = "/a\0b".into())),
        ("path", Box::new(|p| p.root = format!("/{}", "a".repeat(MAX_PATH_BYTES)))),
        ("path", Box::new(|p| p.root_key.git_common_dir = Some("../.git".into()))),
        ("timestamp", Box::new(|p| p.created = "yesterday".into())),
        ("timestamp", Box::new(|p| p.archived = Some("2026-13-40T00:00:00Z".into()))),
        ("rev", Box::new(|p| p.rev = 0)),
    ];
    for (why, mutate) in cases {
        let mut p = project();
        mutate(&mut p);
        let bytes = serde_json::to_vec(&p).unwrap();
        assert_eq!(parse_project(&bytes), Err(ContractError::Invalid(why)), "{why}");
    }
    assert!(project().validate().is_ok());
    let mut named = project();
    named.name = "x".repeat(MAX_NAME_CHARS);
    assert!(named.validate().is_ok(), "the name limit is inclusive");
}

// --------------------------------------------------------------------------------------------------- session

#[test]
fn a_session_round_trips_and_validates() {
    let s = session();
    let bytes = serde_json::to_vec(&s).unwrap();
    assert_eq!(parse_session(&bytes).unwrap(), s);
}

#[test]
fn an_unknown_kind_or_status_is_read_as_the_catch_all_not_as_a_failure() {
    let line = br#"{"schema":"vajra.session/1","id":"s_01JC5X8Z3B0K2M4N6P7Q8R9S0T","project_id":"p_01JC5X8Z3A9K2M4N6P7Q8R9S0T","kind":"reviewer","provider":"codex","generation":1,"status":"paused","started":"2026-11-08T09:30:00Z","extra":true}"#;
    let s = parse_session(line).unwrap();
    assert_eq!((s.kind, s.status), (SessionKind::Other, SessionStatus::Unknown));
}

#[test]
fn session_fields_are_validated_one_by_one() {
    let cases: Vec<(&str, Mutate<Session>)> = vec![
        ("session id", Box::new(|s| s.id = "s_x".into())),
        ("project_id", Box::new(|s| s.project_id = S1.into())),
        ("parent", Box::new(|s| s.parent = Some("nope".into()))),
        ("parent", Box::new(|s| s.parent = Some(S1.into()))), // its own parent
        ("provider", Box::new(|s| s.provider = "Claude Code".into())),
        ("provider", Box::new(|s| s.provider = "".into())),
        ("native_session_id", Box::new(|s| s.native_session_id = Some("--last".into()))),
        ("run_id", Box::new(|s| s.run_id = Some("../../x".into()))),
        ("campaign", Box::new(|s| s.campaign = Some("a/b".into()))),
        ("campaign", Box::new(|s| s.campaign = Some(".hidden".into()))),
        ("task_id", Box::new(|s| s.task_id = Some("t 3".into()))),
        ("timestamp", Box::new(|s| s.started = "soon".into())),
        ("timestamp", Box::new(|s| s.ended = Some("later".into()))),
    ];
    for (why, mutate) in cases {
        let mut s = session();
        mutate(&mut s);
        let bytes = serde_json::to_vec(&s).unwrap();
        assert_eq!(parse_session(&bytes), Err(ContractError::Invalid(why)), "{why}");
    }
    let mut ok = session();
    ok.native_session_id = Some("1b4e28ba-2fa1-41d2-883f-0016d3cca427".into());
    assert!(ok.validate().is_ok());
}

#[test]
fn the_newest_line_per_session_wins_and_a_bad_line_is_skipped() {
    let mut a = session();
    a.id = S2.into();
    a.parent = None;
    a.kind = SessionKind::Run;
    let mut a2 = a.clone();
    a2.status = SessionStatus::Done;
    a2.ended = Some("2026-11-08T10:00:00Z".into());
    let b = session();
    let l = |s: &Session| serde_json::to_vec(s).unwrap();
    let (la, la2, lb) = (l(&a), l(&a2), l(&b));
    let lines: Vec<&[u8]> = vec![&la, b"not json", &lb, br#"{"schema":"vajra.session/2","id":"s_01JC5X8Z3D2K2M4N6P7Q8R9S0T"}"#, &la2];
    let log = latest_sessions(&lines);
    assert_eq!((log.skipped, log.unsupported), (2, true), "garbage and the version-2 line are counted, and the second is flagged");
    let got = log.sessions;
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!((got[0].id.as_str(), got[0].status), (S2, SessionStatus::Done), "first-seen order, newest content");
    assert_eq!(got[1].id, S1);
}

#[test]
fn a_newer_major_line_for_a_known_session_is_flagged_not_silently_hidden() {
    let l = |s: &Session| serde_json::to_vec(s).unwrap();
    let v1 = l(&session());
    let v2 = br#"{"schema":"vajra.session/2","id":"s_01JC5X8Z3B0K2M4N6P7Q8R9S0T","status":"done"}"#;
    let log = latest_sessions(&[&v1, v2]);
    assert_eq!(log.sessions.len(), 1);
    assert_eq!(log.sessions[0].status, SessionStatus::Running, "the old line is all this build can show");
    assert!(log.unsupported, "so the caller must be told it may be out of date");
    let clean = latest_sessions(&[&v1]);
    assert_eq!((clean.skipped, clean.unsupported), (0, false));
}

// -------------------------------------------------------------------------------------------------- app_info

#[test]
fn app_info_names_the_supported_majors() {
    let i = app_info();
    assert_eq!(i.schema, "vajra.app_info/1");
    assert_eq!((i.contracts.project, i.contracts.session), (PROJECT_MAJOR, SESSION_MAJOR));
    let v = serde_json::to_value(&i).unwrap();
    assert_eq!(v["contracts"]["project"], 1);
}
