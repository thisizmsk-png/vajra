use super::harness::*;
use super::*;
use std::time::Instant;

#[test]
fn types_a_command_and_sees_its_output_in_the_root_with_the_temp_home() {
    let f = Fixture::new("echo");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    assert_eq!((info.cwd.clone(), info.root.clone(), info.alive), (f.root.clone(), f.root.clone(), true));
    t.write(info.id, b"pwd; echo HOME=$HOME; echo MARK$((20+22))\n").unwrap();
    let seen = out.until("MARK42", 5);
    assert!(seen.contains(&path_of(&f.root)) && seen.contains(&format!("HOME={}", f.dir.display())) && seen.contains("MARK42"), "{seen}");
}

#[test]
fn a_worker_worktree_is_a_valid_place() {
    let f = Fixture::new("wt");
    let t = f.terminals();
    let (info, out) = open_at(&t, &f, &f.worktree);
    assert_eq!((info.cwd.clone(), info.root.clone()), (f.worktree.clone(), f.root.clone()));
    t.write(info.id, b"pwd\n").unwrap();
    assert!(out.until("w1", 5).contains("worktrees/w1"));
}

#[test]
fn refuses_a_cwd_outside_the_roots_a_bad_cwd_and_a_zero_size() {
    let f = Fixture::new("refuse");
    let t = f.terminals();
    let (s, _o) = sink();
    assert_eq!(t.open(&f.dir, &f.roots(), 24, 80, s.clone()).unwrap_err(), TermError::NotAllowed);
    assert_eq!(t.open(&f.root.join("src"), &f.roots(), 24, 80, s.clone()).unwrap_err(), TermError::BadCwd);
    assert_eq!(t.open(&f.root, &[], 24, 80, s.clone()).unwrap_err(), TermError::NotAllowed, "no roots, no terminals");
    assert_eq!(t.open(&f.root, &f.roots(), 0, 80, s).unwrap_err(), TermError::InvalidSize);
    assert_eq!(t.count(), 0);
}

#[test]
fn resize_reaches_the_shell_and_is_listed() {
    let f = Fixture::new("resize");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    t.resize(info.id, 40, 132).unwrap();
    t.write(info.id, b"stty size\n").unwrap();
    assert!(out.until("40 132", 5).contains("40 132"));
    assert_eq!((t.list()[0].rows, t.list()[0].cols), (40, 132));
    assert_eq!(t.resize(info.id, 0, 10), Err(TermError::InvalidSize));
    assert_eq!(t.resize(999, 10, 10), Err(TermError::NotFound));
}

#[test]
fn the_shell_gets_the_scrubbed_env_plus_the_resolved_path() {
    let f = Fixture::new("env");
    // SAFETY: a unique name no other test reads.
    unsafe { std::env::set_var("VAJRA_TERM_SECRET", "leakme") };
    let env = Arc::new(TestEnv { home: f.dir.clone(), path: Some("/usr/bin:/bin:/resolved/by/login".into()), shell: "/bin/sh".into() });
    let t = Terminals::with(Limits::default(), env);
    let (info, out) = open_root(&t, &f);
    // A login shell may rebuild PATH from the system profile (macOS path_helper), but keeps what it was started with.
    t.write(info.id, b"echo S=[$VAJRA_TERM_SECRET] T=$TERM C=$COLORTERM; case \":$PATH:\" in *:/resolved/by/login:*) echo PATH$((1+1))OK;; esac\n").unwrap();
    let seen = out.until("PATH2OK", 5);
    assert!(seen.contains("S=[] T=xterm-256color C=truecolor"), "{seen}");
    assert!(seen.contains("PATH2OK"), "the resolved PATH did not reach the shell: {seen}");
}

