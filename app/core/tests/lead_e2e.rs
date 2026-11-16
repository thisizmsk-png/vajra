//! End to end with fake harness binaries (ADR-16 16.5, 6d2c): a core run with a Codex lead and one with a Claude lead.
//! Each has two writer seats on different accounts (the user's Codex login and a gateway account) plus a verifier and
//! a reviewer seat. The lead is a fake that speaks MCP to the real `vajra-desktop --mcp-run`, which talks to the app's
//! socket, its peer check and the run registry; it plans two tasks, dispatches both, awaits them, integrates, awaits
//! the round and finishes. The workers are a fake Codex that writes the file its brief names. With the 6d1 stub gate
//! the run ends `not_verified`, every worker ticket with its receipt and commit.
//!
//! Temp HOME, data dir and repo under /tmp; no key anywhere (the gateway home holds no key and the fake never calls
//! out). The supervisor kills every lead and worker by its process group, and `Drop` shuts the lead down.
#![allow(clippy::disallowed_methods)] // test code: the app's own spawns go through the spawn lock

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vajra_desktop::adapter::profiles;
use vajra_desktop::bus::Bus;
use vajra_desktop::control::runreg::RunRegistry;
use vajra_desktop::control::service::{Service, WorkerSeat};
use vajra_desktop::control::{nonce, socket};
use vajra_desktop::crew::redact::Redactor;
use vajra_desktop::desktop::run_lead::{LeadEnv, LeadSeat, RunLead};
use vajra_desktop::desktop::runproto::{CoreWorkers, Limits, RunConfig, RunProtocol, RunStatus, StubGate, SystemClock};
use vajra_desktop::desktop::workers::{self, Hooks, WorkerRuntime};
use vajra_desktop::sessions::Supervisor;
use vajra_desktop::vajra::gitw::{self, Gitw, GlobalBaseline, Pause, Quiesce};

const GIT: &str = "/usr/bin/git";

#[derive(Debug)]
struct NoAgents;
impl Quiesce for NoAgents {
    fn pause(&self, _: &Path) -> Pause {
        Pause::new(|| {})
    }
}

