use super::*;
use std::time::Duration;

/// Invented ids only. Real ones are never used in a fixture.
const SENTINEL_USER: &str = "user_SENTINEL0000TESTONLY";
const SENTINEL_RAY: &str = "0000deadbeef0000-XXX";

/// The spike's seven failure texts, in the shapes Codex printed them, with replaced ids.
fn fixtures() -> Vec<(&'static str, String, Kind, Option<u16>)> {
    vec![
        ("401", format!("unexpected status 401 Unauthorized: {{\"error\":{{\"message\":\"No auth credentials found\",\"code\":401,\"user_id\":\"{SENTINEL_USER}\"}}}}, cf-ray: {SENTINEL_RAY}"), Kind::Auth, Some(401)),
        ("402", "unexpected status 402 Payment Required: {\"error\":{\"message\":\"Insufficient credits\",\"code\":402}}".into(), Kind::Quota, Some(402)),
        ("429", "exceeded retry limit, last status: 429 Too Many Requests, retry-after: 7".into(), Kind::RateLimit, Some(429)),
        ("502", "exceeded retry limit, last status: 502 Bad Gateway: Provider returned error".into(), Kind::Upstream, Some(502)),
        ("504", "unexpected status 504 Gateway Timeout: Upstream idle timeout exceeded".into(), Kind::Timeout, Some(504)),
        ("hang", "idle timeout waiting for SSE".into(), Kind::Timeout, None),
        ("bad model", "unexpected status 400 Bad Request: {\"error\":{\"message\":\"vendor/none is not a valid model ID\",\"code\":400}}".into(), Kind::BadRequest, Some(400)),
    ]
}

#[test]
fn the_spikes_seven_failure_texts_classify() {
    for (name, text, kind, status) in fixtures() {
        let c = classify(&text);
        assert_eq!((c.kind, c.http_status), (kind, status), "{name}");
    }
    assert_eq!(classify("something odd happened").kind, Kind::UnexplainedExit);
    assert_eq!(classify("").kind, Kind::UnexplainedExit);
}

#[test]
fn a_429_carries_its_retry_after() {
    assert_eq!(classify("last status: 429, retry-after: 7").retry_after_s, Some(7));
    assert_eq!(classify("last status: 429").retry_after_s, None);
    // Only a rate limit has a window.
    assert_eq!(classify("last status: 504, retry-after: 7").retry_after_s, None);
}

#[test]
fn a_retry_after_needs_a_unit_it_understands() {
    let ra = |t: &str| classify(&format!("last status: 429 {t}")).retry_after_s;
    assert_eq!(ra("Please retry after 20 minutes"), Some(1200));
    assert_eq!(ra("retry after 20 min"), Some(1200));
    assert_eq!(ra("retry after 2 hours"), Some(7200));
    assert_eq!(ra("retry after 30 seconds"), Some(30));
    assert_eq!(ra("retry-after: 7"), Some(7));
    // A date or an unknown unit is not a count of seconds: None, so the user is asked.
    assert_eq!(ra("retry after 2026-11-09T10:00Z"), None);
    assert_eq!(ra("retry-after: 2026-11-09T10:00:00Z"), None);
    assert_eq!(ra("retry after 5 fortnights"), None);
    assert_eq!(decide(Kind::RateLimit, 1, ra("retry after 20 minutes")).action, Action::AskingUser);
}

#[test]
fn claudes_stream_errors_classify() {
    for (text, kind) in [
        ("API Error: 401 {\"type\":\"error\",\"error\":{\"type\":\"authentication_error\"}}", Kind::Auth),
        ("API Error: 429 rate_limit_error", Kind::RateLimit),
        ("Claude usage limit reached", Kind::RateLimit),
        ("API Error: 529 overloaded_error", Kind::Upstream),
        ("Credit balance is too low", Kind::Quota),
        ("Invalid API key · Please run /login", Kind::Auth),
        ("invalid_request_error: bad field", Kind::BadRequest),
    ] {
        assert_eq!(classify(text).kind, kind, "{text}");
    }
}

#[test]
fn the_policy_table() {
    let s = Duration::from_secs;
    let wait = |k, n, ra| decide(k, n, ra);
    for k in [Kind::Timeout, Kind::Upstream] {
        assert_eq!(wait(k, 1, None), Decision { action: Action::Retrying, wait: Some(s(30)) });
        assert_eq!(wait(k, 2, None), Decision { action: Action::Retrying, wait: Some(s(120)) });
        assert_eq!(wait(k, 3, None).action, Action::AskingUser);
        assert_eq!(wait(k, 0, None).action, Action::AskingUser);
    }
    // 429: the window, if known and at most 15 minutes.
    assert_eq!(wait(Kind::RateLimit, 1, Some(7)), Decision { action: Action::Retrying, wait: Some(s(7)) });
    assert_eq!(wait(Kind::RateLimit, 1, Some(900)).wait, Some(s(900)));
    assert_eq!(wait(Kind::RateLimit, 1, Some(901)).action, Action::AskingUser);
    assert_eq!(wait(Kind::RateLimit, 1, None).action, Action::AskingUser);
    assert_eq!(wait(Kind::RateLimit, MAX_RATE_WAITS + 1, Some(7)).action, Action::AskingUser);
    for k in [Kind::Auth, Kind::Quota, Kind::BadRequest, Kind::UnexplainedExit] {
        assert_eq!(wait(k, 1, Some(7)), ASK, "{k:?}");
    }
}

const WHO: Ident<'static> = Ident { slot: "w3", session_id: "s_fixture", account: "gateway-fixture" };

