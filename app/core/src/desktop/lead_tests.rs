use super::*;
use crate::control::ledger::Ledger;
use crate::control::lead_run::expected_tools;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

const RUN_ID: &str = "20261106-0911";
const SID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
const NONCE: &str = "0123456789abcdef0123456789abcdef";

/// A doctor that passes and reports the OS sandbox and its denyWrite on the app dir as active.
const DOCTOR_OK: &str = "#!/bin/sh\necho 'PASS  OS sandbox enabled'\necho 'PASS  Vajra Desktop app dir write-protected by the OS sandbox (denyWrite active)'\nexit 0\n";

const FAKE_CLAUDE: &str = r#"#!/bin/sh
D="$(cd "$(dirname "$0")" && pwd)"
case "$1" in --warm) exit 0;; --version) cat "$D/version"; exit 0;; esac
N="$$"
cat > "$D/stdin.$N"
printf '%s\n' "$@" > "$D/argv.$N"
S=""; for a in "$@"; do [ "$P" = --settings ] && S="$a"; P="$a"; done
[ -n "$S" ] && cp "$S" "$D/settings-seen.$N"
cp "$(cat "$D/ledger.path")" "$D/ledger-at-spawn.$N" 2>/dev/null
if grep -q 'Read the file' "$D/stdin.$N"; then
  echo x >> "$D/selftest.count"
  # The real CLI refuses a session id whose transcript exists already.
  SIDARG=""; P2=""; for a in "$@"; do [ "$P2" = --session-id ] && SIDARG="$a"; P2="$a"; done
  if [ -n "$SIDARG" ]; then
    if [ -e "$D/sid.$SIDARG" ]; then echo "Session ID $SIDARG is already in use" >&2; exit 1; fi
    : > "$D/sid.$SIDARG"
  fi
  cat "$D/init.json"
  P=$(sed -n 's/.*Read the file \(.*\) and show its content\..*/\1/p' "$D/stdin.$N")
  case "$(cat "$D/selftest")" in
    leak)
      C=$(cat "$P")
      printf '{"type":"assistant","message":{"content":[{"type":"text","text":"%s"}]}}\n' "$C"
      echo '{"type":"result","is_error":false,"result":"x","permission_denials":[]}';;
    hang)
      sleep 60 &
      echo $! > "$D/selftest.child"
      wait;;
    toolerr)
      echo '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"x"}}]}}'
      echo '{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":"File does not exist."}]}}'
      echo '{"type":"result","is_error":false,"result":"x","permission_denials":[]}';;
    wait)
      while [ ! -e "$D/release" ]; do sleep 0.05; done
      printf '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"%s"}}]}}\n' "$P"
      echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"<tool_use_error>File is in a directory that is denied by your permission settings.</tool_use_error>"}]}}'
      printf '{"type":"result","is_error":false,"result":"x","permission_denials":[{"tool_name":"Read","tool_use_id":"t1","tool_input":{"file_path":"%s"}}]}\n' "$P";;
    retry)
      if [ "$(wc -l < "$D/selftest.count")" -le 1 ]; then
        echo '{"type":"assistant","message":{"content":[{"type":"text","text":"I will not."}]}}'
        echo '{"type":"result","is_error":false,"result":"x","permission_denials":[]}'
      else
        printf '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"%s"}}]}}\n' "$P"
        echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"<tool_use_error>File is in a directory that is denied by your permission settings.</tool_use_error>"}]}}'
        printf '{"type":"result","is_error":false,"result":"x","permission_denials":[{"tool_name":"Read","tool_use_id":"t1","tool_input":{"file_path":"%s"}}]}\n' "$P"
      fi;;
    otherdeny)
      echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/etc/some-other-protected-file"}}]}}'
      echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"<tool_use_error>File is in a directory that is denied by your permission settings.</tool_use_error>"}]}}'
      echo '{"type":"result","is_error":false,"result":"x","permission_denials":[{"tool_name":"Read","tool_use_id":"t1","tool_input":{"file_path":"/etc/some-other-protected-file"}}]}';;
    notry)
      echo '{"type":"assistant","message":{"content":[{"type":"text","text":"I will not."}]}}'
      echo '{"type":"result","is_error":false,"result":"x","permission_denials":[]}';;
    *)
      printf '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"%s"}}]}}\n' "$P"
      echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"<tool_use_error>File is in a directory that is denied by your permission settings.</tool_use_error>"}]}}'
      printf '{"type":"result","is_error":false,"result":"x","permission_denials":[{"tool_name":"Read","tool_use_id":"t1","tool_input":{"file_path":"%s"}}]}\n' "$P";;
  esac
  exit 0
fi
echo x >> "$D/ask.count"
case "$(cat "$D/mode")" in
  slow) sleep 60 &
        echo $! > "$D/slow.child"
        cat "$D/init.json"
        wait;;
  late) while [ ! -e "$D/release" ]; do sleep 0.05; done
        cat "$D/init.json"
        echo '{"type":"result","is_error":false,"result":"x","permission_denials":[]}';;
  bad_tools) cat "$D/init_bad.json"; sleep 30;;
  card) cat "$D/init.json"
        echo '{"type":"assistant","message":{"content":[{"type":"text","text":"{\"card\":\"stop\",\"worker\":\"bhima\"}"}]}}'
        echo '{"type":"result","is_error":false,"result":"x","permission_denials":[]}';;
  *) cat "$D/init.json"
     echo '{"type":"assistant","message":{"content":[{"type":"text","text":"hello from the lead"}]}}'
     echo '{"type":"result","is_error":false,"result":"hello from the lead","permission_denials":[]}';;
