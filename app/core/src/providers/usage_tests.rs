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
fn the_percentage_is_the_current_limits_spend_not_lifetime_usage() {
    // After a daily reset: 100 spent all time, but only 1 of the current 10 allowance is gone.
    let reset = br#"{"data":{"usage":100.0,"usage_daily":1.0,"limit":10.0,"limit_remaining":9.0}}"#;
    let u = parse_key_response("openrouter", reset, AT).unwrap();
    assert_eq!((u.windows[0].used_pct, u.windows[0].warn), (10, false), "lifetime usage does not drive the window");
    assert_eq!(u.amounts.as_ref().unwrap().usage, 100.0, "the reported figure is still shown as it is");
    // The remaining amount decides: 8.4 of 10 gone is 84% even when usage is far larger or smaller.
    let near = br#"{"data":{"usage":0.5,"limit":10.0,"limit_remaining":1.6}}"#;
    assert_eq!(parse_key_response("openrouter", near, AT).unwrap().windows[0].used_pct, 84);
    // More remaining than the limit (a raised cap) clamps to zero use.
    let over = br#"{"data":{"usage":3.0,"limit":10.0,"limit_remaining":12.0}}"#;
    assert_eq!(parse_key_response("openrouter", over, AT).unwrap().windows[0].used_pct, 0);
}

#[test]
fn without_the_current_limit_figures_there_is_no_percentage() {
    // Spent a lot, no limit set; a limit without the remaining amount; a zero limit: none of these gets a made-up number.
    for body in [
        &br#"{"data":{"usage":100.0,"limit":null,"limit_remaining":null}}"#[..],
        br#"{"data":{"usage":100.0,"limit":10.0}}"#,
        br#"{"data":{"usage":100.0,"limit":10.0,"limit_remaining":null}}"#,
        br#"{"data":{"usage":100.0,"limit":0.0,"limit_remaining":0.0}}"#,
        br#"{"data":{"usage":100.0,"limit_remaining":3.0}}"#,
    ] {
        let u = parse_key_response("openrouter", body, AT).unwrap();
        assert!(u.windows.is_empty(), "{}", String::from_utf8_lossy(body));
        assert_eq!(u.amounts.as_ref().unwrap().usage, 100.0);
    }
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

#[test]
fn the_fingerprint_ignores_when_it_was_read_and_notices_what_it_says() {
    let a = parse_key_response("openrouter", FIXTURE.as_bytes(), "2026-11-09T12:00:00Z").unwrap();
    let later = Usage { read_at: Some("2026-11-09T12:05:00Z".into()), ..a.clone() };
    let list = |u: Usage| UsageList { schema: USAGE_SCHEMA.into(), usage: vec![u] };
    assert_eq!(list(a.clone()).fingerprint(), list(later).fingerprint(), "a re-read of the same figures is not a change");
    let moved = parse_key_response("openrouter", br#"{"data":{"usage":9.0,"limit":10.0,"limit_remaining":1.0}}"#, AT).unwrap();
    assert_ne!(list(a.clone()).fingerprint(), list(moved).fingerprint());
    assert_ne!(list(a).fingerprint(), list(Usage::not_reported("openrouter")).fingerprint());
}

// ---- the Keychain wiring: a fake `security` and a fake `curl`, a temp HOME, never the real Keychain -------------------

const SENTINEL_KEY: &str = "SENTINEL-sk-or-v1-0123456789abcdef";

/// A `security` stand-in over files under `store/`: `find-generic-password -s <svc> -a <acct> [-w]` exits 44 when there is
/// no item, prints the item only with `-w`. Every call's argv and `HOME` are logged.
fn fake_security(d: &std::path::Path) -> PathBuf {
    script(
        d,
        "security",
        &format!(
            "D='{d}'\nprintf '%s|HOME=%s\\n' \"$*\" \"$HOME\" >> \"$D/security.log\"\n[ \"$1\" = find-generic-password ] || exit 1\nsvc=; w=\nshift\nwhile [ $# -gt 0 ]; do case $1 in -s) svc=$2; shift 2;; -w) w=1; shift;; -a) shift 2;; *) shift;; esac; done\n[ -f \"$D/store/$svc\" ] || exit 44\n[ -n \"$w\" ] && /bin/cat \"$D/store/$svc\"\nexit 0",
            d = d.display()
        ),
    )
}

fn wiring_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vajra-usage-wiring-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("store")).unwrap();
    d
}

