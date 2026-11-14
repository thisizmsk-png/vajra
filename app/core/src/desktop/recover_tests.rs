//! After a crash (6d2d): the ledger records every spawn and refuses one it can't record, the start-up reconcile kills
//! an earlier life's leads and workers (re-proved by pid and start stamp) and writes their tickets and runs ended, a
//! reused pid is never touched, and the core runs list says why a run can't be resumed. Temp dirs only.

use super::*;
use crate::owned::{Owner, MAX_TRIES};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("vajra-rc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("data")).unwrap();
        Tmp(p.canonicalize().unwrap())
    }
    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A process in its own session, as the supervisor's PTY children are; killed and reaped on drop if it still runs.
pub(crate) struct Kid(pub(crate) Child);
impl Drop for Kid {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
/// The mark every `kid` carries, as the supervisor puts one in each agent's environment.
const MARK: &str = "0123456789abcdef0123456789abcdef";

/// A session leader that carries `MARK`: this test binary, sleeping (not `/bin/sleep`, whose environment macOS hides).
fn kid() -> Kid {
    marked_kid(MARK)
}

pub(crate) fn marked_kid(mark: &str) -> Kid {
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.args(["--exact", "procsweep::tests::sleeper_body", "--ignored", "--nocapture"]).env("VAJRA_SWEEP_SLEEP", "1");
    c.env(crate::owned::AGENT_MARK, mark).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
    // SAFETY: setsid is async-signal-safe and has no preconditions.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let k = Kid(c.spawn().unwrap());
    // Its environment is readable once it has exec'd: wait for it, so a record of it is provable.
    let t0 = Instant::now();
    while crate::procsweep::env_var(k.0.id(), crate::owned::AGENT_MARK).is_none_or(|v| v != mark) {
        assert!(t0.elapsed() < Duration::from_secs(10), "its mark never became readable");
        std::thread::sleep(Duration::from_millis(10));
    }
    k
}

fn alive(k: &mut Kid) -> bool {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match k.0.try_wait().unwrap() {
            Some(_) => return false,
            None if Instant::now() >= until => return true,
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn agent(k: &Kid, kind: Kind, run: &str, ticket: Option<&str>) -> Owner {
    let pid = k.0.id();
    Owner { pid, stamp: crate::procid::start_stamp(pid).unwrap(), cwd: "/tmp".into(), kind, run: Some(run.into()), ticket: ticket.map(String::from), tries: 0, mark: Some(MARK.into()) }
}

const RUN: &str = "20261112-0930";

/// A run as an earlier life left it: started, one ticket line still running.
fn crashed_run(data: &Path, ticket: &str) {
    record_started(data, RUN, "p", Path::new("/tmp"), "codex").unwrap();
    let line = json!({"v": 1, "event": "dispatch", "ticket": {"id": ticket, "run": RUN, "task": "t1", "round": 1, "kind": "writer", "slot": "w1", "account": "codex", "state": "running", "attempt": 1, "elapsed_s": 0}});
    std::fs::write(data.join("runs").join(RUN).join("tickets.jsonl"), format!("{line}\n")).unwrap();
}

fn last_line(data: &Path) -> Value {
    let t = std::fs::read_to_string(data.join("runs").join(RUN).join("tickets.jsonl")).unwrap();
    serde_json::from_str(t.lines().last().unwrap()).unwrap()
}

#[test]
fn a_crash_reconcile_kills_the_earlier_lifes_lead_and_worker_and_writes_them_ended() {
    let t = Tmp::new("crash");
    let data = t.data();
    let ticket = format!("{RUN}.t1.r1");
    crashed_run(&data, &ticket);
    let (mut lead, mut worker, mut shell) = (kid(), kid(), kid());
    let ledger = Ledger::new(crate::owned::ledger_path(&data));
    ledger.record(agent(&lead, Kind::Lead, RUN, None)).unwrap();
    ledger.record(agent(&worker, Kind::Worker, RUN, Some(&ticket))).unwrap();
    ledger.record(Owner::shell(shell.0.id(), crate::procid::start_stamp(shell.0.id()).unwrap(), "/tmp".into())).unwrap();

    // The new life, on the same data dir, before any agent of its own.
    let ledger = Ledger::new(crate::owned::ledger_path(&data));
    let r = reconcile(&data, &ledger);
    assert_eq!((r.killed, r.survived, r.reused, r.tickets), (2, 0, 0, 1), "{r:?}");
    assert_eq!(r.runs, vec![RUN.to_string()]);
    assert!(!alive(&mut lead) && !alive(&mut worker), "both were ours and are gone before any agent starts");
    assert!(alive(&mut shell), "a shell stays the human's choice");
    assert_eq!(ledger.load().len(), 1, "only the shell's record is left");

    let l = last_line(&data);
    assert_eq!((l["ticket"]["state"].as_str(), l["ticket"]["outcome"].as_str()), (Some("ended"), Some("failed")));
    assert_eq!((l["ticket"]["reason"].as_str(), l["ticket"]["detail"].as_str()), (Some("abandoned"), Some("app_crashed")));
    let ended = read(&run_dir(&data, RUN, false).unwrap(), ENDED_FILE).unwrap();
    assert_eq!((ended["status"].as_str(), ended["reason"].as_str()), (Some("lead_lost"), Some("app_restarted")));

    // A second start finds nothing more to do, and the run's end is not rewritten.
    let again = reconcile(&data, &ledger);
    assert_eq!((again.killed, again.tickets, again.runs.len()), (0, 0, 0));
}

#[test]
fn a_record_whose_pid_another_process_holds_is_never_signalled_and_is_forgotten() {
    let t = Tmp::new("reused");
    let data = t.data();
    let ticket = format!("{RUN}.t1.r1");
    crashed_run(&data, &ticket);
    let mut other = kid();
    let ledger = Ledger::new(crate::owned::ledger_path(&data));
    let mut o = agent(&other, Kind::Worker, RUN, Some(&ticket));
    o.stamp += 1; // the pid is this live process, but the record's process started at another time
    ledger.record(o).unwrap();
    let r = reconcile(&data, &ledger);
    assert_eq!((r.killed, r.reused, r.tickets), (0, 1, 0), "{r:?}");
    assert!(alive(&mut other), "someone else's process is left alone");
    assert!(ledger.load().is_empty(), "the stale record is dropped");
    assert_eq!(last_line(&data)["ticket"]["state"], "running", "no ticket line is written for it; the replay ends it");
}

#[test]
fn a_spawn_whose_record_cannot_be_written_is_refused_and_its_process_killed() {
    use crate::sessions::{SessionError, SpawnSpec, Supervisor};
    let t = Tmp::new("unwritable");
    let bus = Arc::new(crate::bus::Bus::open(&t.0.join("spill"), crate::bus::Config::default()).unwrap());
    let sup = Supervisor::new(bus, None);
    // A ledger whose directory does not exist: nothing can be written there.
    sup.set_ledger(Arc::new(Ledger::new(t.0.join("missing/owned.jsonl"))));
    let marker = t.0.join("ran");
    let script = format!("sleep 1; echo x > '{}'", marker.display());
    let spec = SpawnSpec { argv: vec!["/bin/sh".into(), "-c".into(), script], cwd: t.0.clone(), rows: 24, cols: 80, redact: Default::default(), extra_env: vec![] };
    let r = sup.spawn("lead-x", spec);
    assert!(matches!(r, Err(SessionError::Ledger(_))), "{r:?}");
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!marker.exists(), "the process was killed before it could do anything");

    // With a writable ledger the spawn is recorded before it returns and forgotten once it is reaped.
    let ledger = Arc::new(Ledger::new(t.0.join("owned.jsonl")));
    sup.set_ledger(ledger.clone());
    let spec = SpawnSpec { argv: vec!["/bin/sh".into(), "-c".into(), "sleep 1".into()], cwd: t.0.clone(), rows: 24, cols: 80, redact: Default::default(), extra_env: vec![] };
    sup.spawn("lead-y", spec).unwrap();
    let rec = ledger.load();
    assert_eq!(rec.len(), 1);
    assert_eq!((rec[0].kind, rec[0].run.as_deref()), (Kind::Lead, Some("lead-y")));
    sup.wait("lead-y", Duration::from_secs(10)).unwrap();
    assert!(ledger.load().is_empty(), "reaped, so forgotten");
}

#[test]
fn a_ledger_full_of_agent_records_refuses_one_more_and_a_shell_too() {
    let t = Tmp::new("full");
    let ledger = Ledger::new(t.0.join("owned.jsonl"));
    let worker = |pid: u32| Owner { pid, stamp: 1, cwd: "/tmp".into(), kind: Kind::Worker, run: Some(RUN.into()), ticket: Some("t".into()), tries: 0, mark: Some(MARK.into()) };
    for i in 0..256u32 {
        ledger.record(worker(10_000 + i)).unwrap();
    }
    assert!(ledger.record(worker(9_999)).is_err(), "no agent runs without its record");
    // A shell is best effort: with no shell to drop, the new one is the one that goes.
    let _ = ledger.record(Owner::shell(9_998, 1, "/tmp".into()));
    let all = ledger.load();
    assert_eq!(all.len(), 256);
    assert!(all.iter().all(|o| o.kind == Kind::Worker), "no agent record was dropped to make room");
}

fn env_with<'a>(seats: &'a dyn Fn(&str) -> std::io::Result<Option<Value>>, installed: &'a dyn Fn(&str) -> bool, lead_ok: &'a dyn Fn(&str) -> bool) -> ResumeEnv<'a> {
    ResumeEnv { seats, installed, lead_ok }
}

