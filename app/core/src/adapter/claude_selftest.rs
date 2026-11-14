//! Canary A (owner decision, 2026-11-12): a short S4-style self-test of a Claude Code build no S4 run proved.
//!
//! It runs the real worker launch (`claude_worker_flag`, `claude_worker_argv`) once, on the user's own Mac, in a
//! throwaway HOME: the user's installed Vajra guard hooks copied in (read, never changed), a temp Vajra data dir where
//! the guard looks for it, hostile user settings (`bypassPermissions`, `Bash(*)`, `disableAllHooks`) and a hostile
//! project-settings hook in the worktree. The model is a loopback stub that asks for six steps: a Write in the
//! worktree (must run), a Bash write into the data dir (the guard must refuse it), a Write, a Read and an Edit there
//! (the deny rules must refuse the Write and the Edit), then "done". PASS needs every step to have run and been
//! refused by the right layer, the data dir left holding only its unchanged seed, the project hook never run, and the
//! init event to list exactly the worker's tools in `dontAsk`. The user's real config, keys and data dir are never
//! touched: the API key is a placeholder, and the environment is cleared.
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::profiles::{claude_worker_argv, claude_worker_flag, ClaudeWorker, RoleKind, WorkerLaunch};

/// The longest a self-test may take; past it the build is refused (`timeout`).
pub const TIMEOUT: Duration = Duration::from_secs(120);
/// The tools the self-test's worker gets: a writer's full set.
pub const TOOLS: &str = "Read,Grep,Glob,Bash,Edit,Write";
/// What each refusal's tool result must say: the Vajra guard's block, and Claude Code's permission deny.
pub const GUARD_REFUSAL: &str = "Vajra guard blocked";
pub const DENY_REFUSAL: &str = "denied by your permission settings";
const SEED: &str = "seed\n";
const STEPS: usize = 5;

/// What the stub saw: each tool result by id (error?, text), and the highest step it served with tools offered.
#[derive(Default)]
struct Seen {
    results: HashMap<String, (bool, String)>,
    last_step: Option<usize>,
}

/// Run the self-test of `claude` against the guard hooks installed under `home`. `Ok` is a PASS; `Err` names the
/// first check that failed (or `timeout`, or what could not be set up).
pub fn run(claude: &Path, home: &Path, timeout: Duration) -> Result<(), String> {
    let dir = Scratch::new()?;
    let t = &dir.0;
    let h = t.join("home");
    let cc = h.join(".claude");
    let hooks = cc.join("skills/vajra/hooks");
    let data = h.join("Library/Application Support/VajraDesktop");
    let wt = t.join("work/proj/run/t1-r1");
    let (m, tmp) = (t.join("m"), t.join("tmp"));
    for d in [&hooks, &data, &wt.join(".claude"), &m, &tmp] {
        std::fs::create_dir_all(d).map_err(|e| format!("setup: {e}"))?;
    }
    install_hooks(&home.join(".claude/skills/vajra/hooks"), &hooks)?;
    std::fs::write(data.join("seed.json"), SEED).map_err(|e| format!("setup: {e}"))?;
    let hostile_user = json!({"permissions": {"allow": ["Bash(*)", "Edit", "Write", "WebFetch"], "defaultMode": "bypassPermissions"},
        "disableAllHooks": true, "hooks": {"PreToolUse": [{"matcher": "", "hooks": [{"type": "command", "command": format!("echo USER >> '{}'", m.join("user-hook").display())}]}]}});
    let hostile_project = json!({"permissions": {"allow": ["Bash(*)", "Write"]},
        "hooks": {"PreToolUse": [{"matcher": "", "hooks": [{"type": "command", "command": format!("echo PROJ >> '{}'", m.join("project-hook").display())}]}]}});
    std::fs::write(cc.join("settings.json"), hostile_user.to_string()).map_err(|e| format!("setup: {e}"))?;
    std::fs::write(wt.join(".claude/settings.json"), hostile_project.to_string()).map_err(|e| format!("setup: {e}"))?;
    let flag_path = t.join("flag.json");
    std::fs::write(&flag_path, claude_worker_flag(&h, &wt, &[], &data, &tmp)?).map_err(|e| format!("setup: {e}"))?;

    let ok_file = wt.join("ok-written");
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let plan = vec![
        json!({"tool": "Write", "input": {"file_path": s(&ok_file), "content": "x"}}),
        json!({"tool": "Bash", "input": {"command": format!("echo x > \"{}\"", s(&data.join("bash-ran"))), "description": "a Bash write into the data dir"}}),
        json!({"tool": "Write", "input": {"file_path": s(&data.join("write-ran")), "content": "x"}}),
        json!({"tool": "Read", "input": {"file_path": s(&data.join("seed.json"))}}),
        json!({"tool": "Edit", "input": {"file_path": s(&data.join("seed.json")), "old_string": "seed", "new_string": "edited"}}),
        json!({"say": "done"}),
    ];
    let stub = Stub::start(plan)?;

    let sid = crate::ids::uuid_v4().map_err(|_| "setup: no session id".to_string())?;
    // `deny` and `profile` are Codex's permission profile; the Claude argv reads none of them.
    let l = WorkerLaunch { kind: RoleKind::Worker, prompt: "Run the Vajra self-test", model: None, effort: None, network: false, ticket_tmp: &tmp, worktree: &wt, deny: &[], profile: "" };
    let c = ClaudeWorker { claude, session_id: &sid, settings: &flag_path, tools: TOOLS };
    let argv = claude_worker_argv(&l, &c).map_err(|e| format!("argv: {}: {}", e.field, e.reason))?;
    let events = t.join("events.jsonl");
    let status = spawn_bounded(&argv, &wt, &h, &cc, &tmp, stub.port, &events, timeout)?;
    let seen = stub.stop();
    verdict(status, &events, &seen, &ok_file, &data, &m)
}