fn ugit(home: &Path, cwd: &Path, args: &[&str]) -> String {
    let o = Command::new(GIT).args(args).current_dir(cwd).env_clear().env("PATH", "/usr/bin:/bin").env("HOME", home).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

fn exe(p: &Path, body: &str) {
    fs::write(p, body).unwrap();
    fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
}

/// What codex 0.166 writes as a session's `file_system_sandbox_policy` under a per-launch permission profile (S12, run
/// live in the bench spike for the plain Codex Lead): every entry of the launch's own `permissions.<name>.filesystem`
/// argument, in order, each path as given and `none` reported as `deny`. The fakes build theirs from their own argv, so
/// the deny read-back judges what the core asked for, as it does with a real Codex. With `<role>.no-fs` next to the
/// script (role `worker` or `lead`), it prints `null` instead: the shape S12 saw when a sandbox flag beat the profile,
/// for the refusal check. (The supervisor clears the environment, so a flag file, not a variable.)
///
///     python3 -I fs_policy.py <role> [argv...]   prints the policy as JSON, or `null`
const FS_POLICY: &str = r#"import json, os, re, sys
role, args = sys.argv[1], sys.argv[2:]
fs = [a.split("=", 1)[1] for a in args if re.match(r"permissions\.vajra-[0-9a-f]{16}\.filesystem=", a)]
if not fs or os.path.exists(os.path.join(os.path.dirname(os.path.abspath(__file__)), role + ".no-fs")):
    print("null")
    sys.exit(0)
body = fs[-1].strip()
assert body.startswith("{") and body.endswith("}"), body
entries = []
for path, access in re.findall(r'"((?:[^"\\]|\\.)*)"\s*=\s*"(read|write|none)"', body[1:-1]):
    entries.append({"path": {"type": "path", "path": json.loads('"' + path + '"')}, "access": "deny" if access == "none" else access})
print(json.dumps({"kind": "restricted", "entries": entries}))
"#;

/// A fake Codex worker: `thread.started`, a rollout whose `turn_context` is the worker profile's (its sandbox and, as
/// codex 0.166 reports them, the file-system entries of the profile on its own command line), then it writes the file
/// named `touch:<file>` in its brief.
fn fake_worker(dir: &Path, user_codex_home: &Path) -> PathBuf {
    let p = dir.join("codex-worker");
    let fs_policy = dir.join("fs_policy.py");
    fs::write(&fs_policy, FS_POLICY).unwrap();
    exe(&p, &format!(
        r#"#!/bin/sh
WT=$(pwd -P)
TID=$(printf '%08x-0000-4000-8000-%012d' $$ $(date +%s))
D="${{CODEX_HOME:-{home}}}/sessions/2026/11/12"
mkdir -p "$D"
FS=$(/usr/bin/python3 -I '{fs_policy}' worker "$@")
printf '{{"type":"turn_context","payload":{{"approval_policy":"never","cwd":"%s","model":"m","sandbox_policy":{{"type":"workspace-write","network_access":false,"exclude_slash_tmp":true,"exclude_tmpdir_env_var":true,"writable_roots":["%s"]}},"file_system_sandbox_policy":%s}}}}\n' "$WT" "$TMPDIR" "$FS" > "$D/rollout-x-$TID.jsonl"
echo '{{"type":"thread.started","thread_id":"'$TID'"}}'
for a in "$@"; do LAST=$a; done
F=$(printf '%s' "$LAST" | sed -n 's/.*touch:\([a-z0-9.]*\).*/\1/p' | head -n 1)
[ -n "$F" ] && echo "made by a worker" > "$F"
exit 0
"#,
        home = user_codex_home.display(),
        fs_policy = fs_policy.display()
    ));
    p
}

/// The fake lead, for both harnesses. As Claude (`-p`) it prints the init event the lead read-back wants and reads the
/// server from `--mcp-config`; as Codex it writes a read-only rollout for its `-C` dir, prints `thread.started`, and
/// reads the server from the `-c mcp_servers=` value. Then it runs the charter's steps over MCP.
const FAKE_LEAD: &str = r#"#!/usr/bin/python3 -I
import json, os, re, subprocess, sys, time, uuid
argv = sys.argv[1:]
LOG = open(os.environ.get("FAKE_LOG", "/dev/null"), "a")
def log(*a):
    print(*a, file=LOG, flush=True)
if argv and argv[0] == "-p":
    tools = TOOLS.split(",")
    print(json.dumps({"type": "system", "subtype": "init", "permissionMode": "dontAsk", "tools": tools, "mcp_servers": [{"name": "vajra_run", "status": "connected"}]}), flush=True)
    cfg = json.load(open(argv[argv.index("--mcp-config") + 1]))["mcpServers"]["vajra_run"]
    cmd, env = [cfg["command"]] + cfg["args"], cfg["env"]
else:
    lead_dir = argv[argv.index("-C") + 1]
    tid = str(uuid.uuid4())
    d = os.path.join(CODEX_HOME, "sessions/2026/11/12")
    os.makedirs(d, exist_ok=True)
    # As codex 0.166 reports a no-write profile: a read-only sandbox, with the profile's entries ("none" as "deny").
    fs = json.loads(subprocess.run(["/usr/bin/python3", "-I", FS_POLICY, "lead"] + argv, capture_output=True, text=True, check=True).stdout)
    with open(os.path.join(d, "rollout-x-%s.jsonl" % tid), "w") as f:
        f.write(json.dumps({"type": "turn_context", "payload": {"approval_policy": "never", "cwd": lead_dir, "sandbox_policy": {"type": "read-only"}, "file_system_sandbox_policy": fs}}) + "\n")
    print(json.dumps({"type": "thread.started", "thread_id": tid}), flush=True)
    srv = [a for a in argv if a.startswith("mcp_servers=")][0]
    g = lambda k: re.search(k + '="([^"]+)"', srv).group(1)
    cmd = [g("command"), "--mcp-run"]
    env = {k: g(k) for k in ("VAJRA_RUN", "VAJRA_RUN_TOKEN_FILE", "VAJRA_RUN_SOCKET")}
env = dict(env, PATH="/usr/bin:/bin", HOME=TEST_HOME)
p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, text=True)
n = [0]
def rpc(method, params):
    n[0] += 1
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": n[0], "method": method, "params": params}) + "\n")
    p.stdin.flush()
    return json.loads(p.stdout.readline())
def call(name, args):
    r = rpc("tools/call", {"name": name, "arguments": args})["result"]
    log(name, json.dumps(r["structuredContent"]))
    if r["isError"]:
        sys.exit(3)
    return r["structuredContent"]
rpc("initialize", {"protocolVersion": "2025-06-18"})
task = lambda i, f: {"id": i, "title": "write " + f, "owner_role": "backend", "files": [f], "deps": [], "criteria": ["ac-1"], "brief": "touch:" + f, "touches_security": False}
call("plan_tasks", {"round": 1, "criteria": [{"id": "ac-1", "criterion": "both files exist", "check": "test -f a1.txt && test -f a2.txt"}], "tasks": [task("t1", "a1.txt"), task("t2", "a2.txt")]})
running = [call("dispatch_worker", {"task_id": t})["ticket"] for t in ("t1", "t2")]
while running:
    a = call("await_any", {"tickets": running, "wait_s": 20})
    running = [t["ticket"] for t in a["running"]]