/// #176 review: the list's cap is not the reconcile's. An unfinished core run older than 250 other run folders (most of
/// them not core runs) is still written ended, and the list caps only the ended core runs it shows, newest first.
#[test]
fn an_unfinished_core_run_beyond_the_listed_cap_is_still_reconciled() {
    let t = Tmp::new("beyond-cap");
    let data = t.data();
    let old = "20250101-0000";
    record_started(&data, old, "p", Path::new("/tmp"), "codex").unwrap();
    for i in 0..250 {
        std::fs::create_dir_all(data.join("runs").join(format!("20261112-{i:04}"))).unwrap();
    }
    for i in 0..(MAX_LISTED + 5) {
        let run = format!("20260601-{i:04}");
        record_started(&data, &run, "p", Path::new("/tmp"), "codex").unwrap();
        record_ended(&data, &run, "done", "finished").unwrap();
    }
    let r = reconcile(&data, &Ledger::new(crate::owned::ledger_path(&data)));
    assert_eq!(r.runs, vec![old.to_string()], "the old unfinished run is reconciled");
    assert!(data.join("runs").join(old).join(ENDED_FILE).is_file());
    let (no_seats, yes) = (|_: &str| Ok(None), |_: &str| true);
    let listed = core_runs(&data, &env_with(&no_seats, &yes, &yes));
    assert_eq!(listed.len(), MAX_LISTED, "the cap counts ended core runs only, not the empty folders");
    assert_eq!(listed[0]["run"], format!("20260601-{:04}", MAX_LISTED + 4), "newest first");
}