#[test]
fn a_terminal_gets_the_users_login_path_from_the_resolved_login_environment() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("userpath");
    // A stand-in login shell that prints the PATH a real one would after sourcing the user's profile.
    let fake = f.dir.join("fake-login-shell");
    std::fs::write(&fake, "#!/bin/sh\nprintf '%s' '__VAJRA_PATH_BEGIN__/user/tools/bin:/usr/bin:/bin__VAJRA_PATH_END__'\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Resolve it the way the app does at startup (#70), with a temp HOME, then hand it to the terminal.
    let login = crate::userpath::detect(&fake, &f.dir, Duration::from_secs(5)).expect("the stand-in shell resolves");
    let dirs = crate::userpath::effective(Some(login), &[PathBuf::from("/usr/bin")], &f.dir);
    let env = Arc::new(UserPathEnv::with_dirs(f.dir.clone(), dirs).with_shell("/bin/sh".into()));
    let t = Terminals::with(Limits::default(), env);
    let (info, out) = open_root(&t, &f);
    t.write(info.id, b"case \":$PATH:\" in *:/user/tools/bin:*) echo USERPATH$((1+1))OK;; esac; echo HOME=$HOME\n").unwrap();
    let home_line = format!("HOME={}", f.dir.display()); // the typed echo says HOME=$HOME, so this is the shell's answer
    let seen = out.until(&home_line, 5);
    assert!(seen.contains("USERPATH2OK"), "the user's login PATH did not reach the tab: {seen}");
    assert!(seen.contains(&home_line), "{seen}");
}

#[test]
fn the_app_environment_is_the_resolved_login_environment_and_nothing_secret_goes_with_it() {
    let env = UserPathEnv::app(PathBuf::from("/home/someone"));
    assert_eq!(env.path(), crate::userpath::joined());
    // The rule, not a machine's shell: the configured $SHELL when usable, else the resolved login shell.
    assert_eq!(env.shell(), policy::pick_shell(std::env::var_os("SHELL").map(PathBuf::from), crate::userpath::login_shell()));
    assert_eq!(env.home(), Some(PathBuf::from("/home/someone")));
    // The variables a terminal may inherit from the app are the same four as before: no token, key or nonce.
    assert_eq!(policy::ENV_ALLOW, ["HOME", "USER", "LOGNAME", "LANG"]);
}

#[test]
fn an_unusable_login_shell_falls_back_to_bin_sh() {
    let f = Fixture::new("badshell");
    let env = Arc::new(TestEnv { home: f.dir.clone(), path: None, shell: "relative-zsh".into() });
    let t = Terminals::with(Limits::default(), env);
    let (info, out) = open_root(&t, &f);
    t.write(info.id, b"echo SH=$SHELL\n").unwrap();
    assert!(out.until("SH=/bin/sh", 5).contains("SH=/bin/sh"));
}

#[test]
fn caps_hold_globally_and_per_root_and_closing_frees_a_slot() {
    let f = Fixture::new("caps");
    let t = Terminals::with(Limits { max_open: 3, max_per_root: 2, ..Limits::default() }, f.env());
    let other = f.dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let roots = vec![f.root.clone(), other.clone()];
    let open = |cwd: &Path| t.open(cwd, &roots, 24, 80, sink().0);
    let a = open(&f.root).unwrap();
    open(&f.root).unwrap();
    assert_eq!(open(&f.root).unwrap_err(), TermError::TooMany, "a third in one root");
    let c = open(&other).unwrap();
    assert_eq!(open(&other).unwrap_err(), TermError::TooMany, "a fourth in all");
    assert_eq!(t.count(), 3);
    t.close(a.id).unwrap();
    assert!(open(&other).is_ok(), "closing freed a slot");
    t.close(c.id).unwrap();
}

#[test]
fn the_default_cap_is_eight_open() {
    let f = Fixture::new("cap8");
    let t = f.terminals();
    for _ in 0..8 {
        open_root(&t, &f);
    }
    let (s, _o) = sink();
    assert_eq!(t.open(&f.root, &f.roots(), 24, 80, s).unwrap_err(), TermError::TooMany);
}

