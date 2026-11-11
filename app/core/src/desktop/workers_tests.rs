//! 6c acceptance tests: a fake harness on a temp dir (its mode written into the script), a real repo, a real `Gitw`
//! and a real `Supervisor`. No test touches the owner's own harness homes, and every process is killed on drop.

use super::*;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::providers::errors::{FakeClock, ProviderError};
use crate::vajra::gitw::{GlobalBaseline, Pause, Quiesce};

const GIT: &str = "/usr/bin/git";
const WAIT: Duration = Duration::from_secs(20);

#[derive(Debug)]
struct NoAgents;

impl Quiesce for NoAgents {
    fn pause(&self, _: &Path) -> Pause {
        Pause::new(|| {})
    }
}

/// Writes `file` into every worktree of run `r1` each time the core pauses the repo: the next clean check sees it.
#[derive(Debug)]
struct Spoil {
    run_dir: PathBuf,
    file: &'static str,
}

impl Quiesce for Spoil {
    fn pause(&self, _: &Path) -> Pause {
        for wt in std::fs::read_dir(&self.run_dir).into_iter().flatten().flatten() {
            let _ = std::fs::write(wt.path().join(self.file), "spoiled\n");
        }
        Pause::new(|| {})
    }
}

/// Holds the first pause after the harness has exited (its `end` line is in the log): that is the snapshot. The
/// test sees `entered`, acts, then `open`s the gate.
#[derive(Debug, Default)]
struct Gate {
    log: Mutex<Option<PathBuf>>,
    state: Mutex<(bool, bool)>,
    cv: Condvar,
}

impl Gate {
    fn wait_entered(&self) {
        let g = crate::lock(&self.state);
        let (g, t) = self.cv.wait_timeout_while(g, WAIT, |s| !s.0).unwrap();
        assert!(g.0 && !t.timed_out(), "the snapshot never paused");
    }

    fn open(&self) {
        crate::lock(&self.state).1 = true;
        self.cv.notify_all();
    }
}

impl Quiesce for Gate {
    fn pause(&self, _: &Path) -> Pause {
        let exited = crate::lock(&self.log).as_ref().and_then(|l| std::fs::read_to_string(l).ok()).is_some_and(|t| t.contains("end "));
        let mut g = crate::lock(&self.state);
        if exited && !g.0 {
            g.0 = true;
            self.cv.notify_all();
            let _g = self.cv.wait_while(g, |s| !s.1).unwrap();
        }
        Pause::new(|| {})
    }
}

fn tmpdir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("vajra-workers-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

fn ugit(home: &Path, cwd: &Path, args: &[&str]) -> String {
    let o = Command::new(GIT).args(args).current_dir(cwd).env_clear().env("PATH", "/usr/bin:/bin").env("HOME", home).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

/// The fake harness. It answers as either harness: as Codex it prints `thread.started` and writes a rollout whose
/// `turn_context` is what the profile asked for; as Claude (`-p`) it prints the `system`/`init` event. Then `mode`.
fn fake(dir: &Path, name: &str, mode: &str, codex_home: &Path, pids: &Path) -> PathBuf {
    let p = dir.join(name);
    let script = format!(
        r#"#!/bin/sh
MODE='{mode}'
WT=$(pwd -P)
case "$1" in
  -p) echo '{{"type":"system","subtype":"init","model":"claude-fake-1","permissionMode":"dontAsk","tools":["Read","Edit","Write","Bash","Grep","Glob"]}}' ;;
  *)
    TID=$(printf '%08x-0000-4000-8000-%012d' $$ $(date +%s))
    D="${{CODEX_HOME:-{codex_home}}}/sessions/2026/11/11"
    mkdir -p "$D"
    NET=false
    [ "$MODE" = wide ] && NET=true
    printf '{{"type":"turn_context","payload":{{"approval_policy":"never","cwd":"%s","model":"model-fake-1","effort":"medium","sandbox_policy":{{"type":"workspace-write","network_access":%s,"exclude_slash_tmp":true,"exclude_tmpdir_env_var":true,"writable_roots":["%s"]}}}}}}\n' "$WT" "$NET" "$TMPDIR" > "$D/rollout-x-$TID.jsonl"
    echo '{{"type":"thread.started","thread_id":"'$TID'"}}'
    ;;
esac
edit() {{ echo "worker line" >> src/lib.rs; }}
[ -n "$SLOW" ] || SLOW=0
echo "start $(basename "$WT") $(perl -MTime::HiRes=time -e 'printf "%.3f", time')" >> '{log}'
case "$MODE" in
  edit|wide) edit ;;
  flood) perl -e 'print "x" x 1000, "\n" for 1..400'; edit ;;
  slow) edit; sleep 0.4 ;;
  edit_then_504) edit; echo 'unexpected status 504 Gateway Timeout'; exit 1 ;;
  504) echo 'unexpected status 504 Gateway Timeout'; exit 1 ;;
  hang) exec sleep 600 ;;
  rewrite_dotgit) edit; echo 'gitdir: /tmp/elsewhere' > .git ;;
  commit) edit; git -c core.hooksPath=/dev/null -c user.name=W -c user.email=w@e commit -qam worker ;;
  nested_git) edit; mkdir -p src/sub/.git; echo 'ref: refs/heads/main' > src/sub/.git/HEAD ;;
  scratch_file) edit; echo scratch > other/scratch.txt ;;
  nonce) echo x > "src/NONCE-abcdef0123456789.rs" ;;
  setsid_child) edit; perl -MPOSIX=setsid -e 'if (fork() == 0) {{ setsid(); open(my $f, ">", $ARGV[0]); print $f $$; close $f; sleep 600; }}' '{pids}/setsid.pid' ;;
  nothing) : ;;
