use super::*;
use std::path::Path;

const TMP: &str = "/w/proj/run/tmp/t1-r1";
const WT: &str = "/w/proj/run/t1-r1";
/// A value that would be a key if one leaked. Never a real key.
const SENTINEL: &str = "sk-vajra-sentinel-0000";

fn launch(kind: RoleKind, network: bool) -> WorkerLaunch<'static> {
    WorkerLaunch { kind, prompt: "Do the task", model: Some("vendor/model-a"), effort: Some("high"), network, ticket_tmp: Path::new(TMP) }
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vajra-prof-{tag}-{}-{}", std::process::id(), crate::ids::unix_now()));
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

fn pairs(a: &[String]) -> Vec<&str> {
    a.windows(2).filter(|w| w[0] == "-c").map(|w| w[1].as_str()).collect()
}

#[test]
fn codex_gateway_worker_argv_snapshot() {
    let a = codex_worker_argv(&launch(RoleKind::Worker, false)).unwrap();
    let id = crate::providers::codex_home::PROVIDER_ID;
    let want: Vec<String> = [
        "/bin/sh", "-c", r#"exec "$@" </dev/null"#, "sh", "codex", "exec", "--json", "--sandbox", "workspace-write", "-m", "vendor/model-a",
        "-c", "model_reasoning_effort=\"high\"",
        "-c", &format!("model_providers.{id}.request_max_retries=0"), "-c", &format!("model_providers.{id}.stream_max_retries=0"),
        "-c", "sandbox_workspace_write.exclude_slash_tmp=true", "-c", "sandbox_workspace_write.exclude_tmpdir_env_var=true",
        "-c", "approval_policy=\"never\"", "-c", "sandbox_workspace_write.network_access=false",
        "-c", &format!("sandbox_workspace_write.writable_roots=[\"{TMP}\"]"),
        "-c", "web_search=\"disabled\"", "--", "Do the task",
    ].map(String::from).into();
    assert_eq!(a, want);
}

#[test]
fn codex_login_worker_argv_snapshot_has_no_builtin_provider_override() {
    let a = codex_login_worker_argv(&launch(RoleKind::Verifier, false)).unwrap();
    let want: Vec<String> = [
        "/bin/sh", "-c", r#"exec "$@" </dev/null"#, "sh", "codex", "exec", "--json", "--sandbox", "workspace-write", "-m", "vendor/model-a",
        "-c", "model_reasoning_effort=\"high\"",
        "-c", "approval_policy=\"never\"", "-c", "sandbox_workspace_write.network_access=false",
        "-c", &format!("sandbox_workspace_write.writable_roots=[\"{TMP}\"]"),
        "-c", "sandbox_workspace_write.exclude_slash_tmp=true", "-c", "sandbox_workspace_write.exclude_tmpdir_env_var=true",
        "-c", "web_search=\"disabled\"", "--", "Do the task",
    ].map(String::from).into();
    assert_eq!(a, want);
    // S3: `model_providers.openai.*` makes codex refuse to load, so the login route never passes one.
    assert!(!a.iter().any(|x| x.contains("model_providers.openai")));
}

#[test]
fn a_network_seat_turns_network_on_and_leaves_web_search_to_the_user() {
    let a = codex_login_worker_argv(&launch(RoleKind::Worker, true)).unwrap();
    let c = pairs(&a);
    assert!(c.contains(&"sandbox_workspace_write.network_access=true"));
    assert!(!c.iter().any(|x| x.starts_with("web_search")), "{c:?}");
}