#[test]
fn an_ended_run_says_why_it_cannot_be_resumed() {
    let t = Tmp::new("list");
    let data = t.data();
    let cwd = t.0.join("proj");
    std::fs::create_dir_all(&cwd).unwrap();
    let ended = |run: &str, account: &str, folder: &Path, plan: bool| {
        record_started(&data, run, "p", folder, account).unwrap();
        if plan {
            record_plan(&data, run, &json!({"v": 1, "round": 1})).unwrap();
        }
        record_ended(&data, run, "lead_lost", "app_restarted").unwrap();
    };
    ended("20261112-0901", "codex", &cwd, true);
    ended("20261112-0902", "codex", &t.0.join("gone"), true);
    ended("20261112-0903", "openrouter", &cwd, true);
    ended("20261112-0904", "claude", &cwd, true);
    ended("20261112-0905", "codex", &cwd, false);
    ended("20261112-0906", "zai", &cwd, true);
    // A run that never ended is not listed.
    record_started(&data, "20261112-0907", "p", &cwd, "codex").unwrap();
    let seats = |run: &str| if run == "20261112-0906" { Ok(None) } else { Ok(Some(json!({"seats": []}))) };
    let installed = |a: &str| a != "openrouter";
    let lead_ok = |a: &str| a != "claude";
    let runs = core_runs(&data, &env_with(&seats, &installed, &lead_ok));
    let why: Vec<(String, Option<String>)> = runs.iter().map(|r| (r["run"].as_str().unwrap().to_string(), r["why"].as_str().map(String::from))).collect();
    let want = |r: &str, w: Option<&str>| (r.to_string(), w.map(String::from));
    assert_eq!(
        why,
        vec![
            want("20261112-0906", Some(SEATS_UNREADABLE)),
            want("20261112-0905", Some(PLAN_UNREADABLE)),
            want("20261112-0904", Some(LEAD_UNVERIFIED)),
            want("20261112-0903", Some(PROVIDER_NOT_INSTALLED)),
            want("20261112-0902", Some(CWD_MISSING)),
            want("20261112-0901", None),
        ]
    );
    assert_eq!(runs.last().unwrap()["resumable"], true);
    assert!(runs.iter().take(5).all(|r| r["resumable"] == false));
}