esac
echo "end $(basename "$WT") $(perl -MTime::HiRes=time -e 'printf "%.3f", time')" >> '{log}'
"#,
        codex_home = codex_home.display(),
        log = dir.join("log").display(),
        pids = pids.display(),
    );
    std::fs::write(&p, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

struct Rig {
    d: PathBuf,
    home: PathBuf,
    repo: PathBuf,
    base: String,
    gw: Arc<Gitw>,
    sup: Arc<Supervisor>,
    rt: WorkerRuntime,
    clock: Arc<FakeClock>,
    errors: Arc<Mutex<Vec<ProviderError>>>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        for run in ["r1", "r2"] {
            self.rt.kill_run_workers(run, KillReason::Stopped);
            // Every ticket thread has written its last line before the dir goes.
            for v in self.rt.list(run) {
                self.rt.await_any(&[v.id], WAIT);
            }
        }
        self.sup.shutdown();
        // A `setsid` child left its group: killed by its recorded pid.
        if let Ok(p) = std::fs::read_to_string(self.d.join("pids/setsid.pid")) {
            if let Ok(pid) = p.trim().parse::<i32>() {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
        let _ = std::fs::remove_dir_all(&self.d);
    }
}

struct Opts {
    mode: &'static str,
    timeout: Duration,
    caps: BTreeMap<String, u32>,
    files: Vec<(&'static str, &'static str)>,
    redact: Redactor,
    claude_ok: bool,
    hooks: Hooks,
    spoil: Option<&'static str>,
    gate: Option<Arc<Gate>>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { mode: "edit", timeout: WORKER_TIMEOUT, caps: BTreeMap::new(), files: Vec::new(), redact: Redactor::default(), claude_ok: false, hooks: Hooks::default(), spoil: None, gate: None }
    }
}

fn rig(tag: &str, o: Opts) -> Rig {
    let d = tmpdir(tag);
    let (home, data, work, repo, codex_home, pids) = (d.join("home"), d.join("data"), d.join("work"), d.join("repo"), d.join("codex-home"), d.join("pids"));
    for p in [&home, &data, &repo, &codex_home, &pids] {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(home.join(".gitconfig"), "[user]\n\tname = U\n\temail = u@e\n").unwrap();
    ugit(&home, &repo, &["init", "-q", "-b", "main"]);
    let mut files = vec![("src/lib.rs", "fn a() {}\n"), ("other/x.rs", "x\n"), ("a.txt", "a\n"), (".gitignore", "target/\nnode_modules/\n")];
    files.extend(o.files.iter().copied());
    for (p, c) in files {
        let f = repo.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    ugit(&home, &repo, &["add", "-A"]);
    ugit(&home, &repo, &["commit", "-q", "-m", "base"]);
    let base = ugit(&home, &repo, &["rev-parse", "HEAD"]);
    let g = GlobalBaseline::take_or_load(Path::new(GIT), &data, &home, None).unwrap();
    gitw::check_roots(&data, &work).unwrap();
    let quiesce: Arc<dyn Quiesce> = match (o.spoil, o.gate) {
        (Some(file), _) => Arc::new(Spoil { run_dir: work.canonicalize().unwrap().join("p/r1"), file }),
        (None, Some(g)) => {
            *crate::lock(&g.log) = Some(d.join("log"));
            g
        }
        (None, None) => Arc::new(NoAgents),
    };
    let gw = Arc::new(Gitw::new(GIT.into(), home.clone(), data.canonicalize().unwrap(), work.canonicalize().unwrap(), &g, quiesce));
    let common = gw.common_of(&repo).unwrap();
    gw.repos.acquire(&gw, &common, false).unwrap();
    let bus = Arc::new(Bus::open(&d.join("spill"), crate::bus::Config::default()).unwrap());
    let sup = Arc::new(Supervisor::new(bus.clone(), None));
    let errors: Arc<Mutex<Vec<ProviderError>>> = Arc::default();
    let e2 = errors.clone();
    sup.set_error_sink(Arc::new(move |e: &ProviderError| crate::lock(&e2).push(e.clone())));
    let clock = Arc::new(FakeClock::default());
    let bin = fake(&d, "harness", o.mode, &codex_home, &pids);
    let cfg = Config {
        home: home.clone(),
        data: data.canonicalize().unwrap(),
        codex: bin.clone(),
        claude: bin,
        codex_home,
        tools: WriterTools::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bundled-agents")),
        account_caps: o.caps,
        worker_timeout: o.timeout,
        poll: Duration::from_millis(20),
        clock: Some(clock.clone()),
        redact: o.redact,
        claude_unverified_ok: o.claude_ok,
    };
    let rt = WorkerRuntime::new(cfg, gw.clone(), sup.clone(), bus, o.hooks);
    Rig { d, home, repo: repo.canonicalize().unwrap(), base, gw, sup, rt, clock, errors }
}

fn seat(slot: &str, account: &str) -> WorkerSeat {
    WorkerSeat {
        slot: slot.into(), role: "backend".into(), task: "t1".into(), account: account.into(), model: None, effort: None, security: false, network: false,
        why: String::new(), source: "user".into(),
    }
}

impl Rig {
    fn task(&self, run: &str, task: &str) -> TaskSpec {
        TaskSpec {
            project: "p".into(), run: run.into(), repo: self.repo.clone(), task: task.into(), round: 1, from: self.base.clone(), deps: Vec::new(),
            globs: vec!["src/**".into()], named: Vec::new(), brief: "Do the task.".into(),
        }
    }

    fn done(&self, id: &str) -> TicketView {
        self.rt.await_any(&[id.to_string()], WAIT).unwrap_or_else(|| panic!("{id} never ended: {:?}", self.rt.view(id)))
    }

    fn log(&self) -> Vec<(String, String, f64)> {
        std::fs::read_to_string(self.d.join("log"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let mut w = l.split(' ');
                Some((w.next()?.into(), w.next()?.into(), w.next()?.parse().ok()?))
            })
            .collect()
    }
}

// ---------------------------------------------------------------- queues and caps

#[test]
fn three_tasks_on_one_seat_run_one_after_another() {
    let r = rig("seq", Opts { mode: "slow", ..Opts::default() });
    let ids: Vec<String> = ["t1", "t2", "t3"].iter().map(|t| r.rt.dispatch(r.task("r1", t), seat("w1", "codex")).unwrap()).collect();
    let (v2, v3) = (r.rt.view(&ids[1]).unwrap(), r.rt.view(&ids[2]).unwrap());
    assert_eq!((v2.state, v2.queue_position, v3.queue_position), (State::Queued, Some(1), Some(2)), "queued behind the busy seat");
    for id in &ids {
        assert_eq!(r.done(id).outcome, Some(Outcome::Done), "{:?}", r.rt.view(id));
    }
    let log = r.log();
    let span = |t: &str| {
        let s = log.iter().find(|(k, w, _)| k == "start" && w == &format!("{t}-r1")).unwrap().2;
        let e = log.iter().find(|(k, w, _)| k == "end" && w == &format!("{t}-r1")).unwrap().2;
        (s, e)
    };
    let (a, b, c) = (span("t1"), span("t2"), span("t3"));
    assert!(a.1 <= b.0 && b.1 <= c.0, "one at a time: {log:?}");
}

#[test]
fn two_default_seat_runs_start_side_by_side() {
    let r = rig("side", Opts { mode: "hang", ..Opts::default() });
    let a = r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap();
    let b = r.rt.dispatch(r.task("r2", "t1"), seat("w1", "codex")).unwrap();
    assert_eq!((r.rt.view(&a).unwrap().state, r.rt.view(&b).unwrap().state), (State::Running, State::Running));
}

#[test]
fn an_account_cap_queues_across_runs_and_mixed_providers_and_releases() {
    let caps: BTreeMap<String, u32> = [("codex".to_string(), 1)].into();
    let r = rig("cap", Opts { mode: "slow", caps, ..Opts::default() });
    let a = r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap();
    let b = r.rt.dispatch(r.task("r2", "t1"), seat("w1", "codex")).unwrap();
    let c = r.rt.dispatch(r.task("r2", "t2"), seat("w2", "openrouter")).unwrap();
    let vb = r.rt.view(&b).unwrap();
    assert_eq!(vb.state, State::Queued);
    assert_eq!(vb.wait.as_ref().unwrap()["code"], "account_busy", "{vb:?}");
    assert!(vb.wait.unwrap()["detail"].as_str().unwrap().starts_with("Waiting for codex"));
    assert_ne!(r.rt.view(&c).unwrap().state, State::Queued, "another account is not held by codex's cap");
    r.done(&a);
    assert_eq!(r.done(&b).outcome, Some(Outcome::Done));
}

#[test]
fn a_fresh_gateway_account_gets_its_codex_home_before_the_worker_starts() {
    let r = rig("gateway", Opts::default());
    let home = r.gw.data.join("codex-home/openrouter");
    assert!(!home.exists(), "no account home before the launch");
    let mut s = seat("w1", "openrouter");
    s.model = Some("openai/gpt-fake-1".into());
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), s).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    let cfg = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(cfg.contains("base_url = \"https://openrouter.ai/api/v1\""), "{cfg}");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(home.join("config.toml")).unwrap().permissions().mode() & 0o777, 0o600);
    // The worker ran with that home: its rollout landed there, not in the app's own Codex home.
    assert!(home.join("sessions").exists(), "the harness saw CODEX_HOME");
    // A failed write stops the launch: a file where the homes folder goes.
    let r = rig("gateway-fail", Opts::default());
    std::fs::write(r.gw.data.join("codex-home"), "not a folder\n").unwrap();
    let mut s = seat("w1", "openrouter");
    s.model = Some("openai/gpt-fake-1".into());
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), s).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("profile_not_applied")), "{v:?}");
    assert!(r.log().is_empty(), "nothing ran");
}