/// The "always" settings are present whatever the seat, and nothing switches the user's MCP servers or notify off for
/// a worker (Q4: they are kept, and disclosed). Spike S3 ran this argv's overrides with a real codex 0.166 against a
/// hostile temp `config.toml` (`danger-full-access`, `on-request`, network on, a user writable root, web search live,
/// a user MCP server, notify) and every override won; this test pins the argv the spike proved.
#[test]
fn codex_worker_forces_every_always_setting_and_keeps_the_users_mcp_servers() {
    for a in [codex_worker_argv(&launch(RoleKind::Worker, false)).unwrap(), codex_login_worker_argv(&launch(RoleKind::Worker, false)).unwrap()] {
        let c = pairs(&a);
        for want in ["approval_policy=\"never\"", "sandbox_workspace_write.network_access=false", "sandbox_workspace_write.exclude_slash_tmp=true", "sandbox_workspace_write.exclude_tmpdir_env_var=true", "web_search=\"disabled\""] {
            assert!(c.contains(&want), "{want} missing from {c:?}");
        }
        assert!(c.iter().any(|x| x.starts_with("sandbox_workspace_write.writable_roots=")));
        assert_eq!(a[7..9], ["--sandbox".to_string(), "workspace-write".into()]);
        assert!(!c.iter().any(|x| x.starts_with("mcp_servers") || x.starts_with("notify")), "{c:?}");
        assert!(!a.iter().any(|x| x.contains("danger") || x.contains("bypass")));
        assert_eq!(a[a.len() - 2], "--");
    }
}

#[test]
fn worker_builders_refuse_bad_inputs() {
    let mut l = launch(RoleKind::Worker, false);
    l.kind = RoleKind::Lead;
    assert_eq!(codex_login_worker_argv(&l).unwrap_err().field, "kind");
    let mut l = launch(RoleKind::Worker, false);
    l.ticket_tmp = Path::new("relative/tmp");
    assert_eq!(codex_login_worker_argv(&l).unwrap_err().field, "ticket_tmp");
    l.ticket_tmp = Path::new("/w/a\"],x=[\"b");
    assert_eq!(codex_login_worker_argv(&l).unwrap_err().field, "ticket_tmp");
    let mut l = launch(RoleKind::Worker, false);
    l.effort = Some("high\" ,x=\"");
    assert_eq!(codex_login_worker_argv(&l).unwrap_err().field, "effort");
    l.effort = None;
    l.model = Some("--yolo");
    assert_eq!(codex_login_worker_argv(&l).unwrap_err().field, "model");
    let mut l = launch(RoleKind::Worker, false);
    l.model = None;
    assert_eq!(codex_worker_argv(&l).unwrap_err().field, "model");
    assert!(codex_lead_argv().is_err() && claude_lead_argv().is_err());
}

