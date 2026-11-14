use super::*;
use std::collections::HashMap;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Not a key: a marker searched for in every output.
pub(crate) const SENTINEL: &str = "SENTINELnotakey-0123456789abcdef";
const OTHER: &str = "SENTINELsecond-0123456789abcdef";

fn key(s: &str) -> Key {
    Key::new(s.to_string()).unwrap()
}

/// An in-memory store that records every call. It holds the bytes only so the tests can see what was written.
#[derive(Default)]
struct Fake {
    items: Mutex<HashMap<String, Vec<u8>>>,
    calls: Mutex<Vec<String>>,
    /// When set, `add` "succeeds" without keeping anything (as `security -i` may exit 0 on a failed command).
    drop_writes: bool,
}

impl Store for std::sync::Arc<Fake> {
    fn exists(&self, service: &str, _: &str) -> Result<bool, Error> {
        self.calls.lock().unwrap().push(format!("exists {service}"));
        Ok(self.items.lock().unwrap().contains_key(service))
    }
    fn add(&self, service: &str, _: &str, key: &Key) -> Result<(), Error> {
        self.calls.lock().unwrap().push(format!("add {service}"));
        let mut items = self.items.lock().unwrap();
        if items.contains_key(service) {
            return Err(Error::Unavailable);
        }
        if !self.drop_writes {
            items.insert(service.into(), key.bytes().to_vec());
        }
        Ok(())
    }
    fn delete(&self, service: &str, _: &str) -> Result<bool, Error> {
        self.calls.lock().unwrap().push(format!("delete {service}"));
        Ok(self.items.lock().unwrap().remove(service).is_some())
    }
}

fn fake() -> (Keys, std::sync::Arc<Fake>) {
    let f = std::sync::Arc::new(Fake::default());
    (Keys::new(Box::new(f.clone())), f)
}

const SVC: &str = "vajra.provider.openrouter";

#[test]
fn save_records_one_write_and_answers_saved() {
    let (k, f) = fake();
    assert_eq!(k.saved("openrouter"), Ok(false));
    k.save("openrouter", key(SENTINEL)).unwrap();
    assert_eq!(k.saved("openrouter"), Ok(true));
    assert_eq!(f.items.lock().unwrap()[SVC], SENTINEL.as_bytes());
    assert_eq!(f.calls.lock().unwrap().iter().filter(|c| c.starts_with("add")).count(), 1);
}

#[test]
fn a_second_save_is_refused_and_keeps_the_first_key() {
    let (k, f) = fake();
    k.save("openrouter", key(SENTINEL)).unwrap();
    assert_eq!(k.save("openrouter", key(OTHER)), Err(Error::AlreadySaved));
    assert_eq!(f.items.lock().unwrap()[SVC], SENTINEL.as_bytes());
}

#[test]
fn replace_overwrites_and_remove_deletes() {
    let (k, f) = fake();
    k.save("openrouter", key(SENTINEL)).unwrap();
    k.replace("openrouter", key(OTHER)).unwrap();
    assert_eq!(f.items.lock().unwrap()[SVC], OTHER.as_bytes());
    k.remove("openrouter").unwrap();
    assert_eq!(k.saved("openrouter"), Ok(false));
    // Removing again is fine; Replace with nothing stored stores the first key.
    k.remove("openrouter").unwrap();
    k.replace("openrouter", key(SENTINEL)).unwrap();
    assert_eq!(k.saved("openrouter"), Ok(true));
}

#[test]
fn a_write_that_leaves_no_item_is_a_failure() {
    let f = std::sync::Arc::new(Fake { drop_writes: true, ..Default::default() });
    let k = Keys::new(Box::new(f.clone()));
    assert_eq!(k.save("openrouter", key(SENTINEL)), Err(Error::WriteFailed));
}

#[test]
fn an_unknown_account_is_refused_before_the_store() {
    let (k, f) = fake();
    for a in ["claude", "codex", "", "openrouter\n", "../x", "openrouter -A"] {
        assert_eq!(k.save(a, key(SENTINEL)), Err(Error::UnknownAccount), "{a:?}");
        assert_eq!(k.remove(a), Err(Error::UnknownAccount));
        assert_eq!(k.saved(a), Err(Error::UnknownAccount));
    }
    assert!(f.calls.lock().unwrap().is_empty());
}