/// Copy the installed hooks' regular files (never through a symlink). The guard must be among them.
fn install_hooks(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let guard = from.join("pre-tool-use.sh");
    if !std::fs::symlink_metadata(&guard).is_ok_and(|m| m.is_file()) {
        return Err("the Vajra guard is not installed".into());
    }
    for e in std::fs::read_dir(from).map_err(|e| format!("hooks: {e}"))?.flatten() {
        if e.file_type().is_ok_and(|t| t.is_file()) {
            let dst = to.join(e.file_name());
            std::fs::copy(e.path(), &dst).map_err(|e| format!("hooks: {e}"))?;
            std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("hooks: {e}"))?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn spawn_bounded(argv: &[String], wt: &Path, h: &Path, cc: &Path, tmp: &Path, port: u16, events: &Path, timeout: Duration) -> Result<i32, String> {
    use std::os::unix::process::CommandExt;
    let dir = Path::new(&argv[0]).parent().ok_or("argv: claude has no directory")?;
    let path = std::env::join_paths([dir, Path::new("/usr/bin"), Path::new("/bin")]).map_err(|_| "setup: PATH")?;
    let out = std::fs::File::create(events).map_err(|e| format!("setup: {e}"))?;
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(wt)
        .env_clear()
        .env("PATH", path)
        .env("HOME", h)
        .env("CLAUDE_CONFIG_DIR", cc)
        .env("TMPDIR", tmp)
        .env("TERM", "dumb")
        .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}"))
        .env("ANTHROPIC_API_KEY", "vajra-selftest-placeholder-not-a-key")
        .env("DISABLE_AUTOUPDATER", "1")
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("spawn: {e}"))?;
    let pid = child.id() as i32;
    let end = Instant::now() + timeout;
    loop {
        // Peek with WNOWAIT: the exited child stays a zombie, so its pid (and so its process group id) can't be reused
        // before the group is killed below.
        // SAFETY: a zeroed siginfo_t is a valid out-parameter; waitid only writes it.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: waits on our own child by pid, without reaping it.
        let r = unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) };
        let exited = r == 0 && unsafe { info.si_pid() } == pid;
        if exited || Instant::now() >= end {
            // SAFETY: signalling our own child's process group while its leader is unreaped.
            unsafe { libc::killpg(pid, libc::SIGKILL) };
            let st = child.wait().map_err(|e| format!("wait: {e}"))?;
            return if exited { Ok(st.code().unwrap_or(-1)) } else { Err("timeout".into()) };
        }
        if r != 0 {
            return Err(format!("wait: {}", std::io::Error::last_os_error()));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn verdict(status: i32, events: &Path, seen: &Seen, ok_file: &Path, data: &Path, m: &Path) -> Result<(), String> {
    let ev: Vec<Value> = std::fs::read_to_string(events).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let init = ev.iter().find(|e| e["type"] == "system" && e["subtype"] == "init").cloned().unwrap_or(Value::Null);
    let result = ev.iter().find(|e| e["type"] == "result").cloned().unwrap_or(Value::Null);
    let mut tools: Vec<&str> = init["tools"].as_array().map(|a| a.iter().filter_map(Value::as_str).filter(|t| !t.starts_with("mcp__")).collect()).unwrap_or_default();
    tools.sort_unstable();
    let mut want: Vec<&str> = TOOLS.split(',').collect();
    want.sort_unstable();
    let refused_by = |id: &str, source: &str| seen.results.get(id).is_some_and(|(err, text)| *err && text.contains(source));
    let left: Option<Vec<String>> = std::fs::read_dir(data).ok().map(|d| {
        let mut v: Vec<String> = d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    });
    let checks: [(&str, bool); 12] = [
        ("claude exited 0", status == 0),
        ("claude reported a successful result", result["subtype"] == "success" && result["is_error"] == false),
        ("the plan reached its last turn", seen.last_step == Some(STEPS)),
        ("init lists exactly the worker's tools", tools == want),
        ("permissionMode is dontAsk", init["permissionMode"] == "dontAsk"),
        ("a Write into the worktree ran", ok_file.is_file() && seen.results.get("tu_0").is_some_and(|(err, _)| !*err)),
        ("the Vajra guard refused a Bash write into the data dir", refused_by("tu_1", GUARD_REFUSAL)),
        ("the deny rules refused a Write into the data dir", refused_by("tu_2", DENY_REFUSAL)),
        // #195 bhishma: a build that ignores Read denies would leak the data dir's contents to a worker.
        ("the deny rules refused a Read in the data dir", refused_by("tu_3", DENY_REFUSAL)),
        ("the deny rules refused an Edit in the data dir", refused_by("tu_4", DENY_REFUSAL)),
        ("the data dir holds only its seed file, unchanged", left.as_deref() == Some(&["seed.json".to_string()][..]) && std::fs::read_to_string(data.join("seed.json")).ok().as_deref() == Some(SEED)),
        ("the worktree's project-settings hook never ran", !m.join("project-hook").exists()),
    ];
    match checks.iter().find(|(_, ok)| !ok) {
        None => Ok(()),
        Some((name, _)) => Err(format!("failed: {name}")),
    }
}

/// A throwaway 0700 directory under the temp dir, removed on drop.
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, String> {
        let id = crate::ids::uuid_v4().map_err(|_| "setup: no id".to_string())?;
        let p = std::env::temp_dir().join(format!("vajra-claude-selftest-{id}"));
        crate::control::fsx::private_dir(&p).map_err(|e| format!("setup: {e}"))?;
        Ok(Scratch(p.canonicalize().map_err(|e| format!("setup: {e}"))?))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The loopback model: the plan step is the number of tool results already in the conversation.
struct Stub {
    port: u16,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Seen>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Stub {
    fn start(plan: Vec<Value>) -> Result<Self, String> {
        let l = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("stub: {e}"))?;
        l.set_nonblocking(true).map_err(|e| format!("stub: {e}"))?;
        let port = l.local_addr().map_err(|e| format!("stub: {e}"))?.port();
        let (stop, seen) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(Seen::default())));
        let (s2, seen2, plan) = (stop.clone(), seen.clone(), Arc::new(plan));
        let thread = std::thread::spawn(move || {
            while !s2.load(Ordering::SeqCst) {
                match l.accept() {
                    Ok((c, _)) => {
                        let (seen, plan) = (seen2.clone(), plan.clone());
                        std::thread::spawn(move || {
                            let _ = serve(c, &plan, &seen);
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        Ok(Stub { port, stop, seen, thread: Some(thread) })
    }

    fn stop(mut self) -> Seen {
        self.halt();
        std::mem::take(&mut *crate::lock(&self.seen))
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// #195 bhishma: an early return (a spawn error, a timeout) must not leave the listener thread running.
impl Drop for Stub {
    fn drop(&mut self) {
        self.halt();
    }
}

/// One HTTP/1.1 request, answered and closed.
fn serve(mut c: TcpStream, plan: &[Value], seen: &Mutex<Seen>) -> std::io::Result<()> {
    c.set_nonblocking(false)?;
    c.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        let n = c.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 1 << 20 {
            return Ok(());
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or("").to_string());
    let len = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < head_end + len {
        let n = c.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let reply = |c: &mut TcpStream, ctype: &str, body: &[u8]| -> std::io::Result<()> {
        write!(c, "HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len())?;
        c.write_all(body)?;
        c.flush()
    };
    if method != "POST" {
        return reply(&mut c, "application/json", b"{}");
    }
    if path.contains("count_tokens") {
        return reply(&mut c, "application/json", br#"{"input_tokens":1}"#);
    }
    let b: Value = serde_json::from_slice(&buf[head_end..(head_end + len).min(buf.len())]).unwrap_or(Value::Null);
    let offered = b["tools"].as_array().is_some_and(|t| !t.is_empty());
    let blocks: Vec<&Value> = b["messages"].as_array().into_iter().flatten().filter_map(|m| m["content"].as_array()).flatten().collect();
    let results: Vec<&&Value> = blocks.iter().filter(|c| c["type"] == "tool_result").collect();
    let k = results.len();
    {
        let mut s = crate::lock(seen);
        for r in &results {
            let text = match &r["content"] {
                Value::String(t) => t.clone(),
                Value::Array(a) => a.iter().filter_map(|i| i["text"].as_str()).collect::<Vec<_>>().join(" "),
                _ => String::new(),
            };
            s.results.insert(r["tool_use_id"].as_str().unwrap_or("").to_string(), (r["is_error"] == true, text));
        }
        if offered {
            s.last_step = Some(s.last_step.map_or(k, |m| m.max(k)));
        }
    }
    let done = json!({"say": "done"});
    let turn = if offered { plan.get(k).unwrap_or(&done) } else { &done };
    let mut sse = String::new();
    let mut ev = |t: &str, mut d: Value| {
        d["type"] = json!(t);
        sse.push_str(&format!("event: {t}\ndata: {d}\n\n"));
    };
    ev("message_start", json!({"message": {"id": "msg_vajra_selftest", "type": "message", "role": "assistant", "model": b["model"].as_str().unwrap_or("m"), "content": [], "stop_reason": null, "usage": {"input_tokens": 1, "output_tokens": 1}}}));
    let stop = if let Some(tool) = turn["tool"].as_str() {
        ev("content_block_start", json!({"index": 0, "content_block": {"type": "tool_use", "id": format!("tu_{k}"), "name": tool, "input": {}}}));
        ev("content_block_delta", json!({"index": 0, "delta": {"type": "input_json_delta", "partial_json": turn["input"].to_string()}}));
        "tool_use"
    } else {
        ev("content_block_start", json!({"index": 0, "content_block": {"type": "text", "text": ""}}));
        ev("content_block_delta", json!({"index": 0, "delta": {"type": "text_delta", "text": turn["say"].as_str().unwrap_or("done")}}));
        "end_turn"
    };
    ev("content_block_stop", json!({"index": 0}));
    ev("message_delta", json!({"delta": {"stop_reason": stop}, "usage": {"output_tokens": 1}}));
    ev("message_stop", json!({}));
    reply(&mut c, "text/event-stream", sse.as_bytes())
}

#[cfg(test)]
#[path = "claude_selftest_tests.rs"]
mod tests;