esac
"#;

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        let d = std::env::temp_dir().join(format!("vajra-lead-app-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("home/.claude/vajra/campaigns")).unwrap();
        std::fs::create_dir_all(d.join("home/.claude/skills/vajra/hooks")).unwrap();
        std::fs::create_dir_all(d.join("home/.claude/skills/vajra/scripts")).unwrap();
        std::fs::create_dir_all(d.join("repo")).unwrap();
        std::fs::create_dir_all(d.join("bin")).unwrap();
        // A canonical root: the macOS temp dir is a /var → /private/var symlink, and these tests compare paths.
        let d = std::fs::canonicalize(&d).unwrap();
        std::fs::write(d.join("home/.claude/skills/vajra/hooks/pre-tool-use.sh"), "#!/bin/bash\nexit 0\n").unwrap();
        let doctor = d.join("home/.claude/skills/vajra/scripts/vajra-doctor.sh");
        std::fs::write(&doctor, DOCTOR_OK).unwrap();
        std::fs::set_permissions(&doctor, std::fs::Permissions::from_mode(0o755)).unwrap();
        // macOS scans a new executable on its first run; do that here, not inside a timed call.
        let _ = std::process::Command::new(&doctor).env_clear().status();
        for stub in ["bash", "jq"] {
            let p = d.join("bin").join(stub);
            std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let claude = d.join("bin/claude");
        std::fs::write(&claude, FAKE_CLAUDE).unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::process::Command::new(&claude).arg("--warm").env_clear().status();
        let tools = serde_json::to_string(&expected_tools()).unwrap();
        let init = |tools: &str, mode: &str| format!(r#"{{"type":"system","subtype":"init","session_id":"{SID}","tools":{tools},"permissionMode":"{mode}"}}"#);
        for (f, body) in [
            ("init.json", init(&tools, "dontAsk") + "\n"),
            ("init_bad.json", init(r#"["Bash","Read"]"#, "dontAsk") + "\n"),
            ("version", "9.9.9 (Claude Code)".to_string()),
            ("mode", "ok".to_string()),
            ("selftest", "deny".to_string()),
            ("ledger.path", d.join("home/Library/Application Support/VajraDesktop/control/ledger.jsonl").to_string_lossy().into_owned()),
        ] {
            std::fs::write(d.join("bin").join(f), body).unwrap();
        }
        Self(d)
    }
    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
    /// The default data dir under the temp HOME: the only one the Lead accepts (the worker guard protects that one).
    fn data(&self) -> PathBuf {
        self.home().join("Library/Application Support/VajraDesktop")
    }
    fn repo(&self) -> PathBuf {
        self.0.join("repo")
    }
    fn bin(&self, f: &str) -> PathBuf {
        self.0.join("bin").join(f)
    }
    fn set(&self, f: &str, body: &str) {
        std::fs::write(self.bin(f), body).unwrap();
    }
    fn count(&self, f: &str) -> usize {
        std::fs::read_to_string(self.bin(f)).map(|s| s.lines().count()).unwrap_or(0)
    }
    fn files(&self, prefix: &str) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(self.0.join("bin"))
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix))
            .collect();
        v.sort();
        v
    }
    /// An app whose Lead is the fake claude, with a run this app launched (still in memory).
    fn app(&self) -> App {
        let app = App::open(&self.data(), &self.home()).unwrap();
        app.lead.set_bin(Some(self.bin("claude")));
        app.lead.set_env(vec![("HOME", self.home().into()), ("PATH", format!("{}:/usr/bin:/bin", self.0.join("bin").display()).into())]);
        let mut nonces = Redactor::default();
        nonces.add(Some(NONCE));
        lock(&app.runs).insert(
            format!("run-{SID}"),
            Run { session_id: SID.into(), run_id: RUN_ID.into(), campaign: "deliver-x".into(), cwd: self.repo(), nonces },
        );
        app
    }
    fn ledger(&self) -> Vec<Value> {
        Ledger::read_all(&self.data().join("control/ledger.jsonl")).unwrap()
    }
    fn ask_states(&self) -> Vec<String> {
        self.ledger().iter().filter(|r| r["kind"] == "ask").map(|r| format!("{}:{}", r["state"].as_str().unwrap(), r["reason"].as_str().unwrap_or(""))).collect()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        // Stop anything a failed test left running.
        let _ = std::process::Command::new("pkill").args(["-f", &self.0.to_string_lossy()]).status();
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The Lead session id of RUN_ID as the app stores it: in the guarded control dir.
fn remember_lead(t: &Tmp, sid: &str) {
    let dir = t.data().join("control/lead").join(RUN_ID);
    crate::control::fsx::private_dir(&t.data().join("control")).unwrap();
    crate::control::fsx::private_dir(&t.data().join("control/lead")).unwrap();
    crate::control::fsx::private_dir(&dir).unwrap();
    crate::control::fsx::write_atomic(&dir, "lead_session", sid.as_bytes()).unwrap();
    crate::control::fsx::write_atomic(&dir, "cwd", t.repo().to_string_lossy().as_bytes()).unwrap();
}

fn wait_until(what: &str, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn code(v: &Value) -> &str {
    v["error"].as_str().unwrap_or("")
}

fn idle(app: &App) -> bool {
    lock(&app.lead.active).is_empty()
}

fn stream_text(app: &App) -> Vec<String> {
    app.bus.tail(&format!("lead-{RUN_ID}"), 0, 1 << 20).lines.iter().map(|l| l.text.to_string()).collect()
}

// ------------------------------------------------------------ the happy path

#[test]
fn ask_round_trip_streams_records_and_resumes() {
    let t = Tmp::new();
    let app = t.app();
    let r = app.ask_lead(RUN_ID, &format!("what is bhima doing? {NONCE}")).unwrap();
    assert_eq!(r["state"], "queued");
    assert_eq!(r["stream"], format!("lead-{RUN_ID}"));
    let sid = r["lead_session_id"].as_str().unwrap().to_string();
    assert!(ids::valid_session_id(&sid));
    wait_until("the ask to finish", || idle(&app) && t.ask_states().len() == 3);
    assert_eq!(t.ask_states(), ["queued:", "delivered:", "done:"]);

    // The reply is on the bus, tagged, with the question first.
    let lines = stream_text(&app);
    assert_eq!(lines[0], "you\twhat is bhima doing? [redacted]");
    assert!(lines.contains(&"lead\thello from the lead".to_string()), "{lines:?}");

    // The Lead was started with the fixed argv, and the framed ask on stdin (nonce redacted, header first).
    // (The first Lead process of the app's life is the self-test turn, with its own throwaway session.)
    let argvs: Vec<String> = t.files("argv.").iter().map(|p| std::fs::read_to_string(p).unwrap()).collect();
    let argv = argvs.iter().find(|a| a.contains(&sid)).expect("the real ask's argv").clone();
    assert!(argv.starts_with("-p\n--session-id\n"), "{argv}");
    assert!(argv.contains("--setting-sources\n\n--settings\n") && argv.contains("--setting-sources\n\n--settings\n"), "{argv}");
    assert!(!argv.contains("bhima"), "ask text is never an argv element");
    let stdin = std::fs::read_to_string(t.files("stdin.").iter().find(|p| std::fs::read_to_string(p).unwrap().contains("what is bhima")).unwrap()).unwrap();
    assert!(stdin.starts_with("[Vajra Desktop: question from the human, ask "), "{stdin}");
    assert!(stdin.ends_with("what is bhima doing? [redacted]") && !stdin.contains(NONCE));

    // The ledger holds the redacted text once, and the Lead session was remembered after `init`.
    let q = &t.ledger().into_iter().find(|r| r["kind"] == "ask" && r["state"] == "queued").unwrap();
    assert_eq!(q["text"], "what is bhima doing? [redacted]");
    assert_eq!(q["actor"], "human");
    assert_eq!(std::fs::read_to_string(t.data().join("control/lead").join(RUN_ID).join("lead_session")).unwrap(), sid);
    assert!(!std::fs::read_to_string(t.data().join("runs.jsonl")).unwrap_or_default().contains(&sid), "the worker-writable index holds no Lead id");

    // The second ask resumes the same Lead session. The self-test ran once, not twice.
    let r2 = app.ask_lead(RUN_ID, "and now?").unwrap();
    assert_eq!(r2["lead_session_id"], sid);
    wait_until("the second ask", || idle(&app) && t.ask_states().len() == 6);
    let argvs: Vec<String> = t.files("argv.").iter().map(|p| std::fs::read_to_string(p).unwrap()).collect();
    assert!(argvs.iter().any(|a| a.starts_with("-p\n--resume\n") && a.contains(&sid)), "{argvs:?}");
    assert_eq!(t.count("selftest.count"), 1, "proven once per (settings sha, version)");
    assert_eq!(t.count("ask.count"), 2);
    // An ask writes no audit.log line and adds no queue entry (AC-3.1.3).
    assert!(!t.home().join(".claude/vajra/audit.log").exists());
    assert!(t.ledger().iter().all(|r| r["kind"] == "ask"));
}

#[test]
fn ask_ledger_record_before_spawn() {
    let t = Tmp::new();
    let app = t.app();
    app.ask_lead(RUN_ID, "is the record there first?").unwrap();
    wait_until("the ask", || idle(&app));
    // The fake copied the ledger the moment it was started: the queued line was already in it.
    let snaps = t.files("ledger-at-spawn.");
    assert!(!snaps.is_empty());
    let any = snaps.iter().any(|p| std::fs::read_to_string(p).unwrap().contains("is the record there first?"));
    assert!(any, "the ask was in the ledger before the Lead existed");
    // And if the ledger cannot take the line, the Lead is never started.
    let t2 = Tmp::new();
    let app2 = t2.app();
    lock(&app2.control).break_ledger_for_test();
    assert_eq!(code(&app2.ask_lead(RUN_ID, "x").unwrap_err()), "io_error");
    assert!(t2.files("stdin.").len() <= 1, "only the self-test turn may have run: {:?}", t2.files("stdin."));
    assert_eq!(t2.count("ask.count"), 0);
}

#[test]
fn reply_card_lookalike_stays_text() {
    let t = Tmp::new();
    let app = t.app();
    t.set("mode", "card");
    app.ask_lead(RUN_ID, "stop bhima").unwrap();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    assert!(stream_text(&app).contains(&"lead\t{\"card\":\"stop\",\"worker\":\"bhima\"}".to_string()));
    // Action cards come from the ledger only, and the ledger holds nothing but the ask.
    assert!(t.ledger().iter().all(|r| r["kind"] == "ask"));
}

// ------------------------------------------------------------ scope, concurrency, cancel, timeout

#[test]
fn ask_refused_for_terminal_run() {
    let t = Tmp::new();
    let app = t.app();
    lock(&app.runs).clear();
    let e = app.ask_lead(RUN_ID, "hello?").unwrap_err();
    assert_eq!((code(&e), e["detail"].as_str()), ("read_only", Some("run not launched by this app")));
    let s = app.lead_status(RUN_ID).unwrap();
    assert_eq!((s["available"].as_bool(), s["read_only_reason"].as_str()), (Some(false), Some("run not launched by this app")));
    assert!(t.ledger().is_empty(), "a refused ask leaves no record");
    assert!(t.files("stdin.").is_empty(), "nothing was spawned");
    // A run that only runs.jsonl remembers (a finished run of this app) can still be asked about.
    let entry = runindex::Entry { campaign: "deliver-x".into(), session_id: SID.into(), run_id: RUN_ID.into(), cwd: t.repo(), lead_session_id: None };
    runindex::append(&t.data().join("runs.jsonl"), &entry).unwrap();
    // ...once the app's own record of the run's directory agrees (without it the index alone is not believed).
    assert_eq!(app.lead_status(RUN_ID).unwrap()["available"], false);
    let dir = t.data().join("control/lead").join(RUN_ID);
    crate::control::fsx::private_dir(&t.data().join("control")).unwrap();
    crate::control::fsx::private_dir(&t.data().join("control/lead")).unwrap();
    crate::control::fsx::private_dir(&dir).unwrap();
    crate::control::fsx::write_atomic(&dir, "cwd", t.repo().to_string_lossy().as_bytes()).unwrap();
    assert_eq!(app.lead_status(RUN_ID).unwrap()["available"], true);
    assert!(app.ask_lead(RUN_ID, "still there?").is_ok());
    wait_until("the ask", || idle(&app));
    // Bad ids and bad text are refused before anything else.
    assert_eq!(code(&app.ask_lead("../x", "hi").unwrap_err()), "invalid_args");
    assert_eq!(code(&app.ask_lead(RUN_ID, "").unwrap_err()), "invalid_args");
}

#[test]
fn ask_lead_busy_second_call_and_cancel_kills_group() {
    let t = Tmp::new();
    let app = t.app();
    t.set("mode", "slow");
    let first = app.ask_lead(RUN_ID, "take your time").unwrap();
    let ask_id = first["ask_id"].as_str().unwrap().to_string();
    wait_until("the Lead to be running", || std::fs::read_to_string(t.bin("slow.child")).is_ok_and(|s| !s.trim().is_empty()) && t.ask_states().contains(&"delivered:".to_string()));
    // One ask in flight per Lead session.
    assert_eq!(code(&app.ask_lead(RUN_ID, "me too").unwrap_err()), "lead_busy");
    let s = app.lead_status(RUN_ID).unwrap();
    assert_eq!((s["busy"].as_bool(), s["ask_id"].as_str()), (Some(true), Some(ask_id.as_str())));
    // Cancel: the group (the CLI and the child it started) is killed and the ask is failed/cancelled.
    assert_eq!(code(&app.cancel_ask("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap_err()), "ask_not_found");
    assert_eq!(code(&app.cancel_ask("nope").unwrap_err()), "invalid_args");
    let c = app.cancel_ask(&ask_id).unwrap();
    assert_eq!((c["state"].as_str(), c["reason"].as_str()), (Some("failed"), Some("cancelled")));
    wait_until("the cancel to settle", || idle(&app));
    assert_eq!(t.ask_states(), ["queued:", "delivered:", "failed:cancelled"]);
    let child: i32 = std::fs::read_to_string(t.bin("slow.child")).unwrap().trim().parse().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    // SAFETY: signal 0 only checks existence.
    assert!(unsafe { libc::kill(child, 0) } != 0, "the Lead's child is gone");
    assert_eq!(app.lead_status(RUN_ID).unwrap()["busy"], false);
    // The Lead can be asked again once it is free; a session that never finished is resumed, not recreated.
    t.set("mode", "ok");
    assert!(app.ask_lead(RUN_ID, "back").is_ok());
    wait_until("the next ask", || idle(&app));
}

#[test]
fn ask_timeout_marks_failed() {
    let t = Tmp::new();
    let app = t.app();
    t.set("mode", "slow");
    app.lead.set_limits(Limits { init: Duration::from_secs(10), total: Duration::from_millis(800) });
    app.ask_lead(RUN_ID, "too slow").unwrap();
    wait_until("the timeout", || idle(&app));
    assert_eq!(t.ask_states().last().unwrap(), "failed:lead_timeout");
    assert!(stream_text(&app).contains(&"error\tlead_timeout".to_string()));
}

// ------------------------------------------------------------ the init check, at the app level

#[test]
fn a_lead_with_the_wrong_tools_is_killed_and_not_remembered() {
    let t = Tmp::new();
    let app = t.app();
    t.set("mode", "bad_tools");
    app.ask_lead(RUN_ID, "hi").unwrap();
    wait_until("the failure", || idle(&app));
    assert_eq!(t.ask_states(), ["queued:", "failed:lead_tools_mismatch"]);
    assert!(stream_text(&app).contains(&"error\tlead_tools_mismatch".to_string()));
    // No Lead id was written: the next ask starts a fresh session instead of resuming one that never passed.
    assert!(!t.data().join("runs.jsonl").exists());
}

// ------------------------------------------------------------ the guard (ADR-13 2a)

#[test]
fn lead_refused_without_guard_script() {
    let t = Tmp::new();
    let app = t.app();
    std::fs::remove_file(t.home().join(".claude/skills/vajra/hooks/pre-tool-use.sh")).unwrap();
    let e = app.ask_lead(RUN_ID, "hi").unwrap_err();
    assert_eq!(code(&e), "lead_guard_missing", "{e}");
    assert!(t.ledger().is_empty() && t.files("stdin.").is_empty(), "nothing recorded, nothing spawned");
    assert!(idle(&app), "the busy lock is released");
    // A directory where the script should be is not a regular file.
    std::fs::create_dir_all(t.home().join(".claude/skills/vajra/hooks/pre-tool-use.sh")).unwrap();
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "lead_guard_missing");
}

#[test]
fn lead_refused_when_the_doctor_fails_or_the_env_cannot_run_the_guard() {
    let t = Tmp::new();
    let app = t.app();
    std::fs::write(t.home().join(".claude/skills/vajra/scripts/vajra-doctor.sh"), "#!/bin/sh\necho 'FAIL hooks'\nexit 1\n").unwrap();
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "lead_guard_missing");
    std::fs::write(t.home().join(".claude/skills/vajra/scripts/vajra-doctor.sh"), DOCTOR_OK).unwrap();
    *lock(&app.lead.doctor) = None;
    // No jq on the Lead's PATH: the guard would fail closed on every tool, so preflight refuses.
    app.lead.set_env(vec![("HOME", t.home().into()), ("PATH", "/usr/bin:/bin".into())]);
    std::fs::remove_file(t.bin("jq")).unwrap();
    let no_jq = find_on_path("jq", &[PathBuf::from("/usr/bin"), PathBuf::from("/bin")]).is_none();
    if no_jq {
        assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "lead_guard_missing");
    }
    app.lead.set_env(vec![("PATH", format!("{}:/usr/bin:/bin", t.0.join("bin").display()).into())]);
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "lead_guard_missing", "no HOME");
    assert!(t.ledger().is_empty() && t.files("stdin.").is_empty());
}

#[test]
fn lead_settings_tamper_refused() {
    let t = Tmp::new();
    let app = t.app();
    // Someone edits the file between the core writing it and spawning: fewer deny rules.
    *lock(&app.lead.after_write) = Some(Box::new(|p: &Path| {
        let s = std::fs::read_to_string(p).unwrap().replace("\"disable\"", "\"enable\"");
        std::fs::write(p, s).unwrap();
    }));
    let e = app.ask_lead(RUN_ID, "hi").unwrap_err();
    assert_eq!(code(&e), "lead_guard_missing", "{e}");
    assert!(t.ledger().is_empty() && t.files("stdin.").is_empty(), "nothing spawned");
    // A loosened mode is refused too, and the unit check agrees.
    let p = t.0.join("s.json");
    std::fs::write(&p, "{}").unwrap();
    assert!(verify_settings_file(&p, "{}").is_err(), "0644 is not private");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(verify_settings_file(&p, "{}").is_ok());
    assert!(verify_settings_file(&p, "{ }").is_err());
    let link = t.0.join("link.json");
    std::os::unix::fs::symlink(&p, &link).unwrap();
    assert!(verify_settings_file(&link, "{}").is_err(), "a symlink is never read");
}

#[test]
fn lead_settings_are_written_private_atomic_and_constant() {
    let t = Tmp::new();
    let app = t.app();
    app.ask_lead(RUN_ID, "hi").unwrap();
    wait_until("the ask", || idle(&app));
    let dir = t.data().join("control").join("lead").join(RUN_ID);
    let got = std::fs::read_to_string(dir.join("settings.json")).unwrap();
    assert_eq!(got, lead_settings::settings(&t.home(), &t.repo(), &t.data()).unwrap());
    for f in ["settings.json", "mcp.json"] {
        assert_eq!(std::fs::metadata(dir.join(f)).unwrap().permissions().mode() & 0o777, 0o600, "{f}");
    }
    assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
    assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")), "no temp file left");
    let mcp: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("mcp.json")).unwrap()).unwrap();
    assert_eq!(mcp["mcpServers"]["vajra-desktop"]["args"], json!(["--mcp"]));
    // The Lead's MCP server is bound to this run: it is refused if another run is current.
    assert_eq!(mcp["mcpServers"]["vajra-desktop"]["env"]["VAJRA_LEAD_RUN"], RUN_ID);
    // The argv names exactly those files.
    let argv = std::fs::read_to_string(t.files("argv.").pop().unwrap()).unwrap();
    assert!(argv.contains(&format!("--settings\n{}\n", dir.join("settings.json").display())), "{argv}");
}

#[test]
fn lead_startup_selftest_failclosed() {
    let t = Tmp::new();
    let app = t.app();
    // A claude that hands back the canary: the deny layer is broken, so the Lead is never started.
    t.set("selftest", "leak");
    let e = app.ask_lead(RUN_ID, "hi").unwrap_err();
    assert_eq!(code(&e), "lead_guard_missing", "{e}");
    assert_eq!(t.count("selftest.count"), 1);
    assert_eq!(t.count("ask.count"), 0, "the real ask never ran");
    assert!(t.ledger().is_empty() && idle(&app));
    assert!(!t.data().join("control").join("lead").join(RUN_ID).join("canary").read_dir().unwrap().any(|_| true), "the canary file is removed");
    // A failure is not cached: it is tried again, and is still refused.
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "lead_guard_missing");
    assert_eq!(t.count("selftest.count"), 2);
    // A model that never tries the read proves nothing: two tries, then refused.
    t.set("selftest", "notry");
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "lead_guard_missing");
    assert_eq!(t.count("selftest.count"), 4);
    // A read that fails for another reason (a tool error) never exercised the deny rule: refused, not passed.
    // (Counts: one tool-error turn, no retry, because the model did try the read.)
    t.set("selftest", "toolerr");
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "lead_guard_missing");
    assert_eq!(t.count("selftest.count"), 5);
    // A denied read passes, and the pass is cached per (settings sha256, CLI version).
    t.set("selftest", "deny");
    app.ask_lead(RUN_ID, "hi").unwrap();
    wait_until("the ask", || idle(&app));
    assert_eq!(t.count("selftest.count"), 6);
    app.ask_lead(RUN_ID, "again").unwrap();
    wait_until("the ask", || idle(&app));
    assert_eq!(t.count("selftest.count"), 6, "cached");
    // A new CLI version is a new key.
    t.set("version", "9.9.10 (Claude Code)");
    app.ask_lead(RUN_ID, "again").unwrap();
    wait_until("the ask", || idle(&app));
    assert_eq!(t.count("selftest.count"), 7);
    // So is a different repo (the generated rules change with it).
    let other = t.0.join("other-repo");
    std::fs::create_dir_all(&other).unwrap();
    lock(&app.runs).values_mut().for_each(|r| r.cwd = other.clone());
    app.ask_lead(RUN_ID, "again").unwrap();
    wait_until("the ask", || idle(&app));
    assert_eq!(t.count("selftest.count"), 8);
}