#[test]
fn list_shows_open_terminals_and_drops_closed_ones() {
    let f = Fixture::new("list");
    let t = f.terminals();
    let (a, _oa) = open_root(&t, &f);
    let (s, _ob) = sink();
    let b = t.open(&f.worktree, &f.roots(), 30, 100, s).unwrap();
    let l = t.list();
    assert_eq!(l.iter().map(|i| i.id).collect::<Vec<_>>(), vec![a.id, b.id]);
    assert_eq!((l[1].cwd.clone(), l[1].rows, l[1].cols), (f.worktree.clone(), 30, 100));
    t.close(a.id).unwrap();
    assert_eq!(t.list().iter().map(|i| i.id).collect::<Vec<_>>(), vec![b.id]);
    assert_eq!(t.close(a.id), Err(TermError::NotFound));
}

#[test]
fn an_ended_shell_is_listed_as_not_alive_until_its_tab_is_closed() {
    let f = Fixture::new("ended");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    t.write(info.id, b"exit 7\n").unwrap();
    assert_eq!(out.exit(5), Some(Some(7)));
    assert!(!t.list()[0].alive);
    t.close(info.id).unwrap();
}

#[test]
fn close_kills_the_shell_and_what_it_started_in_the_background() {
    let f = Fixture::new("close");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    t.write(info.id, b"sleep 300 & echo BG=$!\n").unwrap();
    // The tty echoes the typed line (`BG=$!`) first; the answer is the occurrence followed by digits.
    let mut seen = out.until("BG=1", 5);
    for _ in 0..50 {
        if seen.split("BG=").skip(1).any(|r| r.starts_with(|c: char| c.is_ascii_digit())) {
            break;
        }
        seen.push_str(&out.until("BG=", 1));
    }
    let bg: u32 = seen.split("BG=").skip(1).find_map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()).unwrap();
    let shell = t.pid(info.id).unwrap();
    assert!(alive(bg) && alive(shell));
    t.close(info.id).unwrap();
    assert!(all_gone(&[bg, shell], 3), "the shell or its background job outlived its tab");
    assert_eq!(t.count(), 0);
}

#[test]
fn a_shell_that_ignores_hangup_is_killed_within_the_grace() {
    let f = Fixture::new("trap");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    t.write(info.id, b"trap '' HUP TERM; echo PID=$$; while :; do sleep 1; done\n").unwrap();
    assert!(out.until("PID=", 5).contains("PID="));
    let pid = t.pid(info.id).unwrap();
    let t0 = Instant::now();
    t.close(info.id).unwrap();
    assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
    assert!(all_gone(&[pid], 2));
}

#[test]
fn shutdown_reaps_every_terminal_and_refuses_new_ones() {
    let f = Fixture::new("quit");
    let t = f.terminals();
    let pids: Vec<u32> = (0..3).map(|_| t.pid(open_root(&t, &f).0.id).unwrap()).collect();
    t.shutdown();
    assert!(all_gone(&pids, 3));
    assert_eq!(t.open(&f.root, &f.roots(), 24, 80, sink().0).unwrap_err(), TermError::ShuttingDown);
    assert_eq!(t.count(), 0);
}

#[test]
fn dropping_the_registry_kills_the_shells() {
    let f = Fixture::new("drop");
    let pid = {
        let t = f.terminals();
        t.pid(open_root(&t, &f).0.id).unwrap()
    };
    assert!(all_gone(&[pid], 3));
}

#[test]
fn a_hidden_terminal_keeps_running_with_nobody_reading_it() {
    // Hiding the dock drops nothing in the core: a background job keeps going while no one reads or acks.
    let f = Fixture::new("hidden");
    let t = f.terminals();
    let (info, _out_never_read) = open_root(&t, &f);
    let tick = f.dir.join("tick");
    t.write(info.id, format!("(while :; do echo x >> '{}'; sleep 0.1; done) &\n", tick.display()).as_bytes()).unwrap();
    let lines = |p: &Path| std::fs::read_to_string(p).map(|s| s.lines().count()).unwrap_or(0);
    let end = Instant::now() + Duration::from_secs(5);
    while lines(&tick) < 5 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(lines(&tick) >= 5, "the job did not run while the terminal was hidden");
    t.close(info.id).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let n = lines(&tick);
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(lines(&tick), n, "the job kept running after close");
}