#[test]
fn a_records_dir_that_is_a_link_is_not_read() {
    let t = Tmp::new("link");
    let data = t.data();
    let real = t.0.join("elsewhere");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::create_dir_all(data.join("runs")).unwrap();
    std::fs::write(real.join(ENDED_FILE), r#"{"v":1,"status":"verified","reason":""}"#).unwrap();
    std::os::unix::fs::symlink(&real, data.join("runs").join(RUN)).unwrap();
    let (seats, installed, lead_ok) = (|_: &str| Ok(None), |_: &str| true, |_: &str| true);
    assert!(core_runs(&data, &env_with(&seats, &installed, &lead_ok)).is_empty());
    assert!(record_started(&data, RUN, "p", Path::new("/tmp"), "codex").is_err(), "nothing is written through the link");
}

#[test]
fn a_second_holder_of_the_data_folder_gets_no_instance_lock_until_the_first_lets_go() {
    let t = Tmp::new("lock");
    let first = crate::desktop::instance::InstanceLock::take(&t.data()).expect("the first app holds the folder");
    assert!(crate::desktop::instance::InstanceLock::take(&t.data()).is_none(), "a second one does not");
    drop(first);
    assert!(crate::desktop::instance::InstanceLock::take(&t.data()).is_some(), "the lock goes with its holder, however it ends");
}

#[test]
fn a_record_naming_this_app_its_parent_or_a_non_leader_is_never_signalled() {
    let t = Tmp::new("notours");
    let ledger = Ledger::new(t.0.join("owned.jsonl"));
    let me = std::process::id();
    // SAFETY: getppid has no preconditions.
    let parent = unsafe { libc::getppid() } as u32;
    // A child in our own session: alive and stamped right, but not a session leader.
    let mut plain = Kid(Command::new("/bin/sleep").arg("60").spawn().unwrap());
    for pid in [me, parent, plain.0.id()] {
        let o = Owner { pid, stamp: crate::procid::start_stamp(pid).unwrap(), cwd: "/tmp".into(), kind: Kind::Lead, run: Some(RUN.into()), ticket: None, tries: 0, mark: Some(MARK.into()) };
        ledger.record(o).unwrap();
    }
    let ended: Vec<Ended> = ledger.end_agents().into_iter().map(|(_, e)| e).collect();
    assert_eq!(ended, vec![Ended::Reused; 3]);
    assert!(alive(&mut plain), "untouched");
    assert!(ledger.load().is_empty(), "and forgotten");
}

/// #176 review: a forged or damaged record naming another of this user's session leaders ends nothing. The record's pid
/// and stamp are right, and the target is a same-user session leader that is not this app, but its environment does not
/// carry the record's mark: one with no mark at all, one with another mark, and a record with no mark naming a process
/// that does carry one. Each is left running, and only the one the record and the process agree on is ended.
#[test]
fn a_record_whose_process_does_not_carry_its_mark_is_never_signalled() {
    let t = Tmp::new("mark");
    let ledger = Ledger::new(t.0.join("owned.jsonl"));
    let (mut a, mut b, mut c, mut ours) = (marked_kid("aaaa"), marked_kid("bbbb"), kid(), kid());
    let rec = |k: &Kid, mark: Option<&str>| Owner { mark: mark.map(String::from), ..agent(k, Kind::Lead, RUN, None) };
    ledger.record(rec(&a, Some(MARK))).unwrap(); // forged: claims our mark, the process has "aaaa"
    ledger.record(rec(&b, Some("cccc"))).unwrap(); // damaged: another mark than its own
    ledger.record(rec(&c, None)).unwrap(); // no mark: a record cannot vouch for itself
    ledger.record(rec(&ours, Some(MARK))).unwrap();
    let ended: Vec<Ended> = ledger.end_agents().into_iter().map(|(_, e)| e).collect();
    assert_eq!(ended, vec![Ended::Reused, Ended::Reused, Ended::Reused, Ended::Killed]);
    assert!(alive(&mut a) && alive(&mut b) && alive(&mut c), "no other session is signalled");
    assert!(!alive(&mut ours), "the one whose process carries its mark is ended");
}

/// #248 L1: a lead whose process is now a zombie (killed, not yet reaped) can't be read through `proc_pidinfo`, so its
/// stamp is unreadable and it judges `Gone`. Its pid is still held and the zombie's own stamp is the recorded one, so it
/// isn't proven gone or someone else's: it is never signalled, its record stays with one more try, and the next start
/// tries again. Once the pid is free (reaped), the record is forgotten.
#[test]
fn a_lead_whose_pid_is_held_but_unproven_is_kept_for_the_next_start() {
    let t = Tmp::new("unproven");
    let ledger = Ledger::new(t.0.join("owned.jsonl"));
    let mut z = Command::new("/bin/sleep").arg("60").spawn().unwrap();
    let o = Owner { pid: z.id(), stamp: crate::procid::start_stamp(z.id()).unwrap(), cwd: "/tmp".into(), kind: Kind::Lead, run: Some(RUN.into()), ticket: None, tries: 0, mark: Some(MARK.into()) };
    z.kill().unwrap();
    let t0 = std::time::Instant::now();
    while !crate::procid::is_zombie(z.id()) && t0.elapsed() < std::time::Duration::from_secs(2) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(crate::procid::is_zombie(z.id()), "the lead is a zombie: its pid is held");
    ledger.record(o.clone()).unwrap();
    let ended: Vec<Ended> = ledger.end_agents().into_iter().map(|(_, e)| e).collect();
    assert_eq!(ended, vec![Ended::NotProven]);
    let kept = ledger.load();
    assert_eq!((kept.len(), kept[0].pid, kept[0].tries), (1, o.pid, 1), "kept, with one more try: {kept:?}");
    let r = reconcile(&t.0, &ledger);
    assert_eq!(r.not_proven, 1, "{r:?}");
    // Reaped: the pid is free, so the record is proven gone and forgotten.
    z.wait().unwrap();
    let ended: Vec<Ended> = ledger.end_agents().into_iter().map(|(_, e)| e).collect();
    assert_eq!(ended, vec![Ended::Gone]);
    assert!(ledger.load().is_empty());
}

#[test]
fn a_record_that_survives_its_kill_is_tried_a_few_times_then_dropped() {
    let t = Tmp::new("tries");
    let ledger = Ledger::new(t.0.join("owned.jsonl"));
    // A pid that is free: a fixed number can be held by any process on a busy runner (it read Reused on CI, #182).
    let mut done = Command::new("/usr/bin/true").spawn().unwrap();
    done.wait().unwrap();
    let o = Owner { pid: done.id(), stamp: 7, cwd: "/tmp".into(), kind: Kind::Worker, run: Some(RUN.into()), ticket: Some("t".into()), tries: MAX_TRIES - 1, mark: Some(MARK.into()) };
    ledger.record(o).unwrap();
    // Gone (no such pid with that stamp): dropped at once whatever its count.
    assert_eq!(ledger.end_agents()[0].1, Ended::Gone);
    assert!(ledger.load().is_empty());
}

/// PR 259 review: an agent whose session has a job that can't be watched is not half-ended. At start-up nothing of it
/// is signalled, the leader included, so the kept record still names a live leader with its tries counted; the next
/// start, once the job can be watched, ends the whole session and forgets the record.
#[test]
fn an_agent_with_an_unwatched_job_is_kept_whole_until_its_session_can_be_ended() {
    let t = Tmp::new("unwatched-agent");
    let ledger = Ledger::new(t.0.join("owned.jsonl"));
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.args(["--exact", "procsweep::tests::sleeper_body", "--ignored", "--nocapture"]).env("VAJRA_SWEEP_SLEEP", "1").env("VAJRA_SWEEP_JOB", "1");
    c.env(crate::owned::AGENT_MARK, MARK).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
    // SAFETY: setsid is async-signal-safe and has no preconditions.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut lead = Kid(c.spawn().unwrap());
    let t0 = Instant::now();
    let job = loop {
        let job = crate::terminal::kill::session_members(lead.0.id()).into_iter().map(|p| p as u32).find(|&p| p != lead.0.id());
        let readable = crate::procsweep::env_var(lead.0.id(), crate::owned::AGENT_MARK).is_some_and(|v| v == MARK);
        if let (Some(j), true) = (job, readable) {
            break j;
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "the lead and its job never came up");
        std::thread::sleep(Duration::from_millis(20));
    };
    ledger.record(agent(&lead, Kind::Lead, RUN, None)).unwrap();
    crate::owned::REFUSE_WATCH.with(|r| r.borrow_mut().insert(job));
    for start in 1..=2u32 {
        let ended: Vec<Ended> = ledger.end_agents().into_iter().map(|(_, e)| e).collect();
        assert_eq!(ended, vec![Ended::Survived], "start {start}");
        // SAFETY: kill(pid, 0) only checks the pid.
        assert!(alive(&mut lead) && unsafe { libc::kill(job as i32, 0) } == 0, "start {start}: neither the lead nor its job was signalled");
        let kept = ledger.load();
        assert_eq!((kept.len(), kept[0].pid, kept[0].tries), (1, lead.0.id(), start), "start {start}: the record still names the live lead");
    }
    crate::owned::REFUSE_WATCH.with(|r| r.borrow_mut().clear());
    let ended: Vec<Ended> = ledger.end_agents().into_iter().map(|(_, e)| e).collect();
    assert_eq!(ended, vec![Ended::Killed], "now the whole session is ended");
    assert!(!alive(&mut lead) && ledger.load().is_empty());
    let t1 = Instant::now();
    // SAFETY: kill(pid, 0) only checks the pid.
    while unsafe { libc::kill(job as i32, 0) } == 0 && t1.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(20));
    }
    // SAFETY: as above.
    assert_ne!(unsafe { libc::kill(job as i32, 0) }, 0, "the job is ended with its session");
}

