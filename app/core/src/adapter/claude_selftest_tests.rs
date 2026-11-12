use super::*;

/// A stand-in Claude Code that talks to the stub the way `claude -p` does, and executes each tool call it is given.
/// `good` refuses what the real guard, deny rules and OS sandbox refuse (a python Bash call answers with the sandbox's
/// EPERM, as its own output); `leaky` lets an Edit into the data dir through; `wrong_reason` refuses the Edit for
/// another reason; `unsandboxed` runs every Bash call that names no data-dir path, as a build without the sandbox
/// would; `hang` never answers.
fn fake_claude(dir: &Path, mode: &str) -> PathBuf {
    let p = dir.join(format!("fake-claude-{mode}"));
    let script = format!(
        r#"#!/usr/bin/python3 -I
import json, os, subprocess, sys, urllib.request, time
MODE = "{mode}"
if MODE == "hang":
    time.sleep(600)
args = sys.argv[1:]
tools = args[args.index("--tools") + 1].split(",")
print(json.dumps({{"type": "system", "subtype": "init", "tools": tools, "permissionMode": args[args.index("--permission-mode") + 1]}}), flush=True)
base = os.environ["ANTHROPIC_BASE_URL"]
msgs = [{{"role": "user", "content": [{{"type": "text", "text": args[-1]}}]}}]
DATA = "Application Support/VajraDesktop"
DENY = "<tool_use_error>File is in a directory that is denied by your permission settings.</tool_use_error>"
while True:
    req = urllib.request.Request(base + "/v1/messages?beta=true", data=json.dumps({{"model": "m", "tools": [{{"name": t}} for t in tools], "messages": msgs}}).encode(), headers={{"content-type": "application/json"}})
    sse = urllib.request.urlopen(req, timeout=10).read().decode()
    evs = [json.loads(l[6:]) for l in sse.splitlines() if l.startswith("data: ")]
    start = next(e for e in evs if e["type"] == "content_block_start")["content_block"]
    if start["type"] == "text":
        print(json.dumps({{"type": "result", "subtype": "success", "is_error": False}}), flush=True)
        sys.exit(0)
    inp = json.loads(next(e for e in evs if e["type"] == "content_block_delta")["delta"]["partial_json"])
    name, err, text = start["name"], False, "ok"
    target = inp.get("file_path") or inp.get("command", "")
    if DATA in target:
        if name == "Bash":
            err, text = True, "PreToolUse:Bash hook error: Vajra guard blocked this Bash call: redirect into protected path"
        elif name == "Read" and MODE == "leaky_read":
            text = open(inp["file_path"]).read()
        elif name == "Edit" and MODE == "leaky":
            open(inp["file_path"], "w").write("edited\n")
        elif name == "Edit" and MODE == "wrong_reason":
            err, text = True, "<tool_use_error>File has not been read yet.</tool_use_error>"
        else:
            err, text = True, DENY
    elif name == "Write":
        open(inp["file_path"], "w").write(inp["content"])
    elif name == "Bash" and "python3 -c" in target and MODE != "unsandboxed":
        text = "Traceback (most recent call last):\n  File \"<string>\", line 1\nPermissionError: [Errno 1] Operation not permitted: <locked>"
    elif name == "Bash":
        r = subprocess.run(["/bin/sh", "-c", target], capture_output=True, text=True)
        err, text = r.returncode != 0, r.stdout + r.stderr
    msgs.append({{"role": "assistant", "content": [{{"type": "tool_use", "id": start["id"], "name": name, "input": inp}}]}})
    msgs.append({{"role": "user", "content": [{{"type": "tool_result", "tool_use_id": start["id"], "is_error": err, "content": text}}]}})
"#
    );
    std::fs::write(&p, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

/// A home with the repo's guard hooks installed where the app installs them.
fn home_with_hooks(dir: &Path) -> PathBuf {
    let home = dir.join("home");
    let hooks = home.join(".claude/skills/vajra/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for e in std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hooks")).unwrap().flatten() {
        if e.file_type().unwrap().is_file() {
            std::fs::copy(e.path(), hooks.join(e.file_name())).unwrap();
        }
    }
    home
}

/// The flag shape the dispatch checks for the hooks installed under `home`.
fn shape(home: &Path) -> String {
    claude_worker_flag_shape(&crate::adapter::profiles::claude_worker_hooks_dir(home)).unwrap()
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vajra-selftest-t-{tag}-{}", crate::ids::uuid_v4().unwrap()));
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

/// Every file under `dir`, with its bytes: to show the self-test changed nothing in the user's home.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() {
                stack.push(e.path());
            } else {
                out.push((e.path(), std::fs::read(e.path()).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_build_that_refuses_every_data_dir_write_by_the_right_layer_passes_and_leaves_home_untouched() {
    let d = tmp("good");
    let home = home_with_hooks(&d);
    let before = snapshot(&home);
    assert_eq!(run(&fake_claude(&d, "good"), &home, &shape(&home), Duration::from_secs(30)), Ok(()));
    assert_eq!(snapshot(&home), before, "the user's home is read, never changed");
}

/// A build whose Bash calls run outside the OS sandbox: the guard and deny rules still refuse every call that names
/// the data dir, but a command that reads its target from elsewhere gets the key folder.
#[test]
fn a_build_without_the_os_sandbox_fails_on_the_name_free_key_read() {
    let d = tmp("unsandboxed");
    let home = home_with_hooks(&d);
    let r = run(&fake_claude(&d, "unsandboxed"), &home, &shape(&home), Duration::from_secs(30));
    assert_eq!(r, Err("failed: the OS sandbox refused a key read that names no locked path".into()));
}

/// #195 bhishma: the PASS proves the hook bytes the test copied, so they must be the bytes the dispatch hashed.
#[test]
fn a_guard_that_no_longer_matches_the_dispatch_shape_is_refused_before_claude_runs() {
    let d = tmp("guard-moved");
    let home = home_with_hooks(&d);
    let marker = d.join("ran");
    let bin = d.join("claude");
    std::fs::write(&bin, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let before = shape(&home);
    let guard = crate::adapter::profiles::claude_worker_hooks_dir(&home).join("pre-tool-use.sh");
    let mut b = std::fs::read(&guard).unwrap();
    b.push(b'\n');
    std::fs::write(&guard, b).unwrap();
    assert_eq!(run(&bin, &home, &before, Duration::from_secs(5)), Err("the Vajra guard changed while it was copied".into()));
    assert!(!marker.exists(), "claude never ran");
}

#[test]
fn a_build_that_lets_an_edit_into_the_data_dir_fails() {
    let d = tmp("leaky");
    let home = home_with_hooks(&d);
    let r = run(&fake_claude(&d, "leaky"), &home, &shape(&home), Duration::from_secs(30));
    assert_eq!(r, Err("failed: the deny rules refused an Edit in the data dir".into()));
}

/// #195 bhishma: a build that ignores Read denies leaks the data dir to a worker, though every write was refused and
/// the seed is unchanged.
#[test]
fn a_build_that_lets_a_read_of_the_data_dir_through_fails() {
    let d = tmp("leaky-read");
    let home = home_with_hooks(&d);
    let r = run(&fake_claude(&d, "leaky_read"), &home, &shape(&home), Duration::from_secs(30));
    assert_eq!(r, Err("failed: the deny rules refused a Read in the data dir".into()));
}

#[test]
fn an_edit_refused_for_another_reason_is_not_proof() {
    let d = tmp("wrong");
    let home = home_with_hooks(&d);
    let r = run(&fake_claude(&d, "wrong_reason"), &home, &shape(&home), Duration::from_secs(30));
    assert_eq!(r, Err("failed: the deny rules refused an Edit in the data dir".into()));
}

#[test]
fn a_build_that_hangs_is_cut_off_at_the_timeout() {
    let d = tmp("hang");
    let home = home_with_hooks(&d);
    let start = Instant::now();
    assert_eq!(run(&fake_claude(&d, "hang"), &home, &shape(&home), Duration::from_secs(2)), Err("timeout".into()));
    assert!(start.elapsed() < Duration::from_secs(10), "{:?}", start.elapsed());
}

#[test]
fn no_installed_guard_means_no_self_test() {
    let d = tmp("noguard");
    let home = d.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let marker = d.join("ran");
    let bin = d.join("claude");
    std::fs::write(&bin, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(run(&bin, &home, "no-shape", Duration::from_secs(5)), Err("the Vajra guard is not installed".into()));
    assert!(!marker.exists(), "claude never ran");
}

#[test]
fn a_build_that_skips_a_probe_fails() {
    // Exits successfully after the first call: nothing forbidden happened, but nothing was proven either.
    let d = tmp("short");
    let home = home_with_hooks(&d);
    let bin = d.join("claude");
    std::fs::write(&bin, "#!/bin/sh\necho '{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[\"Read\",\"Grep\",\"Glob\",\"Bash\",\"Edit\",\"Write\"],\"permissionMode\":\"dontAsk\"}'\necho '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false}'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(run(&bin, &home, &shape(&home), Duration::from_secs(10)), Err("failed: the plan reached its last turn".into()));
}