#[test]
fn the_selftest_runs_with_the_hooks_removed_and_the_canary_covered_by_a_rule() {
    let t = Tmp::new();
    let app = t.app();
    app.ask_lead(RUN_ID, "hi").unwrap();
    wait_until("the ask", || idle(&app));
    let bare = std::fs::read_to_string(t.data().join("control").join("lead").join(RUN_ID).join("selftest-settings.json")).unwrap();
    assert_eq!(bare, lead_settings::settings_no_hooks(&t.home(), &t.repo(), &t.data()).unwrap());
    let v: Value = serde_json::from_str(&bare).unwrap();
    assert!(v.get("hooks").is_none());
    // The canary lives under the data dir, which a generated rule denies.
    let rules = lead_settings::deny_rules(&t.home(), &t.repo(), &t.data()).unwrap();
    let data_rule = format!("Read(//{}/**)", t.data().to_str().unwrap().trim_start_matches('/'));
    assert!(rules.contains(&data_rule), "{data_rule}");
}

// ------------------------------------------------------------ restart

#[test]
fn lead_stream_rebuilt_from_transcript_after_restart() {
    let t = Tmp::new();
    let lead_sid = "1a8fad5b-d9cb-469f-a165-70867728950f";
    std::fs::create_dir_all(t.data()).unwrap();
    let entry = runindex::Entry { campaign: "deliver-x".into(), session_id: SID.into(), run_id: RUN_ID.into(), cwd: t.repo(), lead_session_id: None };
    runindex::append(&t.data().join("runs.jsonl"), &entry).unwrap();
    remember_lead(&t, lead_sid);
    let proj = t.home().join(".claude/projects").join(project_slug(&t.repo()));
    std::fs::create_dir_all(&proj).unwrap();
    let hdr = format!("[Vajra Desktop: question from the human, ask 01ARZ3NDEKTSV4RRFFQ69G5FAV, run {RUN_ID}. Answer with the read and control tools only.]\nwhat is bhima doing?");
    let rec = |v: Value| v.to_string();
    let body = [
        rec(json!({"type": "user", "message": {"role": "user", "content": hdr}})),
        rec(json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "bhima is on task t1"}, {"type": "tool_use", "name": "mcp__vajra-desktop__list_workers", "input": {}}]}})),
        rec(json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "content": "SECRET WORKER OUTPUT"}]}})),
        "not json".to_string(),
        rec(json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "done"}]}})),
    ]
    .join("\n");
    std::fs::write(proj.join(format!("{lead_sid}.jsonl")), body).unwrap();
    // A fresh app (a restart): its bus is empty.
    let app = App::open(&t.data(), &t.home()).unwrap();
    assert!(app.bus.tail(&format!("lead-{RUN_ID}"), 0, 1 << 20).lines.is_empty());
    let s = app.lead_status(RUN_ID);
    // (claude may not be installed on the test machine; the history is rebuilt either way)
    assert!(s.is_ok());
    let lines = stream_text(&app);
    assert_eq!(lines, vec!["you\twhat is bhima doing?", "lead\tbhima is on task t1", "tool\tmcp__vajra-desktop__list_workers: {}", "lead\tdone"], "{lines:?}");
    assert!(lines.iter().all(|l| !l.contains("SECRET")), "tool results are not replayed");
    // Asking again does not duplicate it.
    app.lead_status(RUN_ID).unwrap();
    assert_eq!(stream_text(&app).len(), 4);
}