#[test]
fn shells_at_the_cap_never_push_out_a_lead_or_worker_record() {
    let t = Tmp::new("trim");
    let ledger = Ledger::new(t.0.join("owned.jsonl"));
    let worker = Owner { pid: 9_000, stamp: 1, cwd: "/tmp".into(), kind: Kind::Worker, run: Some(RUN.into()), ticket: Some("t".into()), tries: 0, mark: Some(MARK.into()) };
    ledger.record(worker.clone()).unwrap();
    for i in 0..300u32 {
        ledger.record(Owner::shell(10_000 + i, 1, "/tmp".into())).unwrap();
    }
    let all = ledger.load();
    assert_eq!(all.len(), 256);
    assert!(all.contains(&worker), "the oldest shells went, the worker stayed");
}

#[test]
fn a_crashed_ticket_after_a_torn_last_line_is_still_read() {
    let t = Tmp::new("torn");
    let data = t.data();
    let ticket = format!("{RUN}.t1.r1");
    crashed_run(&data, &ticket);
    // The crash cut the next line short.
    let f = data.join("runs").join(RUN).join("tickets.jsonl");
    let mut text = std::fs::read_to_string(&f).unwrap();
    text.push_str(r#"{"v":1,"event":"att"#);
    std::fs::write(&f, text).unwrap();
    assert!(crate::desktop::workers::record_crashed(&data, RUN, &ticket));
    assert_eq!(last_line(&data)["ticket"]["detail"], "app_crashed");
}

/// A FIFO planted as a run's `tickets.jsonl` (no writer, no reader) never holds up the start-up: the crash reconcile's
/// `record_crashed` and the runtime's replay both refuse it at once instead of waiting in `open`.
#[test]
fn a_fifo_planted_as_a_tickets_file_is_refused_without_blocking() {
    let t = Tmp::new("fifo");
    let data = t.data();
    let dir = data.join("runs").join(RUN);
    std::fs::create_dir_all(&dir).unwrap();
    let c = std::ffi::CString::new(dir.join("tickets.jsonl").into_os_string().into_encoded_bytes()).unwrap();
    // SAFETY: a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    let d = data.clone();
    std::thread::spawn(move || {
        let wrote = crate::desktop::workers::record_crashed(&d, RUN, &format!("{RUN}.t1.r1"));
        let _ = tx.send(wrote);
    });
    let wrote = rx.recv_timeout(Duration::from_secs(5)).expect("opening a planted FIFO blocked the reconcile");
    assert!(!wrote, "nothing is written into a FIFO");
    let meta = std::fs::symlink_metadata(dir.join("tickets.jsonl")).unwrap();
    assert!(std::os::unix::fs::FileTypeExt::is_fifo(&meta.file_type()), "it is left as it was");
}

#[test]
fn a_record_too_long_to_read_back_refuses_the_spawn_and_kills_its_process() {
    use crate::sessions::{SessionError, SpawnSpec, Supervisor};
    let t = Tmp::new("long");
    let ledger = Arc::new(Ledger::new(t.0.join("owned.jsonl")));
    // A shell record too: every caller is refused, not only the supervisor.
    assert!(ledger.record(Owner::shell(4242, 1, PathBuf::from(format!("/{}", "\"".repeat(2100))))).is_err(), "escaping doubles it past the cap");
    assert!(ledger.load().is_empty());

    let bus = Arc::new(crate::bus::Bus::open(&t.0.join("spill"), crate::bus::Config::default()).unwrap());
    let sup = Supervisor::new(bus, None);
    sup.set_ledger(ledger.clone());
    let marker = t.0.join("ran");
    let script = format!("sleep 1; echo x > '{}'", marker.display());
    let spec = SpawnSpec { argv: vec!["/bin/sh".into(), "-c".into(), script], cwd: t.0.clone(), rows: 24, cols: 80, redact: Default::default(), extra_env: vec![] };
    let who = crate::sessions::OwnedAs { kind: Some(Kind::Worker), run: Some(RUN.into()), ticket: Some("t".repeat(5000)) };
    let r = sup.spawn_owned("lead-long", spec, None, who);
    assert!(matches!(r, Err(SessionError::Ledger(_))), "{r:?}");
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!marker.exists(), "the process was killed before it could do anything");
    assert!(ledger.load().is_empty(), "no record the reader would skip was written");
}