#[test]
fn two_ticket_ids_that_differ_only_in_dots_and_hyphens_get_their_own_sessions() {
    // `a-b.c.r1` and `a.b-c.r1` are both valid; rewriting dots to hyphens made them one supervisor session.
    let r = rig("sids", Opts { mode: "slow", ..Opts::default() });
    let a = r.rt.dispatch(r.task("a-b", "c"), seat("w1", "codex")).unwrap();
    let b = r.rt.dispatch(r.task("a", "b-c"), seat("w1", "codex")).unwrap();
    assert_eq!((a.as_str(), b.as_str()), ("a-b.c.r1", "a.b-c.r1"));
    let (va, vb) = (r.done(&a), r.done(&b));
    assert_eq!((va.outcome, vb.outcome), (Some(Outcome::Done), Some(Outcome::Done)), "{va:?} {vb:?}");
    let sid = |v: &TicketView| v.receipt.as_ref().unwrap()["session_id"].as_str().unwrap().to_string();
    assert_ne!(sid(&va), sid(&vb));
}

#[test]
fn a_startup_event_pushed_out_of_the_ring_still_validates_and_fills_the_receipt() {
    // About 400 KiB after `thread.started`: more than the bus ring (256 KiB) holds.
    let r = rig("flood", Opts { mode: "flood", ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    let receipt = v.receipt.unwrap();
    assert_eq!(receipt["effective"]["model"], "model-fake-1", "{receipt}");
    assert_eq!(receipt["effective"]["read_from"], "harness_session_file", "{receipt}");
}

// ---------------------------------------------------------------- controls

#[test]
fn a_stop_while_the_snapshot_runs_ends_the_ticket_stopped_and_not_integratable() {
    let gate = Arc::new(Gate::default());
    let r = rig("stopsnap", Opts { gate: Some(gate.clone()), ..Opts::default() });
    let id = r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap();
    gate.wait_entered();
    assert!(r.rt.cancel(&id), "the Stop is accepted while the snapshot runs");
    gate.open();
    let v = r.done(&id);
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Stopped), Some("stopped")), "{v:?}");
    // Whatever the snapshot committed, integration can't pick the ticket.
    let commits: Vec<String> = v.commit.clone().into_iter().collect();
    assert!(r.rt.integrated("r1", &commits, "refs/heads/vajra/r1/integration").is_empty(), "{v:?}");
    assert_eq!(r.rt.view(&id).unwrap().outcome, Some(Outcome::Stopped));
}