fn claude(tools: &'static str) -> ClaudeWorker<'static> {
    ClaudeWorker { claude: Path::new("/opt/bin/claude"), session_id: "3f1d7e8a-1b2c-4d5e-8f90-a1b2c3d4e5f6", settings: Path::new("/d/profiles/claude-worker.json"), tools }
}

#[test]
fn claude_worker_argv_snapshot() {
    let a = claude_worker_argv(&launch(RoleKind::Worker, false), &claude("Read,Grep,Glob,Edit,Write,Bash")).unwrap();
    let want: Vec<String> = [
        "/opt/bin/claude", "-p", "--output-format", "stream-json", "--verbose", "--session-id", "3f1d7e8a-1b2c-4d5e-8f90-a1b2c3d4e5f6",
        "--settings", "/d/profiles/claude-worker.json", "--permission-mode", "dontAsk",
        "--tools", "Read,Grep,Glob,Edit,Write,Bash", "--allowedTools", "Read,Grep,Glob,Edit,Write,Bash",
        "--model", "vendor/model-a", "--", "Do the task",
    ].map(String::from).into();
    assert_eq!(a, want);
    assert!(!a.iter().any(|x| x.contains("bypass") || x == "--setting-sources" || x == "--strict-mcp-config"));
}

#[test]
fn a_claude_verifier_gets_no_edit_tool_and_unknown_tools_are_refused() {
    let v = launch(RoleKind::Verifier, false);
    assert!(claude_worker_argv(&v, &claude(CHECKER_TOOLS)).is_ok());
    assert_eq!(claude_worker_argv(&v, &claude("Read,Bash,Write")).unwrap_err().field, "tools");
    assert_eq!(claude_worker_argv(&launch(RoleKind::Worker, false), &claude("Read,Agent")).unwrap_err().field, "tools");
    let mut l = launch(RoleKind::Worker, false);
    l.prompt = "--dangerously-skip-permissions";
    let a = claude_worker_argv(&l, &claude(CHECKER_TOOLS)).unwrap();
    assert_eq!(a[a.len() - 2..], ["--".to_string(), "--dangerously-skip-permissions".into()], "the prompt stays a positional");
}

#[test]
fn writer_tools_come_from_the_bundled_agents_and_drop_spawn_and_web_tools() {
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../bundled-agents"));
    let t = WriterTools::load(dir);
    for (role, kind) in crate::crew::roles::ROLES {
        let list = t.list(role).unwrap_or_else(|| panic!("{role} has no tools"));
        assert!(list.split(',').all(|x| CLAUDE_TOOLS.contains(&x)), "{role}: {list}");
        if kind == crate::crew::roles::Kind::Checker {
            assert_eq!(list, CHECKER_TOOLS);
        }
    }
    assert_eq!(t.list("backend"), Some("Read,Grep,Glob,Edit,Write,Bash"));
    assert_eq!(parse_tools("---\nname: x\ntools: Read, Agent(a, b), WebFetch, Read, Bash\n---\n"), Some("Read,Bash".into()));
    assert_eq!(parse_tools("no front matter\ntools: Read\n"), None);
    assert_eq!(parse_tools("---\nname: x\n---\ntools: Read\n"), None, "a tools line after the front matter is not read");
    assert!(WriterTools::load(Path::new("/nonexistent")).list("backend").is_none());
}

#[test]
fn the_flag_file_always_carries_the_guard_and_turns_hooks_and_bypass_back_on() {
    let d = tmpdir("flag");
    let (home, work, repo, data) = (d.join("home"), d.join("work"), d.join("repo"), d.join("data"));
    for p in [&home, &work, &repo, &data] {
        std::fs::create_dir_all(p).unwrap();
    }
    let s = claude_worker_flag(&home, &work, std::slice::from_ref(&repo), &data).unwrap();
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["disableAllHooks"], false);
    assert_eq!(v["permissions"]["disableBypassPermissionsMode"], "disable");
    assert_eq!(v["permissions"]["defaultMode"], "dontAsk");
    let pre = v["hooks"]["PreToolUse"].to_string();
    assert!(pre.contains("pre-tool-use.sh") && pre.contains("desktop-control.sh"), "{pre}");
    let deny: Vec<&str> = v["permissions"]["deny"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    let generated = crate::control::lead_settings::deny_rules_for(&home, &[work.clone(), repo.clone()], &data, &[]).unwrap();
    assert_eq!(deny, generated.iter().map(String::as_str).collect::<Vec<_>>(), "the rules come from the one generator");
    for want in [format!("{}/**", data.display()), format!("{}/.claude/vajra/**", home.display()), format!("{}/**/.env", work.display())] {
        assert!(deny.iter().any(|r| r.ends_with(&format!("{})", &want[1..]))), "{want} missing from {deny:?}");
    }
    let path = write_claude_worker_flag(&data, &s).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), s);
}

/// A temp HOME with a sentinel "key" in the app's environment and hostile user settings for both harnesses: the
/// sentinel appears in no argv, no worker env, the flag file, or any file the profile code writes.
#[test]
fn no_sentinel_key_reaches_argv_env_or_files() {
    let d = tmpdir("sentinel");
    let (home, work, repo, data) = (d.join("home"), d.join("work"), d.join("repo"), d.join("data"));
    for p in [&home.join(".codex"), &work, &repo, &data] {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(home.join(".codex/config.toml"), format!("sandbox_mode = \"danger-full-access\"\napproval_policy = \"on-request\"\n[sandbox_workspace_write]\nnetwork_access = true\n[mcp_servers.u]\ncommand = \"/bin/true\"\nenv = {{ OPENAI_API_KEY = \"{SENTINEL}\" }}\n")).unwrap();
    let l = launch(RoleKind::Worker, false);
    let mut all: Vec<String> = Vec::new();
    all.extend(codex_worker_argv(&l).unwrap());
    all.extend(codex_login_worker_argv(&l).unwrap());
    all.extend(claude_worker_argv(&l, &claude(CHECKER_TOOLS)).unwrap());
    for (k, v) in worker_env(Engine::Codex, &l, Some(&home.join("codex-home"))).into_iter().chain(worker_env(Engine::Claude, &l, None)) {
        assert!(crate::sessions::EXTRA_ENV_ALLOW.contains(&k), "{k}");
        all.push(v.to_string_lossy().into_owned());
    }
    let flag = claude_worker_flag(&home, &work, &[repo], &data).unwrap();
    write_claude_worker_flag(&data, &flag).unwrap();
    all.push(flag);
    assert!(!all.iter().any(|x| x.contains(SENTINEL)));
    for e in walk(&data) {
        assert!(!std::fs::read_to_string(&e).unwrap_or_default().contains(SENTINEL), "{}", e.display());
    }
}

fn walk(d: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(d).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() { out.extend(walk(&p)) } else { out.push(p) }
    }
    out
}

