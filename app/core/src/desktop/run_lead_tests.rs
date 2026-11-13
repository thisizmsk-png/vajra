//! A run lead's life with a fake `claude` (ADR-16 16.5, N9; 6d2c): the supervisor runs a shell script that prints an
//! init event, then does what its mode file says. Temp HOME and data dir only; the supervisor kills every lead by its
//! process group, and `Drop` shuts the lead down.

use super::*;
use crate::bus::Bus;
use crate::control::service::WorkerSeat;
use crate::desktop::runproto::{Integration, Limits, RunConfig, StubGate, SystemClock, Tick, Workers};
use crate::desktop::workers::{DispatchError, KillReason, Outcome, State as TState, TaskSpec};
use crate::vajra::gitw::GitwError;
use std::collections::BTreeMap;

const RUN: &str = "20261112-0930";

/// One worker ticket per dispatch, ended only by the test (or a run kill).
#[derive(Default)]
struct Held(Mutex<BTreeMap<String, Tick>>);

impl Workers for Held {
    fn dispatch(&self, spec: TaskSpec, _: WorkerSeat) -> Result<String, DispatchError> {
        let id = format!("{}.{}.r{}", spec.run, spec.task, spec.round);
        let t = Tick {
            id: id.clone(), state: TState::Running, outcome: None, reason: None, commit: None, attempt: 1, elapsed_s: 0, queue_position: None, replayed: false,
            worktree: None,
        };
        crate::lock(&self.0).insert(id.clone(), t);
        Ok(id)
    }
    fn view(&self, ticket: &str) -> Option<Tick> {
        crate::lock(&self.0).get(ticket).cloned()
    }
    fn await_any(&self, tickets: &[String], _: Duration) -> Option<Tick> {
        let b = crate::lock(&self.0);
        tickets.iter().filter_map(|t| b.get(t)).find(|t| t.state == TState::Ended).cloned()
    }
    fn cancel(&self, _: &str) -> bool {
        false
    }
    fn kill_run(&self, _: &str, _: KillReason) {
        for t in crate::lock(&self.0).values_mut() {
            t.state = TState::Ended;
            t.outcome = Some(Outcome::Failed);
        }
    }
    fn integrate(&self, _: &Integration<'_>, _: &[String]) -> Result<String, GitwError> {
        Err(GitwError::Invalid("no merges here"))
    }
    // These tests are about the lead's life, not the writer cap: tickets start when dispatched.
    fn limit_writers(&self, _: &str, _: u32) {}
}

fn seat(slot: &str, role: &str) -> WorkerSeat {
    WorkerSeat {
        slot: slot.into(), role: role.into(), task: "t".into(), account: "claude".into(), model: None, effort: None, security: false, network: false,
        why: String::new(), source: "user".into(),
    }
}

struct Rig {
    d: PathBuf,
    lead: RunLead,
    reg: Arc<RunRegistry>,
    held: Arc<Held>,
    _sup: Arc<Supervisor>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.lead.shutdown(Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(&self.d);
    }
}

/// The fake reads `<dir>/mode` at each launch: `exit` (init, then exit 0), `hang` (init, then sleep), `wide` (an init
/// with Bash, which the read-back refuses, then sleep), `silent` (no init, sleep), `early` (no init, exit 0), `orphan`
/// (init, a background child that ignores the hangup, its fds away from the terminal, its pid in `<dir>/orphan`, then exit 0). Each launch appends its argv's
/// token-file path to `<dir>/launches`.
fn fake_claude(dir: &Path) -> PathBuf {
    let tools = profiles::claude_lead_tools().split(',').map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(",");
    let p = dir.join("claude");
    let script = format!(
        r#"#!/bin/sh
MODE=$(cat '{dir}/mode')
echo "launch $MCP_TOOL_TIMEOUT" >> '{dir}/launches'
for a in "$@"; do LAST=$a; done
printf '%s\n' "$LAST" | head -n 1 >> '{dir}/prompts'
case "$MODE" in
  wide) echo '{{"type":"system","subtype":"init","permissionMode":"dontAsk","tools":["Bash",{tools}],"mcp_servers":[{{"name":"vajra_run","status":"connected"}}]}}'; exec sleep 600 ;;
  silent) exec sleep 600 ;;
  early) exit 0 ;;