rt = call("integrate", {"round": 1})["round_ticket"]
while True:
    a = call("await_any", {"tickets": [rt], "wait_s": 20})
    if a["ended"]:
        break
call("finish_run", {"report": "both tasks ran"})
p.stdin.close()
p.wait(timeout=10)
"#;

struct Rig {
    d: PathBuf,
    lead: RunLead,
    rt: WorkerRuntime,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.lead.shutdown(Duration::from_secs(2));
        let _ = fs::remove_dir_all(&self.d);
    }
}

fn seat(slot: &str, role: &str, account: &str) -> WorkerSeat {
    WorkerSeat {
        slot: slot.into(), role: role.into(), task: String::new(), account: account.into(), model: Some("vendor/model-a".into()), effort: None,
        security: false, network: false, why: String::new(), source: "user".into(),
    }
}

fn rig(lead_account: &str) -> Rig {
    // Socket paths stay under 104 bytes, so not under $TMPDIR.
    let d = PathBuf::from(format!("/tmp/vle-{}-{}", std::process::id(), nonce::random_hex(4).unwrap()));
    let _ = fs::remove_dir_all(&d);
    let (home, data, work, repo, codex_home) = (d.join("h"), d.join("d"), d.join("w"), d.join("r"), d.join("ch"));
    for p in [&home, &data, &repo, &codex_home] {
        fs::create_dir_all(p).unwrap();
    }
    let d = d.canonicalize().unwrap();
    let (home, data, repo, codex_home) = (d.join("h"), d.join("d"), d.join("r"), d.join("ch"));
    fs::write(home.join(".gitconfig"), "[user]\n\tname = U\n\temail = u@e\n").unwrap();
    ugit(&home, &repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("README"), "base\n").unwrap();
    ugit(&home, &repo, &["add", "-A"]);
    ugit(&home, &repo, &["commit", "-q", "-m", "base"]);
    let base = ugit(&home, &repo, &["rev-parse", "HEAD"]);

    let g = GlobalBaseline::take_or_load(Path::new(GIT), &data, &home, None).unwrap();
    gitw::check_roots(&data, &work).unwrap();
    let work = work.canonicalize().unwrap();
    let gw = Arc::new(Gitw::new(GIT.into(), home.clone(), data.clone(), work, &g, Arc::new(NoAgents)));
    let common = gw.common_of(&repo).unwrap();
    gw.repos.acquire(&gw, &common, false).unwrap();
    let bus = Arc::new(Bus::open(&d.join("spill"), vajra_desktop::bus::Config::default()).unwrap());
    let sup = Arc::new(Supervisor::new(bus.clone(), None));
    let worker = fake_worker(&d, &codex_home);
    let cfg = workers::Config {
        home: home.clone(), data: data.clone(), codex: worker.clone(), claude: worker, codex_home: codex_home.clone(),
        tools: profiles::WriterTools::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bundled-agents")),
        account_caps: Default::default(), worker_timeout: Duration::from_secs(60), poll: Duration::from_millis(50), clock: None,
        redact: Redactor::default(), steer: None,
    };
    let rt = WorkerRuntime::new(cfg, gw.clone(), sup.clone(), bus.clone(), Hooks::default());

    let run = "20261112-0930";
    let seats = vec![seat("w1", "backend", "codex"), seat("w2", "backend", "openrouter"), seat("w3", "verifier", "codex"), seat("w4", "reviewer", "codex")];
    let rc = RunConfig {
        project: "p".into(), run: run.into(), repo: repo.clone(), base, ask: "write two files".into(), seats, max_in_flight: 2,
        limits: Limits { rounds: 1, ..Limits::default() }, redact: Redactor::default(),
    };
    let proto = RunProtocol::new(rc, Arc::new(CoreWorkers { rt: rt.clone(), gw }), Arc::new(StubGate), Arc::new(SystemClock::default()));
    let reg = RunRegistry::new();
    reg.register(run, proto.clone(), [0; 32]);
    let svc = Arc::new(Mutex::new(Service::open(&data, d.join("audit.jsonl"), None).unwrap()));
    let sock = vajra_desktop::control::socket_path(&data);
    let listener = socket::bind(&sock).unwrap();
    let r2 = reg.clone();
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    std::thread::spawn(move || socket::serve(listener, svc, r2, uid));

    let fake = d.join("lead");
    // The shebang stays the first line; the constants go right after it.
    let (shebang, rest) = FAKE_LEAD.split_once('\n').unwrap();
    // The fake and the server it starts see only the temp HOME, never the runner's.
    let script = format!(
        "{shebang}\nTOOLS = {:?}\nCODEX_HOME = {:?}\nTEST_HOME = {:?}\nFS_POLICY = {:?}\n{rest}",
        profiles::claude_lead_tools(), codex_home.to_str().unwrap(), home.to_str().unwrap(), d.join("fs_policy.py").to_str().unwrap()
    );
    exe(&fake, &script);
    let env = LeadEnv {
        data: data.clone(), home: home.clone(), exe: PathBuf::from(env!("CARGO_BIN_EXE_vajra-desktop")), socket: sock, codex: fake.clone(), claude: fake,
        user_codex_home: codex_home, repos: vec![repo.clone()], others: vec![], read_back_timeout: Duration::from_secs(30),
    };
    let lead_seat = LeadSeat { account: lead_account.into(), model: Some("model-l".into()), effort: None };
    let lead = RunLead::new(run, repo.to_str().unwrap(), "write two files", 1, vec!["backend".into()], lead_seat, env, proto, reg, sup, bus).unwrap();
    Rig { d, lead, rt }
}

