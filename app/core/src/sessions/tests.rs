use super::*;
use crate::crew::redact::Redactor;
use crate::bus::{Bus, Config};
use std::ffi::OsString;
use std::sync::Arc;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("vajra-sessions-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn setup(tag: &str) -> (Supervisor, Arc<Bus>, std::path::PathBuf) {
    let dir = tmpdir(tag);
    let bus = Arc::new(Bus::open(&dir.join("spill"), Config::default()).unwrap());
    (Supervisor::new(bus.clone(), None), bus, dir)
}

fn spec(argv: &[&str], cwd: &std::path::Path) -> SpawnSpec {
    SpawnSpec {
        argv: argv.iter().map(|s| s.to_string()).collect(),
        cwd: cwd.to_path_buf(),
        rows: 24,
        cols: 80,
        redact: Redactor::default(),
        extra_env: Vec::new(),
    }
}

fn lines(bus: &Bus, stream: &str) -> Vec<String> {
    bus.tail(stream, 0, usize::MAX)
        .lines
        .iter()
        .map(|l| l.text.to_string())
        .collect()
}

/// Poll the bus until `pred` holds on the stream's lines.
fn wait_lines(bus: &Bus, stream: &str, pred: impl Fn(&[String]) -> bool) -> Vec<String> {
    let start = Instant::now();
    loop {
        let l = lines(bus, stream);
        if pred(&l) || start.elapsed() > WAIT {
            return l;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn sessions_spawn_argv_reader_feeds_bus_and_reaps() {
    let (sup, bus, dir) = setup("echo");
    sup.spawn("s-echo", spec(&["echo", "hello", "world"], &dir))
        .unwrap();
    let exit = sup.wait("s-echo", WAIT).unwrap().expect("exited");
    assert_eq!(exit.code, 0);
    let l = wait_lines(&bus, "s-echo", |l| !l.is_empty());
    assert_eq!(l, vec!["hello world".to_string()]);
    assert!(!sup.is_running("s-echo").unwrap());
}

#[test]
fn sessions_shell_metacharacters_are_one_argv_element() {
    let (sup, bus, dir) = setup("meta");
    let marker = dir.join("pwned");
    let evil = format!("a; touch {} && echo $(id) `id` | cat", marker.display());
    sup.spawn("s-meta", spec(&["echo", &evil], &dir)).unwrap();
    sup.wait("s-meta", WAIT).unwrap().expect("exited");
    let l = wait_lines(&bus, "s-meta", |l| !l.is_empty());
    assert_eq!(l, vec![evil.clone()]);
    assert!(!marker.exists(), "a shell ran the argument");
}

#[test]
fn sessions_env_is_scrubbed() {
    // Cargo sets this for the test process; the child must not see it.
    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
    let (sup, bus, dir) = setup("env");
    sup.spawn("s-env", spec(&["/usr/bin/env"], &dir)).unwrap();
    sup.wait("s-env", WAIT).unwrap().expect("exited");
    let l = wait_lines(&bus, "s-env", |l| l.iter().any(|x| x.starts_with("TERM=")));
    let keys: Vec<&str> = l
        .iter()
        .filter_map(|x| x.split_once('=').map(|kv| kv.0))
        .collect();
    assert!(!keys.is_empty());
    for k in &keys {
        assert!(ENV_ALLOW.contains(k), "leaked env var {k}");
    }
    assert!(keys.contains(&"PATH") && keys.contains(&"TERM"));
}

#[test]
fn sessions_scrubbed_env_pure() {
    let src = |k: &str| match k {
        "PATH" => Some(OsString::from("/usr/bin:/bin")),
        "HOME" => Some(OsString::from("/home/u")),
        "AWS_SECRET_ACCESS_KEY" => Some(OsString::from("x")),
        _ => None,
    };
    let env = scrubbed_env(src);
    let keys: Vec<&str> = env.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, vec!["PATH", "HOME", "TERM"]);
    assert_eq!(env[2].1, OsString::from(DEFAULT_TERM));
}

#[test]
fn sessions_write_refused_unless_take_over() {
    let (sup, bus, dir) = setup("write");
    sup.spawn("s-cat", spec(&["cat"], &dir)).unwrap();
    assert!(!sup.take_over("s-cat").unwrap());
    assert_eq!(sup.write("s-cat", b"nope\n"), Err(SessionError::ReadOnly));

    sup.set_take_over("s-cat", true).unwrap();
    sup.write("s-cat", b"typed-by-user\n").unwrap();
    let l = wait_lines(&bus, "s-cat", |l| {
        l.iter().filter(|x| *x == "typed-by-user").count() >= 2
    });
    // Echoed by the tty and again by cat; "nope" never reached the PTY.
    assert!(
        l.iter().filter(|x| *x == "typed-by-user").count() >= 2,
        "{l:?}"
    );
    assert!(!l.iter().any(|x| x.contains("nope")), "{l:?}");

    sup.set_take_over("s-cat", false).unwrap();
    assert_eq!(sup.write("s-cat", b"again\n"), Err(SessionError::ReadOnly));
    sup.kill("s-cat").unwrap();
    assert!(sup.wait("s-cat", WAIT).unwrap().is_some());
}

#[test]
fn sessions_kill_stops_and_reaps() {
    let (sup, _bus, dir) = setup("kill");
    sup.spawn("s-sleep", spec(&["sleep", "60"], &dir)).unwrap();
    assert!(sup.is_running("s-sleep").unwrap());
    let t = Instant::now();
    sup.kill("s-sleep").unwrap();
    let exit = sup
        .wait("s-sleep", WAIT)
        .unwrap()
        .expect("reaped after kill");
    assert!(t.elapsed() < Duration::from_secs(5));
    assert!(exit.signal.is_some() || exit.code != 0, "{exit:?}");
    assert!(!sup.is_running("s-sleep").unwrap());
    // Killing an exited session is a no-op, not a stray signal.
    sup.kill("s-sleep").unwrap();
}

#[test]
fn sessions_resize_reaches_tty() {
    let (sup, bus, dir) = setup("resize");
    sup.spawn("s-stty", spec(&["sleep", "0.5"], &dir)).unwrap();
    sup.resize("s-stty", 40, 120).unwrap();
    assert_eq!(sup.size("s-stty").unwrap(), (40, 120));
    sup.wait("s-stty", WAIT).unwrap();
    // stty reads the size from its controlling tty: spawn one at a set size.
    let mut s = spec(&["stty", "size"], &dir);
    s.rows = 33;
    s.cols = 101;
    sup.spawn("s-stty2", s).unwrap();
    sup.wait("s-stty2", WAIT).unwrap();
    let l = wait_lines(&bus, "s-stty2", |l| !l.is_empty());
    assert_eq!(l, vec!["33 101".to_string()]);
    assert_eq!(sup.resize("s-stty2", 0, 80), Err(SessionError::InvalidSize));
}

#[test]
fn sessions_output_is_sanitized_before_the_bus() {
    let (sup, bus, dir) = setup("ansi");
    let evil = "\x1b]52;c;cm0gLXJmIH4=\x07\x1b]0;title\x07\x1b[31mred\x1b[0m";
    sup.spawn("s-ansi", spec(&["printf", "%s\\n", evil], &dir))
        .unwrap();
    sup.wait("s-ansi", WAIT).unwrap();
    let l = wait_lines(&bus, "s-ansi", |l| !l.is_empty());
    assert_eq!(l, vec!["\x1b[31mred\x1b[0m".to_string()]);
}

#[test]
fn sessions_partial_last_line_flushed_on_exit() {
    let (sup, bus, dir) = setup("partial");
    sup.spawn("s-part", spec(&["printf", "a\\nno-newline"], &dir))
        .unwrap();
    sup.wait("s-part", WAIT).unwrap();
    let l = wait_lines(&bus, "s-part", |l| l.len() >= 2);
    assert_eq!(l, vec!["a".to_string(), "no-newline".to_string()]);
}

#[test]
fn sessions_spawn_rejects_bad_input() {
    let (sup, _bus, dir) = setup("bad");
    assert_eq!(
        sup.spawn("s-x", spec(&[], &dir)),
        Err(SessionError::EmptyArgv)
    );
    assert_eq!(
        sup.spawn("s-x", spec(&["echo", "a\0b"], &dir)),
        Err(SessionError::NulInArgv)
    );
    assert_eq!(
        sup.spawn("../x", spec(&["echo"], &dir)),
        Err(SessionError::InvalidId)
    );
    assert_eq!(
        sup.spawn("s-x", spec(&["echo"], &dir.join("missing"))),
        Err(SessionError::BadCwd)
    );
    assert!(matches!(
        sup.spawn("s-x", spec(&["no-such-binary-vajra"], &dir)),
        Err(SessionError::Spawn(_))
    ));
    sup.spawn("s-dup", spec(&["sleep", "5"], &dir)).unwrap();
    assert_eq!(
        sup.spawn("s-dup", spec(&["echo"], &dir)),
        Err(SessionError::AlreadyRunning)
    );
    assert_eq!(sup.write("nope", b"x"), Err(SessionError::NotFound));
}

#[test]
fn sessions_respawn_after_exit_reuses_id() {
    let (sup, bus, dir) = setup("respawn");
    sup.spawn("s-re", spec(&["echo", "one"], &dir)).unwrap();
    sup.wait("s-re", WAIT).unwrap();
    wait_lines(&bus, "s-re", |l| !l.is_empty());
    sup.spawn("s-re", spec(&["echo", "two"], &dir)).unwrap();
    sup.wait("s-re", WAIT).unwrap();
    let l = wait_lines(&bus, "s-re", |l| l.len() >= 2);
    assert_eq!(l, vec!["one".to_string(), "two".to_string()]);
}

#[test]
fn sessions_shutdown_kills_and_reaps_all() {
    let (sup, _bus, dir) = setup("quit");
    sup.spawn("s-a", spec(&["sleep", "60"], &dir)).unwrap();
    sup.spawn("s-b", spec(&["cat"], &dir)).unwrap();
    let pids: Vec<u32> = ["s-a", "s-b"]
        .iter()
        .map(|id| sup.pid(id).unwrap())
        .collect();
    let t = Instant::now();
    sup.shutdown();
    assert!(t.elapsed() < Duration::from_secs(5));
    for id in ["s-a", "s-b"] {
        assert!(!sup.is_running(id).unwrap());
    }
    for pid in pids {
        // Reaped: the pid no longer names a child of ours.
        let r = unsafe { libc::waitpid(pid as i32, std::ptr::null_mut(), libc::WNOHANG) };
        assert_eq!(r, -1);
    }
}

#[test]
fn sessions_drop_supervisor_kills_children() {
    let (sup, _bus, dir) = setup("drop");
    sup.spawn("s-d", spec(&["sleep", "60"], &dir)).unwrap();
    let pid = sup.pid("s-d").unwrap() as i32;
    drop(sup);
    let r = unsafe { libc::kill(pid, 0) };
    assert_eq!(r, -1, "child survived supervisor drop");
}

#[test]
fn sessions_build_run_argv_spawns_as_argv() {
    // The launch argv goes to spawn unchanged; use echo as a stand-in for
    // claude so the test is harmless.
    let (sup, bus, dir) = setup("launch");
    let input = argv::RunInput {
        ask: "fix it; rm -rf / $(whoami)".into(),
        max_rounds: 1,
        base: "ea73cb05d639b945e21c667e17103e56404861a8".into(),
        max_in_flight: 2,
        campaign: None,
        provider: None,
    };
    let mut a = argv::build_run_argv(&input, "0123456789abcdef0123456789abcdef").unwrap();
    a[0] = "echo".into();
    let argv: Vec<&str> = a.iter().map(String::as_str).collect();
    sup.spawn("s-launch", spec(&argv, &dir)).unwrap();
    sup.wait("s-launch", WAIT).unwrap();
    let l = wait_lines(&bus, "s-launch", |l| !l.is_empty());
    assert_eq!(l, vec![a[1..].join(" ")]);
}

#[test]
fn sessions_redact_the_steer_nonce_from_pty_lines() {
    // Claude's TUI echoes the launch prompt, which carries the nonce; it may wrap mid-nonce.
    let n = "0123456789abcdef0123456789abcdef";
    let sha = "a".repeat(40);
    let (sup, bus, dir) = setup("redact");
    let script = format!("printf 'steer_nonce: {n} ok\\n{}\\n{}\\ncommit {sha}\\n'", &n[..20], &n[20..]);
    let mut sp = spec(&["/bin/sh", "-c", &script], &dir);
    sp.redact = Redactor::new(Some(n));
    sup.spawn("s-redact", sp).unwrap();
    sup.wait("s-redact", WAIT).unwrap();
    let l = wait_lines(&bus, "s-redact", |l| l.iter().any(|x| x.starts_with("commit")));
    let all = l.join("\n");
    assert!(!all.contains(n) && !all.contains(&n[..20]) && !all.contains(&n[20..]), "{all}");
    assert!(all.contains("steer_nonce: [redacted] ok"), "{all}");
    assert!(all.contains(&format!("commit {sha}")), "unrelated hex stays: {all}");
}

#[test]
fn sessions_redact_a_nonce_split_by_an_sgr_sequence() {
    // Review P2: `ansi::sanitize` keeps SGR, and the terminal draws the whole nonce even though the bytes between its
    // halves are an escape. The literal replace used to miss it.
    let n = "0123456789abcdef0123456789abcdef";
    let sha = "b".repeat(40);
    let (sup, bus, dir) = setup("redact-sgr");
    let script = format!(
        "printf 'steer {}\\033[31m{} ok\\n{}\\033[1m{}\\n\\033[32mgreen text\\033[0m\\ncommit {sha}\\n'",
        &n[..16], &n[16..], &n[..20], &n[20..]
    );
    let mut sp = spec(&["/bin/sh", "-c", &script], &dir);
    sp.redact = Redactor::new(Some(n));
    sup.spawn("s-redact-sgr", sp).unwrap();
    sup.wait("s-redact-sgr", WAIT).unwrap();
    let l = wait_lines(&bus, "s-redact-sgr", |l| l.iter().any(|x| x.starts_with("commit")));
    let all = l.join("\n");
    // Strip the escapes to ask what a terminal would show: no nonce, and no run of its characters.
    let shown = crate::sessions::ansi::sanitize(&all);
    let visible: String = {
        let mut o = String::new();
        let mut it = shown.chars();
        while let Some(c) = it.next() {
            if c == '\u{1b}' {
                for d in it.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&d) && d != '[' {
                        break;
                    }
                }
            } else {
                o.push(c);
            }
        }
        o
    };
    assert!(!visible.contains(n) && !visible.contains(&n[..16]) && !visible.contains(&n[16..]), "{visible}");
    assert!(visible.contains("steer [redacted] ok"), "{visible}");
    assert!(all.contains("\u{1b}[32mgreen text"), "colours of unrelated text are kept: {all:?}");
    assert!(visible.contains(&format!("commit {sha}")), "unrelated hex stays: {visible}");
}