#[test]
fn profiles_refuse_what_is_not_proven() {
    assert!(profile(Engine::Codex, RoleKind::Worker).supported.is_ok());
    assert!(profile(Engine::Codex, RoleKind::Verifier).supported.is_ok());
    assert!(profile(Engine::Codex, RoleKind::Worker).supported.is_ok(), "S1 and S3 passed for Codex");
    for e in [Engine::Codex, Engine::Claude] {
        assert!(profile(e, RoleKind::Lead).supported.is_err());
        assert!(profile(e, RoleKind::DirectorGeneral).supported.is_err());
    }
    assert_eq!(RoleKind::of_role("reviewer"), Some(RoleKind::Verifier));
    assert_eq!(RoleKind::of_role("security_reviewer"), Some(RoleKind::Verifier));
    assert_eq!(RoleKind::of_role("backend"), Some(RoleKind::Worker));
    let p = profile(Engine::Codex, RoleKind::Worker);
    assert_eq!(disclosures(&p, false), vec![crate::desktop::seat_start::CODEX_WARNING]);
    assert_eq!(disclosures(&p, true).len(), 2);
    assert_eq!(disclosures(&profile(Engine::Claude, RoleKind::Worker), false), vec![crate::desktop::seat_start::CLAUDE_NOTE]);
}

/// The `turn_context` shape codex 0.166 wrote in spike S3 (paths shortened).
fn rollout(sandbox: Value, writes: &[&str], approval: &str) -> String {
    let entries: Vec<Value> = std::iter::once(json!({"path": {"type": "special", "value": {"kind": "root"}}, "access": "read"}))
        .chain(writes.iter().map(|w| if w.starts_with('/') { json!({"path": {"type": "path", "path": w}, "access": "write"}) } else { json!({"path": {"type": "special", "value": {"kind": w}}, "access": "write"}) }))
        .collect();
    let meta = json!({"type": "session_meta", "payload": {"model_provider": "vajra-openrouter"}});
    let tc = json!({"type": "turn_context", "payload": {"approval_policy": approval, "sandbox_policy": sandbox, "file_system_sandbox_policy": {"kind": "restricted", "entries": entries}}});
    format!("{meta}\n{tc}\n")
}

fn good_sandbox() -> Value {
    json!({"type": "workspace-write", "writable_roots": [TMP], "network_access": false, "exclude_tmpdir_env_var": true, "exclude_slash_tmp": true})
}

fn want() -> CodexWant {
    CodexWant { cwd: WT.into(), ticket_tmp: TMP.into(), network: false }
}

#[test]
fn the_read_back_accepts_the_applied_profile_and_refuses_anything_wider() {
    assert_eq!(check_codex_applied(&rollout(good_sandbox(), &[WT, TMP, TMP], "never"), &want()), Ok(()));
    let bad = |r: String| assert!(check_codex_applied(&r, &want()).is_err(), "{r}");
    bad(rollout(good_sandbox(), &[WT, TMP], "on-request"));
    bad(rollout(json!({"type": "danger-full-access"}), &[], "never"));
    let mut s = good_sandbox();
    s["network_access"] = json!(true);
    bad(rollout(s, &[WT, TMP], "never"));
    let mut s = good_sandbox();
    s["exclude_slash_tmp"] = json!(false);
    bad(rollout(s, &[WT, TMP], "never"));
    let mut s = good_sandbox();
    s["writable_roots"] = json!([TMP, "/Users/u"]);
    bad(rollout(s, &[WT, TMP], "never"));
    bad(rollout(good_sandbox(), &[WT, TMP, "slash_tmp"], "never"));
    bad(rollout(good_sandbox(), &[WT, "/Users/u/.ssh"], "never"));
    bad("{\"type\":\"session_meta\",\"payload\":{}}\n".into());
    let mut w = want();
    w.network = true;
    let mut s = good_sandbox();
    s["network_access"] = json!(true);
    assert_eq!(check_codex_applied(&rollout(s, &[WT, TMP], "never"), &w), Ok(()));
}