esac
echo '{{"type":"system","subtype":"init","permissionMode":"dontAsk","tools":[{tools}],"mcp_servers":[{{"name":"vajra_run","status":"connected"}}]}}'
case "$MODE" in
  exit) exit 0 ;;
  hang) exec sleep 600 ;;
  orphan) nohup sleep 600 < /dev/null > /dev/null 2>&1 & echo $! > '{dir}/orphan'; sleep 1; exit 0 ;;
esac
"#,
        dir = dir.display()
    );
    std::fs::write(&p, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn rig(tag: &str, mode: &str, read_back: Duration) -> Rig {
    let d = std::env::temp_dir().join(format!("vajra-rl-{tag}-{}", std::process::id())).canonicalize().unwrap_or_else(|_| {
        let p = std::env::temp_dir().join(format!("vajra-rl-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p.canonicalize().unwrap()
    });
    let (home, data, repo) = (d.join("home"), d.join("data"), d.join("repo"));
    for p in [&home, &data, &repo] {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(d.join("mode"), mode).unwrap();
    let claude = fake_claude(&d);
    let held = Arc::new(Held::default());
    let cfg = RunConfig {
        project: "p".into(), run: RUN.into(), repo: repo.clone(), base: "0".repeat(40), ask: "fix it".into(),
        seats: vec![seat("w1", "backend"), seat("w2", "verifier"), seat("w3", "reviewer")], max_in_flight: 2, limits: Limits::default(),
        redact: Redactor::default(),
    };
    let proto = RunProtocol::new(cfg, held.clone(), Arc::new(StubGate), Arc::new(SystemClock::default()));
    let reg = RunRegistry::new();
    let (_, hash) = runreg::write_token(&data.join("runs").join(RUN)).unwrap();
    reg.register(RUN, proto.clone(), hash);
    let bus = Arc::new(Bus::open(&d.join("spill"), crate::bus::Config::default()).unwrap());
    let sup = Arc::new(Supervisor::new(bus.clone(), None));
    let env = LeadEnv {
        data: data.clone(), home, exe: PathBuf::from("/bin/echo"), socket: d.join("app.sock"), codex: PathBuf::from("/nonexistent/codex"), claude,
        user_codex_home: d.join("codex-home"), repos: vec![repo.clone()], others: vec![], read_back_timeout: read_back,
    };
    let lead_seat = LeadSeat { account: "claude".into(), model: None, effort: None };
    let lead = RunLead::new(RUN, repo.to_str().unwrap(), "fix it", 2, vec!["backend".into()], lead_seat, env, proto, reg.clone(), sup.clone(), bus).unwrap();
    Rig { d, lead, reg, held, _sup: sup }
}

fn plan(p: &RunProtocol) {
    use crate::desktop::plan::{Criterion, PlanTask};
    use crate::desktop::runproto::PlanInput;
    let t = PlanTask { id: "t1".into(), title: "do".into(), owner_role: "backend".into(), files: vec!["src/".into()], deps: vec![], criteria: vec!["ac-1".into()], brief: "b".into(), touches_security: false };
    p.plan_tasks(PlanInput { round: 1, criteria: Some(vec![Criterion { id: "ac-1".into(), criterion: "c".into(), check: "true".into() }]), tasks: vec![t], ..PlanInput::default() }).unwrap();
    p.dispatch_worker("t1").unwrap();
}

fn token(r: &Rig) -> String {
    std::fs::read_to_string(r.d.join("data/runs").join(RUN).join(runreg::TOKEN_FILE)).unwrap()
}

/// Until the fake has started and printed its init (the read-back passed), at most 20 s under load.
fn wait_launched(r: &Rig) {
    let until = Instant::now() + Duration::from_secs(20);
    while launches(r) == 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
}

fn launches(r: &Rig) -> usize {
    std::fs::read_to_string(r.d.join("launches")).map(|s| s.lines().count()).unwrap_or(0)
}

#[test]
fn an_early_exit_with_a_live_worker_restarts_once_with_a_rotated_token_and_a_second_is_lead_lost() {
    let r = rig("n9", "exit", Duration::from_secs(10));
    plan(r.lead.proto());
    r.lead.start().unwrap();
    let first = token(&r);
    r.lead.watch();
    assert_eq!(launches(&r), 2, "one restart, then no more");
    assert_ne!(token(&r), first, "the restart got a fresh token");
    assert_eq!(r.lead.proto().card().status, Some(RunStatus::LeadLost));
    assert!(crate::lock(&r.held.0).values().all(|t| t.state == TState::Ended), "lead_lost kills the run's workers");
    let lines = std::fs::read_to_string(r.d.join("launches")).unwrap();
    assert!(lines.lines().all(|l| l == format!("launch {}", profiles::CLAUDE_LEAD_TOOL_TIMEOUT_MS)), "the MCP timeout is pinned: {lines}");
    let prompts = std::fs::read_to_string(r.d.join("prompts")).unwrap();
    let firsts: Vec<&str> = prompts.lines().collect();
    assert!(charter::CHARTER.starts_with(firsts[0]), "the first lead gets the charter: {firsts:?}");
    assert!(charter::RESUME_PREAMBLE.starts_with(firsts[1]), "the restart gets the resume charter: {firsts:?}");
}

#[test]
fn an_exit_with_nothing_running_ends_the_run_without_a_restart() {
    let r = rig("idle", "exit", Duration::from_secs(10));
    r.lead.start().unwrap();
    r.lead.watch();
    assert_eq!(launches(&r), 1);
    assert!(r.lead.proto().card().status.is_some());
}

#[test]
fn a_lead_wider_than_its_profile_is_killed_and_the_run_is_lead_lost_with_the_reason() {
    let r = rig("wide", "wide", Duration::from_secs(10));
    plan(r.lead.proto());
    r.lead.start().unwrap();
    r.lead.watch();
    assert_eq!(launches(&r), 1, "a lead that is not confined is never restarted");
    let (status, why) = r.lead.ended();
    assert_eq!(status, Some(RunStatus::LeadLost));
    assert!(why.unwrap().contains("Bash"), "the card says what was wider");
}

#[test]
fn a_lead_that_never_shows_its_read_back_is_ended() {
    let r = rig("silent", "silent", Duration::from_millis(800));
    plan(r.lead.proto());
    r.lead.start().unwrap();
    r.lead.watch();
    assert!(r.lead.ended().1.unwrap().contains("no read-back"));
}

#[test]
fn shutdown_kills_the_lead_within_its_wait_and_never_restarts_it() {
    let r = rig("quit", "hang", Duration::from_secs(10));
    plan(r.lead.proto());
    r.lead.start().unwrap();
    let w = r.lead.clone();
    let watch = std::thread::spawn(move || w.watch());
    wait_launched(&r);
    let started = Instant::now();
    assert_eq!(r.lead.shutdown(SHUTDOWN_WAIT), Shutdown::Stopped);
    assert!(started.elapsed() < SHUTDOWN_WAIT);
    watch.join().unwrap();
    assert_eq!(launches(&r), 1, "a shutdown is not an early exit");
    assert_eq!(r.lead.shutdown(SHUTDOWN_WAIT), Shutdown::NotRunning);
    assert_eq!(r.reg.serve(&json!({"vajra_run": RUN, "token": token(&r), "tool": "list_workers", "args": {}}), None)["error"], "lead_not_running");
}

/// The app's quit with a live core run (a dispatched worker, so an ordinary exit would restart the lead): the lead is
/// stopped by the quit's lead step, never restarted, and admits no call once `quit` has begun.
#[test]
fn the_apps_quit_stops_a_live_core_runs_lead_and_never_restarts_it() {
    let r = rig("appquit", "hang", Duration::from_secs(10));
    plan(r.lead.proto());
    r.lead.start().unwrap();
    let w = r.lead.clone();
    let watch = std::thread::spawn(move || w.watch());
    wait_launched(&r);
    let call = || r.reg.serve(&json!({"vajra_run": RUN, "token": token(&r), "tool": "list_workers", "args": {}}), None);
    // A call with no peer reaches the lead check while the lead is live (`peer_not_lead`), and stops short of it once
    // no lead is running (`lead_not_running`).
    assert_eq!(call()["error"], "peer_not_lead", "the lead is live before the quit");

    let (data, home) = (r.d.join("app-data"), r.d.join("app-home"));
    std::fs::create_dir_all(&home).unwrap();
    let app = crate::desktop::App::open(&data, &home).unwrap();
    app.adopt_lead_run(RUN, r.lead.clone());
    app.quit();
    // The watch ended on the quit's stop, not on an exit it would restart from (bounded: a quit that left the lead
    // running fails here instead of hanging).
    let until = Instant::now() + Duration::from_secs(10);
    while !watch.is_finished() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(watch.is_finished(), "the quit left the core run's lead running");
    watch.join().unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(launches(&r), 1, "the quit restarted the lead");
    assert_eq!(call()["error"], "lead_not_running", "a call was admitted after the quit began");
    assert_eq!(r.lead.shutdown(SHUTDOWN_WAIT), Shutdown::NotRunning, "the quit already stopped it");
    let input = crate::sessions::argv::RunInput { ask: "x".into(), max_rounds: 1, base: "main".into(), max_in_flight: 1, campaign: None, provider: None };
    assert_eq!(app.start_lead_run("p", input, "/tmp", "20261112-0931").unwrap_err()["error"], "quitting");
}

#[test]
fn a_run_that_ends_stops_its_lead() {
    let r = rig("end", "hang", Duration::from_secs(10));
    plan(r.lead.proto());
    r.lead.start().unwrap();
    let w = r.lead.clone();
    let watch = std::thread::spawn(move || w.watch());
    wait_launched(&r);
    r.lead.proto().stop();
    watch.join().unwrap();
    assert_eq!(launches(&r), 1);
    assert_eq!(r.lead.shutdown(SHUTDOWN_WAIT), Shutdown::NotRunning, "the watch already ended it");
}

#[test]
fn a_lead_that_exits_before_its_read_back_is_not_applied_and_never_restarted() {
    let r = rig("early", "early", Duration::from_secs(10));
    plan(r.lead.proto());
    r.lead.start().unwrap();
    r.lead.watch();
    assert_eq!(launches(&r), 1, "a live worker would earn a restart, but this lead was never confirmed confined");
    let (status, why) = r.lead.ended();
    assert_eq!(status, Some(RunStatus::LeadLost));
    assert!(why.unwrap().contains("before its read-back"));
}

#[test]
fn a_lead_that_exits_leaves_no_child_of_its_group_behind() {
    let r = rig("orphan", "orphan", Duration::from_secs(10));
    r.lead.start().unwrap();
    r.lead.watch();
    let pid: i32 = std::fs::read_to_string(r.d.join("orphan")).unwrap().trim().parse().unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    // SAFETY: signal 0 only asks whether the pid exists.
    while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    if alive {
        // SAFETY: the pid is this test's own leftover child, still alive a moment ago.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert!(!alive, "the lead's background child was killed with its group");
}

#[test]
fn a_lead_launched_after_the_shutdown_began_is_killed_and_never_admitted() {
    let r = rig("late", "hang", Duration::from_secs(10));
    plan(r.lead.proto());
    assert_eq!(r.lead.shutdown(SHUTDOWN_WAIT), Shutdown::NotRunning);
    assert!(r.lead.start().is_err(), "a launch that finds the shutdown is refused");
    assert_eq!(r.lead.shutdown(SHUTDOWN_WAIT), Shutdown::NotRunning, "and it never became the current lead");
    assert_eq!(r.reg.serve(&json!({"vajra_run": RUN, "token": token(&r), "tool": "list_workers", "args": {}}), None)["error"], "lead_not_running");
}