#[test]
fn values_that_could_break_the_stdin_line_are_refused() {
    let ok = "sk-or-v1-0123456789abcdefABCDEF._~+/=";
    assert!(Key::new(ok.into()).is_ok());
    for bad in [
        format!("{SENTINEL}\nadd-generic-password -A"),
        format!("{SENTINEL} -A"),
        format!("{SENTINEL}\""),
        format!("{SENTINEL}'"),
        format!("{SENTINEL}\\"),
        format!("{SENTINEL}#"),
        format!("{SENTINEL}\t"),
        format!("{SENTINEL}\0"),
        format!("{SENTINEL}é"),
        format!("-{SENTINEL}"),
        "short".to_string(),
        "a".repeat(257),
        String::new(),
    ] {
        assert_eq!(Key::new(bad.clone()).err(), Some(Error::InvalidKey), "{bad:?}");
    }
}

#[test]
fn debug_and_errors_never_show_the_key() {
    let k = key(SENTINEL);
    assert_eq!(format!("{k:?}"), "Key(redacted)");
    assert!(!format!("{:?}", Some(&k)).contains("SENTINEL"));
    for e in [Error::UnknownAccount, Error::InvalidKey, Error::AlreadySaved, Error::Unavailable, Error::WriteFailed] {
        assert!(!format!("{e:?} {} {}", e.code(), e.detail()).contains("SENTINEL"));
    }
}

/// The module's public surface is an allowlist, and no function anywhere in it is shaped like a getter. Adding a read-back
/// path means changing this list on purpose.
#[test]
fn there_is_no_getter() {
    let src = include_str!("keychain.rs");
    let body = src.split("#[cfg(test)]").next().unwrap();
    let mut pub_fns: Vec<&str> = body
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("pub fn "))
        .map(|r| r.split(['(', '<']).next().unwrap())
        .collect();
    pub_fns.sort();
    assert_eq!(pub_fns, ["cli", "code", "detail", "new", "new", "new", "remove", "replace", "save", "saved"]);
    let all_fns = body.lines().filter_map(|l| l.split("fn ").nth(1)).map(|r| r.split(['(', '<']).next().unwrap().to_string());
    for name in all_fns {
        for bad in ["get", "read", "find", "show", "reveal", "export", "copy", "load", "fetch", "value", "password"] {
            assert!(!name.contains(bad), "getter-shaped fn `{name}` in keychain.rs");
        }
    }
    // `Key` has no way out: no Clone, Display, Deref, AsRef or Serialize.
    for bad in ["derive(Clone", "impl Clone for Key", "Display for Key", "Deref for Key", "AsRef<", "Serialize"] {
        assert!(!body.contains(bad), "{bad}");
    }
    // The write's access list is `/usr/bin/security` only, never "any app".
    assert!(!body.contains("-A "));
}

// ---- the real `Cli` against a fake `security` that records argv and stdin --------------------------------------------

static N: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct FakeBin(pub(crate) PathBuf);

impl FakeBin {
    /// A `security` stand-in. The environment is cleared, so its directory is baked in. It models find, add (refusing a
    /// duplicate) and delete on files under `store/`, including the `-i` form that reads one command from stdin.
    pub(crate) fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("vajra-keychain-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("store")).unwrap();
        let script = format!(
            r#"#!/bin/sh
D='{d}'
printf '%s\n' "$*" >> "$D/argv.log"
env >> "$D/env.log"
if [ "$1" = "-i" ]; then
  IFS= read -r line
  printf '%s\n' "$line" >> "$D/stdin.log"
  set -- $line
fi
cmd=$1; shift
svc=; pw=
while [ $# -gt 0 ]; do
  # A flag that takes a value but has none (a trailing `-w`) is refused: `shift 2` with one word left does not shift, and
  # the loop would never end.
  case $1 in -s|-w|-a|-T) if [ $# -lt 2 ]; then echo "security: missing value for $1" >&2; exit 64; fi;; esac
  case $1 in -s) svc=$2; shift 2;; -w) pw=$2; shift 2;; -a|-T) shift 2;; *) shift;; esac