#[test]
fn stop_terminates_the_worker_with_no_provider_error() {
    let r = rig("stop", Opts { mode: "hang", ..Opts::default() });
    let id = r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap();
    let start = Instant::now();
    while r.rt.view(&id).unwrap().attempt == 0 && start.elapsed() < WAIT {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    assert!(r.rt.cancel(&id));
    let v = r.done(&id);
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Stopped), Some("stopped")));
    std::thread::sleep(Duration::from_millis(200));
    assert!(crate::lock(&r.errors).is_empty(), "{:?}", crate::lock(&r.errors));
    assert!(v.worktree.unwrap().exists(), "its work stays in its worktree");
}

#[test]
fn kill_run_workers_ends_the_running_one_and_drops_the_queue() {
    let r = rig("killrun", Opts { mode: "hang", ..Opts::default() });
    let a = r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap();
    let b = r.rt.dispatch(r.task("r1", "t2"), seat("w1", "codex")).unwrap();
    let other = r.rt.dispatch(r.task("r2", "t1"), seat("w1", "codex")).unwrap();
    assert_eq!(r.rt.kill_run_workers("r1", KillReason::Abandoned), 2);
    for id in [&a, &b] {
        let v = r.done(id);
        assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("abandoned")), "{v:?}");
    }
    assert_eq!(r.rt.view(&other).unwrap().state, State::Running, "another run's worker lives on");
}