fn run_to_the_end(lead_account: &str) {
    let r = rig(lead_account);
    r.lead.start().unwrap();
    r.lead.watch();
    let (status, why) = r.lead.ended();
    let log = fs::read_to_string(r.d.join("lead.log")).unwrap_or_default();
    assert_eq!(status, Some(RunStatus::NotVerified), "the stub gate never passes a round: {why:?} {log}");
    assert_eq!(r.lead.launches(), 1, "the lead finished; it was not restarted");
    let views = r.rt.list("20261112-0930");
    assert_eq!(views.len(), 2, "two worker tickets: {views:?}");
    let accounts: std::collections::BTreeSet<_> = views.iter().map(|v| v.account.clone()).collect();
    assert_eq!(accounts.len(), 2, "the two workers ran on different accounts");
    for v in &views {
        assert!(v.receipt.is_some(), "every worker ticket has its launch receipt: {v:?}");
        assert!(v.commit.is_some(), "and its commit: {v:?}");
    }
    assert!(!r.lead.proto().rounds().is_empty(), "the round was integrated");
}

#[test]
fn a_codex_lead_runs_a_core_run_to_not_verified() {
    run_to_the_end("codex");
}

#[test]
fn a_claude_lead_runs_a_core_run_to_not_verified() {
    run_to_the_end("claude");
}

/// The stubs prove nothing unless the read-back still refuses a session that leaves the deny entries out: a Codex lead
/// whose session file shows no file-system entries (S12's shape when a sandbox flag beat the profile) is never let run,
/// so the run ends with the lead lost and no worker dispatched.
#[test]
fn a_codex_lead_whose_session_hides_the_deny_entries_is_refused() {
    let r = rig("codex");
    fs::write(r.d.join("lead.no-fs"), "").unwrap();
    r.lead.start().unwrap();
    r.lead.watch();
    let (status, why) = r.lead.ended();
    assert_eq!(status, Some(RunStatus::LeadLost), "{why:?}");
    assert!(why.as_deref().is_some_and(|w| w.contains("profile_not_applied")), "{why:?}");
    assert!(r.rt.list("20261112-0930").is_empty(), "no worker was dispatched");
}

/// The same for a worker: one whose session file leaves the deny entries out ends `profile_not_applied`, and its work
/// never counts.
#[test]
fn a_codex_worker_whose_session_hides_the_deny_entries_is_refused() {
    let r = rig("claude");
    fs::write(r.d.join("worker.no-fs"), "").unwrap();
    r.lead.start().unwrap();
    r.lead.watch();
    let codex: Vec<_> = r.rt.list("20261112-0930").into_iter().filter(|v| v.account == "codex").collect();
    assert!(!codex.is_empty(), "a Codex worker ran");
    for v in &codex {
        assert_eq!(v.reason.as_deref(), Some("profile_not_applied"), "{v:?}");
        assert!(v.commit.is_none(), "{v:?}");
    }
    // The stub gate never passes a round, so the run ends not verified either way: what counts is that the refused
    // worker's files never reach the round.
    let wt_files: Vec<String> = codex.iter().flat_map(|v| v.files.clone()).collect();
    assert!(wt_files.is_empty(), "a refused worker's files are never counted: {wt_files:?}");
}