#[test]
fn claude_is_resolved_to_an_absolute_path_once() {
    let t = Tmp::new();
    let found = resolve_claude(&t.home(), &[t.0.join("bin")]).unwrap();
    assert!(found.is_absolute() && found.ends_with("bin/claude"));
    // Not on PATH: the usual install places are searched, newest nvm node first.
    let nvm_old = t.home().join(".nvm/versions/node/v20.1.0/bin");
    let nvm_new = t.home().join(".nvm/versions/node/v22.2.0/bin");
    for d in [&nvm_old, &nvm_new] {
        std::fs::create_dir_all(d).unwrap();
        std::fs::copy(t.bin("claude"), d.join("claude")).unwrap();
        std::fs::set_permissions(d.join("claude"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(resolve_claude(&t.home(), &[]).unwrap(), nvm_new.join("claude"));
    assert!(resolve_claude(&t.0.join("empty-home"), &[]).is_none() || Path::new("/opt/homebrew/bin/claude").exists() || Path::new("/usr/local/bin/claude").exists());
    // No binary: the ask says so instead of searching the PATH at ask time.
    let app = t.app();
    app.lead.set_bin(None);
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "claude_not_found");
    assert_eq!(app.lead_status(RUN_ID).unwrap()["read_only_reason"], "claude not found");
}

#[test]
fn two_runs_asking_at_once_each_get_their_own_settings() {
    let t = Tmp::new();
    let app = t.app();
    let (sid_b, run_b) = ("2b8fad5b-d9cb-469f-a165-70867728950e", "20261106-0912");
    let repo_b = t.0.join("repo-b");
    std::fs::create_dir_all(&repo_b).unwrap();
    lock(&app.runs).insert(
        format!("run-{sid_b}"),
        Run { session_id: sid_b.into(), run_id: run_b.into(), campaign: "deliver-y".into(), cwd: repo_b.clone(), nonces: Redactor::default() },
    );
    t.set("mode", "slow");
    let (a, b) = std::thread::scope(|s| {
        let ha = s.spawn(|| app.ask_lead(RUN_ID, "a"));
        let hb = s.spawn(|| app.ask_lead(run_b, "b"));
        (ha.join().unwrap(), hb.join().unwrap())
    });
    a.unwrap();
    b.unwrap();
    wait_until("both Leads running", || t.files("settings-seen.").len() >= 4);
    // Every real Lead (not the self-test turns) was pointed at a file under its own run dir, holding its own repo's rules.
    let mut checked = 0;
    for argv in t.files("argv.") {
        let text = std::fs::read_to_string(&argv).unwrap();
        let Some(path) = text.lines().skip_while(|l| *l != "--settings").nth(1).map(PathBuf::from) else { continue };
        let seen = std::fs::read_to_string(t.bin(&format!("settings-seen.{}", argv.file_name().unwrap().to_string_lossy().trim_start_matches("argv.")))).unwrap();
        let (mine, other) = if path.to_string_lossy().contains(&format!("/lead/{RUN_ID}/")) { (t.repo(), repo_b.clone()) } else { (repo_b.clone(), t.repo()) };
        assert!(path.to_string_lossy().contains("/lead/2026110"), "{path:?}");
        let rule = |r: &Path| format!("Read(//{}/**/.env)", r.to_str().unwrap().trim_start_matches('/'));
        assert!(seen.contains(&rule(&mine)), "{path:?} lacks its own repo rules");
        assert!(!seen.contains(&rule(&other)), "{path:?} holds the other repo's rules");
        checked += 1;
    }
    assert!(checked >= 4, "two self-tests and two asks: {checked}");
    for id in [RUN_ID, run_b] {
        assert!(t.data().join("control").join("lead").join(id).join("settings.json").is_file());
        let mcp: Value = serde_json::from_str(&std::fs::read_to_string(t.data().join("control").join("lead").join(id).join("mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["vajra-desktop"]["env"]["VAJRA_LEAD_RUN"], id, "each Lead is bound to its own run");
    }
    for h in lock(&app.lead.active).values() {
        h.handle.cancel();
    }
    wait_until("idle", || idle(&app));
}

#[test]
fn cancel_reaches_the_selftest_and_ask_lead_returns_cancelled() {
    let t = Tmp::new();
    let app = t.app();
    t.set("selftest", "hang");
    let res = std::thread::scope(|s| {
        let h = s.spawn(|| app.ask_lead(RUN_ID, "hi"));
        wait_until("the self-test to be running", || std::fs::read_to_string(t.bin("selftest.child")).is_ok_and(|s| !s.trim().is_empty()));
        let ask_id = lock(&app.lead.active).values().next().map(|a| a.ask_id.clone()).expect("reserved ask");
        app.cancel_ask(&ask_id).unwrap();
        h.join().unwrap()
    });
    assert_eq!(code(&res.unwrap_err()), "cancelled");
    let child: i32 = std::fs::read_to_string(t.bin("selftest.child")).unwrap().trim().parse().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    // SAFETY: signal 0 only checks existence.
    assert!(unsafe { libc::kill(child, 0) } != 0, "the self-test child is gone");
    assert!(idle(&app) && t.ledger().is_empty() && t.count("ask.count") == 0, "nothing recorded, no real ask ran");
}

#[test]
fn app_shutdown_kills_every_lead_and_refuses_new_asks() {
    let t = Tmp::new();
    let app = t.app();
    t.set("mode", "slow");
    app.ask_lead(RUN_ID, "take your time").unwrap();
    wait_until("the Lead to be running", || std::fs::read_to_string(t.bin("slow.child")).is_ok_and(|s| !s.trim().is_empty()) && t.ask_states().contains(&"delivered:".to_string()));
    let child: i32 = std::fs::read_to_string(t.bin("slow.child")).unwrap().trim().parse().unwrap();
    app.shutdown_lead();
    // It returns only after the driver reaped the Lead: nothing is left running and the ask is closed in the ledger.
    assert!(idle(&app), "no ask is left in flight");
    // SAFETY: signal 0 only checks existence.
    assert!(unsafe { libc::kill(child, 0) } != 0, "the Lead's child is gone");
    assert_eq!(t.ask_states(), ["queued:", "delivered:", "failed:cancelled"]);
    assert_eq!(code(&app.ask_lead(RUN_ID, "too late").unwrap_err()), "shutting_down");
}

#[test]
fn app_shutdown_reaches_a_self_test_in_progress() {
    let t = Tmp::new();
    let app = t.app();
    t.set("selftest", "hang");
    std::thread::scope(|s| {
        let h = s.spawn(|| app.ask_lead(RUN_ID, "hi"));
        wait_until("the self-test to be running", || std::fs::read_to_string(t.bin("selftest.child")).is_ok_and(|s| !s.trim().is_empty()));
        app.shutdown_lead();
        assert_eq!(code(&h.join().unwrap().unwrap_err()), "cancelled");
    });
    assert!(idle(&app));
}

#[test]
fn a_delayed_lead_init_does_not_roll_the_campaign_back_to_an_older_run() {
    let t = Tmp::new();
    let app = t.app();
    let path = t.data().join("runs.jsonl");
    let launch = |run: &str| runindex::Entry { campaign: "deliver-x".into(), session_id: SID.into(), run_id: run.into(), cwd: t.repo(), lead_session_id: None };
    runindex::append(&path, &launch(RUN_ID)).unwrap();
    t.set("mode", "late");
    app.ask_lead(RUN_ID, "first question").unwrap();
    wait_until("the Lead to be started", || t.count("ask.count") == 1);
    // The human resumes the campaign into run B while A's Lead is still starting.
    runindex::append(&path, &launch("20261106-1000")).unwrap();
    std::fs::write(t.bin("release"), "").unwrap();
    wait_until("the ask", || idle(&app));
    let latest = &runindex::latest(&path)["deliver-x"];
    assert_eq!((latest.run_id.as_str(), latest.lead_session_id.clone()), ("20261106-1000", None), "B stays the campaign's newest run");
}

#[test]
fn an_nvm_style_claude_runs_with_node_from_its_own_directory() {
    // `claude` is `#!/usr/bin/env fakenode`, and fakenode exists only beside it: not on the Lead's PATH, not on a system path.
    let t = Tmp::new();
    let nvm = t.0.join("nvm/bin");
    std::fs::create_dir_all(&nvm).unwrap();
    let script = FAKE_CLAUDE
        .replace("#!/bin/sh\n", "")
        .replace(r#"D="$(cd "$(dirname "$0")" && pwd)""#, &format!("D={}", t.0.join("bin").display()));
    let claude = nvm.join("claude");
    std::fs::write(&claude, format!("#!/usr/bin/env fakenode\n{script}")).unwrap();
    let node = nvm.join("fakenode");
    std::fs::write(&node, "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
    for p in [&claude, &node] {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let app = t.app();
    app.lead.set_bin(Some(claude));
    let r = app.ask_lead(RUN_ID, "hello").unwrap();
    assert_eq!(r["state"], "queued");
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    assert_eq!(t.ask_states(), ["queued:", "delivered:", "done:"]);
}

#[test]
fn after_a_restart_the_on_disk_nonce_stays_out_of_stdin_ledger_and_bus() {
    let t = Tmp::new();
    let lead_sid = "1a8fad5b-d9cb-469f-a165-70867728950f";
    std::fs::create_dir_all(t.data()).unwrap();
    let entry = runindex::Entry { campaign: "deliver-x".into(), session_id: SID.into(), run_id: RUN_ID.into(), cwd: t.repo(), lead_session_id: None };
    runindex::append(&t.data().join("runs.jsonl"), &entry).unwrap();
    remember_lead(&t, lead_sid);
    // The run's control nonce, as the earlier app life left it.
    let disk = nonce::create(&t.data().join("control").join(SID)).unwrap();
    // A history line of the Lead's own transcript that quotes it.
    let proj = t.home().join(".claude/projects").join(project_slug(&t.repo()));
    std::fs::create_dir_all(&proj).unwrap();
    let line = json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": format!("the nonce is {disk}")}]}});
    std::fs::write(proj.join(format!("{lead_sid}.jsonl")), line.to_string()).unwrap();
    // A fresh app: no in-memory run, no control session.
    let app = App::open(&t.data(), &t.home()).unwrap();
    app.lead.set_bin(Some(t.bin("claude")));
    app.lead.set_env(vec![("HOME", t.home().into()), ("PATH", format!("{}:/usr/bin:/bin", t.0.join("bin").display()).into())]);
    app.ask_lead(RUN_ID, &format!("is {disk} still valid?")).unwrap();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    let stdin = t.files("stdin.").iter().map(|p| std::fs::read_to_string(p).unwrap()).collect::<String>();
    assert!(stdin.contains("is [redacted] still valid?") && !stdin.contains(&disk), "stdin: {stdin}");
    assert!(!serde_json::to_string(&t.ledger()).unwrap().contains(&disk), "ledger");
    let lines = stream_text(&app);
    assert!(lines.iter().any(|l| l.contains("[redacted]")) && lines.iter().all(|l| !l.contains(&disk)), "bus: {lines:?}");
}

#[test]
fn the_selftest_retry_uses_a_new_session_id() {
    // The first probe never tries the read (and leaves a transcript behind); the fake CLI rejects a reused
    // `--session-id` and denies only on the second attempt.
    let t = Tmp::new();
    let app = t.app();
    t.set("selftest", "retry");
    app.ask_lead(RUN_ID, "hi").unwrap();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    assert_eq!(t.count("selftest.count"), 2);
    assert_eq!(t.files("sid.").len(), 2, "two distinct session ids");
}

#[test]
fn lead_files_tampered_during_the_selftest_never_reach_the_real_ask() {
    let t = Tmp::new();
    let app = t.app();
    t.set("selftest", "wait");
    let dir = t.data().join("control").join("lead").join(RUN_ID);
    std::thread::scope(|s| {
        let h = s.spawn(|| app.ask_lead(RUN_ID, "hi"));
        wait_until("the self-test to be running", || t.count("selftest.count") == 1);
        // A worker (same uid) rewrites both generated files while the self-test runs.
        for f in ["settings.json", "mcp.json"] {
            let p = dir.join(f);
            let _ = std::fs::remove_file(&p);
            std::fs::write(&p, "{\"TAMPER\":true}").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        std::fs::write(t.bin("release"), "").unwrap();
        h.join().unwrap().unwrap();
    });
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    for f in ["settings.json", "mcp.json"] {
        assert!(!std::fs::read_to_string(dir.join(f)).unwrap().contains("TAMPER"), "{f} was restored before launch");
    }
    let seen: Vec<String> = t.files("settings-seen.").iter().map(|p| std::fs::read_to_string(p).unwrap()).collect();
    assert!(seen.len() >= 2 && seen.iter().all(|c| !c.contains("TAMPER")), "no Lead process read the tampered settings");
}

#[test]
fn a_data_dir_override_outside_the_guarded_location_is_refused() {
    let t = Tmp::new();
    let other = t.0.join("elsewhere");
    std::fs::create_dir_all(&other).unwrap();
    let app = App::open(&other, &t.home()).unwrap();
    app.lead.set_bin(Some(t.bin("claude")));
    app.lead.set_env(vec![("HOME", t.home().into()), ("PATH", format!("{}:/usr/bin:/bin", t.0.join("bin").display()).into())]);
    lock(&app.runs).insert(
        format!("run-{SID}"),
        Run { session_id: SID.into(), run_id: RUN_ID.into(), campaign: "deliver-x".into(), cwd: t.repo(), nonces: Redactor::default() },
    );
    let e = app.ask_lead(RUN_ID, "hi").unwrap_err();
    assert_eq!(code(&e), "lead_guard_missing");
    assert!(e.to_string().contains("not guard-protected"), "{e}");
    assert!(t.count("ask.count") == 0 && t.files("stdin.").is_empty(), "nothing was spawned");
    assert!(!other.join("control/lead").exists(), "no Lead files were written there");
}

#[test]
fn a_forged_lead_session_id_in_the_worker_writable_index_is_ignored() {
    let t = Tmp::new();
    let app = t.app();
    // A worker rewrites runs.jsonl and points the run's Lead session at another session (here the run's own worker).
    let forged = runindex::Entry { campaign: "deliver-x".into(), session_id: SID.into(), run_id: RUN_ID.into(), cwd: t.repo(), lead_session_id: Some(SID.into()) };
    std::fs::create_dir_all(t.data()).unwrap();
    runindex::append(&t.data().join("runs.jsonl"), &forged).unwrap();
    let r = app.ask_lead(RUN_ID, "hi").unwrap();
    assert_ne!(r["lead_session_id"], SID, "the forged id is not used");
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    let argvs: Vec<String> = t.files("argv.").iter().map(|p| std::fs::read_to_string(p).unwrap()).collect();
    assert!(argvs.iter().all(|a| !a.contains("--resume")), "nothing was resumed: {argvs:?}");
    // Nor is it replayed into the Lead stream.
    assert!(stream_text(&app).iter().all(|l| !l.contains("forged")));
}

#[test]
fn lead_is_unavailable_when_the_doctor_passes_but_the_sandbox_denywrite_is_off() {
    let t = Tmp::new();
    let app = t.app();
    let doctor = t.home().join(".claude/skills/vajra/scripts/vajra-doctor.sh");
    // Exit 0 with only WARN lines: exactly what the real doctor does when sandbox/denyWrite is off.
    std::fs::write(&doctor, "#!/bin/sh\necho 'WARN  OS sandbox is off (the enforced boundary; enable with /sandbox)'\necho 'WARN  Vajra Desktop denyWrite not active (the guard is the only layer for its control queue)'\nexit 0\n").unwrap();
    let e = app.ask_lead(RUN_ID, "hi").unwrap_err();
    assert_eq!(code(&e), "lead_guard_missing");
    assert!(e["detail"].as_str().unwrap_or_default()== SANDBOX_NEEDED, "{e}");
    assert!(t.ledger().is_empty() && t.files("stdin.").is_empty(), "nothing recorded or spawned");
    // Once the sandbox is on, the same ask goes through.
    std::fs::write(&doctor, DOCTOR_OK).unwrap();
    *lock(&app.lead.doctor) = None;
    assert!(app.ask_lead(RUN_ID, "hi").is_ok());
    wait_until("the ask", || idle(&app));
}

#[test]
fn two_runs_with_one_minute_id_are_never_confused_across_a_restart() {
    // The reviewer's sequence: campaign A (resumed in minute M, no `deliver-M` dir) is in the index; after a
    // restart in minute M campaign B also holds M in memory. The id cannot be told apart, so no Lead is chosen.
    let t = Tmp::new();
    let sid_a = "2b8fad5b-d9cb-469f-a165-70867728950e";
    std::fs::create_dir_all(t.data()).unwrap();
    let a = runindex::Entry { campaign: "deliver-old".into(), session_id: sid_a.into(), run_id: RUN_ID.into(), cwd: t.repo(), lead_session_id: None };
    runindex::append(&t.data().join("runs.jsonl"), &a).unwrap();
    let app = t.app(); // in memory: campaign deliver-x, session SID, the same run id
    let e = app.ask_lead(RUN_ID, "hi").unwrap_err();
    assert_eq!(code(&e), "invalid_args");
    assert!(e["detail"].as_str().unwrap_or_default().contains("more than one run"), "{e}");
    assert!(t.ledger().is_empty() && t.files("stdin.").is_empty(), "nothing recorded or spawned, no MCP binding made");
    let s = app.lead_status(RUN_ID).unwrap();
    assert_eq!(s["available"], false);
    // Two indexed runs sharing an id (the same sequence seen only from disk after a restart) are refused too.
    let t2 = Tmp::new();
    std::fs::create_dir_all(t2.data()).unwrap();
    for (c, s) in [("deliver-a", sid_a), ("deliver-b", SID)] {
        let e = runindex::Entry { campaign: c.into(), session_id: s.into(), run_id: RUN_ID.into(), cwd: t2.repo(), lead_session_id: None };
        runindex::append(&t2.data().join("runs.jsonl"), &e).unwrap();
    }
    let app2 = App::open(&t2.data(), &t2.home()).unwrap();
    app2.lead.set_bin(Some(t2.bin("claude")));
    assert_eq!(code(&app2.ask_lead(RUN_ID, "hi").unwrap_err()), "invalid_args");
    // One run is fine, even when it appears on several index lines (a resume appends another line).
    let t3 = Tmp::new();
    std::fs::create_dir_all(t3.data()).unwrap();
    for _ in 0..2 {
        let e = runindex::Entry { campaign: "deliver-x".into(), session_id: SID.into(), run_id: RUN_ID.into(), cwd: t3.repo(), lead_session_id: None };
        runindex::append(&t3.data().join("runs.jsonl"), &e).unwrap();
    }
    let app3 = t3.app();
    assert!(app3.ask_lead(RUN_ID, "hi").is_ok());
    wait_until("the ask", || idle(&app3));
}

#[test]
fn a_forged_cwd_in_the_worker_writable_index_changes_nothing() {
    let t = Tmp::new();
    // Life 1: the run is asked about, so the app records its directory under the guarded dir.
    let app = t.app();
    app.ask_lead(RUN_ID, "first").unwrap();
    wait_until("the ask", || idle(&app) && t.ask_states().len() == 3);
    let rec = t.data().join("control/lead").join(RUN_ID).join("cwd");
    assert_eq!(std::fs::read_to_string(&rec).unwrap(), t.repo().to_string_lossy());
    let settings = t.data().join("control/lead").join(RUN_ID).join("settings.json");
    let real_rules = std::fs::read_to_string(&settings).unwrap();
    let asks = t.count("ask.count");
    drop(app);
    // Life 2 (a restart): only the index tells the app about the run, and a worker forged its cwd.
    let evil = t.0.join("evil-repo");
    std::fs::create_dir_all(&evil).unwrap();
    let path = t.data().join("runs.jsonl");
    let line = |cwd: &Path| runindex::Entry { campaign: "deliver-x".into(), session_id: SID.into(), run_id: RUN_ID.into(), cwd: cwd.to_path_buf(), lead_session_id: None };
    runindex::append(&path, &line(&evil)).unwrap();
    let app2 = App::open(&t.data(), &t.home()).unwrap();
    app2.lead.set_bin(Some(t.bin("claude")));
    app2.lead.set_env(vec![("HOME", t.home().into()), ("PATH", format!("{}:/usr/bin:/bin", t.0.join("bin").display()).into())]);
    let e = app2.ask_lead(RUN_ID, "again").unwrap_err();
    assert_eq!(code(&e), "read_only");
    assert_eq!(t.count("ask.count"), asks, "no Lead was spawned in the forged directory");
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), real_rules, "the repo deny rules are untouched");
    assert_eq!(app2.lead_status(RUN_ID).unwrap()["available"], false);
    // The honest index line (same cwd as the record) is served, in the real directory.
    runindex::append(&path, &line(&t.repo())).unwrap();
    app2.ask_lead(RUN_ID, "again").unwrap();
    wait_until("the ask", || idle(&app2));
    assert!(std::fs::read_to_string(&settings).unwrap().contains(&*t.repo().to_string_lossy()) || std::fs::read_to_string(&settings).unwrap() == real_rules);
}

#[test]
fn a_run_loaded_from_the_index_with_no_cwd_record_is_read_only() {
    let t = Tmp::new();
    std::fs::create_dir_all(t.data()).unwrap();
    let e = runindex::Entry { campaign: "deliver-x".into(), session_id: SID.into(), run_id: RUN_ID.into(), cwd: t.repo(), lead_session_id: None };
    runindex::append(&t.data().join("runs.jsonl"), &e).unwrap();
    let app = App::open(&t.data(), &t.home()).unwrap();
    app.lead.set_bin(Some(t.bin("claude")));
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "read_only");
    assert_eq!(app.lead_status(RUN_ID).unwrap()["available"], false);
}

#[test]
fn a_run_launched_but_never_asked_about_can_be_asked_after_a_restart() {
    let t = Tmp::new();
    // Life 1: the app launches the run (the real launch path) and the run finishes. Nobody asks the Lead anything.
    let app = App::open(&t.data(), &t.home()).unwrap();
    app.launch(SID, RUN_ID, "deliver-x", &t.repo(), |_| Ok(vec!["sleep".to_string(), "5".to_string()]), false).unwrap();
    let rec = t.data().join("control/lead").join(RUN_ID).join("cwd");
    assert_eq!(std::fs::read_to_string(&rec).unwrap(), t.repo().to_string_lossy(), "the launch wrote the guarded record");
    app.sup.kill(&format!("run-{SID}")).unwrap();
    drop(app);
    // Life 2: only the index (and the guarded record) remember it. The first ask works.
    let app2 = App::open(&t.data(), &t.home()).unwrap();
    app2.lead.set_bin(Some(t.bin("claude")));
    app2.lead.set_env(vec![("HOME", t.home().into()), ("PATH", format!("{}:/usr/bin:/bin", t.0.join("bin").display()).into())]);
    assert_eq!(app2.lead_status(RUN_ID).unwrap()["available"], true);
    assert!(app2.ask_lead(RUN_ID, "how did it go?").is_ok());
    wait_until("the ask", || idle(&app2) && t.ask_states().len() == 3);
    // An index line with no guarded record (a legacy or forged entry) is still refused.
    let other = ("20261106-0912", "2b8fad5b-d9cb-469f-a165-70867728950e");
    let e = runindex::Entry { campaign: "deliver-y".into(), session_id: other.1.into(), run_id: other.0.into(), cwd: t.repo(), lead_session_id: None };
    runindex::append(&t.data().join("runs.jsonl"), &e).unwrap();
    assert_eq!(code(&app2.ask_lead(other.0, "hi").unwrap_err()), "read_only");
}

#[test]
fn a_selftest_denied_on_a_different_file_does_not_pass_and_is_not_cached() {
    let t = Tmp::new();
    let app = t.app();
    // The probe reads (and is denied on) some other protected file and never touches the canary.
    t.set("selftest", "otherdeny");
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "lead_guard_missing");
    assert_eq!(code(&app.ask_lead(RUN_ID, "hi").unwrap_err()), "lead_guard_missing", "a failure is not cached");
    assert_eq!(t.count("selftest.count"), 2);
    assert!(t.ledger().is_empty() && t.count("ask.count") == 0, "no real ask ran");
    // A denial that names the canary passes.
    t.set("selftest", "deny");
    app.ask_lead(RUN_ID, "hi").unwrap();
    wait_until("the ask", || idle(&app));
}