#[test]
fn attach_binds_a_new_sink_and_output_resumes() {
    let f = Fixture::new("attach");
    let t = f.terminals();
    let (info, old) = open_root(&t, &f);
    t.write(info.id, b"echo FIRST\n").unwrap();
    assert!(old.until("FIRST", 5).contains("FIRST"));
    let (s2, fresh) = sink();
    assert_eq!(t.attach(info.id, s2).unwrap().id, info.id);
    t.write(info.id, b"echo SECOND\n").unwrap();
    assert!(fresh.until("SECOND", 5).contains("SECOND"));
    assert_eq!(t.attach(999, sink().0).unwrap_err(), TermError::NotFound);
}

#[test]
fn a_write_over_the_cap_is_refused_and_input_to_a_missing_terminal_too() {
    let f = Fixture::new("write");
    let t = Terminals::with(Limits { max_write: 16, ..Limits::default() }, f.env());
    let (info, _o) = open_root(&t, &f);
    assert_eq!(t.write(info.id, &[b'a'; 17]), Err(TermError::TooLarge));
    assert!(t.write(info.id, &[b'a'; 16]).is_ok());
    assert_eq!(t.write(999, b"x"), Err(TermError::NotFound));
}

#[test]
fn output_waits_for_acks_instead_of_queueing_without_bound() {
    let f = Fixture::new("flow");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    let window = Limits::default().window;
    t.write(info.id, b"yes | head -c 20000000; echo DO$((1+1))NE\n").unwrap();
    // Nobody acks: the reader stops near the window. Wait for it to get there (a busy machine is slow), then let it
    // settle and count what arrived.
    let mut got = 0usize;
    let until = Instant::now() + Duration::from_secs(15);
    while got < window && Instant::now() < until {
        if let Ok(Event::Data(d)) = out.0.recv_timeout(Duration::from_millis(50)) {
            got += d.len();
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    while let Ok(Event::Data(d)) = out.0.try_recv() {
        got += d.len();
    }
    assert!((window..=window + 2 * CHUNK).contains(&got), "read {got} bytes before the first ack");
    t.ack(info.id, got).unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    let mut tail = String::new();
    while Instant::now() < end && !tail.contains("DO2NE") {
        if let Ok(Event::Data(d)) = out.0.recv_timeout(Duration::from_millis(100)) {
            t.ack(info.id, d.len()).unwrap();
            tail.push_str(&String::from_utf8_lossy(&d[d.len().saturating_sub(64)..]));
        }
    }
    assert!(tail.contains("DO2NE"), "the flood never finished after acks");
}

#[test]
fn a_flood_that_is_watched_and_acked_at_once_is_still_paced_to_the_output_rate() {
    let f = Fixture::new("pace");
    let rate = 1024 * 1024; // 1 MiB/s keeps the test short; the default is 8 MiB/s
    let t = Terminals::with(Limits { max_rate: rate, ..Limits::default() }, f.env());
    let (info, out) = open_root(&t, &f);
    t.write(info.id, b"yes | head -c 60000000\n").unwrap();
    // A view that acknowledges everything immediately: nothing but the rate holds the flood back.
    let started = Instant::now();
    let mut got = 0usize;
    while started.elapsed() < Duration::from_millis(2500) {
        if let Ok(Event::Data(d)) = out.0.recv_timeout(Duration::from_millis(20)) {
            got += d.len();
            t.ack(info.id, d.len()).unwrap();
        }
    }
    let secs = started.elapsed().as_secs_f64();
    let burst = 256 * 1024;
    let ceiling = burst + (rate as f64 * secs * 1.25) as usize + 2 * CHUNK;
    assert!(got <= ceiling, "read {got} bytes in {secs:.1}s, over the pace ceiling {ceiling}");
    assert!(got as f64 >= rate as f64 * secs * 0.3, "the flood must still flow: only {got} bytes in {secs:.1}s");
}

/// A shell running `sleep` in the foreground does not read its input, so a PTY write of any size would block.
fn sleeping_terminal(t: &Terminals, f: &Fixture) -> (TermInfo, Collect) {
    let (info, out) = open_root(t, f);
    t.write(info.id, b"echo SLEEPING$((1+1)); sleep 60\n").unwrap();
    out.until("SLEEPING2", 5);
    std::thread::sleep(Duration::from_millis(100)); // the shell has printed; give it a moment to start `sleep`
    (info, out)
}

#[test]
fn a_big_paste_to_a_shell_that_is_not_reading_returns_at_once_and_other_terminals_stay_responsive() {
    let f = Fixture::new("blocked");
    let t = f.terminals();
    let (a, _oa) = sleeping_terminal(&t, &f);
    let (b, ob) = open_root(&t, &f);
    let max = Limits::default().max_write;
    let start = Instant::now();
    let mut results = Vec::new();
    for _ in 0..8 {
        let one = Instant::now();
        results.push(t.write(a.id, &vec![b'x'; max]));
        assert!(one.elapsed() < Duration::from_millis(500), "a write waited on the shell: {:?}", one.elapsed());
    }
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(results.contains(&Err(TermError::QueueFull)), "the bounded queue never said full: {results:?}");
    // Another terminal, and the blocked one's own resize and ack, still answer.
    t.write(b.id, b"echo MARK$((20+22))\n").unwrap();
    assert!(ob.until("MARK42", 5).contains("MARK42"));
    assert!(t.resize(a.id, 30, 100).is_ok() && t.ack(a.id, 0).is_ok());
    // And closing the blocked tab does not hang.
    let one = Instant::now();
    t.close(a.id).unwrap();
    assert!(one.elapsed() < Duration::from_secs(3), "close waited {:?}", one.elapsed());
}

#[test]
fn the_input_queue_is_bounded_and_drains_in_order() {
    let f = Fixture::new("queue");
    let t = Terminals::with(Limits { write_queue: 4096, max_write: 4096, ..Limits::default() }, f.env());
    let (a, _oa) = sleeping_terminal(&t, &f);
    let mut busy = false;
    for _ in 0..200 {
        busy |= t.write(a.id, &[b'y'; 4096]) == Err(TermError::QueueFull);
    }
    assert!(busy, "200 x 4 KiB never filled a 4 KiB queue");
    t.close(a.id).unwrap();
    // A reading shell gets its keystrokes in the order typed.
    let (b, ob) = open_root(&t, &f);
    for part in [&b"echo A"[..], b"B", b"C$((1+1))\n"] {
        t.write(b.id, part).unwrap();
    }
    assert!(ob.until("ABC2", 5).contains("ABC2"));
}

#[test]
fn signalling_a_reaped_terminal_never_touches_a_session_that_reuses_its_pid() {
    use std::os::unix::process::CommandExt;
    // An unrelated session whose leader pid stands in for a recycled pid/session id.
    let mut other = unsafe {
        std::process::Command::new("/bin/sleep").arg("60").pre_exec(|| if libc::setsid() < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }).spawn().unwrap()
    };
    let pid = other.id();
    assert!(alive(pid));
    super::kill::signal_all(pid, false, libc::SIGKILL);
    std::thread::sleep(Duration::from_millis(200));
    assert!(alive(pid), "an ended terminal signalled a session it no longer owns");
    super::kill::signal_all(pid, true, libc::SIGKILL); // control: a live terminal's session is ended
    assert!(other.wait().unwrap().code().is_none(), "a live terminal's signal did not end the session");
}

#[test]
fn a_shell_that_exits_takes_its_background_jobs_with_it_and_a_later_close_is_harmless() {
    let f = Fixture::new("ended-bg");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    t.write(info.id, b"sleep 300 & echo BG=$!\n").unwrap();
    let mut seen = out.until("BG=", 5);
    for _ in 0..50 {
        if seen.split("BG=").skip(1).any(|r| r.starts_with(|c: char| c.is_ascii_digit())) {
            break;
        }
        seen.push_str(&out.until("BG=", 1));
    }
    let bg: u32 = seen.split("BG=").skip(1).find_map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()).unwrap();
    assert!(alive(bg));
    t.write(info.id, b"exit 0\n").unwrap();
    assert_eq!(out.exit(5), Some(Some(0)));
    assert!(all_gone(&[bg], 3), "the session was not ended before the leader was reaped");
    assert!(!t.list()[0].alive);
    t.close(info.id).unwrap();
}

