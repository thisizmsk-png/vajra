//! Recorded ownership of user shells across a crash (FG-24, ADR-9 item 6b').
//!
//! Each terminal is recorded as (pid, start stamp) when it opens and forgotten when it ends cleanly. After a crash or a
//! SIGKILL of the app the file still lists the shells that were running. At launch `reconcile` checks each against the
//! live process table: a record is *owned* only if the pid is alive AND its start stamp is the recorded one. A pid that
//! is gone is dropped; a pid that is alive with another start stamp was reused by someone else and is dropped too,
//! untouched. `recover` signals only what `reconcile` proves is owned, re-proves it first, and never matches by name.
//! Nothing here runs by itself: the human chooses to recover.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::lock;
use crate::procid::{same_process, start_stamp};

/// Entries kept, and the longest line read back. A damaged or hostile file cannot grow memory without bound.
const MAX_ENTRIES: usize = 256;
const MAX_LINE: usize = 4096;
pub const GRACE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Owner {
    pub pid: u32,
    pub stamp: u64,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Alive, and the start stamp matches: provably ours. `members` counts its session (the shell and its jobs).
    Owned { members: usize },
    /// No such process any more: nothing to do.
    Gone,
    /// The pid is alive but started at another time: someone else's. Never touched.
    Reused,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Found {
    #[serde(flatten)]
    pub owner: Owner,
    #[serde(flatten)]
    pub verdict: Verdict,
}

pub struct Ledger {
    path: PathBuf,
    guard: Mutex<()>,
}

impl Ledger {
    pub fn new(path: PathBuf) -> Self {
        Self { path, guard: Mutex::new(()) }
    }

    /// Every readable entry. A bad line is skipped, not fatal.
    pub fn load(&self) -> Vec<Owner> {
        let _g = lock(&self.guard);
        self.read()
    }

    fn read(&self) -> Vec<Owner> {
        let Ok(text) = std::fs::read_to_string(&self.path) else { return Vec::new() };
        text.lines().filter(|l| l.len() <= MAX_LINE).filter_map(|l| serde_json::from_str::<Owner>(l).ok()).filter(|o| o.pid > 1).take(MAX_ENTRIES).collect()
    }

    fn write(&self, all: &[Owner]) -> std::io::Result<()> {
        let tmp = self.path.with_extension("tmp");
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            for o in all {
                writeln!(f, "{}", serde_json::to_string(o).map_err(std::io::Error::other)?)?;
            }
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)
    }

    /// Record a shell the moment it starts. Best effort: failing to record only means a crash cannot be recovered.
    pub fn record(&self, o: Owner) -> std::io::Result<()> {
        let _g = lock(&self.guard);
        let mut all = self.read();
        all.retain(|x| !(x.pid == o.pid && x.stamp == o.stamp));
        all.push(o);
        if all.len() > MAX_ENTRIES {
            all.drain(..all.len() - MAX_ENTRIES);
        }
        self.write(&all)
    }

    pub fn forget(&self, pid: u32, stamp: u64) -> std::io::Result<()> {
        let _g = lock(&self.guard);
        let mut all = self.read();
        let n = all.len();
        all.retain(|x| !(x.pid == pid && x.stamp == stamp));
        if all.len() == n {
            return Ok(());
        }
        self.write(&all)
    }

    /// Forget a record by pid when the human closes its tab. Removing our own record signals nothing.
    pub fn forget_pid(&self, pid: u32) -> std::io::Result<()> {
        let _g = lock(&self.guard);
        let mut all = self.read();
        let n = all.len();
        all.retain(|x| x.pid != pid);
        if all.len() == n {
            return Ok(());
        }
        self.write(&all)
    }

    /// A clean quit ended every shell: nothing is left to recover.
    pub fn clear(&self) {
        let _g = lock(&self.guard);
        let _ = std::fs::remove_file(&self.path);
    }

    /// Check every record against the live process table, drop the ones that are not ours any more, and return what is
    /// left to report. Kept records are those still `Owned`.
    pub fn settle(&self) -> Vec<Found> {
        let _g = lock(&self.guard);
        let found = reconcile(&self.read());
        let keep: Vec<Owner> = found.iter().filter(|f| matches!(f.verdict, Verdict::Owned { .. })).map(|f| f.owner.clone()).collect();
        let _ = self.write(&keep);
        found
    }
}

pub fn judge(o: &Owner) -> Verdict {
    match start_stamp(o.pid) {
        None => Verdict::Gone,
        Some(s) if s == o.stamp => Verdict::Owned { members: 1 + crate::terminal::kill::session_members(o.pid).into_iter().filter(|&p| p as u32 != o.pid).count() },
        Some(_) => Verdict::Reused,
    }
}

pub fn reconcile(entries: &[Owner]) -> Vec<Found> {
    entries.iter().map(|o| Found { owner: o.clone(), verdict: judge(o) }).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    /// Signalled, and gone within the grace period or ended with SIGKILL.
    Ended,
    /// Not provably ours (gone, or the pid was reused): nothing was signalled.
    NotOwned,
}

