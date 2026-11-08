//! Hidden panes: the core keeps reading at full speed, keeps the newest 256 KiB, sends nothing until the pane is shown,
//! then sends how much it left out and the tail once. Each test is a Given / When / Then from the design.

use super::harness::*;
use super::hidden::RING;
use super::*;
use std::time::Instant;

/// A loop made only of shell builtins, so the shell itself (not a `sleep` child) is the foreground process that gets SIGWINCH,
/// as a full-screen program would. It prints a dot now and then, so a hidden pane has output.
const BUSY: &str = "i=0; while :; do i=$((i+1)); if [ $((i % 30000)) -eq 0 ]; then printf .; fi; done";

fn skipped_of(events: &[Event]) -> Vec<(u64, Option<bool>)> {
    events.iter().filter_map(|e| if let Event::Skipped { bytes, alt } = e { Some((*bytes, *alt)) } else { None }).collect()
}

fn text(events: &[Event]) -> String {
    String::from_utf8_lossy(&data_of(events)).into_owned()
}

/// A hidden terminal that has finished the command `cmd` (which must end by creating `done`).
fn run_hidden(t: &Terminals, f: &Fixture, id: u32, cmd: &str) {
    let done = f.dir.join(format!("done-{id}"));
    let _ = std::fs::remove_file(&done);
    t.write(id, format!("{cmd}; touch {}\n", done.display()).as_bytes()).unwrap();
    assert!(wait_file(&done, 20), "the command did not finish while hidden: {cmd}");
    std::thread::sleep(Duration::from_millis(150)); // the prompt after it
}

#[test]
fn a_hidden_flood_sends_nothing_and_shows_only_the_newest_tail_once_with_what_was_skipped() {
    let f = Fixture::new("hid-flood");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    t.ack(info.id, 1 << 20).unwrap();
    out.now();
    t.set_visible(info.id, false, false).unwrap();
    run_hidden(&t, &f, info.id, "yes 'hidden output line' | head -n 60000; echo ENDMARK");
    let while_hidden = out.now();
    assert!(!while_hidden.iter().any(|e| matches!(e, Event::Data(_) | Event::Skipped { .. })), "nothing is sent while hidden: {} events", while_hidden.len());
    t.set_visible(info.id, true, false).unwrap();
    let shown = out.for_ms(400);
    let skipped = skipped_of(&shown);
    assert_eq!(skipped.len(), 1, "one skipped notice: {shown:?}");
    assert!(skipped[0].0 > 500_000, "most of the 1.3 MB was left out: {}", skipped[0].0);
    let tail = data_of(&shown);
    assert!(tail.len() <= RING && tail.len() > RING / 2, "{}", tail.len());
    assert!(tail.starts_with(b"hidden output line"), "starts on a whole line: {:?}", String::from_utf8_lossy(&tail[..30]));
    assert!(String::from_utf8_lossy(&tail).contains("ENDMARK"));
    let order = shown.iter().position(|e| matches!(e, Event::Skipped { .. })).unwrap() < shown.iter().position(|e| matches!(e, Event::Data(_))).unwrap();
    assert!(order, "the notice comes before the tail");
    assert!(out.for_ms(300).iter().all(|e| !matches!(e, Event::Skipped { .. })), "and never again");
}

#[test]
fn a_hidden_pane_does_not_slow_the_program_even_with_an_output_rate_limit() {
    let f = Fixture::new("hid-speed");
    let t = Terminals::with(Limits { max_rate: 1 << 20, ..Limits::default() }, f.env()); // a visible pane is held to 1 MiB/s
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    t.set_visible(info.id, false, false).unwrap();
    let t0 = Instant::now();
    run_hidden(&t, &f, info.id, "head -c 8000000 /dev/zero | tr '\\0' x");
    assert!(t0.elapsed() < Duration::from_secs(4), "8 MB took {:?}: a background build must not wait", t0.elapsed());
}

