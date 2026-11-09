use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::*;

const AT: &str = "2026-11-09T12:00:00Z";
/// A realistic key-endpoint answer. The label is a canary: it must never reach an answer.
const FIXTURE: &str = r#"{"data":{"label":"LABEL-CANARY-sk-or-v1-abc","usage":8.4,"usage_daily":1.25,"limit":10.0,"limit_remaining":1.6,"is_free_tier":false,"rate_limit":{"requests":10}}}"#;

#[test]
fn the_parser_keeps_only_the_four_figures_and_the_label_never_reaches_the_answer() {
    let u = parse_key_response("openrouter", FIXTURE.as_bytes(), AT).unwrap();
    assert_eq!(u.amounts, Some(Amounts { usage: 8.4, usage_daily: Some(1.25), limit: Some(10.0), limit_remaining: Some(1.6) }));
    assert_eq!((u.source.as_str(), u.read_at.as_deref()), ("gateway_key", Some(AT)));
    let json = serde_json::to_string(&u).unwrap();
    assert!(!json.contains("CANARY") && !json.contains("label") && !json.contains("is_free_tier") && !json.contains("rate_limit"), "{json}");
}

#[test]
fn eighty_percent_sets_the_flag_and_a_reset_time_is_carried_only_if_given() {
    // 8.4 of 10 is 84%.
    let w = &parse_key_response("openrouter", FIXTURE.as_bytes(), AT).unwrap().windows[0];
    assert_eq!((w.kind.as_str(), w.used_pct, w.warn, w.resets_at.clone()), ("credit", 84, true, None));
    assert!(!Window::new("weekly", 79.4, None).warn);
    let at = Window::new("weekly", 80.0, Some("2026-11-12T09:00:00Z".into()));
    assert_eq!((at.used_pct, at.warn, at.resets_at.as_deref()), (80, true, Some("2026-11-12T09:00:00Z")));
    assert_eq!(Window::new("5h", 250.0, None).used_pct, 100, "clamped");
    assert_eq!(Window::new("5h", f64::NAN, None).used_pct, 0);
}

#[test]
fn without_a_spending_limit_there_is_no_percentage_and_odd_answers_read_as_not_reported() {
    let no_limit = br#"{"data":{"usage":3.0,"usage_daily":0.5,"limit":null,"limit_remaining":null}}"#;
    let u = parse_key_response("openrouter", no_limit, AT).unwrap();
    assert!(u.windows.is_empty() && u.amounts.as_ref().unwrap().usage == 3.0, "a price is shown, never a made-up percent");
    for bad in [&br#"{"data":{"usage":-1.0}}"#[..], br#"{"data":{"limit":10}}"#, br#"{"data":{"usage":"3"}}"#, b"not json", br#"{"error":"nope"}"#, b""] {
        assert!(parse_key_response("openrouter", bad, AT).is_none(), "{}", String::from_utf8_lossy(bad));
    }
    assert!(parse_key_response("openrouter", &vec![b' '; 70 * 1024], AT).is_none());
}

#[derive(Default)]
struct Counting {
    calls: AtomicUsize,
    answer: Mutex<Option<&'static str>>,
}
impl UsageSource for Arc<Counting> {
    fn accounts(&self) -> Vec<String> {
        vec!["openrouter".into()]
    }
    fn key_info(&self, _: &str) -> Option<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer.lock().unwrap().map(|s| s.as_bytes().to_vec())
    }
}

#[test]
fn no_call_is_made_unless_the_page_is_open_or_a_run_is_active() {
    let (m, src, t0) = (Meter::default(), Arc::new(Counting::default()), Instant::now());
    *src.answer.lock().unwrap() = Some(FIXTURE);
    let u = m.read("openrouter", false, t0, AT, &src);
    assert_eq!((src.calls.load(Ordering::SeqCst), u.source.as_str()), (0, "none"), "inactive: Not reported, no call");
    m.read("openrouter", true, t0, AT, &src);
    assert_eq!(src.calls.load(Ordering::SeqCst), 1);
    // Closed again later: the last answer, still no call.
    let later = m.read("openrouter", false, t0 + Duration::from_secs(600), AT, &src);
    assert_eq!((src.calls.load(Ordering::SeqCst), later.source.as_str()), (1, "gateway_key"));
}