// ---------------------------------------------------------------- timeout and retries

#[test]
fn a_worker_past_its_timeout_is_timed_out() {
    let r = rig("timeout", Opts { mode: "hang", timeout: Duration::from_millis(400), ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("timed_out")));
}

#[test]
fn a_504_on_an_unchanged_tree_is_retried_twice_then_needs_the_user() {
    let r = rig("retry", Opts { mode: "504", ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref(), v.attempt), (Some(Outcome::NeedsUser), Some("timeout"), 3), "{v:?}");
    assert_eq!(r.clock.slept(), errors::BACKOFF.to_vec());
    let attempts: Vec<u32> = crate::lock(&r.errors).iter().map(|e| e.attempt).collect();
    assert_eq!(attempts, vec![1, 2, 3], "each retry carries its own attempt, reported once (by the core alone)");
    let actions: Vec<Action> = crate::lock(&r.errors).iter().map(|e| e.action).collect();
    assert_eq!(actions, vec![Action::Retrying, Action::Retrying, Action::AskingUser]);
}

#[test]
fn a_retryable_error_after_an_edit_is_progress() {
    let r = rig("progress", Opts { mode: "edit_then_504", ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!((v.outcome, v.attempt), (Some(Outcome::Progress), 1), "{v:?}");
    assert!(r.clock.slept().is_empty(), "never retried on a changed tree");
    // The card agrees with the ticket (#149): one event, and it never says the core is retrying.
    let ev: Vec<(u32, Action)> = crate::lock(&r.errors).iter().map(|e| (e.attempt, e.action)).collect();
    assert_eq!(ev, vec![(1, Action::AskingUser)]);
}

// ---------------------------------------------------------------- dispatch_head, diff, tamper

#[test]
fn a_ticket_with_two_deps_is_not_tampered_and_its_diff_holds_only_the_worker_lines() {
    let r = rig("deps2", Opts::default());
    let mut deps = Vec::new();
    for (b, f) in [("d1", "dep1.txt"), ("d2", "dep2.txt")] {
        ugit(&r.home, &r.repo, &["checkout", "-q", "-b", b, &r.base]);
        std::fs::write(r.repo.join(f), "one\ntwo\nthree\n").unwrap();
        ugit(&r.home, &r.repo, &["add", f]);
        ugit(&r.home, &r.repo, &["commit", "-q", "-m", b]);
        deps.push(ugit(&r.home, &r.repo, &["rev-parse", "HEAD"]));
    }
    ugit(&r.home, &r.repo, &["checkout", "-q", "main"]);
    let mut spec = r.task("r1", "t1");
    spec.deps = deps.clone();
    let v = r.done(&r.rt.dispatch(spec, seat("w1", "codex")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    let head = v.dispatch_head.clone().unwrap();
    assert_ne!(head, r.base, "dispatch_head is after merge_deps");
    assert_eq!((v.files.clone(), v.added, v.removed), (vec!["src/lib.rs".to_string()], 1, 0), "only the worker's line");
    assert_eq!(ugit(&r.home, &r.repo, &["rev-parse", &format!("{}^", v.commit.unwrap())]), head, "the commit's parent is dispatch_head");
}

#[test]
fn a_rewritten_git_file_a_worker_commit_and_a_nested_repo_are_tampered() {
    for mode in ["rewrite_dotgit", "commit", "nested_git"] {
        let r = rig(mode, Opts { mode, ..Opts::default() });
        let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
        assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("tampered")), "{mode}: {v:?}");
        assert!(v.commit.is_none() && v.detail.is_some(), "the card names what changed: {v:?}");
    }
}

#[test]
fn no_changes_fail_and_a_wider_sandbox_is_killed() {
    let r = rig("nochange", Opts { mode: "nothing", ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("no_changes")));
    let r = rig("wide", Opts { mode: "wide", ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some(profiles::PROFILE_NOT_APPLIED)), "network on, asked off");
    assert!(v.commit.is_none());
}

#[test]
fn a_setsid_grandchild_does_not_hold_the_ticket() {
    let r = rig("setsid", Opts { mode: "setsid_child", ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
}

#[test]
fn a_failed_clean_check_at_dispatch_discards_the_worktree() {
    // `stray.rs` makes the check answer "changed"; an untracked `.gitattributes` makes it fail (#149).
    for file in ["src/stray.rs", "src/.gitattributes"] {
        let r = rig("dirtyatdispatch", Opts { spoil: Some(file), ..Opts::default() });
        let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
        assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("tampered")), "{file}: {v:?}");
        assert!(r.log().is_empty(), "{file}: no worker ran");
        // Only the run's shared `tmp` parent stays, empty: the ticket's own tmp dir went with the tree.
        let ls = |p: PathBuf| -> Vec<PathBuf> { std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).collect() };
        let run = r.d.join("work/p/r1");
        let left: Vec<_> = ls(run.clone()).into_iter().filter(|p| *p != run.join("tmp")).collect();
        assert!(left.is_empty() && ls(run.join("tmp")).is_empty(), "{file}: the worktree is gone: {left:?} {:?}", ls(run.join("tmp")));
        let list = ugit(&r.home, &r.repo, &["worktree", "list", "--porcelain"]);
        assert_eq!(list.matches("worktree ").count(), 1, "{file}: git lists only the main tree: {list}");
    }
}

// ---------------------------------------------------------------- dependencies and retention

#[test]
fn a_dependency_folder_is_cloned_into_the_worktree() {
    let r = rig("depclone", Opts { files: vec![("node_modules/pkg/index.js", "1\n")], ..Opts::default() });
    // node_modules is ignored, so the commit above didn't take it: it is only on disk.
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    assert_eq!(v.deps, vec![json!({"name": "node_modules", "cloned": true})]);
    assert!(v.worktree.unwrap().join("node_modules/pkg/index.js").exists());
}

#[test]
fn a_clean_worktree_is_removed_only_after_integration_and_a_leftover_keeps_one() {
    let r = rig("retain", Opts::default());
    let a = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    let wt = a.worktree.clone().unwrap();
    assert!(wt.exists(), "not removed at snapshot");
    let it = r.gw.integration_tree(&r.repo, "p", "r1", 1, &r.base).unwrap();
    let iref = Gitw::integration_ref(&it);
    let commit = a.commit.clone().unwrap();
    r.gw.integrate(&it, &r.base, std::slice::from_ref(&commit)).unwrap();
    let after = r.rt.integrated("r1", std::slice::from_ref(&commit), &iref);
    assert_eq!(after[0].retention.as_ref().unwrap()["state"], "removed", "{after:?}");
    assert!(!wt.exists());
    let refs = ugit(&r.home, &r.repo, &["for-each-ref", "--format=%(refname)", "refs/heads/vajra/r1/"]);
    assert!(!refs.contains("task-t1-r1"), "the task branch is deleted: {refs}");
}

#[test]
fn a_leftover_outside_the_task_globs_keeps_the_worktree() {
    let r = rig("leftover", Opts { mode: "scratch_file", ..Opts::default() });
    let a = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!((a.outcome, a.leftovers.clone()), (Some(Outcome::Done), vec!["other/scratch.txt".to_string()]));
    let it = r.gw.integration_tree(&r.repo, "p", "r1", 1, &r.base).unwrap();
    let commit = a.commit.clone().unwrap();
    r.gw.integrate(&it, &r.base, std::slice::from_ref(&commit)).unwrap();
    let after = r.rt.integrated("r1", &[commit], &Gitw::integration_ref(&it));
    let ret = after[0].retention.clone().unwrap();
    assert_eq!((ret["state"].as_str(), ret["paths"][0].as_str()), (Some("kept"), Some("other/scratch.txt")));
    assert!(a.worktree.unwrap().exists());
}

// ---------------------------------------------------------------- records, summary, hooks, harnesses

#[test]
fn tickets_jsonl_is_never_written_through_a_planted_link() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    // #149 bhishma M2: a link at `runs`, at `runs/<run>` or at the file fails the open; nothing lands outside.
    for at in ["runs", "runs/r1", "runs/r1/tickets.jsonl"] {
        let d = tmpdir("recordlink");
        let (data, out) = (d.join("data"), d.join("out"));
        std::fs::create_dir_all(&out).unwrap();
        std::fs::create_dir_all(data.join(at).parent().unwrap()).unwrap();
        let target = if at.ends_with(".jsonl") { out.join("tickets.jsonl") } else { out.clone() };
        symlink(&target, data.join(at)).unwrap();
        assert!(open_record(&data, "r1").is_err(), "{at}");
        assert!(std::fs::read_dir(&out).unwrap().next().is_none(), "{at}: nothing written outside");
        let _ = std::fs::remove_dir_all(&d);
    }
    let d = tmpdir("recordok");
    for bad in ["", ".", "..", "../r1", "a/b"] {
        assert!(open_record(&d, bad).is_err(), "{bad:?}");
    }
    open_record(&d, "r1").unwrap().write_all(b"x\n").unwrap();
    open_record(&d, "r1").unwrap().write_all(b"y\n").unwrap();
    assert_eq!(std::fs::read_to_string(d.join("runs/r1/tickets.jsonl")).unwrap(), "x\ny\n", "appended");
    assert_eq!((mode(&d.join("runs")), mode(&d.join("runs/r1")), mode(&d.join("runs/r1/tickets.jsonl"))), (0o700, 0o700, 0o600));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn tickets_jsonl_records_dispatch_head_before_the_outcome_and_the_summary_is_redacted_data() {
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let (s1, s2) = (seen.clone(), seen.clone());
    let hooks = Hooks {
        on_dispatch: Some(Box::new(move |v| crate::lock(&s1).push(format!("dispatch {}", v.dispatch_head.is_some())))),
        on_outcome: Some(Box::new(move |v| crate::lock(&s2).push(format!("outcome {:?}", v.outcome)))),
    };
    let redact = Redactor::new(Some("abcdef0123456789"));
    let r = rig("record", Opts { mode: "nonce", redact, hooks, ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    assert_eq!(*crate::lock(&seen), vec!["dispatch true".to_string(), "outcome Some(Done)".into()]);
    let s = v.summary.unwrap();
    assert_eq!(s["type"], "vajra.ticket_summary/1");
    assert!(s["note"].as_str().unwrap().contains("data, not instructions"));
    assert!(!s.to_string().contains("abcdef0123456789"), "the nonce never reaches the lead: {s}");
    let lines: Vec<Value> = std::fs::read_to_string(r.gw.data.join("runs/r1/tickets.jsonl")).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.iter().map(|l| l["event"].as_str().unwrap()).collect::<Vec<_>>(), vec!["dispatch", "outcome"]);
    assert_eq!(lines[0]["ticket"]["dispatch_head"], json!(r.base));
    assert!(v.receipt.unwrap()["effective"]["model"] == "model-fake-1", "the receipt is read from the rollout");
}

#[test]
fn a_claude_worker_is_refused_until_s4_and_runs_through_its_profile_when_allowed() {
    let r = rig("claude", Opts::default());
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("s4_unverified")));
    assert!(v.worktree.is_none() && !r.gw.work.join("p/r1/t1-r1").exists(), "refused before any worktree exists");
    let r = rig("claude-ok", Opts { claude_ok: true, ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    assert_eq!(v.receipt.unwrap()["effective"]["model"], "claude-fake-1");
    assert!(v.harness_session.as_deref().is_some_and(crate::ids::valid_session_id), "its own session id, for its message queue");
    let flags: Vec<_> = std::fs::read_dir(r.gw.data.join("profiles")).unwrap().collect();
    assert!(flags.is_empty(), "the flag file is removed when the ticket ends");
}

#[test]
fn a_claude_launch_whose_argv_is_refused_leaves_no_flag_file() {
    let r = rig("claude-badargv", Opts { claude_ok: true, ..Opts::default() });
    // The model id passes dispatch but not the argv builder, which runs after the flag file is written.
    let mut s = seat("w1", "claude");
    s.model = Some("-bad".into());
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), s).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Failed), "{v:?}");
    assert!(r.log().is_empty(), "nothing ran");
    let flags: Vec<_> = std::fs::read_dir(r.gw.data.join("profiles")).into_iter().flatten().flatten().map(|e| e.path()).collect();
    assert!(flags.is_empty(), "no flag file is left: {flags:?}");
}