#[test]
fn a_hidden_rate_limit_when_set_holds_a_hidden_pane_to_it() {
    let f = Fixture::new("hid-rate");
    let t = Terminals::with(Limits { hidden_rate: 2 << 20, ..Limits::default() }, f.env()); // an opt-in 2 MiB/s for hidden panes
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    t.set_visible(info.id, false, false).unwrap();
    let t0 = Instant::now();
    run_hidden(&t, &f, info.id, "head -c 6000000 /dev/zero | tr '\\0' x");
    // 6 MB at 2 MiB/s is about 2.7 s after the free 256 KiB burst; unpaced it is well under a second.
    assert!(t0.elapsed() > Duration::from_millis(1800), "{:?}", t0.elapsed());
}

#[test]
fn a_hidden_pane_that_is_blocked_on_unacknowledged_output_is_freed_by_hiding_it() {
    let f = Fixture::new("hid-credit");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    let done = f.dir.join("done-credit");
    // No view reads (nothing acks): after 1 MiB the reader waits for credit.
    t.write(info.id, format!("head -c 4000000 /dev/zero | tr '\\0' x; touch {}\n", done.display()).as_bytes()).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert!(!done.exists(), "held by the window, as before");
    t.set_visible(info.id, false, false).unwrap();
    assert!(wait_file(&done, 10), "hiding it lets the shell finish");
}

#[test]
fn a_tail_with_no_newline_starts_at_the_start_of_the_ring_not_after_nothing() {
    let f = Fixture::new("hid-nonl");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    out.now();
    t.set_visible(info.id, false, false).unwrap();
    run_hidden(&t, &f, info.id, "head -c 1000000 /dev/zero | tr '\\0' x"); // one 1 MB line, like a progress bar
    t.set_visible(info.id, true, false).unwrap();
    let shown = out.for_ms(300);
    let tail = data_of(&shown);
    assert!(tail.len() >= RING - 8 && tail.len() <= RING, "kept, not dropped: {}", tail.len());
    assert!(!skipped_of(&shown).is_empty());
}

#[test]
fn output_that_fits_is_delivered_whole_in_order_after_what_the_view_already_had_with_no_notice() {
    let f = Fixture::new("hid-small");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    t.write(info.id, b"echo FIRST1\n").unwrap();
    out.until("FIRST1", 5);
    t.set_visible(info.id, false, false).unwrap();
    t.write(info.id, b"echo SECOND2\necho THIRD3\n").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    t.set_visible(info.id, true, false).unwrap();
    let shown = out.for_ms(300);
    assert!(skipped_of(&shown).is_empty(), "nothing was left out, so nothing is announced");
    let s = text(&shown);
    assert!(s.find("SECOND2").unwrap() < s.find("THIRD3").unwrap());
}

#[test]
fn hiding_twice_keeps_what_was_kept_and_showing_a_visible_pane_does_nothing() {
    let f = Fixture::new("hid-twice");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    t.set_visible(info.id, true, false).unwrap();
    assert!(out.for_ms(150).iter().all(|e| !matches!(e, Event::Skipped { .. } | Event::Activity)));
    t.set_visible(info.id, false, false).unwrap();
    t.write(info.id, b"echo KEPT77\n").unwrap();
    std::thread::sleep(Duration::from_millis(400));
    t.set_visible(info.id, false, false).unwrap(); // again
    t.set_visible(info.id, true, false).unwrap();
    assert!(text(&out.for_ms(300)).contains("KEPT77"));
}

#[test]
fn an_unknown_id_is_an_error_and_changes_nothing_for_any_terminal() {
    let f = Fixture::new("hid-unknown");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    assert_eq!(t.set_visible(999, false, false), Err(TermError::NotFound));
    assert_eq!(t.set_visible(info.id + 1, false, true), Err(TermError::NotFound));
    t.write(info.id, b"echo STILLVISIBLE\n").unwrap();
    assert!(out.until("STILLVISIBLE", 5).contains("STILLVISIBLE"), "the real terminal is untouched and still sends");
}