#[test]
fn redact_handles_escapes_and_invisible_characters_inside_a_nonce() {
    let n = "0123456789abcdef0123456789abcdef";
    let r = Redactor::new(Some(n));
    let with = |mid: &str| format!("a {}{}{} b", &n[..10], mid, &n[10..]);
    for mid in ["\u{1b}[31m", "\u{1b}[0m\u{1b}[1;32m", "\u{200b}", "\u{202e}", "\u{1b}[31m\u{200b}\u{1b}[0m"] {
        let out = r.redact(with(mid));
        let flat: String = out.chars().filter(|c| c.is_ascii_hexdigit() || c.is_alphabetic()).collect();
        assert!(!flat.contains(&n[..10]) && !flat.contains(&n[10..]), "{mid:?} -> {out:?}");
        assert!(out.contains("[redacted]") && out.starts_with("a ") && out.ends_with(" b"), "{mid:?} -> {out:?}");
    }
    // No nonce: nothing changes, escapes included.
    assert_eq!(r.redact("x \u{1b}[31mred\u{1b}[0m y".to_string()), "x \u{1b}[31mred\u{1b}[0m y");
}

#[test]
fn take_over_changes_are_serialised_with_delivery() {
    // A delivery holds the writer lock around its mode check. Turning Take over on must wait for it, so an approval
    // turn that passed its check is fully written before the human has the keyboard, and a later one sees the new mode.
    let (sup, _bus, dir) = setup("toggle-lock");
    sup.spawn("s-cat", spec(&["cat"], &dir)).unwrap();
    let held = sup.get("s-cat").unwrap();
    let guard = lock(&held.writer);
    let done = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|sc| {
        let t = sc.spawn(|| {
            sup.set_take_over("s-cat", true).unwrap();
            done.store(true, Ordering::SeqCst);
        });
        std::thread::sleep(Duration::from_millis(200));
        assert!(!done.load(Ordering::SeqCst), "the toggle waited for the delivery in progress");
        assert!(!sup.take_over("s-cat").unwrap(), "and the mode did not change under it");
        drop(guard);
        t.join().unwrap();
    });
    assert!(sup.take_over("s-cat").unwrap());
    let turn = ApprovedPlanTurn::new("20261106-0911", 0, "push").unwrap();
    assert_eq!(sup.submit_fixed_turn("s-cat", &turn), Err(SessionError::TakeOverActive));
    sup.kill("s-cat").unwrap();
}

