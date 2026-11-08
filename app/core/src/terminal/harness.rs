//! A minimal headless harness for terminal tests: a temp HOME and project root, a collecting sink, and helpers to type
//! and wait for output. No UI, no Tauri.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{Event, LoginEnv, Sink, TermInfo, Terminals};
use crate::lock;

static N: AtomicUsize = AtomicUsize::new(0);

/// `/bin/sh` with a temp HOME and an optional resolved PATH: what the app's login-shell resolution will provide.
pub struct TestEnv {
    pub home: PathBuf,
    pub path: Option<std::ffi::OsString>,
    pub shell: PathBuf,
}

impl LoginEnv for TestEnv {
    fn path(&self) -> Option<std::ffi::OsString> {
        self.path.clone()
    }
    fn shell(&self) -> Option<PathBuf> {
        Some(self.shell.clone())
    }
    fn home(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }
}

/// A temp dir that is the HOME, with a project root `repo/` and one worker worktree below it.
pub struct Fixture {
    pub dir: PathBuf,
    pub root: PathBuf,
    pub worktree: PathBuf,
}

impl Fixture {
    pub fn new(tag: &str) -> Self {
        let d = std::env::temp_dir().join(format!("vajra-term-{}-{}-{tag}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&d);
        let root = d.join("repo");
        let worktree = root.join(".claude/worktrees/w1");
        std::fs::create_dir_all(&worktree).unwrap();
        let dir = d.canonicalize().unwrap();
        Self { root: dir.join("repo"), worktree: dir.join("repo/.claude/worktrees/w1"), dir }
    }

    pub fn env(&self) -> Arc<TestEnv> {
        Arc::new(TestEnv { home: self.dir.clone(), path: None, shell: "/bin/sh".into() })
    }

    pub fn terminals(&self) -> Terminals {
        Terminals::with(super::Limits::default(), self.env())
    }

    pub fn roots(&self) -> Vec<PathBuf> {
        vec![self.root.clone()]
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub struct Collect(pub mpsc::Receiver<Event>);

pub fn sink() -> (Sink, Collect) {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    (Arc::new(move |e| lock(&tx).send(e).is_ok()), Collect(rx))
}

impl Collect {
    /// Everything that has arrived so far, without waiting.
    pub fn now(&self) -> Vec<Event> {
        std::iter::from_fn(|| self.0.try_recv().ok()).collect()
    }

    /// Everything that arrives in the next `ms` milliseconds.
    pub fn for_ms(&self, ms: u64) -> Vec<Event> {
        let end = Instant::now() + Duration::from_millis(ms);
        let mut v = Vec::new();
        while Instant::now() < end {
            if let Ok(e) = self.0.recv_timeout(Duration::from_millis(20)) {
                v.push(e);
            }
        }
        v
    }

    /// Read until `needle` appears in the output (or `secs` pass); returns everything seen.
    pub fn until(&self, needle: &str, secs: u64) -> String {
        let mut all = Vec::new();
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if let Ok(Event::Data(d)) = self.0.recv_timeout(Duration::from_millis(100)) {
                all.extend(d);
                if String::from_utf8_lossy(&all).contains(needle) {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&all).into_owned()
    }

    pub fn exit(&self, secs: u64) -> Option<Option<i32>> {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if let Ok(Event::Exit(c)) = self.0.recv_timeout(Duration::from_millis(100)) {
                return Some(c);
            }
        }
        None
    }
}

/// Open a terminal in the fixture's root.
pub fn open_root(t: &Terminals, f: &Fixture) -> (TermInfo, Collect) {
    let (s, out) = sink();
    (t.open(&f.root, &f.roots(), 24, 80, s).unwrap(), out)
}

pub fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Wait until none of `pids` is alive (or `secs` pass).
pub fn all_gone(pids: &[u32], secs: u64) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while pids.iter().any(|p| alive(*p)) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    !pids.iter().any(|p| alive(*p))
}

pub fn path_of(p: &Path) -> String {
    p.display().to_string()
}

/// Wait until `path` exists (or `secs` pass).
pub fn wait_file(path: &Path, secs: u64) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while !path.exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    path.exists()
}

/// The bytes of every `Data` event, joined.
pub fn data_of(events: &[Event]) -> Vec<u8> {
    events.iter().filter_map(|e| if let Event::Data(d) = e { Some(d.as_slice()) } else { None }).flatten().copied().collect()
}