#[test]
fn the_claude_read_back_checks_the_mode_and_the_tool_set() {
    let init = |mode: &str, tools: &[&str]| format!("{}\n", json!({"type": "system", "subtype": "init", "permissionMode": mode, "tools": tools}));
    assert!(check_claude_applied(&init("dontAsk", &["Read", "Bash", "mcp__user__x"]), CHECKER_TOOLS).is_ok());
    assert!(check_claude_applied(&init("bypassPermissions", &["Read"]), CHECKER_TOOLS).is_err());
    assert!(check_claude_applied(&init("dontAsk", &["Read", "Write"]), CHECKER_TOOLS).is_err());
    assert!(check_claude_applied("{}\n", CHECKER_TOOLS).is_err());
}

#[test]
fn a_read_back_mismatch_kills_the_session() {
    use crate::bus::{Bus, Config};
    let d = tmpdir("kill");
    let bus = std::sync::Arc::new(Bus::open(&d.join("spill"), Config::default()).unwrap());
    let sup = crate::sessions::Supervisor::new(bus, None);
    let spec = crate::sessions::SpawnSpec { argv: vec!["sleep".into(), "30".into()], cwd: d.clone(), rows: 24, cols: 80, redact: Default::default(), extra_env: Vec::new() };
    sup.spawn("w-readback", spec).unwrap();
    let r = enforce(&sup, "w-readback", check_codex_applied(&rollout(good_sandbox(), &[WT, TMP], "on-request"), &want()));
    assert!(r.is_err());
    assert!(sup.wait("w-readback", std::time::Duration::from_secs(5)).unwrap().is_some(), "the session was killed");
    let spec = crate::sessions::SpawnSpec { argv: vec!["sleep".into(), "30".into()], cwd: d.clone(), rows: 24, cols: 80, redact: Default::default(), extra_env: Vec::new() };
    sup.spawn("w-ok", spec).unwrap();
    assert!(enforce(&sup, "w-ok", Ok(())).is_ok());
    assert!(sup.is_running("w-ok").unwrap());
    sup.kill("w-ok").unwrap();
}

/// Pins today's state: the owner's S4 run passed (claude 2.1.322), so both Claude worker kinds are supported. The
/// refusal path stays: with the flag off, `S4_UNVERIFIED` is what `profile()` returns.
#[test]
fn the_claude_worker_profile_is_supported_after_s4_passed() {
    const { assert!(CLAUDE_WORKER_S4_PASSED) };
    for kind in [RoleKind::Worker, RoleKind::Verifier] {
        let p = profile(Engine::Claude, kind);
        assert_eq!(p.supported, Ok(()));
        assert!(!p.always.is_empty());
    }
    assert_eq!(S4_UNVERIFIED.reason, "s4_unverified");
}

#[test]
fn the_s4_owner_script_runs_the_argv_the_builder_makes() {
    let script = include_str!("../../../bench/spikes/s4-claude-settings.sh");
    let a = claude_worker_argv(&launch(RoleKind::Verifier, false), &claude(CHECKER_TOOLS)).unwrap();
    for flag in ["-p", "--output-format", "stream-json", "--verbose", "--session-id", "--settings", "--permission-mode", "dontAsk", "--tools", "--allowedTools"] {
        assert!(a.iter().any(|x| x == flag) && script.contains(&format!(" {flag} ")), "{flag}");
    }
    assert!(script.contains(CHECKER_TOOLS));
    for key in ["\"disableAllHooks\":false", "\"disableBypassPermissionsMode\":\"disable\""] {
        assert!(script.contains(key), "{key}");
    }
}
