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
/// A Claude Code build S4 proved with today's flag shape (`profiles::CLAUDE_WORKER_PROVEN`).
const PROVEN_VERSION: &str = "2.1.296 (Claude Code)";

fn fake(dir: &Path, name: &str, mode: &str, codex_home: &Path, pids: &Path, version: &str) -> PathBuf {
    let p = dir.join(name);
    let script = format!(
        r#"#!/bin/sh
MODE='{mode}'
WT=$(pwd -P)
[ "$1" = --version ] && {{ if [ -f '{log}.upgraded' ]; then echo '9.9.9 (Claude Code)'; else echo '{version}'; fi; exit 0; }}
# Run as canary A's self-test (it talks to the loopback stub): this stand-in is no real Claude Code, so it fails.
[ -n "$ANTHROPIC_BASE_URL" ] && {{ echo selftest >> '{log}.selftest'; exit 1; }}
case "$1" in
  -p) printf '%s\n' "$@" > '{log}.argv'
    if [ "$MODE" = mcp ]; then
      echo '{{"type":"system","subtype":"init","model":"claude-fake-1","permissionMode":"dontAsk","tools":["Read","Edit","Write","Bash","Grep","Glob","mcp__files__read"],"mcp_servers":[{{"name":"files","status":"connected","source":"user"}}]}}'
    else
      echo '{{"type":"system","subtype":"init","model":"claude-fake-1","permissionMode":"dontAsk","tools":["Read","Edit","Write","Bash","Grep","Glob"]}}'
    fi ;;
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
  edit|wide|mcp) edit ;;
  flood) perl -e 'print "x" x 1000, "\n" for 1..400'; edit ;;
  slow) edit; sleep 0.4 ;;
  edit_then_504) edit; echo 'unexpected status 504 Gateway Timeout'; exit 1 ;;
  504) echo 'unexpected status 504 Gateway Timeout'; exit 1 ;;
  504_upgrade) touch '{log}.upgraded'; echo 'unexpected status 504 Gateway Timeout'; exit 1 ;;
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
    /// What the fake harness answers to `--version`; the default is a build S4 proved.
    claude_version: &'static str,
    /// Claude is `canary_claude` (a build only a self-test here can prove); `Some(true)` is the leaky one.
    canary: Option<bool>,
    hooks: Hooks,
    spoil: Option<&'static str>,
    steer: Option<Steer>,
    gate: Option<Arc<Gate>>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { mode: "edit", timeout: WORKER_TIMEOUT, caps: BTreeMap::new(), files: Vec::new(), redact: Redactor::default(), claude_version: PROVEN_VERSION, canary: None, hooks: Hooks::default(), spoil: None, gate: None, steer: None }
    }
}