#[test]
fn reads_are_limited_to_one_a_minute_and_a_failed_read_keeps_the_last_answer() {
    let (m, src, t0) = (Meter::default(), Arc::new(Counting::default()), Instant::now());
    *src.answer.lock().unwrap() = Some(FIXTURE);
    m.read("openrouter", true, t0, "first", &src);
    m.read("openrouter", true, t0 + Duration::from_secs(59), "x", &src);
    assert_eq!(src.calls.load(Ordering::SeqCst), 1, "inside the minute: the cache");
    // The endpoint fails after the minute: the old reading stays, with its own read_at, and the attempt counts.
    *src.answer.lock().unwrap() = None;
    let kept = m.read("openrouter", true, t0 + Duration::from_secs(61), "second", &src);
    assert_eq!((src.calls.load(Ordering::SeqCst), kept.read_at.as_deref(), kept.source.as_str()), (2, Some("first"), "gateway_key"));
    m.read("openrouter", true, t0 + Duration::from_secs(100), "third", &src);
    assert_eq!(src.calls.load(Ordering::SeqCst), 2, "the failed attempt was rate limited too");
    // A first read that fails is Not reported.
    let fresh = Meter::default().read("openrouter", true, t0, AT, &src);
    assert_eq!((fresh.source.as_str(), fresh.windows.len(), fresh.read_at), ("none", 0, None));
}

struct Keys(Vec<String>);
impl KeyCommands for Keys {
    fn accounts(&self) -> Vec<String> {
        vec!["openrouter".into()]
    }
    fn argv(&self, _: &str) -> Option<Vec<String>> {
        Some(self.0.clone())
    }
}

fn script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[test]
fn the_key_goes_from_its_command_into_curls_stdin_and_never_into_argv_or_the_answer() {
    let d = std::env::temp_dir().join(format!("vajra-usage-curl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let key_cmd = script(&d, "keycmd", "echo SENTINEL-KEY-123");
    let curl = script(&d, "curl", &format!("echo \"$@\" > {0}/argv.txt\n/bin/cat > {0}/stdin.txt\nprintf '%s' '{FIXTURE}'", d.display()));
    let src = CurlSource::new(Box::new(Keys(vec![key_cmd.to_string_lossy().into()]))).with_curl(curl);
    let body = src.key_info("openrouter").expect("the fake curl answered");
    assert_eq!(String::from_utf8(body.clone()).unwrap(), FIXTURE);
    let (argv, stdin) = (std::fs::read_to_string(d.join("argv.txt")).unwrap(), std::fs::read_to_string(d.join("stdin.txt")).unwrap());
    assert_eq!(stdin, "Authorization: Bearer SENTINEL-KEY-123\n", "the key reaches curl only through stdin");
    assert!(!argv.contains("SENTINEL"), "never in argv: {argv}");
    assert!(argv.starts_with("-q "), "{argv}");
    assert!(argv.contains("-H @-") && argv.contains("--proto =https") && argv.contains("https://openrouter.ai/api/v1/key"), "{argv}");
    // The parsed answer holds neither the key nor the label.
    let json = serde_json::to_string(&parse_key_response("openrouter", &body, AT).unwrap()).unwrap();
    assert!(!json.contains("SENTINEL") && !json.contains("CANARY"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_failing_curl_or_key_command_reads_as_nothing() {
    let d = std::env::temp_dir().join(format!("vajra-usage-fail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let key_cmd: String = script(&d, "keycmd", "echo K").to_string_lossy().into();
    let failing = script(&d, "curl-fail", "/bin/cat >/dev/null; exit 22");
    assert!(CurlSource::new(Box::new(Keys(vec![key_cmd.clone()]))).with_curl(failing).key_info("openrouter").is_none());
    assert!(CurlSource::new(Box::new(Keys(vec![key_cmd]))).with_curl(d.join("missing-curl")).key_info("openrouter").is_none());
    assert!(NoSource.key_info("openrouter").is_none() && NoSource.accounts().is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_call_that_hangs_is_killed_at_the_deadline_and_reads_as_nothing() {
    let d = std::env::temp_dir().join(format!("vajra-usage-hang-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let key_cmd: String = script(&d, "keycmd", "echo K").to_string_lossy().into();
    let hang = script(&d, "curl-hang", "exec /bin/sleep 30");
    let src = CurlSource::new(Box::new(Keys(vec![key_cmd]))).with_curl(hang).with_deadline(Duration::from_millis(300));
    let start = Instant::now();
    assert!(src.key_info("openrouter").is_none());
    assert!(start.elapsed() < Duration::from_secs(10), "killed at the deadline, not left to run");
    let _ = std::fs::remove_dir_all(&d);
}