#[test]
fn the_window_stays_exact_across_a_hide_and_show_so_a_later_flood_still_flows() {
    let f = Fixture::new("hid-credit-exact");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    t.set_visible(info.id, false, false).unwrap();
    run_hidden(&t, &f, info.id, "head -c 2000000 /dev/zero | tr '\\0' y; echo");
    t.set_visible(info.id, true, false).unwrap();
    for e in out.for_ms(300) {
        if let Event::Data(d) = e {
            t.ack(info.id, d.len()).unwrap();
        }
    }
    t.ack(info.id, 1 << 30).unwrap(); // a late or doubled ack is clamped, never a debt
    let done = f.dir.join("done-visible");
    t.write(info.id, format!("head -c 3000000 /dev/zero | tr '\\0' z; touch {}\n", done.display()).as_bytes()).unwrap();
    let t0 = Instant::now();
    while !done.exists() && t0.elapsed() < Duration::from_secs(20) {
        for e in out.for_ms(50) {
            if let Event::Data(d) = e {
                t.ack(info.id, d.len()).unwrap();
            }
        }
    }
    assert!(done.exists(), "a visible flood after showing is not stalled by leftover credit");
}

#[test]
fn a_program_that_enters_the_alternate_screen_while_hidden_gets_the_screen_switch_and_a_redraw() {
    let f = Fixture::new("hid-alt-in");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    let ready = f.dir.join("alt-ready");
    t.set_visible(info.id, false, false).unwrap();
    t.write(info.id, format!("trap 'echo GOT_WINCH' WINCH; printf '\\033[?1049h'; touch {}; {BUSY}\n", ready.display(), BUSY = BUSY).as_bytes()).unwrap();
    assert!(wait_file(&ready, 10));
    std::thread::sleep(Duration::from_millis(200));
    t.set_visible(info.id, true, false).unwrap();
    let shown = out.for_ms(1000);
    assert_eq!(skipped_of(&shown).last().map(|s| s.1), Some(Some(true)), "tell the view to switch screens: {shown:?}");
    assert!(text(&shown).contains("GOT_WINCH"), "the program was nudged to redraw");
    t.close(info.id).unwrap();
}

#[test]
fn a_program_still_on_the_alternate_screen_is_nudged_to_redraw_without_a_second_switch() {
    let f = Fixture::new("hid-alt-still");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    let ready = f.dir.join("alt-ready2");
    t.write(info.id, format!("trap 'echo GOT_WINCH' WINCH; printf '\\033[?1049h'; touch {}; {BUSY}\n", ready.display(), BUSY = BUSY).as_bytes()).unwrap();
    assert!(wait_file(&ready, 10));
    out.until("1049h", 3);
    out.now();
    t.set_visible(info.id, false, true).unwrap(); // the view was on the alternate screen when it hid
    std::thread::sleep(Duration::from_millis(300));
    t.set_visible(info.id, true, false).unwrap();
    let shown = out.for_ms(1000);
    assert!(skipped_of(&shown).iter().all(|s| s.1.is_none()), "no switch: the view is already there: {shown:?}");
    assert!(text(&shown).contains("GOT_WINCH"), "{shown:?}");
    t.close(info.id).unwrap();
}

#[test]
fn a_program_that_left_the_alternate_screen_while_hidden_gets_the_normal_tail_and_no_nudge() {
    let f = Fixture::new("hid-alt-out");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    let ready = f.dir.join("alt-left");
    t.write(info.id, b"trap 'echo GOT_WINCH' WINCH; printf '\\033[?1049h'; sleep 0.3\n").unwrap();
    out.until("1049h", 5);
    out.now();
    t.set_visible(info.id, false, true).unwrap();
    t.write(info.id, format!("printf '\\033[?1049l'; echo LEFT_ALT_OK; touch {}\n", ready.display()).as_bytes()).unwrap();
    assert!(wait_file(&ready, 10));
    std::thread::sleep(Duration::from_millis(400));
    t.set_visible(info.id, true, false).unwrap();
    let shown = out.for_ms(800);
    assert_eq!(skipped_of(&shown).last().map(|s| s.1), Some(Some(false)), "tell the view to leave the alternate screen: {shown:?}");
    let s = text(&shown);
    assert!(s.contains("LEFT_ALT_OK"), "the normal output is delivered");
    assert!(!s.contains("GOT_WINCH"), "no redraw nudge for a program that is no longer full screen");
}