#[test]
fn many_small_writes_reach_the_shell_in_order() {
    let f = Fixture::new("order");
    let t = Arc::new(f.terminals());
    let (info, out) = open_root(&t, &f);
    let file = f.dir.join("typed.txt");
    t.write(info.id, format!("echo CATRDY$((1+1)); cat > {}\n", file.display()).as_bytes()).unwrap();
    out.until("CATRDY2", 5);
    // One caller, many tiny writes: what the UI does while typing fast or pasting in pieces.
    for i in 0..500 {
        t.write(info.id, format!("{i}\n").as_bytes()).unwrap();
    }
    t.write(info.id, b"\x04").unwrap(); // end of input for cat
    t.write(info.id, b"echo DONE$((1+1))\n").unwrap();
    out.until("DONE2", 10);
    let got = std::fs::read_to_string(&file).unwrap();
    let want: String = (0..500).map(|i| format!("{i}\n")).collect();
    assert_eq!(got, want);
}

#[test]
fn a_signal_death_is_reported_as_no_exit_code_and_exit_one_as_one() {
    let f = Fixture::new("sigexit");
    let t = f.terminals();
    let (a, oa) = open_root(&t, &f);
    t.write(a.id, b"kill -9 $$\n").unwrap();
    assert_eq!(oa.exit(5), Some(None), "a shell killed by a signal has no exit code");
    let (b, ob) = open_root(&t, &f);
    t.write(b.id, b"exit 1\n").unwrap();
    assert_eq!(ob.exit(5), Some(Some(1)));
}