/// A gateway launch through the real spawn path with a fake `codex`: the child sees `ENV_ALLOW` plus `CODEX_HOME` and
/// nothing else, with no key, and its stdin is not the terminal.
#[test]
fn sessions_gateway_launch_env_is_allowlist_plus_codex_home_and_stdin_is_null() {
    use std::os::unix::fs::PermissionsExt;
    let (sup, bus, dir) = setup("gw");
    let fake = dir.join("codex");
    std::fs::write(
        &fake,
        "#!/bin/sh\nif [ -t 0 ]; then echo STDIN=tty; else echo STDIN=notty; fi\nif read x; then echo READ=data; else echo READ=eof; fi\necho ARGC=$#\nexec /usr/bin/env\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let home = dir.join("home-for-codex");
    let mut argv = crate::sessions::argv::build_gateway_argv("do it", "deepseek/deepseek-v4-flash", Some("low")).unwrap();
    assert_eq!(argv[4], "codex");
    argv[4] = fake.display().to_string();
    let mut s = spec(&["true"], &dir);
    s.argv = argv;
    s.extra_env = vec![("CODEX_HOME", home.clone().into())];
    sup.spawn("s-gw", s).unwrap();
    sup.wait("s-gw", WAIT).unwrap().expect("exited");
    let l = wait_lines(&bus, "s-gw", |l| l.iter().any(|x| x.starts_with("TERM=")));
    assert!(l.contains(&"STDIN=notty".to_string()) && l.contains(&"READ=eof".to_string()), "{l:?}");
    let names: Vec<&str> = l.iter().filter_map(|x| x.split_once('=').map(|kv| kv.0)).filter(|k| !matches!(*k, "STDIN" | "READ" | "ARGC")).collect();
    // The shell that wraps the fake adds its own bookkeeping variables; nothing else may appear.
    for k in &names {
        assert!(ENV_ALLOW.contains(k) || *k == "CODEX_HOME" || ["PWD", "OLDPWD", "SHLVL", "_"].contains(k), "unexpected env var {k}");
    }
    assert!(l.contains(&format!("CODEX_HOME={}", home.display())), "{l:?}");
    for k in crate::adapter::CODEX_KEYS.iter().chain(crate::adapter::CLAUDE_KEYS.iter()) {
        assert!(!names.contains(k), "{k} leaked");
    }
}

#[test]
fn sessions_extra_env_outside_the_allowlist_refuses_the_spawn() {
    let (sup, _bus, dir) = setup("badenv");
    for env in [
        vec![("OPENAI_API_KEY", OsString::from("sentinel-not-a-key"))],
        vec![("CODEX_HOME", OsString::from("/a")), ("CODEX_HOME", OsString::from("/b"))],
        vec![("TMPDIR", OsString::new())],
    ] {
        let mut s = spec(&["/usr/bin/env"], &dir);
        s.extra_env = env;
        assert_eq!(sup.spawn("s-badenv", s), Err(SessionError::BadEnv));
    }
    let mut s = spec(&["/usr/bin/env"], &dir);
    s.extra_env = vec![("TMPDIR", dir.clone().into()), ("CLAUDE_CONFIG_DIR", dir.join("cfg").into())];
    sup.spawn("s-okenv", s).unwrap();
}

#[test]
fn sessions_without_a_codex_home_never_set_one() {
    let (sup, bus, dir) = setup("nohome");
    sup.spawn("s-nh", spec(&["/usr/bin/env"], &dir)).unwrap();
    sup.wait("s-nh", WAIT).unwrap().expect("exited");
    let l = wait_lines(&bus, "s-nh", |l| l.iter().any(|x| x.starts_with("TERM=")));
    assert!(!l.iter().any(|x| x.starts_with("CODEX_HOME=")));
}

fn error_sink() -> (ErrorSink, Arc<Mutex<Vec<ProviderError>>>) {
    let got = Arc::new(Mutex::new(Vec::new()));
    let g = got.clone();
    (Arc::new(move |e: &ProviderError| lock(&g).push(e.clone())), got)
}

fn tag() -> ProviderTag {
    ProviderTag { slot: "w1".into(), session_id: "s_fixture".into(), account: "gateway-fixture".into(), attempt: 1 }
}

fn wait_events(got: &Arc<Mutex<Vec<ProviderError>>>, n: usize) -> Vec<ProviderError> {
    let start = Instant::now();
    while lock(got).len() < n && start.elapsed() < WAIT {
        std::thread::sleep(Duration::from_millis(20));
    }
    lock(got).clone()
}

#[test]
fn a_worker_that_dies_with_no_error_is_reported_as_unexplained() {
    let (sup, _bus, dir) = setup("unexplained");
    let (sink, got) = error_sink();
    sup.set_error_sink(sink);
    sup.spawn_tagged("s-dead", spec(&["sh", "-c", "echo crashed hard; exit 3"], &dir), Some(tag())).unwrap();
    let ev = wait_events(&got, 1);
    assert_eq!(ev.len(), 1);
    assert_eq!((ev[0].kind, ev[0].action, ev[0].account.as_str()), (errors::Kind::UnexplainedExit, errors::Action::AskingUser, "gateway-fixture"));
    assert!(ev[0].message.contains("exit code 3") && ev[0].message.contains("crashed hard"), "{}", ev[0].message);
}

#[test]
fn a_typed_error_in_the_last_lines_beats_unexplained() {
    let (sup, _bus, dir) = setup("typed");
    let (sink, got) = error_sink();
    sup.set_error_sink(sink);
    sup.spawn_tagged("s-401", spec(&["sh", "-c", "echo 'unexpected status 401 Unauthorized'; exit 1"], &dir), Some(tag())).unwrap();
    let ev = wait_events(&got, 1);
    assert_eq!((ev[0].kind, ev[0].http_status), (errors::Kind::Auth, Some(401)));
}

#[test]
fn a_clean_exit_an_untagged_worker_and_our_own_kill_report_nothing() {
    let (sup, _bus, dir) = setup("quiet");
    let (sink, got) = error_sink();
    sup.set_error_sink(sink);
    sup.spawn_tagged("s-ok", spec(&["sh", "-c", "exit 0"], &dir), Some(tag())).unwrap();
    sup.spawn("s-untagged", spec(&["sh", "-c", "exit 4"], &dir)).unwrap();
    sup.spawn_tagged("s-killed", spec(&["sleep", "30"], &dir), Some(tag())).unwrap();
    sup.kill("s-killed").unwrap();
    for id in ["s-ok", "s-untagged", "s-killed"] {
        assert!(sup.wait(id, WAIT).unwrap().is_some());
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(lock(&got).is_empty(), "{:?}", lock(&got));
}

#[test]
fn a_serialized_codex_error_event_is_decoded_classified_and_redacted_on_exit() {
    let (sup, _bus, dir) = setup("codex-event");
    let (sink, got) = error_sink();
    sup.set_error_sink(sink);
    let event = serde_json::json!({
        "type": "turn.failed",
        "error": {"message": "unexpected status 401 Unauthorized: {\"error\":{\"code\":401,\"user_id\":\"user_SENTINEL0000TESTONLY\"}} cf-ray: 0000deadbeef0000-XXX"}
    });
    let script = dir.join("emit.sh");
    std::fs::write(&script, format!("#!/bin/sh\ncat <<'EOF'\n{event}\nEOF\nexit 1\n")).unwrap();
    sup.spawn_tagged("s-codex", spec(&["sh", script.to_str().unwrap()], &dir), Some(tag())).unwrap();
    let ev = wait_events(&got, 1);
    assert_eq!((ev[0].kind, ev[0].http_status), (errors::Kind::Auth, Some(401)));
    let wire = ev[0].to_json().to_string();
    assert!(!wire.contains("SENTINEL0000") && !wire.contains("0000deadbeef"), "{wire}");
    // The raw escaped line (no decoding) is redacted too.
    let raw = event.to_string();
    assert!(!crate::crew::redact::redact_provider_text(&raw).contains("SENTINEL0000"));
}

/// 6c: a retry's exit is reported with its own attempt (not always 1), the policy decides on that attempt, and the
/// session keeps its last lines for the core's retry rule.
#[test]
fn a_retry_reports_its_own_attempt_and_keeps_its_last_lines() {
    let (sup, _bus, dir) = setup("attempt");
    let (sink, got) = error_sink();
    sup.set_error_sink(sink);
    let t = |n| ProviderTag { attempt: n, ..tag() };
    sup.spawn_tagged("s-a2", spec(&["sh", "-c", "echo 'unexpected status 504'; exit 1"], &dir), Some(t(2))).unwrap();
    let ev = wait_events(&got, 1);
    assert_eq!((ev[0].attempt, ev[0].action), (2, errors::Action::Retrying), "the second failure still retries (2 min)");
    assert!(sup.wait("s-a2", WAIT).unwrap().is_some());
    assert!(sup.last_lines("s-a2").unwrap().iter().any(|l| l.contains("504")));
    sup.spawn_tagged("s-a3", spec(&["sh", "-c", "echo 'unexpected status 504'; exit 1"], &dir), Some(t(3))).unwrap();
    let ev = wait_events(&got, 2);
    assert_eq!((ev[1].attempt, ev[1].action), (3, errors::Action::AskingUser), "the third asks the user");
}