/// End a recorded shell's session, but only if the record still proves it is ours. SIGHUP first, SIGKILL after `GRACE`,
/// and each step re-checks the start stamp so a pid that dies and is reused in between is never signalled.
pub fn recover(o: &Owner) -> Recovery {
    if !same_process(o.pid, o.stamp) {
        return Recovery::NotOwned;
    }
    crate::terminal::kill::signal_all(o.pid, true, libc::SIGHUP);
    let t0 = Instant::now();
    while t0.elapsed() < GRACE {
        if !same_process(o.pid, o.stamp) {
            return Recovery::Ended;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if same_process(o.pid, o.stamp) {
        crate::terminal::kill::signal_all(o.pid, true, libc::SIGKILL);
    }
    Recovery::Ended
}

pub fn ledger_path(data: &Path) -> PathBuf {
    data.join("owned.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};

    /// A session leader with a background job in its session, like a login shell with `sleep 300 &`.
    fn leader() -> (Child, Owner) {
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 300 & wait"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let ch = c.spawn().unwrap();
        // wait for the job to exist so the session has two members
        let t0 = Instant::now();
        while crate::terminal::kill::session_members(ch.id()).len() < 2 && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let o = Owner { pid: ch.id(), stamp: start_stamp(ch.id()).unwrap(), cwd: "/tmp".into() };
        (ch, o)
    }
    fn alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    fn gone(mut c: Child) {
        c.kill().ok();
        c.wait().ok();
    }
    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vajra-owned-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join("owned.jsonl")
    }

    #[test]
    fn given_a_live_shell_with_its_recorded_stamp_when_reconciled_then_it_is_owned_with_its_session() {
        let (c, o) = leader();
        assert_eq!(judge(&o), Verdict::Owned { members: 2 });
        gone(c);
    }

    #[test]
    fn given_a_dead_pid_when_reconciled_then_it_is_gone() {
        let (mut c, o) = leader();
        c.kill().unwrap();
        c.wait().unwrap();
        assert_eq!(judge(&o), Verdict::Gone);
    }

    #[test]
    fn given_a_live_pid_with_another_start_stamp_when_reconciled_then_it_is_reused_and_not_ours() {
        let (c, mut o) = leader();
        o.stamp += 7;
        assert_eq!(judge(&o), Verdict::Reused);
        gone(c);
    }

    #[test]
    fn given_a_reused_pid_when_recovery_is_asked_then_nothing_is_signalled() {
        let (c, mut o) = leader();
        o.stamp += 7;
        assert_eq!(recover(&o), Recovery::NotOwned);
        std::thread::sleep(Duration::from_millis(100));
        assert!(alive(c.id()), "an unowned process must survive");
        assert!(crate::terminal::kill::session_members(c.id()).len() >= 2, "and so must its session");
        gone(c);
    }

    #[test]
    fn given_an_owned_shell_when_recovered_then_it_and_its_background_job_end() {
        let (mut c, o) = leader();
        let before = crate::terminal::kill::session_members(o.pid);
        assert_eq!(recover(&o), Recovery::Ended);
        c.wait().ok();
        for p in before {
            let t0 = Instant::now();
            while alive(p as u32) && t0.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(!alive(p as u32), "member {p} must have ended");
        }
    }

    #[test]
    fn given_a_gone_pid_when_recovery_is_asked_then_it_is_not_owned() {
        let (mut c, o) = leader();
        c.kill().unwrap();
        c.wait().unwrap();
        assert_eq!(recover(&o), Recovery::NotOwned);
    }

    #[test]
    fn given_records_when_settled_then_only_owned_ones_stay_in_the_file() {
        let (c1, o1) = leader();
        let (c2, mut o2) = leader();
        o2.stamp += 1;
        let (mut c3, o3) = leader();
        c3.kill().unwrap();
        c3.wait().unwrap();
        let l = Ledger::new(tmp("settle"));
        for o in [&o1, &o2, &o3] {
            l.record(o.clone()).unwrap();
        }
        let found = l.settle();
        assert_eq!(found.len(), 3);
        assert_eq!(l.load(), vec![o1]);
        gone(c1);
        gone(c2);
    }

    #[test]
    fn the_ledger_round_trips_forgets_and_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let l = Ledger::new(tmp("rt"));
        let a = Owner { pid: 4242, stamp: 1, cwd: "/a".into() };
        let b = Owner { pid: 4243, stamp: 2, cwd: "/b".into() };
        l.record(a.clone()).unwrap();
        l.record(b.clone()).unwrap();
        l.record(a.clone()).unwrap(); // no duplicate
        assert_eq!(l.load(), vec![b.clone(), a.clone()]);
        l.forget(4243, 99).unwrap(); // a stamp mismatch forgets nothing
        assert_eq!(l.load().len(), 2);
        l.forget(4243, 2).unwrap();
        assert_eq!(l.load(), vec![a]);
        assert_eq!(std::fs::metadata(&l.path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_damaged_or_hostile_file_is_bounded_and_does_not_panic() {
        let p = tmp("bad");
        let mut s = String::from("not json\n{\"pid\":0,\"stamp\":1,\"cwd\":\"/\"}\n");
        s.push_str(&format!("{{\"pid\":5,\"stamp\":1,\"cwd\":\"{}\"}}\n", "x".repeat(MAX_LINE)));
        for i in 0..(MAX_ENTRIES + 50) {
            s.push_str(&format!("{{\"pid\":{},\"stamp\":1,\"cwd\":\"/\"}}\n", i + 2));
        }
        std::fs::write(&p, s).unwrap();
        let l = Ledger::new(p);
        assert_eq!(l.load().len(), MAX_ENTRIES);
    }

    #[test]
    fn no_signal_is_ever_sent_by_name() {
        let src = include_str!("owned.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for bad in ["killall", "pkill", "pgrep", "\"ps\""] {
            assert!(!code.contains(bad), "{bad}");
        }
    }
}