#[test]
fn close_returns_even_when_a_child_in_another_session_keeps_the_pty_open() {
    let f = Fixture::new("escape");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    // The child starts its own session (so the terminal's session kill cannot reach it) and keeps the slave open.
    t.write(info.id, b"perl -MPOSIX -e '$p=fork; if($p==0){POSIX::setsid(); sleep 60; exit} print \"ESC=$p\\n\"'\n").unwrap();
    let mut seen = out.until("ESC=", 5);
    for _ in 0..50 {
        if seen.split("ESC=").skip(1).any(|r| r.starts_with(|c: char| c.is_ascii_digit())) {
            break;
        }
        seen.push_str(&out.until("ESC=", 1));
    }
    let esc: u32 = seen.split("ESC=").skip(1).find_map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()).unwrap();
    assert!(alive(esc));
    let shell = t.pid(info.id).unwrap();
    let start = Instant::now();
    t.close(info.id).unwrap();
    assert!(start.elapsed() < Duration::from_secs(4), "close waited {:?} for a PTY that something else held", start.elapsed());
    assert!(all_gone(&[shell], 3), "the shell was not reaped without EOF");
    assert_eq!(t.count(), 0);
    // Shutdown with the same kind of holder is bounded too.
    unsafe { libc::kill(esc as i32, libc::SIGKILL) };
}

/// A holder outside the session that never stops printing, as the reader sees it: an endless stream of bytes.
struct Endless(Arc<std::sync::atomic::AtomicUsize>);

impl Read for Endless {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Fail after a bound so a regression ends the test (with a count that shows it) instead of hanging it.
        if self.0.fetch_add(1, Ordering::SeqCst) >= 5000 {
            return Err(std::io::Error::other("test bound"));
        }
        buf[..1000].fill(b'x');
        Ok(1000)
    }
}