#[test]
fn a_survivor_whose_record_cannot_be_written_back_is_reported_unrecorded() {
    let t = Tmp::new("unrec");
    let o = Owner { pid: 4242, stamp: 7, cwd: "/tmp".into(), kind: Kind::Worker, run: Some(RUN.into()), ticket: Some("t".into()), tries: 0, mark: Some(MARK.into()) };
    let kept = Ledger::new(t.0.join("owned.jsonl"));
    assert_eq!(kept.keep_survivor(&o), Ended::Survived);
    assert_eq!(kept.load()[0].tries, 1, "written back with one more try");
    let lost = Ledger::new(t.0.join("missing/owned.jsonl"));
    assert_eq!(lost.keep_survivor(&o), Ended::Unrecorded, "the write failed, and that is said, not dropped");
    assert_eq!(lost.keep_survivor(&Owner { tries: MAX_TRIES - 1, ..o }), Ended::GivenUp);
}

#[test]
fn a_run_folder_swapped_for_a_link_is_never_written_through() {
    let t = Tmp::new("swap");
    let data = t.data();
    let elsewhere = t.0.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    // `runs` itself a link: the walk from the data folder refuses it, for the start, the plan and the end alike.
    std::os::unix::fs::symlink(&elsewhere, data.join("runs")).unwrap();
    assert!(record_started(&data, RUN, "p", Path::new("/tmp"), "codex").is_err());
    assert!(record_plan(&data, RUN, &serde_json::json!({})).is_err());
    assert!(record_ended(&data, RUN, "lead_lost", "app_restarted").is_err());
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0, "nothing was made or written through the link");
    // A run id of more than one component is refused before any folder is opened.
    assert!(record_started(&data, "../x", "p", Path::new("/tmp"), "codex").is_err());
}