fn drive(texts: &[&str]) -> (Outcome, Vec<ProviderError>, Vec<Duration>, usize) {
    let clock = FakeClock::default();
    let mut events = Vec::new();
    let mut calls = 0usize;
    let out = run_with_policy(&clock, &WHO, &mut |e| events.push(e.clone()), || {
        let t = texts.get(calls).copied();
        calls += 1;
        t.map_or(Ok(()), |t| Err(t.to_string()))
    });
    (out, events, clock.slept(), calls)
}

#[test]
fn the_retry_schedule_runs_on_a_fake_clock() {
    // 504, 504, then success: waits 30 s and 2 min, never real time.
    let (out, ev, slept, calls) = drive(&["last status: 504", "last status: 504"]);
    assert_eq!(out, Outcome::Done);
    assert_eq!(slept, vec![Duration::from_secs(30), Duration::from_secs(120)]);
    assert_eq!(calls, 3);
    assert_eq!(ev.iter().map(|e| (e.attempt, e.action)).collect::<Vec<_>>(), vec![(1, Action::Retrying), (2, Action::Retrying)]);

    // Three failures: two retries, then the user is asked and the task is blocked.
    let (out, ev, slept, calls) = drive(&["idle timeout waiting for SSE"; 5]);
    assert_eq!(slept.len(), 2);
    assert_eq!(calls, 3);
    assert_eq!(ev.last().map(|e| e.action), Some(Action::AskingUser));
    assert!(matches!(out, Outcome::Blocked(e) if e.attempt == 3 && e.kind == Kind::Timeout));
}

#[test]
fn a_429_waits_for_its_window_then_asks() {
    let (out, _, slept, _) = drive(&["last status: 429, retry-after: 7", "last status: 429, retry-after: 7", "last status: 429, retry-after: 7"]);
    assert_eq!(slept, vec![Duration::from_secs(7), Duration::from_secs(7)]);
    assert!(matches!(out, Outcome::Blocked(_)));
    // A window past 15 minutes is not waited for.
    let (out, ev, slept, calls) = drive(&["last status: 429, retry-after: 3600"]);
    assert_eq!((slept.len(), calls, ev[0].action), (0, 1, Action::AskingUser));
    assert!(matches!(out, Outcome::Blocked(_)));
}

#[test]
fn no_retry_on_401_402_or_400() {
    for text in ["unexpected status 401 Unauthorized", "unexpected status 402 Payment Required", "unexpected status 400 Bad Request"] {
        let (out, ev, slept, calls) = drive(&[text, text]);
        assert_eq!((calls, slept.len(), ev.len()), (1, 0, 1), "{text}");
        assert_eq!(ev[0].action, Action::AskingUser);
        assert!(matches!(out, Outcome::Blocked(_)));
    }
}

#[test]
fn no_path_ever_changes_the_account() {
    // Every kind, every attempt: each event names the account the worker started with, and the identity is borrowed
    // read-only, so there is nothing to switch.
    let all = ["unexpected status 401", "unexpected status 402", "last status: 429, retry-after: 7", "last status: 502", "last status: 504", "idle timeout waiting for SSE", "unexpected status 400", "nothing typed"];
    for t in all {
        let (_, ev, _, _) = drive(&[t, t, t, t]);
        assert!(!ev.is_empty());
        for e in ev {
            assert_eq!((e.account.as_str(), e.slot.as_str(), e.session_id.as_str()), (WHO.account, WHO.slot, WHO.session_id), "{t}");
            assert_ne!(e.to_json()["action"], "switch_account");
        }
    }
    // The only actions that exist are these two; neither moves a worker.
    assert_eq!(serde_json::to_value(Action::Retrying).unwrap(), "retrying");
    assert_eq!(serde_json::to_value(Action::AskingUser).unwrap(), "asking_user");
}

#[test]
fn an_event_never_carries_an_identifier_a_key_or_a_query() {
    let raw = format!(
        "unexpected status 401 {{\"user_id\":\"{SENTINEL_USER}\"}} cf-ray: {SENTINEL_RAY} key sk-or-v1-FAKEFAKEFAKEFAKE0000 see https://openrouter.ai/api/v1/key?token=SECRETVALUE&x=1"
    );
    let (_, ev, _, _) = drive(&[&raw]);
    let wire = ev[0].to_json().to_string();
    for leak in [SENTINEL_USER, SENTINEL_RAY, "sk-or-v1", "SECRETVALUE", "token="] {
        assert!(!wire.contains(leak), "{leak} leaked: {wire}");
    }
    assert!(wire.contains("provider_error") && wire.contains("401"));
}

#[test]
fn a_long_message_is_cut() {
    let (_, ev, _, _) = drive(&[&format!("unexpected status 401 {}", "x ".repeat(500))]);
    assert!(ev[0].message.chars().count() <= 240);
}

#[test]
fn an_exit_with_no_typed_error_is_unexplained_and_says_why() {
    let tail = vec!["working".to_string(), "Segmentation fault".to_string()];
    let (c, text) = classify_exit(Some(139), None, &tail);
    assert_eq!(c.kind, Kind::UnexplainedExit);
    assert!(text.starts_with("exit code 139: ") && text.contains("Segmentation fault"));
    let (_, text) = classify_exit(None, Some("Killed"), &tail);
    assert!(text.starts_with("ended by signal Killed"));
    // A typed error in the last lines wins.
    let typed = vec!["unexpected status 401 Unauthorized".to_string()];
    assert_eq!(classify_exit(Some(1), None, &typed).0.kind, Kind::Auth);
}