#[test]
fn a_reader_still_blocked_after_close_reaped_the_shell_stops_at_its_next_read_and_buffers_nothing() {
    let f = Fixture::new("detached");
    let t = f.terminals();
    let (info, _out) = open_root(&t, &f);
    let term = t.get(info.id).unwrap();
    // What `stop` leaves behind when the real reader is still blocked: the shell reaped, so `exited` is set.
    term.exited.store(true, Ordering::SeqCst);
    let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    read_loop(Box::new(Endless(reads.clone())), &term, Limits::default().window, Limits::default().max_rate);
    assert!(reads.load(Ordering::SeqCst) <= 1, "the reader kept consuming after the shell was reaped: {} reads", reads.load(Ordering::SeqCst));
    assert!(lock(&term.pending.0).buf.len() <= 1000, "output was buffered with no flusher to drain it");
    t.close(info.id).unwrap();
}

/// A read that blocks until a pipe has data: the state `stop` meets when a holder outside the session keeps the slave open.
#[test]
fn a_reader_blocked_with_the_slave_held_open_is_woken_without_a_signal_and_ends() {
    use std::os::fd::{AsRawFd, FromRawFd};
    let f = Fixture::new("wake");
    let t = f.terminals();
    let (info, _out) = open_root(&t, &f);
    let term = t.get(info.id).unwrap();
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    // `w` stays open for the whole test: the pipe never reaches EOF, like a PTY whose slave an escaped child holds.
    let (r, w) = unsafe { (std::fs::File::from_raw_fd(fds[0]), std::fs::File::from_raw_fd(fds[1])) };
    let (reader, wake) = WakeReader::wrap(Box::new(r.try_clone().unwrap()), Some(r.as_raw_fd()));
    let wake = wake.expect("a wake pipe");
    let th = {
        let term = term.clone();
        std::thread::spawn(move || read_loop(reader, &term, Limits::default().window, Limits::default().max_rate))
    };
    std::thread::sleep(Duration::from_millis(200)); // let it block in select
    assert!(!th.is_finished(), "the reader should be blocked");
    term.exited.store(true, Ordering::SeqCst); // what `stop` has done by the time it wakes the reader
    assert!(wake_reader(Some(wake.as_raw_fd()), &th, Duration::from_secs(2)), "a blocked reader was not woken");
    th.join().unwrap();
    drop(w);
    t.close(info.id).unwrap();
}

#[test]
fn data_still_flows_through_the_wake_reader() {
    use std::io::Write as _;
    use std::os::fd::{AsRawFd, FromRawFd};
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let (r, mut w) = unsafe { (std::fs::File::from_raw_fd(fds[0]), std::fs::File::from_raw_fd(fds[1])) };
    let (mut reader, wake) = WakeReader::wrap(Box::new(r.try_clone().unwrap()), Some(r.as_raw_fd()));
    w.write_all(b"hello").unwrap();
    let mut buf = [0u8; 16];
    let n = reader.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"hello");
    assert!(wake.is_some());
}

#[test]
fn writing_to_an_ended_shell_is_an_error_and_shutdown_of_many_is_one_grace_period() {
    let f = Fixture::new("ended-write");
    let t = f.terminals();
    let (a, oa) = open_root(&t, &f);
    t.write(a.id, b"exit 0\n").unwrap();
    assert_eq!(oa.exit(5), Some(Some(0)));
    assert!(matches!(t.write(a.id, b"x"), Err(TermError::Io(_))));
    // Four shells that ignore hangup: quitting takes about one grace period, not four.
    let mut ids = Vec::new();
    for _ in 0..4 {
        let (i, o) = open_root(&t, &f);
        t.write(i.id, b"trap '' HUP TERM; echo READY$((1+1))\n").unwrap();
        o.until("READY2", 5);
        ids.push(i.id);
    }
    let start = Instant::now();
    t.shutdown();
    assert!(start.elapsed() < Duration::from_millis(1800), "shutdown took {:?}", start.elapsed());
    assert_eq!(t.count(), 0);
}

