//! The allowlist check and the open must agree: a path component swapped for a symlink between `resolve()` and the
//! open must not let the app read a file outside the allowlist (review P1).

use std::fs;
use std::io::Read;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use vajra_desktop::vajra::allowlist::{Allowlist, Denied};

static N: AtomicUsize = AtomicUsize::new(0);
const SECRET_NAME: &str = ".hmac-key"; // the protected key file's name

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let h = std::env::temp_dir().join(format!("vajra-toctou-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&h);
        fs::create_dir_all(h.join(".claude/projects/s1")).unwrap();
        fs::create_dir_all(h.join("appdata")).unwrap();
        fs::create_dir_all(h.join("outside")).unwrap();
        Self(h)
    }
    fn allow(&self) -> Allowlist {
        Allowlist::new(&self.0.join(".claude"), &self.0.join("appdata")).unwrap()
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read(mut f: fs::File) -> String {
    let mut s = String::new();
    f.read_to_string(&mut s).unwrap();
    s
}

#[test]
fn an_allowlisted_file_opens() {
    let h = Home::new();
    let p = h.0.join(".claude/projects/s1/agent-a.jsonl");
    fs::write(&p, "ok").unwrap();
    assert_eq!(read(h.allow().open_allowlisted(&p).unwrap()), "ok");
}

#[test]
fn a_directory_swapped_for_a_symlink_after_validation_is_caught_at_open() {
    let h = Home::new();
    fs::write(h.0.join("outside/secret.txt"), "TOP SECRET").unwrap();
    // The state after the swap: the validated name now goes through a symlinked directory to a file outside.
    symlink(h.0.join("outside"), h.0.join(".claude/projects/s1/dir")).unwrap();
    let after_swap = h.0.join(".claude/projects/s1/dir/secret.txt");
    // What resolve() said about the pre-swap name is irrelevant: the open itself must refuse.
    let err = h.allow().open_resolved(&after_swap).unwrap_err();
    assert!(matches!(err, Denied::NotAllowed), "{err}");
}

#[test]
fn the_protected_file_is_refused_even_when_reached_through_a_swapped_directory() {
    let h = Home::new();
    fs::create_dir_all(h.0.join(".claude/vajra")).unwrap();
    fs::write(h.0.join(".claude/vajra").join(SECRET_NAME), "KEY").unwrap();
    symlink(h.0.join(".claude/vajra"), h.0.join(".claude/projects/s1/v")).unwrap();
    let err = h.allow().open_resolved(&h.0.join(".claude/projects/s1/v").join(SECRET_NAME)).unwrap_err();
    assert!(matches!(err, Denied::SecretKey), "{err}");
}

#[test]
fn a_final_symlink_is_never_followed() {
    let h = Home::new();
    fs::write(h.0.join("outside/secret.txt"), "TOP SECRET").unwrap();
    let link = h.0.join(".claude/projects/s1/link.jsonl");
    symlink(h.0.join("outside/secret.txt"), &link).unwrap();
    assert!(h.allow().open_resolved(&link).is_err(), "O_NOFOLLOW on the final component");
    assert!(matches!(h.allow().open_allowlisted(&link).unwrap_err(), Denied::NotAllowed), "resolve() also refuses it");
}

#[test]
fn a_hard_link_to_the_protected_file_is_refused_even_under_an_allowed_name() {
    let h = Home::new();
    fs::create_dir_all(h.0.join(".claude/vajra")).unwrap();
    let key = h.0.join(".claude/vajra").join(SECRET_NAME);
    fs::write(&key, "KEY").unwrap();
    let linked = h.0.join(".claude/projects/s1/x.jsonl");
    fs::hard_link(&key, &linked).unwrap();
    let err = h.allow().open_allowlisted(&linked).unwrap_err();
    assert!(matches!(err, Denied::NotAllowed | Denied::SecretKey), "{err}");
}

#[test]
fn a_hard_linked_file_is_refused_even_if_it_is_not_the_key() {
    let h = Home::new();
    fs::write(h.0.join("outside/data.txt"), "x").unwrap();
    let linked = h.0.join(".claude/projects/s1/y.jsonl");
    fs::hard_link(h.0.join("outside/data.txt"), &linked).unwrap();
    assert!(h.allow().open_allowlisted(&linked).is_err(), "only single-link regular files are opened");
}

#[test]
fn a_fifo_is_refused_without_blocking() {
    let h = Home::new();
    let fifo = h.0.join(".claude/projects/s1/pipe.jsonl");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let allow = h.allow();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(allow.open_allowlisted(&fifo).is_err());
    });
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)), Ok(true), "a FIFO must be refused, not block the open");
}