#[test]
fn only_an_account_with_a_stored_key_is_metered_and_asking_never_prints_the_key() {
    let d = wiring_dir("accounts");
    let sec = fake_security(&d);
    let calls = |d: &std::path::Path| std::fs::read_to_string(d.join("security.log")).unwrap_or_default().lines().count();
    let none = KeychainKeys::new(&sec, &d);
    none.refresh(true);
    assert!(none.accounts().is_empty(), "no key stored: nothing to meter");
    std::fs::write(d.join("store/vajra.provider.openrouter"), format!("{SENTINEL_KEY}\n")).unwrap();
    let keys = KeychainKeys::new(&sec, &d);
    let before = calls(&d);
    assert!(keys.accounts().is_empty(), "before a refresh the list is empty");
    assert_eq!(calls(&d), before, "reading the list never runs security");
    keys.refresh(true);
    assert_eq!(keys.accounts(), ["openrouter"]);
    let log = std::fs::read_to_string(d.join("security.log")).unwrap();
    assert!(log.lines().all(|l| !l.contains(" -w") && !l.contains("SENTINEL")), "the existence check never asks for the password: {log}");
    // Reads cost nothing, and a refresh that is not forced reuses the lookup made a moment ago.
    let before = calls(&d);
    for _ in 0..5 {
        assert_eq!(keys.accounts(), ["openrouter"]);
        keys.refresh(false);
    }
    assert_eq!(calls(&d), before, "no further lookup inside the TTL");
    keys.refresh(true);
    assert!(calls(&d) > before, "a forced refresh looks again");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_key_command_is_the_fixed_security_read_for_a_key_account_only() {
    let d = wiring_dir("argv");
    let sec = fake_security(&d);
    let keys = KeychainKeys::new(&sec, &d);
    let want: Vec<String> = [sec.to_str().unwrap(), "find-generic-password", "-s", "vajra.provider.openrouter", "-a", "openrouter", "-w"].map(String::from).into();
    assert_eq!(keys.argv("openrouter"), Some(want));
    for other in ["claude", "codex", "", "../x", "openrouter -w", "openrouter\n"] {
        assert_eq!(keys.argv(other), None, "{other:?}");
    }
    assert_eq!(keys.home(), Some(d.as_path()));
    // The real build points at the system binary.
    let real = KeychainKeys::new(Path::new(keychain::SECURITY), &d);
    assert_eq!(real.argv("openrouter").unwrap()[..2], ["/usr/bin/security", "find-generic-password"]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_stored_key_reaches_the_endpoint_call_only_through_stdin_and_the_answer_holds_no_key() {
    let d = wiring_dir("e2e");
    std::fs::write(d.join("store/vajra.provider.openrouter"), format!("{SENTINEL_KEY}\n")).unwrap();
    let curl = script(&d, "curl", &format!("echo \"$@\" > {0}/curl-argv.txt\nenv > {0}/curl-env.txt\n/bin/cat > {0}/curl-stdin.txt\nprintf '%s' '{FIXTURE}'", d.display()));
    let src = CurlSource::new(Box::new(KeychainKeys::new(&fake_security(&d), &d))).with_curl(curl);

    src.refresh(true);
    assert_eq!(src.accounts(), ["openrouter"]);
    let body = src.key_info("openrouter").expect("the fake curl answered");
    let usage = parse_key_response("openrouter", &body, AT).unwrap();
    assert_eq!(usage.windows[0].used_pct, 84);

    let read = |f: &str| std::fs::read_to_string(d.join(f)).unwrap();
    assert_eq!(read("curl-stdin.txt"), format!("Authorization: Bearer {SENTINEL_KEY}\n"), "the key goes security -> curl's stdin");
    for f in ["curl-argv.txt", "curl-env.txt"] {
        assert!(!read(f).contains("SENTINEL"), "{f} holds the key");
    }
    assert!(!serde_json::to_string(&usage).unwrap().contains("SENTINEL"));
    // The read is exactly the I3 argv, run with the given HOME and nothing else of the environment.
    let reads: Vec<String> = read("security.log").lines().filter(|l| l.contains(" -w")).map(String::from).collect();
    assert_eq!(reads, [format!("find-generic-password -s vajra.provider.openrouter -a openrouter -w|HOME={}", d.display())]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn no_stored_key_means_no_call() {
    let d = wiring_dir("nokey");
    let curl = script(&d, "curl", &format!("echo called > {}/curl-ran.txt", d.display()));
    let src: Box<dyn UsageSource> = Box::new(CurlSource::new(Box::new(KeychainKeys::new(&fake_security(&d), &d))).with_curl(curl));
    // The meter reads only the accounts the source lists, so with no stored key the transfer tool is never started.
    let m = Meter::default();
    src.refresh(true);
    let list: Vec<Usage> = src.accounts().iter().map(|a| m.read(a, true, Instant::now(), AT, src.as_ref())).collect();
    assert!(list.is_empty());
    assert!(!d.join("curl-ran.txt").exists());
    let _ = std::fs::remove_dir_all(&d);
}

/// A `security` that takes two seconds to answer, as a locked or prompting Keychain can.
fn slow_security(d: &std::path::Path) -> PathBuf {
    script(d, "security", &format!("echo \"$*\" >> '{}/slow.log'\n/bin/sleep 2\nexit 0", d.display()))
}

#[test]
fn reading_the_list_never_waits_for_a_slow_lookup_and_a_second_refresh_does_not_queue() {
    let d = wiring_dir("slow");
    let keys = Arc::new(KeychainKeys::new(&slow_security(&d), &d));
    let worker = {
        let keys = keys.clone();
        std::thread::spawn(move || keys.refresh(true))
    };
    std::thread::sleep(Duration::from_millis(300));
    let start = Instant::now();
    assert!(keys.accounts().is_empty(), "the lookup is still running: the last list (empty) is served");
    keys.refresh(false); // a forced one goes round again instead: see the test below
    assert!(start.elapsed() < Duration::from_millis(500), "neither a read nor a second refresh waited for security: {:?}", start.elapsed());
    worker.join().unwrap();
    assert_eq!(keys.accounts(), ["openrouter"], "the finished lookup is what is served next");
    let lookups = std::fs::read_to_string(d.join("slow.log")).unwrap().lines().count();
    assert_eq!(lookups, 1, "one security call, not two");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_key_saved_while_a_slow_lookup_is_running_still_appears() {
    let d = wiring_dir("dirty");
    // The answer is decided when the lookup starts, then it is slow: like a lookup that checked before the key was saved.
    let sec = script(&d, "security", &format!("if [ -f '{0}/saved' ]; then r=0; else r=44; fi\n/bin/sleep 1\nexit $r", d.display()));
    let keys = Arc::new(KeychainKeys::new(&sec, &d));
    let first = {
        let keys = keys.clone();
        std::thread::spawn(move || keys.refresh(false))
    };
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(d.join("saved"), "").unwrap();
    let start = Instant::now();
    keys.refresh(true); // what key_changed does: it finds the lookup running and must not be lost
    assert!(start.elapsed() < Duration::from_millis(500), "the forced refresh did not wait");
    first.join().unwrap();
    assert_eq!(keys.accounts(), ["openrouter"], "the running lookup went round again and saw the saved key");
    let _ = std::fs::remove_dir_all(&d);
}