#[test]
fn the_terminal_module_stays_off_the_bus_the_control_service_mcp_and_agent_sessions() {
    // The wall that keeps agents out: this directory's non-test source may not reach those modules.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/terminal");
    let banned = ["crate::bus", "crate::control", "crate::sessions", "crate::desktop", "crate::crew", "mcp", "socket", "Bus"];
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if name == "tests.rs" || name == "harness.rs" {
            continue;
        }
        // Skip the comments: they may name the walls they keep out.
        let code: String = std::fs::read_to_string(&p).unwrap().lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for b in banned {
            assert!(!code.contains(b), "{name} mentions `{b}`");
        }
    }
}

#[test]
fn no_control_or_mcp_tool_can_reach_a_terminal() {
    let tools = crate::control::mcp::tools().to_string().to_lowercase();
    assert!(!tools.contains("term") && !tools.contains("pty") && !tools.contains("shell"), "{tools}");
}

/// `cargo test --release --lib terminal::tests::bench -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_throughput_cat_50mb() {
    let f = Fixture::new("bench");
    let file = f.dir.join("big.bin");
    let line = b"2026-11-08T04:43:09.1728745Z the quick brown fox jumps over the lazy dog 0123456789\n";
    let mut w = std::fs::File::create(&file).unwrap();
    let mut written = 0usize;
    while written < 50 * 1024 * 1024 {
        std::io::Write::write_all(&mut w, line).unwrap();
        written += line.len();
    }
    drop(w);
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 3);
    let t0 = Instant::now();
    t.write(info.id, format!("cat {}; echo END$((1+1))MARK\n", file.display()).as_bytes()).unwrap();
    let (mut bytes, mut chunks, mut tail) = (0usize, 0usize, String::new());
    while !tail.contains("END2MARK") && t0.elapsed() < Duration::from_secs(120) {
        if let Ok(Event::Data(d)) = out.0.recv_timeout(Duration::from_millis(50)) {
            bytes += d.len();
            chunks += 1;
            t.ack(info.id, d.len()).unwrap();
            tail.push_str(&String::from_utf8_lossy(&d[d.len().saturating_sub(64)..]));
            if tail.len() > 256 {
                tail = tail[tail.len() - 128..].to_string();
            }
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    println!("BENCH throughput: {bytes} bytes in {secs:.2}s = {:.1} MB/s, {chunks} chunks (avg {} B)", bytes as f64 / secs / 1e6, bytes / chunks.max(1));
}

fn ps(pid: u32, col: &str) -> String {
    let o = std::process::Command::new("ps").args(["-o", &format!("{col}="), "-p", &pid.to_string()]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Memory per open terminal in this process, and idle CPU.
#[test]
#[ignore]
fn bench_memory_and_idle_cpu() {
    let f = Fixture::new("mem");
    let me = std::process::id();
    let rss_kb = |pid: u32| ps(pid, "rss").parse::<u64>().unwrap_or(0);
    let t = Terminals::with(Limits { max_open: 8, ..Limits::default() }, Arc::new(TestEnv { home: f.dir.clone(), path: None, shell: "/bin/zsh".into() }));
    std::thread::sleep(Duration::from_millis(300));
    let base = rss_kb(me);
    let keep: Vec<_> = (0..8).map(|_| open_root(&t, &f)).collect();
    std::thread::sleep(Duration::from_secs(2));
    for (info, o) in &keep {
        while let Ok(Event::Data(d)) = o.0.try_recv() {
            t.ack(info.id, d.len()).unwrap();
        }
    }
    let after = rss_kb(me);
    let shells: Vec<u64> = keep.iter().map(|(i, _)| rss_kb(t.pid(i.id).unwrap())).collect();
    println!("BENCH core RSS before {base} KB, after 8 terminals {after} KB => {} KB per terminal (core side)", after.saturating_sub(base) / 8);
    println!("BENCH shell RSS per terminal: avg {} KB", shells.iter().sum::<u64>() / shells.len() as u64);
}