done
case $cmd in
  find-generic-password) [ -f "$D/store/$svc" ] && exit 0; exit 44;;
  add-generic-password) [ -f "$D/store/$svc" ] && exit 45; printf %s "$pw" > "$D/store/$svc"; exit 0;;
  delete-generic-password) [ -f "$D/store/$svc" ] || exit 44; rm "$D/store/$svc"; exit 0;;
esac
exit 1
"#,
            d = d.display()
        );
        let bin = d.join("security");
        fs::write(&bin, script).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        Self(d)
    }
    pub(crate) fn keys(&self) -> Keys {
        Keys::cli(&self.0.join("security"), &self.0)
    }
    pub(crate) fn log(&self, name: &str) -> String {
        fs::read_to_string(self.0.join(name)).unwrap_or_default()
    }
}

impl Drop for FakeBin {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_fake_security_ends_on_a_flag_with_no_value_instead_of_looping() {
    // A trailing `-w` (or any value flag with nothing after it) used to loop forever in the fake's argument loop.
    let b = FakeBin::new();
    for argv in [vec!["find-generic-password", "-s", SVC, "-w"], vec!["find-generic-password", "-s"], vec!["add-generic-password", "-a"]] {
        let mut child = std::process::Command::new(b.0.join("security")).args(&argv).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let status = loop {
            if let Some(s) = child.try_wait().unwrap() {
                break s;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the fake security hung on {argv:?}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(status.code(), Some(64), "{argv:?} is refused");
    }
    // A flag with its value still works.
    assert!(std::process::Command::new(b.0.join("security")).args(["find-generic-password", "-s", SVC]).status().unwrap().code() == Some(44));
}

#[test]
fn the_key_goes_through_stdin_never_argv_or_env() {
    let b = FakeBin::new();
    let k = b.keys();
    k.save("openrouter", key(SENTINEL)).unwrap();
    k.replace("openrouter", key(OTHER)).unwrap();
    assert_eq!(k.saved("openrouter"), Ok(true));
    assert_eq!(fs::read_to_string(b.0.join("store").join(SVC)).unwrap(), OTHER);

    let argv = b.log("argv.log");
    assert!(!argv.contains("SENTINEL"), "{argv}");
    assert!(!b.log("env.log").contains("SENTINEL"));
    // Every write is `security -i`; checks and deletes never ask for the password data (`-w`, `-g`).
    for l in argv.lines() {
        assert!(l == "-i" || l.starts_with("find-generic-password -s ") || l.starts_with("delete-generic-password -s "), "{l}");
        assert!(!l.split(' ').any(|t| t == "-w" || t == "-g"), "{l}");
    }
    let stdin = b.log("stdin.log");
    assert_eq!(
        stdin,
        format!(
            "add-generic-password -a openrouter -s {SVC} -T /usr/bin/security -w {SENTINEL}\n\
             add-generic-password -a openrouter -s {SVC} -T /usr/bin/security -w {OTHER}\n"
        )
    );
    // Only the scrubbed environment reaches the child.
    let env = b.log("env.log");
    assert!(env.lines().all(|l| ["PATH=", "HOME=", "PWD=", "SHLVL=", "_="].iter().any(|p| l.starts_with(p))), "{env}");
}

#[test]
fn second_save_and_remove_through_the_cli() {
    let b = FakeBin::new();
    let k = b.keys();
    k.save("openrouter", key(SENTINEL)).unwrap();
    assert_eq!(k.save("openrouter", key(OTHER)), Err(Error::AlreadySaved));
    k.remove("openrouter").unwrap();
    assert_eq!(k.saved("openrouter"), Ok(false));
    k.remove("openrouter").unwrap();
}

#[test]
fn a_refused_key_runs_no_process() {
    let b = FakeBin::new();
    let k = b.keys();
    assert_eq!(Key::new(format!("{SENTINEL}\nx")).err(), Some(Error::InvalidKey));
    assert_eq!(k.save("nope", key(SENTINEL)), Err(Error::UnknownAccount));
    assert_eq!(b.log("argv.log"), "");
}

#[test]
fn a_missing_security_reads_as_unavailable() {
    let k = Keys::cli(Path::new("/nonexistent/vajra-test/security"), Path::new("/"));
    assert_eq!(k.saved("openrouter"), Err(Error::Unavailable));
    assert_eq!(k.save("openrouter", key(SENTINEL)), Err(Error::Unavailable));
}