#[test]
fn a_discard_is_recorded_in_tickets_jsonl() {
    let r = rig("discardrec", Opts::default());
    let id = r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap();
    let v = r.done(&id);
    r.rt.discard(&id).unwrap();
    assert!(!v.worktree.unwrap().exists());
    let text = std::fs::read_to_string(r.gw.data.join("runs/r1/tickets.jsonl")).unwrap();
    let last: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!((last["event"].as_str(), last["ticket"]["retention"]["state"].as_str()), (Some("discarded"), Some("discarded")), "{last}");
}

#[test]
fn a_check_ticket_starts_fresh_at_its_commit_and_is_not_judged_by_files() {
    let r = rig("check", Opts { mode: "nothing", ..Opts::default() });
    let v = r.done(&r.rt.dispatch_check(CheckKind::Verify, r.task("r1", "ignored"), seat("v1", "codex"), &r.base).unwrap());
    assert_eq!((v.kind, v.outcome, v.dispatch_head.as_deref()), ("verify", Some(Outcome::Done), Some(r.base.as_str())));
    assert!(v.commit.is_none());
}

/// A Stop during a retry's backoff (a real wait, no fake clock) ends the ticket at once, not after the wait.
#[test]
fn a_stop_during_a_retry_backoff_ends_the_ticket_at_once() {
    let mut r = rig("backoff", Opts { mode: "504", ..Opts::default() });
    let rt = {
        let s = &r.rt.0;
        let cfg = Config {
            home: s.cfg.home.clone(), data: s.cfg.data.clone(), codex: s.cfg.codex.clone(), claude: s.cfg.claude.clone(), codex_home: s.cfg.codex_home.clone(),
            tools: s.cfg.tools.clone(), account_caps: BTreeMap::new(), worker_timeout: WORKER_TIMEOUT, poll: s.cfg.poll, clock: None,
            redact: Redactor::default(), claude_unverified_ok: false,
        };
        WorkerRuntime::new(cfg, r.gw.clone(), r.sup.clone(), s.bus.clone(), Hooks::default())
    };
    r.rt = rt;
    let id = r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap();
    let start = Instant::now();
    while crate::lock(&r.errors).is_empty() && start.elapsed() < WAIT {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    let t0 = Instant::now();
    r.rt.cancel(&id);
    let v = r.done(&id);
    assert_eq!(v.outcome, Some(Outcome::Stopped), "{v:?}");
    assert!(t0.elapsed() < Duration::from_secs(5), "not after the 30 s backoff: {:?}", t0.elapsed());
}

// ---------------------------------------------------------------- the run protocol over the real runtime (6d1)

#[test]
fn the_run_protocol_drives_a_real_worker_through_integration_to_the_stub_gate() {
    use crate::desktop::plan::{Criterion, PlanTask};
    use crate::desktop::runproto::*;
    let r = rig("runproto", Opts::default());
    let cfg = RunConfig {
        project: "p".into(), run: "r1".into(), repo: r.repo.clone(), base: r.base.clone(), ask: "add a worker line".into(),
        seats: vec![seat("w1", "codex")], max_in_flight: 4, limits: Limits { rounds: 1, ..Limits::default() }, redact: Redactor::default(),
    };
    let workers = Arc::new(CoreWorkers { rt: r.rt.clone(), gw: r.gw.clone() });
    let p = RunProtocol::new(cfg, workers, Arc::new(StubGate), Arc::new(SystemClock::default()));
    let task = PlanTask {
        id: "t1".into(), title: "add the line".into(), owner_role: "backend".into(), files: vec!["src/**".into()], deps: vec![], criteria: vec!["ac-1".into()],
        brief: "Add a line to src/lib.rs.".into(), touches_security: false,
    };
    let crit = vec![Criterion { id: "ac-1".into(), criterion: "the line is there".into(), check: "grep -q worker src/lib.rs".into() }];
    p.plan_tasks(PlanInput { round: 1, criteria: Some(crit), tasks: vec![task], ..PlanInput::default() }).unwrap();
    let Dispatched::Ticket { ticket, slot } = p.dispatch_worker("t1").unwrap() else { panic!("held") };
    assert_eq!(slot, "w1");
    let ended = p.await_any(std::slice::from_ref(&ticket), Duration::from_secs(20)).unwrap().ended.expect("the worker ended");
    assert_eq!(ended.outcome.as_deref(), Some("done"), "{ended:?}");
    let wt = r.rt.view(&ticket).unwrap().worktree.unwrap();
    let round = p.integrate(1).unwrap();
    let rt = p.await_any(&[round], Duration::from_secs(20)).unwrap().ended.expect("the round ended");
    assert_eq!(rt.outcome.as_deref(), Some("failed"), "the stub gate never passes a round");
    assert_eq!(p.card().status, Some(RunStatus::NotVerified));
    let head = ugit(&r.home, &r.repo, &["rev-parse", "refs/heads/vajra/r1/integration"]);
    let lib = ugit(&r.home, &r.repo, &["show", &format!("{head}:src/lib.rs")]);
    assert!(lib.contains("worker line"), "the worker's change is on the integration branch: {lib}");
    assert!(!wt.exists(), "the merged ticket's worktree is cleaned up after integrate");
}
