//! The spawn lock: a spawn waits out an inheritable-fd window and its child never holds the window's fds. Each child
//! checks its own fd table (`/bin/test -e /dev/fd/N`), so a test elsewhere spawning in parallel can't change the answer.
#![allow(clippy::disallowed_methods)]

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

/// `/bin/test` exits 0 if either fd is open in it: it opens no file of its own, so an open fd there was inherited.
fn holds(fds: [libc::c_int; 2]) -> Command {
    let mut c = Command::new("/bin/test");
    c.args(["-e", &format!("/dev/fd/{}", fds[0]), "-o", "-e", &format!("/dev/fd/{}", fds[1])]);
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c
}

/// Runs `f` inside the next `cloexec_pipe` window on this thread, then makes the pipe.
fn in_window(f: impl Fn([libc::c_int; 2]) + Send + 'static) -> (std::os::fd::OwnedFd, std::os::fd::OwnedFd) {
    *PIPE_HOOK.lock().unwrap() = Some((std::thread::current().id(), Box::new(f)));
    let ends = cloexec_pipe().unwrap();
    *PIPE_HOOK.lock().unwrap() = None;
    ends
}

#[test]
fn an_exclusive_hold_makes_a_spawn_wait_until_it_ends() {
    let g = exclusive();
    let done = Arc::new(AtomicBool::new(false));
    let d = done.clone();
    let t = std::thread::spawn(move || {
        let s = status(Command::new("/usr/bin/true").stdout(Stdio::null())).unwrap();
        d.store(true, Ordering::SeqCst);
        s
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(!done.load(Ordering::SeqCst), "a spawn started while the window was open");
    drop(g);
    assert!(t.join().unwrap().success());
}

/// The #234 follow-up: a spawn another thread starts between `pipe()` and the close-on-exec flags waits for the flags,
/// so its child holds neither end. The bare spawn in the same window is the control: without the lock, the child does
/// inherit them, so the check would see a leak.
#[test]
fn a_spawn_racing_the_pipe_window_inherits_neither_end() {
    let (tx, rx) = mpsc::channel::<std::thread::JoinHandle<bool>>();
    let bare = Arc::new(std::sync::Mutex::new(None::<bool>));
    let b = bare.clone();
    let (_rd, _wr) = in_window(move |fds| {
        // The control: a bare spawn now, unlocked, from another thread (the hook's own thread holds the lock).
        let leaked = std::thread::spawn(move || holds(fds).status().unwrap().success()).join().unwrap();
        *b.lock().unwrap() = Some(leaked);
        // The routed spawn, from another thread: it must wait until the flags are set.
        let started = Arc::new(AtomicBool::new(false));
        let s = started.clone();
        let racer = std::thread::spawn(move || {
            let held = status(&mut holds(fds)).unwrap().success();
            s.store(true, Ordering::SeqCst);
            held
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(!started.load(Ordering::SeqCst), "a routed spawn ran inside the pipe window");
        tx.send(racer).unwrap();
    });
    // Both ends are still open here (`_rd`, `_wr`), so the fd numbers the racer's child checks are this pipe's.
    let racer = rx.recv().unwrap();
    assert!(!racer.join().unwrap(), "a child started during the window inherited the pipe");
    assert_eq!(*bare.lock().unwrap(), Some(true), "control: a bare spawn in the window should inherit the pipe");
}

#[test]
fn output_captures_like_command_output() {
    let o = output(Command::new("/bin/sh").args(["-c", "echo out; echo err >&2; exit 3"])).unwrap();
    assert_eq!(o.stdout, b"out\n");
    assert_eq!(o.stderr, b"err\n");
    assert_eq!(o.status.code(), Some(3));
}