fn rig(tag: &str, o: Opts) -> Rig {
    let d = tmpdir(tag);
    let (home, data, work, repo, codex_home, pids) = (d.join("home"), d.join("data"), d.join("work"), d.join("repo"), d.join("codex-home"), d.join("pids"));
    for p in [&home, &data, &repo, &codex_home, &pids] {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(home.join(".gitconfig"), "[user]\n\tname = U\n\temail = u@e\n").unwrap();
    install_repo_hooks(&home);
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
    let bin = fake(&d, "harness", o.mode, &codex_home, &pids, o.claude_version);
    let cfg = Config {
        home: home.clone(),
        data: data.canonicalize().unwrap(),
        codex: bin.clone(),
        claude: match o.canary {
            Some(leaky) => canary_claude(&d, leaky),
            None => bin,
        },
        codex_home,
        tools: WriterTools::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bundled-agents")),
        account_caps: o.caps,
        worker_timeout: o.timeout,
        poll: Duration::from_millis(20),
        clock: Some(clock.clone()),
        redact: o.redact,
        steer: o.steer,
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

#[test]
fn the_disk_walk_stops_at_its_entry_cap_and_says_the_size_is_partial() {
    let d = tmpdir("diskcap");
    for i in 0..10 {
        std::fs::write(d.join(format!("f{i}")), vec![1u8; 8192]).unwrap();
    }
    let (all, partial) = allocated_upto(&d, 1000);
    assert!(!partial && all >= 10 * 8192, "{all}");
    let (some, partial) = allocated_upto(&d, 4);
    assert!(partial && some < all, "{some} < {all}");
    let _ = std::fs::remove_dir_all(&d);
}

/// #151 review: the budget bounds the work, not only the bytes. A wide directory is read entry by entry and the walk
/// stops at the cap; it never lists the whole level first.
#[test]
fn the_disk_walk_reads_no_more_entries_than_its_budget_in_a_wide_directory() {
    let d = tmpdir("diskwide");
    let wide = d.join("cache");
    std::fs::create_dir_all(&wide).unwrap();
    for i in 0..3000 {
        std::fs::write(wide.join(format!("f{i}")), b"x").unwrap();
    }
    let w = walk(&d, 10);
    assert_eq!((w.read, w.partial), (10, true), "{w:?}");
    // Root, `cache` and its 3000 files: an exact budget reads them all and is not partial.
    let all = walk(&d, 3002);
    assert_eq!((all.read, all.partial), (3002, false), "{all:?}");
    assert!(walk(&d, 3001).partial);
    assert_eq!(walk(&d, 0), Walk { bytes: 0, partial: true, read: 0 });
    let _ = std::fs::remove_dir_all(&d);
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

#[test]
fn clean_up_removes_only_an_integrated_copy_that_is_now_clean() {
    let r = rig("cleanup", Opts { mode: "scratch_file", ..Opts::default() });
    let a = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    let b = r.done(&r.rt.dispatch(r.task("r1", "t2"), seat("w1", "codex")).unwrap());
    let (wa, wb) = (a.worktree.clone().unwrap(), b.worktree.clone().unwrap());
    let it = r.gw.integration_tree(&r.repo, "p", "r1", 1, &r.base).unwrap();
    let iref = Gitw::integration_ref(&it);
    let commit = a.commit.clone().unwrap();
    r.gw.integrate(&it, &r.base, std::slice::from_ref(&commit)).unwrap();
    assert_eq!(r.rt.integrated("r1", std::slice::from_ref(&commit), &iref)[0].retention.as_ref().unwrap()["state"], "kept");

    // The leftover is still there: nothing is removed on a guess.
    assert!(r.rt.clean_up().removed.is_empty());
    assert!(wa.exists() && wb.exists());

    // The human deletes the leftover. Now the capture check passes; the never-integrated copy stays.
    std::fs::remove_file(wa.join("other/scratch.txt")).unwrap();
    let done = r.rt.clean_up();
    assert_eq!(done.removed, vec![a.id.clone()], "{done:?}");
    assert!(!wa.exists() && wb.exists());
    assert_eq!(r.rt.view(&a.id).unwrap().retention.unwrap()["state"], "removed");
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
fn a_claude_worker_runs_through_its_profile_now_that_s4_passed() {
    // No test override: S4 passed, so the profile itself allows the Claude worker.
    let r = rig("claude", Opts::default());
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    assert_eq!(v.receipt.as_ref().unwrap()["effective"]["model"], "claude-fake-1");
    assert!(v.harness_session.as_deref().is_some_and(crate::ids::valid_session_id), "its own session id, for its message queue");
    let flags: Vec<_> = std::fs::read_dir(r.gw.data.join("profiles")).unwrap().collect();
    assert!(flags.is_empty(), "the flag file is removed when the ticket ends");
    assert_eq!(v.key_guard, Some(json!({"guarded": true})), "no MCP server loaded");
    assert!(!v.core_worker().overlays.contains(&"not guarded"));
}

/// An MCP server runs outside the OS sandbox and can read the signing key. A Claude worker whose init event shows one
/// loaded still runs and ends Done, but is not guarded, and the reason names the server.
#[test]
fn a_claude_worker_with_a_loaded_mcp_server_runs_but_is_not_guarded() {
    let r = rig("claude-mcp", Opts { mode: "mcp", ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    let g = v.key_guard.clone().unwrap();
    assert_eq!((g["guarded"].as_bool(), g["mcp_servers"].clone()), (Some(false), json!(["files"])), "{g}");
    assert!(g["reason"].as_str().unwrap().contains("MCP server files runs outside the sandbox"), "{g}");
    assert!(v.core_worker().overlays.contains(&"not guarded"), "the card says so");
    // A Codex worker has no Claude key guard: its rule lives with the Codex sandbox.
    let c = r.done(&r.rt.dispatch(r.task("r2", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!(c.key_guard, None, "{c:?}");
}

/// #180 bhishma: S4 proved one Claude Code build, but users auto-update. A build no S4 run proved (with today's flag
/// shape) and no self-test here proved is refused, before any worktree or flag file exists and before it runs as a
/// worker. Here the self-test runs and the stand-in harness fails it.
#[test]
fn a_claude_worker_on_an_unproven_build_is_refused_before_any_worktree_exists() {
    let r = rig("claude-unproven", Opts { claude_version: "9.9.9 (Claude Code)", ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("claude_selftest_failed")), "{v:?}");
    assert!(v.worktree.is_none() && !r.gw.work.join("p/r1/t1-r1").exists(), "refused before any worktree exists");
    assert!(!r.gw.data.join("control/claude-proven.json").exists(), "nothing was added to this Mac's list");
    assert!(std::fs::read_dir(r.gw.data.join("profiles")).map_or(true, |d| d.count() == 0), "no flag file was written");
    assert!(!r.d.join("log").exists(), "the harness never ran as a worker");
    assert!(r.d.join("log.selftest").exists(), "it ran only as the self-test");
    // A version probe that fails is refused the same way (fail closed).
    let r = rig("claude-noversion", Opts { claude_version: "", ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("claude_unproven")), "{v:?}");
    assert!(v.worktree.is_none(), "{v:?}");
    // A Codex worker is not asked about Claude at all.
    let r = rig("codex-unaffected", Opts { claude_version: "9.9.9 (Claude Code)", ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
}

/// A stand-in for a new Claude Code build: `--version` says 9.9.9; with ANTHROPIC_BASE_URL set (the self-test) it
/// talks to the stub and refuses data-dir writes the way the guard and deny rules do (or, `leaky`, lets an Edit
/// through); without it (a real worker run) it starts and edits like the shell fake. Each self-test is counted.
fn canary_claude(dir: &Path, leaky: bool) -> PathBuf {
    let p = dir.join("canary-claude");
    let script = format!(
        r#"#!/usr/bin/python3 -I
import json, os, subprocess, sys, urllib.request
args = sys.argv[1:]
if args[:1] == ["--version"]:
    print("9.9.10 (Claude Code)" if os.path.exists("{count}.swapped") else "9.9.9 (Claude Code)"); sys.exit(0)
tools = args[args.index("--tools") + 1].split(",")
print(json.dumps({{"type": "system", "subtype": "init", "model": "claude-fake-1", "tools": tools, "permissionMode": "dontAsk"}}), flush=True)
base = os.environ.get("ANTHROPIC_BASE_URL")
if not base:
    open("src/lib.rs", "a").write("worker line\n"); sys.exit(0)
open("{count}", "a").write("selftest\n")
if os.path.exists("{count}.swap"):
    os.rename("{count}.swap", "{count}.swapped")  # the CLI updates in place while its self-test runs
msgs = [{{"role": "user", "content": args[-1]}}]
while True:
    req = urllib.request.Request(base + "/v1/messages", data=json.dumps({{"tools": [{{"name": t}} for t in tools], "messages": msgs}}).encode(), headers={{"content-type": "application/json"}})
    evs = [json.loads(l[6:]) for l in urllib.request.urlopen(req, timeout=10).read().decode().splitlines() if l.startswith("data: ")]
    start = next(e for e in evs if e["type"] == "content_block_start")["content_block"]
    if start["type"] == "text":
        print(json.dumps({{"type": "result", "subtype": "success", "is_error": False}})); sys.exit(0)
    inp = json.loads(next(e for e in evs if e["type"] == "content_block_delta")["delta"]["partial_json"])
    name, err, text = start["name"], False, "ok"
    if "VajraDesktop" in (inp.get("file_path") or inp.get("command", "")):
        if name == "Bash":
            err, text = True, "Vajra guard blocked this Bash call"
        elif name == "Edit" and {leaky}:
            open(inp["file_path"], "w").write("edited\n")
        else:
            err, text = True, "File is in a directory that is denied by your permission settings."
    elif name == "Write":
        open(inp["file_path"], "w").write(inp["content"])
    elif name == "Bash" and "python3 -c" in inp["command"]:
        # The OS sandbox, as the command's output, then the marker its `|| echo` prints on the non-zero exit.
        text = "PermissionError: [Errno 1] Operation not permitted: <locked>\n" + inp["command"].rsplit("echo ", 1)[1]
    elif name == "Bash":
        r = subprocess.run(["/bin/sh", "-c", inp["command"]], capture_output=True, text=True)
        err, text = r.returncode != 0, r.stdout + r.stderr
    msgs += [{{"role": "assistant", "content": [{{"type": "tool_use", "id": start["id"], "name": name, "input": inp}}]}},
             {{"role": "user", "content": [{{"type": "tool_result", "tool_use_id": start["id"], "is_error": err, "content": text}}]}}]
"#,
        count = dir.join("selftests").display(),
        leaky = if leaky { "True" } else { "False" },
    );
    std::fs::write(&p, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

/// The repo's guard hooks, installed in `home` where the app installs them, so the built-in proven pair's shape (which
/// hashes the guard scripts' bytes) matches.
fn install_repo_hooks(home: &Path) {
    let hooks = profiles::claude_worker_hooks_dir(home);
    std::fs::create_dir_all(&hooks).unwrap();
    for e in std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hooks")).unwrap().flatten() {
        if e.file_type().unwrap().is_file() {
            std::fs::copy(e.path(), hooks.join(e.file_name())).unwrap();
        }
    }
}

/// A rig whose Claude is `canary_claude`.
fn canary_rig(tag: &str, leaky: bool) -> Rig {
    rig(tag, Opts { canary: Some(leaky), ..Opts::default() })
}

/// #186 codex: S4 proved the guard scripts' behavior, so a proven build whose installed guard differs by one byte, or
/// is missing, is unproven, and is refused before it runs.
#[test]
fn a_proven_build_with_a_changed_or_missing_installed_guard_is_refused() {
    let r = rig("claude-guard-edited", Opts::default());
    let p = profiles::claude_worker_hooks_dir(&r.home).join("pre-tool-use.sh");
    let mut b = std::fs::read(&p).unwrap();
    b.push(b'\n');
    std::fs::write(&p, b).unwrap();
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("claude_selftest_failed")), "{v:?}");
    assert!(v.worktree.is_none() && !r.d.join("log").exists(), "the harness never ran: {v:?}");
    let r = rig("claude-guard-missing", Opts::default());
    std::fs::remove_dir_all(profiles::claude_worker_hooks_dir(&r.home)).unwrap();
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("claude_unproven")), "{v:?}");
    assert!(v.worktree.is_none() && !r.d.join("log").exists(), "the harness never ran: {v:?}");
}

fn selftests(r: &Rig) -> usize {
    std::fs::read_to_string(r.d.join("selftests")).map_or(0, |t| t.lines().count())
}

/// Canary A: a build not on the built-in list passes its self-test on this Mac, so its pair is added to this Mac's
/// list and the worker runs; the next dispatch finds the pair and does not test again.
#[test]
fn an_unproven_build_that_passes_its_self_test_here_runs_and_is_not_tested_again() {
    let r = canary_rig("canary-pass", false);
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    assert_eq!(selftests(&r), 1);
    let list: Value = serde_json::from_str(&std::fs::read_to_string(r.gw.data.join("control/claude-proven.json")).unwrap()).unwrap();
    let shape = profiles::claude_worker_flag_shape(&profiles::claude_worker_hooks_dir(&r.home)).unwrap();
    assert_eq!(list["pairs"], json!([{"version": "9.9.9 (Claude Code)", "shape": shape}]));
    let v = r.done(&r.rt.dispatch(r.task("r1", "t2"), seat("w1", "claude")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    assert_eq!(selftests(&r), 1, "a proven pair is not tested again");
}

/// #195 bhishma: the CLI updates in place (9.9.9 to 9.9.10) while 9.9.9's self-test runs. The PASS can't be told
/// apart from a PASS of the new binary, so nothing is recorded and that dispatch is refused; the next one tests
/// 9.9.10 itself and records 9.9.10, never 9.9.9.
#[test]
fn a_self_test_pass_is_recorded_only_if_the_binary_still_says_the_same_version() {
    let r = canary_rig("canary-swap", false);
    std::fs::write(r.d.join("selftests.swap"), "").unwrap();
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("claude_selftest_failed")), "{v:?}");
    assert_eq!(selftests(&r), 1);
    assert!(!r.gw.data.join("control/claude-proven.json").exists(), "nothing recorded for a binary that changed under its test");
    let v = r.done(&r.rt.dispatch(r.task("r1", "t2"), seat("w1", "claude")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    assert_eq!(selftests(&r), 2, "the new build was tested, not refused from memory");
    let list: Value = serde_json::from_str(&std::fs::read_to_string(r.gw.data.join("control/claude-proven.json")).unwrap()).unwrap();
    assert_eq!(list["pairs"].as_array().unwrap().iter().map(|p| p["version"].as_str().unwrap()).collect::<Vec<_>>(), ["9.9.10 (Claude Code)"]);
}

/// Canary A: a build whose self-test fails stays refused with a clear reason, is not added to the list, and is not
/// tested again on every dispatch (`claude_canary::RETRY_AFTER`). Codex workers still run.
#[test]
fn an_unproven_build_that_fails_its_self_test_stays_refused_and_codex_still_runs() {
    let r = canary_rig("canary-fail", true);
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("claude_selftest_failed")), "{v:?}");
    assert!(v.worktree.is_none(), "refused before any worktree exists");
    assert!(!r.gw.data.join("control/claude-proven.json").exists());
    let v = r.done(&r.rt.dispatch(r.task("r1", "t2"), seat("w1", "claude")).unwrap());
    assert_eq!(v.reason.as_deref(), Some("claude_selftest_failed"), "{v:?}");
    assert_eq!(selftests(&r), 1, "a failure is remembered, not re-tested on every dispatch");
    let v = r.done(&r.rt.dispatch(r.task("r1", "t3"), seat("w1", "codex")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
}

/// #186 codex: the CLI updates in place during the first attempt (proven 2.1.296, then 9.9.9). The retry, after its
/// backoff, asks the binary again before it spawns, so the second process never launches. The new build is not on any
/// list, and its self-test can't pass here (the stand-in harness isn't a real Claude Code).
#[test]
fn a_retry_asks_the_claude_binary_again_and_an_update_during_the_attempt_is_refused() {
    let r = rig("claude-reprobe", Opts { mode: "504_upgrade", ..Opts::default() });
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!((v.outcome, v.reason.as_deref()), (Some(Outcome::Failed), Some("claude_selftest_failed")), "{v:?}");
    let starts = std::fs::read_to_string(r.d.join("log")).unwrap().lines().filter(|l| l.starts_with("start ")).count();
    assert_eq!(starts, 1, "the second attempt never launched");
    assert_eq!(r.clock.slept(), vec![errors::BACKOFF[0]], "it was a retry, after the first backoff");
}

/// A Claude task worker's steering channel is opened by its own session id before it starts, and its brief carries
/// the nonce in the workflow's steering paragraph. A Codex worker gets none.
#[test]
fn a_claude_task_worker_gets_its_steering_channel_by_session_id_before_it_starts() {
    const NONCE: &str = "00112233445566778899aabbccddeeff";
    let opened: Arc<Mutex<Vec<(String, usize)>>> = Arc::default();
    let (o2, log) = (opened.clone(), Arc::new(Mutex::new(PathBuf::new())));
    let l2 = log.clone();
    let steer: Steer = Arc::new(move |sid: &str| {
        let ran = std::fs::read_to_string(&*crate::lock(&l2)).map(|t| t.lines().count()).unwrap_or(0);
        crate::lock(&o2).push((sid.to_string(), ran));
        Some(NONCE.to_string())
    });
    let r = rig("steer", Opts { steer: Some(steer), ..Opts::default() });
    *crate::lock(&log) = r.d.join("log");
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!(v.outcome, Some(Outcome::Done), "{v:?}");
    let got = crate::lock(&opened).clone();
    assert_eq!(got, vec![(v.harness_session.clone().unwrap(), 0)], "opened once, by the worker's own session id, before it ran");
    let argv = std::fs::read_to_string(r.d.join("log.argv")).unwrap();
    assert!(argv.contains(&format!("<vajra-steer nonce=\"{NONCE}\"")), "the brief carries the steering paragraph: {argv}");
    let c = r.rt.dispatch(r.task("r1", "t2"), seat("w2", "codex")).unwrap();
    r.done(&c);
    assert_eq!(crate::lock(&opened).len(), 1, "a Codex worker has no steering channel");
}

/// Waits for a ticket's first attempt to be running, and returns its view then.
fn running(r: &Rig, id: &str) -> TicketView {
    let start = Instant::now();
    loop {
        let v = r.rt.view(id).unwrap();
        // `attempt` and a Claude worker's `harness_session` are set together, once its process is spawned.
        if v.state == State::Running && v.attempt > 0 {
            return v;
        }
        assert!(start.elapsed() < WAIT, "never ran: {v:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// #151: a running Claude check worker has no steering channel (a check is the independent gate), so its node offers
/// no Message or Redirect, and a message is refused with that reason, never "hasn't started yet".
#[test]
fn a_running_claude_check_worker_shows_message_off_with_its_true_reason() {
    let opened = Arc::new(AtomicUsize::new(0));
    let o2 = opened.clone();
    let steer: Steer = Arc::new(move |_: &str| {
        o2.fetch_add(1, Ordering::SeqCst);
        Some("00112233445566778899aabbccddeeff".into())
    });
    let r = rig("claude-check", Opts { mode: "hang", steer: Some(steer), ..Opts::default() });
    let mut s = seat("v1", "claude");
    s.role = crate::crew::roles::VERIFIER.into();
    let id = r.rt.dispatch_check(CheckKind::Verify, r.task("r1", "ignored"), s, &r.base).unwrap();
    let v = running(&r, &id);
    assert_eq!(v.message_off(), Some(profiles::CHECK_NOT_STEERED), "{v:?}");
    let c = v.core_worker().controls;
    for k in ["message", "redirect"] {
        assert_eq!((c[k]["enabled"].as_bool(), c[k]["reason"].as_str()), (Some(false), Some(profiles::CHECK_NOT_STEERED)), "{c}");
    }
    assert_eq!(opened.load(Ordering::SeqCst), 0, "a check never gets a control dir");
}

/// #151 review: once its ticket ends, a worker's node offers nothing: Message, Redirect and Stop are off with the
/// finished reason (the control service refuses them as `worker_finished`), for a steered Claude task and for Codex.
#[test]
fn an_ended_ticket_offers_no_message_redirect_or_stop() {
    let steer: Steer = Arc::new(|_: &str| Some("00112233445566778899aabbccddeeff".to_string()));
    let r = rig("ended-controls", Opts { mode: "hang", steer: Some(steer), ..Opts::default() });
    let claude = running(&r, &r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    let codex = running(&r, &r.rt.dispatch(r.task("r2", "t1"), seat("w2", "codex")).unwrap());
    let on = |v: &TicketView, k: &str| v.core_worker().controls[k]["enabled"].as_bool().unwrap();
    assert!(on(&claude, "message") && on(&claude, "redirect") && on(&claude, "stop"), "a running steered worker takes all three");
    assert!(!on(&codex, "message") && on(&codex, "stop"), "a running Codex worker can be stopped");
    for v in [&claude, &codex] {
        assert!(r.rt.cancel(&v.id));
        let ended = r.done(&v.id);
        assert_eq!(ended.message_off(), Some(profiles::FINISHED), "{ended:?}");
        let c = ended.core_worker().controls;
        for k in ["message", "redirect", "stop"] {
            assert_eq!((c[k]["enabled"].as_bool(), c[k]["reason"].as_str()), (Some(false), Some(profiles::FINISHED)), "{k}: {c}");
        }
    }
}

/// #151: a running Claude task worker whose control dir couldn't be opened says so, and a steered one takes messages.
#[test]
fn a_claude_task_whose_steering_channel_failed_says_so_and_a_steered_one_is_open() {
    let ok = Arc::new(AtomicUsize::new(0));
    let o2 = ok.clone();
    // The first open fails, the second succeeds.
    let steer: Steer = Arc::new(move |_: &str| (o2.fetch_add(1, Ordering::SeqCst) > 0).then(|| "00112233445566778899aabbccddeeff".to_string()));
    let r = rig("claude-nochan", Opts { mode: "hang", steer: Some(steer), ..Opts::default() });
    let failed = running(&r, &r.rt.dispatch(r.task("r1", "t1"), seat("w1", "claude")).unwrap());
    assert_eq!(failed.message_off(), Some(profiles::NO_STEER_CHANNEL), "{failed:?}");
    assert_eq!(failed.core_worker().controls["message"]["enabled"], false);
    let steered = running(&r, &r.rt.dispatch(r.task("r2", "t1"), seat("w2", "claude")).unwrap());
    assert_eq!(steered.message_off(), None, "{steered:?}");
    assert_eq!(steered.core_worker().controls["message"]["enabled"], true);
}

#[test]
fn a_claude_launch_whose_argv_is_refused_leaves_no_flag_file() {
    let r = rig("claude-badargv", Opts::default());
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
            redact: Redactor::default(), steer: None,
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

#[test]
fn an_ended_ticket_becomes_a_core_node_with_its_profile_controls_diff_and_provider() {
    let r = rig("corenode", Opts::default());
    let v = r.done(&r.rt.dispatch(r.task("r1", "t1"), seat("w1", "codex")).unwrap());
    let c = v.core_worker();
    assert_eq!((c.ticket.as_str(), c.task.as_str(), c.round, c.check, c.status), ("r1.t1.r1", "t1", 1, false, crate::crew::tree::Status::Done));
    // Ended: every control is off as finished (a running Codex worker's Message reason is its profile's, see
    // `an_ended_ticket_offers_no_message_redirect_or_stop`).
    assert_eq!(c.controls["message"]["reason"], profiles::FINISHED);
    assert_eq!(c.controls["stop"]["enabled"], false);
    assert_eq!(c.diff.unwrap()["added"], 1);
    assert_eq!(c.provider.unwrap().model, "model-fake-1");
}