#[test]
fn a_pane_that_was_never_on_the_alternate_screen_is_never_nudged() {
    let f = Fixture::new("hid-no-alt");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    let ready = f.dir.join("noalt-ready");
    t.write(info.id, format!("trap 'echo GOT_WINCH' WINCH; touch {}; {BUSY}\n", ready.display(), BUSY = BUSY).as_bytes()).unwrap();
    assert!(wait_file(&ready, 10));
    out.now();
    t.set_visible(info.id, false, false).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    t.set_visible(info.id, true, false).unwrap();
    let shown = out.for_ms(800);
    assert!(!text(&shown).contains("GOT_WINCH"), "{shown:?}");
    assert!(skipped_of(&shown).is_empty());
    t.close(info.id).unwrap();
}

#[test]
fn the_shell_ending_while_hidden_is_reported_at_once_and_its_output_comes_on_show() {
    let f = Fixture::new("hid-exit");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    t.set_visible(info.id, false, false).unwrap();
    t.write(info.id, b"echo BYE55; exit 3\n").unwrap();
    assert_eq!(out.exit(10), Some(Some(3)), "the exit is not held back");
    t.set_visible(info.id, true, false).unwrap();
    assert!(text(&out.for_ms(300)).contains("BYE55"));
}

#[test]
fn a_hidden_pane_with_new_output_says_so_at_most_twice_a_second_and_a_visible_one_never() {
    let f = Fixture::new("hid-activity");
    let t = f.terminals();
    let (info, out) = open_root(&t, &f);
    out.until("$", 5);
    t.write(info.id, b"echo VISIBLE1\n").unwrap();
    out.until("VISIBLE1", 5);
    assert!(out.for_ms(200).iter().all(|e| !matches!(e, Event::Activity)), "visible: no activity events");
    t.set_visible(info.id, false, false).unwrap();
    t.write(info.id, b"i=0; while [ $i -lt 30 ]; do echo tick$i; sleep 0.05; i=$((i+1)); done\n").unwrap();
    let got = out.for_ms(2200);
    let n = got.iter().filter(|e| matches!(e, Event::Activity)).count();
    assert!((2..=5).contains(&n), "{n} activity events in 2.2 s of output");
    assert!(!got.iter().any(|e| matches!(e, Event::Data(_))));
}

#[test]
fn a_view_that_is_gone_makes_the_pane_hidden_and_the_next_attach_gets_what_it_missed() {
    let f = Fixture::new("hid-dead");
    let t = f.terminals();
    let (s, out) = sink();
    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let a2 = alive.clone();
    let gate: Sink = Arc::new(move |e| a2.load(Ordering::SeqCst) && s(e));
    let info = t.open(&f.root, &f.roots(), 24, 80, gate).unwrap();
    out.until("$", 5);
    alive.store(false, Ordering::SeqCst); // the webview went away
    t.write(info.id, b"echo TRIGGER\n").unwrap(); // the first send to a dead view fails and is lost with its chunk
    std::thread::sleep(Duration::from_millis(400));
    t.write(info.id, b"echo WHILE_GONE_9\n").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let (s2, out2) = sink();
    t.attach(info.id, s2).unwrap();
    let got = out2.for_ms(500);
    assert!(text(&got).contains("WHILE_GONE_9"), "replayed to the new view: {got:?}");
    t.write(info.id, b"echo AFTER_ATTACH\n").unwrap();
    assert!(out2.until("AFTER_ATTACH", 5).contains("AFTER_ATTACH"), "and live output continues");
}

#[test]
fn a_dead_sink_costs_no_sends_after_the_first_failure() {
    let f = Fixture::new("hid-dead2");
    let t = f.terminals();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c2 = calls.clone();
    let sink: Sink = Arc::new(move |_| {
        c2.fetch_add(1, Ordering::SeqCst);
        false
    });
    let info = t.open(&f.root, &f.roots(), 24, 80, sink).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let first = calls.load(Ordering::SeqCst);
    t.write(info.id, b"head -c 1000000 /dev/zero | tr '\\0' q; echo\n").unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(calls.load(Ordering::SeqCst) <= first + 2, "{} calls after {first}", calls.load(Ordering::SeqCst));
    t.close(info.id).unwrap();
}
