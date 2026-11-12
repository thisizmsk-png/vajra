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
}

fn claude(tools: &'static str) -> ClaudeWorker<'static> {
    ClaudeWorker { claude: Path::new("/opt/bin/claude"), session_id: "3f1d7e8a-1b2c-4d5e-8f90-a1b2c3d4e5f6", settings: Path::new("/d/profiles/claude-worker.json"), tools }
}

#[test]
fn claude_worker_argv_snapshot() {
    let a = claude_worker_argv(&launch(RoleKind::Worker, false), &claude("Read,Grep,Glob,Edit,Write,Bash")).unwrap();
    let want: Vec<String> = [
        "/opt/bin/claude", "-p", "--output-format", "stream-json", "--verbose", "--session-id", "3f1d7e8a-1b2c-4d5e-8f90-a1b2c3d4e5f6",
        "--setting-sources", "user", "--settings", "/d/profiles/claude-worker.json", "--permission-mode", "dontAsk",
        "--tools", "Read,Grep,Glob,Edit,Write,Bash", "--allowedTools", "Read,Grep,Glob,Edit,Write,Bash",
        "--model", "vendor/model-a", "--", "Do the task",
    ].map(String::from).into();
    assert_eq!(a, want);
    assert!(!a.iter().any(|x| x.contains("bypass") || x == "--strict-mcp-config"));
    // Only the user's own settings load: never the worktree's project or local settings (#147 review).
    assert_eq!(a.iter().filter(|x| *x == "--setting-sources").count(), 1);
    assert_eq!(a[a.iter().position(|x| x == "--setting-sources").unwrap() + 1], "user");
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
    let mut generated = crate::control::lead_settings::deny_rules_for(&home, &[work.clone(), repo.clone()], &data, &[]).unwrap();
    generated.extend(crate::control::lead_settings::worker_edit_rules(&home, &data).unwrap());
    assert_eq!(deny, generated.iter().map(String::as_str).collect::<Vec<_>>(), "the rules come from the one generator");
    for want in [format!("{}/**", data.display()), format!("{}/.claude/vajra/**", home.display()), format!("{}/**/.env", work.display())] {
        assert!(deny.iter().any(|r| r.ends_with(&format!("{})", &want[1..]))), "{want} missing from {deny:?}");
    }
    // #147 gate H1: the edit tools can't rewrite the data dir (this flag file, the git baseline), any HOME_RULES path or
    // the user's git config, in both spellings of every base.
    use crate::control::lead_settings::{DATA_RULES, HOME_RULES, WORKER_EDIT_HOME_RULES};
    let edit = |base: &Path, suffix: &str| format!("Edit(/{}/{suffix})", base.display());
    for b in [home.clone(), home.canonicalize().unwrap()] {
        for suffix in HOME_RULES.iter().chain(&WORKER_EDIT_HOME_RULES) {
            assert!(deny.contains(&edit(&b, suffix).as_str()), "{suffix} missing from {deny:?}");
        }
    }
    for b in [data.clone(), data.canonicalize().unwrap()] {
        assert!(deny.contains(&edit(&b, DATA_RULES[0]).as_str()), "data missing from {deny:?}");
    }
    let sid = "3f1d7e8a-1b2c-4d5e-8f90-a1b2c3d4e5f6";
    let f = write_claude_worker_flag(&data, sid, &s).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert!(f.path.ends_with(format!("profiles/claude-worker-{sid}.json")));
    assert_eq!(std::fs::metadata(&f.path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(f.path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), s);
    assert!(write_claude_worker_flag(&data, "../x", &s).is_err(), "only a canonical session id names the file");
}

/// #147 gate H1: a worker that rewrites the flag file, or swaps it for a link, between the write and the spawn has the
/// launch refused.
#[test]
fn a_flag_file_changed_before_the_spawn_refuses_the_launch() {
    let d = tmpdir("flagcheck");
    let (home, work, repo, data) = (d.join("home"), d.join("work"), d.join("repo"), d.join("data"));
    for p in [&home, &work, &repo, &data] {
        std::fs::create_dir_all(p).unwrap();
    }
    let s = claude_worker_flag(&home, &work, std::slice::from_ref(&repo), &data).unwrap();
    let f = write_claude_worker_flag(&data, "3f1d7e8a-1b2c-4d5e-8f90-a1b2c3d4e5f6", &s).unwrap();
    assert_eq!(verify_claude_worker_flag(&f), Ok(()));
    let mut v: Value = serde_json::from_str(&s).unwrap();
    v["permissions"]["deny"] = json!([]);
    v["hooks"] = json!({"SessionStart": [{"hooks": [{"type": "command", "command": "true"}]}]});
    std::fs::write(&f.path, v.to_string()).unwrap();
    assert!(verify_claude_worker_flag(&f).is_err(), "rewritten");
    let copy = d.join("copy.json");
    std::fs::write(&copy, &s).unwrap();
    std::fs::remove_file(&f.path).unwrap();
    std::os::unix::fs::symlink(&copy, &f.path).unwrap();
    assert!(verify_claude_worker_flag(&f).is_err(), "a link to the same bytes");
    std::fs::remove_file(&f.path).unwrap();
    assert!(verify_claude_worker_flag(&f).is_err(), "deleted");
    // Session end removes the launch's file, and a missing file is fine.
    let f = write_claude_worker_flag(&data, "3f1d7e8a-1b2c-4d5e-8f90-a1b2c3d4e5f6", &s).unwrap();
    remove_claude_worker_flag(&f).unwrap();
    assert!(!f.path.exists());
    remove_claude_worker_flag(&f).unwrap();
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
    write_claude_worker_flag(&data, "3f1d7e8a-1b2c-4d5e-8f90-a1b2c3d4e5f6", &flag).unwrap();
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
    assert!(profile(Engine::Codex, RoleKind::Worker).supported.is_ok(), "S1 and S3 passed for Codex");
    assert!(profile(Engine::Codex, RoleKind::Verifier).supported.is_ok());
    for e in [Engine::Codex, Engine::Claude] {
        let want = if e == Engine::Claude { Err(LEAD_UNVERIFIED) } else { Ok(()) };
        assert_eq!(profile(e, RoleKind::Lead).supported, want, "S2 and S5 passed for Codex; the Claude lead waits for its own spike");
        assert_eq!(profile(e, RoleKind::Lead).user, &["login"], "a lead keeps only the user's login (N3)");
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
    let tc = json!({"type": "turn_context", "payload": {"cwd": WT, "approval_policy": approval, "sandbox_policy": sandbox, "file_system_sandbox_policy": {"kind": "restricted", "entries": entries}}});
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

/// #147 gate M1: the read-back fails closed on narrower-looking or unknown shapes.
#[test]
fn the_read_back_fails_closed_on_read_only_network_unknown_access_and_another_cwd() {
    let bad = |r: String, why: &str| assert!(check_codex_applied(&r, &want()).is_err(), "{why}: {r}");
    let ro = |net: Option<bool>| {
        let mut s = json!({"type": "read-only"});
        if let Some(n) = net {
            s["network_access"] = json!(n);
        }
        rollout(s, &[], "never")
    };
    assert_eq!(check_codex_applied(&ro(Some(false)), &want()), Ok(()));
    assert_eq!(check_codex_applied(&ro(None), &want()), Ok(()), "read-only with no network field has none");
    bad(ro(Some(true)), "read-only with network on");
    let with_access = |a: &str| rollout(good_sandbox(), &[WT, TMP], "never").replace("\"access\":\"read\"", &format!("\"access\":\"{a}\""));
    assert_eq!(check_codex_applied(&with_access("none"), &want()), Ok(()));
    for a in ["read-write", "full", "Write", ""] {
        bad(with_access(a), a);
    }
    let good = rollout(good_sandbox(), &[WT, TMP], "never");
    bad(good.replace(&format!("\"cwd\":\"{WT}\""), "\"cwd\":\"/Users/u\""), "another cwd");
    bad(good.replace(&format!("\"cwd\":\"{WT}\","), ""), "no cwd");
    let mut v: Value = serde_json::from_str(good.lines().nth(1).unwrap()).unwrap();
    v["payload"]["file_system_sandbox_policy"]["entries"] = json!({"access": "write"});
    bad(format!("{v}\n"), "entries not a list");
}

#[test]
fn the_claude_read_back_checks_the_mode_and_the_tool_set() {
    let init = |mode: &str, tools: &[&str]| format!("{}\n", json!({"type": "system", "subtype": "init", "permissionMode": mode, "tools": tools}));
    assert!(check_claude_applied(&init("dontAsk", &["Read", "Bash", "mcp__user__x"]), CHECKER_TOOLS).is_ok());
    assert!(check_claude_applied(&init("bypassPermissions", &["Read"]), CHECKER_TOOLS).is_err());
    assert!(check_claude_applied(&init("dontAsk", &["Read", "Write"]), CHECKER_TOOLS).is_err());
    assert!(check_claude_applied("{}\n", CHECKER_TOOLS).is_err());
    // #147 gate L1: a worker has no Vajra tools, even from a user server named like Vajra's.
    for t in ["mcp__vajra-desktop__run_start", "mcp__vajra__x", "mcp__Vajra_lookalike__y"] {
        assert!(check_claude_applied(&init("dontAsk", &["Read", t]), CHECKER_TOOLS).is_err(), "{t}");
    }
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

/// Pins today's state: spike S4 has not run, so both Claude worker kinds are refused with `s4_unverified`. When the
/// owner's S4 run prints PASS, flip `CLAUDE_WORKER_S4_PASSED` and change this test with it.
#[test]
fn the_claude_worker_profile_is_refused_until_s4_passes() {
    const { assert!(!CLAUDE_WORKER_S4_PASSED) };
    for kind in [RoleKind::Worker, RoleKind::Verifier] {
        let p = profile(Engine::Claude, kind);
        assert_eq!(p.supported, Err(S4_UNVERIFIED));
        assert_eq!(p.supported.unwrap_err().reason, "s4_unverified");
        assert!(!p.always.is_empty(), "the profile data is complete; only the proof is missing");
    }
}

#[test]
fn the_s4_owner_script_runs_the_argv_the_builder_makes() {
    let script = include_str!("../../../bench/spikes/s4-claude-settings.sh");
    let a = claude_worker_argv(&launch(RoleKind::Verifier, false), &claude(CHECKER_TOOLS)).unwrap();
    for flag in ["-p", "--output-format", "stream-json", "--verbose", "--session-id", "--settings", "--permission-mode", "dontAsk", "--tools", "--allowedTools", "--setting-sources"] {
        assert!(a.iter().any(|x| x == flag) && script.contains(&format!(" {flag} ")), "{flag}");
    }
    assert!(a.windows(2).any(|w| w[0] == "--setting-sources" && w[1] == "user") && script.contains(" --setting-sources user "));
    // The verdict is s4_check.py's; the worktree's project-settings hook counts toward PASS (#147 review).
    assert!(script.contains("/usr/bin/python3 -I \"$HERE/s4_check.py\" \"$SP\" \"$M\" \"$TOOLS\" \"$STATUS\""));
    assert!(S4_CHECK.contains("(\"the worktree's project-settings hook never ran\", not os.path.exists(m + \"/project-hook\"))"));
    assert!(script.contains(CHECKER_TOOLS));
    for key in ["\"disableAllHooks\":false", "\"disableBypassPermissionsMode\":\"disable\""] {
        assert!(script.contains(key), "{key}");
    }
}

const S4_CHECK: &str = include_str!("../../../bench/spikes/s4_check.py");

/// A transcript the S4 script could record: the stub served plan steps `0..served` (step k carries the tool results of
/// the k calls before it), Claude exited with `status`, and `result` says whether it emitted a successful result event.
/// Returns the checker's exit code and output.
fn s4_verdict(tag: &str, served: usize, status: &str, result: bool) -> (Option<i32>, String) {
    use serde_json::json;
    let sp = tmpdir(tag);
    let m = sp.join("m");
    std::fs::create_dir_all(sp.join("stub")).unwrap();
    std::fs::create_dir_all(&m).unwrap();
    let mut ev = vec![json!({"type": "system", "subtype": "init", "tools": ["Read", "Grep", "Glob", "Bash"], "permissionMode": "dontAsk"})];
    if result {
        ev.push(json!({"type": "result", "subtype": "success", "is_error": false}));
    }
    std::fs::write(sp.join("events.jsonl"), ev.iter().map(|e| e.to_string() + "\n").collect::<String>()).unwrap();
    // The allowed call succeeds; the blocked Bash call and the Write call come back as errors.
    let all = [json!({"id": "tu_0", "is_error": false, "text": ""}), json!({"id": "tu_1", "is_error": true, "text": "blocked by the S4 guard"}), json!({"id": "tu_2", "is_error": true, "text": "No such tool available: Write"})];
    let log: String = (0..served).map(|k| json!({"path": "/v1/messages", "tools": ["Bash"], "k": k, "results": all[..k], "body": "Run the S4 plan"}).to_string() + "\n").collect();
    std::fs::write(sp.join("stub/log"), log).unwrap();
    // Each call the stub's step k asked for reaches the guard before step k + 1 is requested.
    let guard: String = ["echo allowed > m/allowed-ran", "echo x > m/forbidden-ran"].iter().take(served.saturating_sub(1)).map(|l| format!("{l}\n")).collect();
    if served >= 2 {
        std::fs::write(m.join("guard"), guard).unwrap();
        std::fs::write(m.join("allowed-ran"), "allowed\n").unwrap();
    }
    let o = std::process::Command::new("/usr/bin/python3").arg("-I").arg("-c").arg(S4_CHECK.replace("if __name__ == \"__main__\":\n    main()", "main()"))
        .args([sp.to_str().unwrap(), m.to_str().unwrap(), "Read,Grep,Glob,Bash", status]).output().unwrap();
    (o.status.code(), String::from_utf8_lossy(&o.stdout).into_owned())
}

/// #147 codex: S4 PASS needs every probe to have run to its end. A run cut short after its first call (an error, or the
/// 180 s alarm), or one that exits 0 without reaching the plan's end, fails, though nothing forbidden happened.
#[test]
fn s4_passes_only_when_every_probe_ran_and_was_refused() {
    let (code, out) = s4_verdict("s4-full", 4, "0", true);
    assert_eq!(code, Some(0), "{out}");
    assert!(out.ends_with("S4 PASS\n"), "{out}");
    // Interrupted after the first (allowed) call: the alarm killed claude, so no result event and no later probe.
    let (code, out) = s4_verdict("s4-alarm", 2, "142", false);
    assert_eq!(code, Some(1), "{out}");
    for line in ["FAIL  claude exited 0", "FAIL  the plan reached its last turn", "FAIL  the blocked Bash call came back refused", "FAIL  the Write call came back refused", "S4 FAIL"] {
        assert!(out.contains(line), "{line}: {out}");
    }
    // A clean exit and a result event are not enough on their own.
    let (code, out) = s4_verdict("s4-short", 3, "0", true);
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("FAIL  the Write call came back refused") && out.contains("FAIL  the plan reached its last turn"), "{out}");
}

/// 16.7: the harness profile decides Message, Redirect and Stop for a core worker.
#[test]
fn controls_follow_the_harness_profile() {
    let codex = controls(Engine::Codex);
    assert_eq!((codex.message, codex.stop), (Some(NO_MESSAGE), StopHow::GroupKill));
    let j = codex.to_json();
    assert_eq!((j["message"]["enabled"].as_bool(), j["redirect"]["reason"].as_str()), (Some(false), Some("This worker can't take a message mid-run.")));
    assert_eq!(j["stop"]["note"], "Stop ends this worker at once; its uncommitted work stays in its worktree.");
    let claude = controls(Engine::Claude);
    // On: the installed control hook serves a top-level session keyed by its session id (#148/#150).
    assert_eq!((claude.message, claude.stop), (None, StopHow::GroupKill));
    let j = claude.to_json();
    assert_eq!((j["message"]["enabled"].as_bool(), j["redirect"]["enabled"].as_bool()), (Some(true), Some(true)));
    let s = steer_brief("0123456789abcdef0123456789abcdef");
    assert!(s.contains("<vajra-steer nonce=\"0123456789abcdef0123456789abcdef\""), "{s}");
    assert!(s.contains("never write the nonce"), "{s}");
}

// ------------------------------------------------------------ run leads (6d2)

const RUN: &str = "20261112-0930";
const GW_HOME: &str = "/d/codex-home/gw";

fn codex_lead_argv_of(l: &LeadLaunch<'_>, r: CodexRoute<'_>) -> Result<Vec<String>, InvalidInput> {
    codex_lead_launch(l, r).map(|c| c.argv)
}

fn server() -> RunServer<'static> {
    RunServer { exe: Path::new("/opt/Vajra/vajra-desktop"), run: RUN, token_file: Path::new("/d/runs/20261112-0930/vajra-run.token"), socket: Path::new("/d/app.sock") }
}

fn lead(model: Option<&'static str>, effort: Option<&'static str>) -> LeadLaunch<'static> {
    LeadLaunch { prompt: "the charter", model, effort, lead_dir: Path::new("/d/runs/20261112-0930/lead"), project: Path::new("/p/repo"), server: server() }
}

fn run_server_kv() -> String {
    let tools = crate::control::runreg::TOOL_NAMES.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(",");
    format!(
        "mcp_servers={{vajra_run={{command=\"/opt/Vajra/vajra-desktop\",args=[\"--mcp-run\"],env={{VAJRA_RUN=\"{RUN}\",\
         VAJRA_RUN_TOKEN_FILE=\"/d/runs/20261112-0930/vajra-run.token\",VAJRA_RUN_SOCKET=\"/d/app.sock\"}},\
         default_tools_approval_mode=\"approve\",enabled_tools=[{tools}],tool_timeout_sec=130}}}}"
    )
}

#[test]
fn codex_login_lead_argv_snapshot() {
    let a = codex_lead_argv_of(&lead(Some("model-a"), Some("high")), CodexRoute::Login).unwrap();
    let want: Vec<String> = [
        "/bin/sh", "-c", r#"exec "$@" </dev/null"#, "sh", "codex", "exec", "--json", "--skip-git-repo-check", "--sandbox", "read-only",
        "-C", "/d/runs/20261112-0930/lead", "--ignore-rules", "--ignore-user-config", "-m", "model-a",
        "-c", "approval_policy=\"never\"", "-c", "notify=[]", "-c", "web_search=\"disabled\"",
        "-c", "features.multi_agent=false", "-c", "features.goals=false", "-c", "model_reasoning_effort=\"high\"",
        "-c", &run_server_kv(), "--", "the charter",
    ].map(String::from).into();
    assert_eq!(a, want);
    // N3 on the login route holds only with the user's config ignored (spike S2): the -c table alone is merged.
    assert_eq!(a.iter().filter(|x| *x == "--ignore-user-config").count(), 1);
    assert!(!a.iter().any(|x| x.contains("model_provider") || x.contains("workspace-write") || x.contains("bypass")));
}

#[test]
fn codex_gateway_lead_argv_keeps_vajras_home_and_retries_nothing() {
    let a = codex_lead_argv_of(&lead(Some("vendor/model-a"), None), CodexRoute::Gateway { home: Path::new(GW_HOME) }).unwrap();
    let id = crate::providers::codex_home::PROVIDER_ID;
    assert!(!a.iter().any(|x| x == "--ignore-user-config"), "the gateway home is Vajra's own config");
    let kv = pairs(&a);
    assert!(kv.contains(&format!("model_providers.{id}.request_max_retries=0").as_str()));
    assert!(kv.contains(&format!("model_providers.{id}.stream_max_retries=0").as_str()));
    assert!(kv.contains(&run_server_kv().as_str()));
    assert_eq!(&a[a.len() - 2..], ["--", "the charter"]);
    assert!(a.iter().any(|x| x == "--ignore-rules"), "no execpolicy rules on either route");
    let gw = codex_lead_launch(&lead(Some("m"), None), CodexRoute::Gateway { home: Path::new(GW_HOME) }).unwrap();
    assert_eq!(gw.env, vec![("CODEX_HOME", OsString::from(GW_HOME))], "the gateway argv and its home come together");
    assert!(codex_lead_launch(&lead(Some("m"), None), CodexRoute::Login).unwrap().env.is_empty(), "the login route keeps the user's own CODEX_HOME");
    for bad in ["/home/u/.codex", "relative/codex-home/gw", "/d/codex-home/a\"x"] {
        assert_eq!(codex_lead_launch(&lead(Some("m"), None), CodexRoute::Gateway { home: Path::new(bad) }).unwrap_err().field, "home", "{bad}");
    }
}

#[test]
fn the_lead_argv_never_holds_the_token_and_the_server_env_is_only_paths() {
    let token = "a".repeat(64);
    for a in [codex_lead_argv_of(&lead(Some("m"), None), CodexRoute::Login).unwrap(), codex_lead_argv_of(&lead(Some("m"), None), CodexRoute::Gateway { home: Path::new(GW_HOME) }).unwrap()] {
        assert!(!a.iter().any(|x| x.contains(&token) || x.contains(SENTINEL)));
    }
    let cfg: Value = serde_json::from_str(&claude_lead_mcp_config(&server()).unwrap()).unwrap();
    let s = &cfg["mcpServers"]["vajra_run"];
    assert_eq!(cfg["mcpServers"].as_object().unwrap().len(), 1, "vajra_run is the only server");
    assert_eq!(s["command"], "/opt/Vajra/vajra-desktop");
    assert_eq!(s["args"], json!(["--mcp-run"]));
    assert_eq!(s["env"], json!({"VAJRA_RUN": RUN, "VAJRA_RUN_TOKEN_FILE": "/d/runs/20261112-0930/vajra-run.token", "VAJRA_RUN_SOCKET": "/d/app.sock"}));
}

#[test]
fn lead_builders_refuse_bad_inputs() {
    let mut l = lead(Some("m"), None);
    assert_eq!(codex_lead_argv_of(&lead(None, None), CodexRoute::Gateway { home: Path::new(GW_HOME) }).unwrap_err().field, "model");
    assert!(codex_lead_argv_of(&lead(None, None), CodexRoute::Login).is_ok(), "the login route may leave the model to Codex's default");
    assert_eq!(codex_lead_argv_of(&lead(Some("--yolo"), None), CodexRoute::Login).unwrap_err().field, "model");
    assert_eq!(codex_lead_argv_of(&lead(Some("m"), Some("high\",x=\"")), CodexRoute::Login).unwrap_err().field, "effort");
    l.lead_dir = Path::new("relative/lead");
    assert_eq!(codex_lead_argv_of(&l, CodexRoute::Login).unwrap_err().field, "lead_dir");
    let mut l = lead(Some("m"), None);
    l.server.run = "not a run";
    assert_eq!(codex_lead_argv_of(&l, CodexRoute::Login).unwrap_err().field, "run");
    let mut l = lead(Some("m"), None);
    l.server.token_file = Path::new("/d/x\"},y={\"");
    assert_eq!(codex_lead_argv_of(&l, CodexRoute::Login).unwrap_err().field, "token_file");
    assert_eq!(claude_lead_mcp_config(&l.server).unwrap_err().field, "token_file");
    let mut l = lead(Some("m"), None);
    l.prompt = " ";
    assert_eq!(codex_lead_argv_of(&l, CodexRoute::Login).unwrap_err().field, "prompt");
    let big = "x".repeat(MAX_LEAD_PROMPT + 1);
    l.prompt = &big;
    assert_eq!(codex_lead_argv_of(&l, CodexRoute::Login).unwrap_err().field, "prompt");
    // A full 32 KiB ask in the charter, escaped as JSON, still builds: the lead prompt has its own cap.
    let ask = "\"".repeat(32 * 1024);
    let roles = vec!["backend".to_string()];
    let p = crate::desktop::charter::charter(&crate::desktop::charter::Brief { ask: &ask, max_rounds: 3, project_root: "/p/repo", roles: &roles });
    assert!(p.len() > 64 * 1024);
    l.prompt = &p;
    assert!(codex_lead_argv_of(&l, CodexRoute::Login).is_ok());
    assert!(claude_lead_argv(&l, &claude_lead()).is_ok());
    let mut l = lead(None, None);
    l.project = Path::new("rel");
    assert_eq!(claude_lead_argv(&l, &claude_lead()).unwrap_err().field, "project");
    assert_eq!(claude_lead_argv(&lead(Some("m"), Some("high")), &claude_lead()).unwrap_err().field, "effort");
    let mut c = claude_lead();
    c.session_id = "nope";
    assert_eq!(claude_lead_argv(&lead(None, None), &c).unwrap_err().field, "session_id");
}

fn claude_lead() -> ClaudeLead<'static> {
    ClaudeLead {
        claude: Path::new("/opt/bin/claude"), session_id: "3f1d7e8a-1b2c-4d5e-8f90-a1b2c3d4e5f6",
        settings: Path::new("/d/runs/20261112-0930/lead/claude-lead.json"), mcp_config: Path::new("/d/runs/20261112-0930/lead/vajra-run.json"),
    }
}

#[test]
fn claude_lead_argv_snapshot() {
    let a = claude_lead_argv(&lead(Some("model-b"), None), &claude_lead()).unwrap();
    let tools = claude_lead_tools();
    let want: Vec<String> = [
        "/opt/bin/claude", "-p", "--output-format", "stream-json", "--verbose", "--session-id", "3f1d7e8a-1b2c-4d5e-8f90-a1b2c3d4e5f6",
        "--setting-sources", "", "--settings", "/d/runs/20261112-0930/lead/claude-lead.json", "--permission-mode", "dontAsk",
        "--tools", &tools, "--allowedTools", &tools,
        "--mcp-config", "/d/runs/20261112-0930/lead/vajra-run.json", "--strict-mcp-config", "--add-dir", "/p/repo",
        "--model", "model-b", "--", "the charter",
    ].map(String::from).into();
    assert_eq!(a, want);
    assert!(!a.iter().any(|x| x.contains("bypass")));
    assert_eq!(claude_lead_env(), vec![("MCP_TOOL_TIMEOUT", OsString::from("130000"))]);
    let gw = codex_lead_launch(&lead(Some("m"), None), CodexRoute::Gateway { home: Path::new(GW_HOME) }).unwrap().env;
    for (k, _) in claude_lead_env().into_iter().chain(gw) {
        assert!(crate::sessions::EXTRA_ENV_ALLOW.contains(&k), "{k}");
    }
}

#[test]
fn the_claude_lead_tools_are_the_three_read_tools_and_every_vajra_run_tool() {
    let t = claude_lead_tools();
    let want: Vec<String> = ["Read", "Grep", "Glob"].map(String::from).into_iter()
        .chain(crate::control::runreg::TOOL_NAMES.iter().map(|n| format!("mcp__vajra_run__{n}"))).collect();
    assert_eq!(t, want.join(","));
    for bad in ["Bash", "Edit", "Write", "Agent", "WebFetch", "mcp__vajra-desktop__"] {
        assert!(!t.split(',').any(|x| x.starts_with(bad)), "{bad}");
    }
    assert!(std::time::Duration::from_secs(LEAD_TOOL_TIMEOUT_SEC) > crate::control::runreg::MAX_CLIENT_TIMEOUT, "the client answers before the harness gives up");
}

#[test]
fn the_claude_lead_flag_allows_exactly_the_lead_tools_with_the_guard_and_deny_rules() {
    let home = tmpdir("lead-home");
    let repo = tmpdir("lead-repo");
    let data = tmpdir("lead-data");
    let v: Value = serde_json::from_str(&claude_lead_flag(&home, std::slice::from_ref(&repo), &data, &[]).unwrap()).unwrap();
    let allow: Vec<&str> = v["permissions"]["allow"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert_eq!(allow.join(","), claude_lead_tools());
    assert_eq!(v["permissions"]["defaultMode"], "dontAsk");
    assert_eq!(v["permissions"]["disableBypassPermissionsMode"], "disable");
    let deny = v["permissions"]["deny"].as_array().unwrap();
    assert!(deny.iter().any(|r| r.as_str().unwrap().contains(data.to_str().unwrap())), "the data dir is denied");
    assert!(v["hooks"]["PreToolUse"].is_array(), "the guard runs");
    for d in [home, repo, data] {
        let _ = std::fs::remove_dir_all(d);
    }
}

fn lead_rollout(sandbox: Value, writes: &[&str]) -> String {
    rollout(sandbox, writes, "never")
}

#[test]
fn the_codex_lead_read_back_wants_read_only_no_network_and_the_lead_dir() {
    let wt = Path::new(WT);
    assert!(check_codex_lead_applied(&lead_rollout(json!({"type": "read-only"}), &[]), wt).is_ok());
    let ws = json!({"type": "workspace-write", "network_access": false, "exclude_slash_tmp": true, "exclude_tmpdir_env_var": true, "writable_roots": []});
    assert!(check_codex_lead_applied(&lead_rollout(ws, &[]), wt).is_err(), "workspace-write is wider than a lead's profile");
    assert!(check_codex_lead_applied(&lead_rollout(json!({"type": "read-only", "network_access": true}), &[]), wt).is_err());
    assert!(check_codex_lead_applied(&lead_rollout(json!({"type": "read-only"}), &[WT]), wt).is_err(), "no write entry at all");
    assert!(check_codex_lead_applied(&lead_rollout(json!({"type": "read-only"}), &[]), Path::new("/elsewhere")).is_err());
    assert!(check_codex_lead_applied(&rollout(json!({"type": "read-only"}), &[], "on-request"), wt).is_err());
    assert!(check_codex_lead_applied("{}\n", wt).is_err());
}

fn lead_init(tools: &[String], servers: &[&str], mode: &str) -> String {
    lead_init_status(tools, servers, mode, "connected")
}

fn lead_init_status(tools: &[String], servers: &[&str], mode: &str, status: &str) -> String {
    let servers: Vec<Value> = servers.iter().map(|s| json!({"name": s, "status": status})).collect();
    format!("{}\n", json!({"type": "system", "subtype": "init", "permissionMode": mode, "tools": tools, "mcp_servers": servers}))
}

#[test]
fn the_claude_lead_read_back_wants_only_the_lead_tools_and_only_vajra_run() {
    let ok: Vec<String> = claude_lead_tools().split(',').map(String::from).collect();
    assert!(check_claude_lead_applied(&lead_init(&ok, &["vajra_run"], "dontAsk")).is_ok());
    let mut more = ok.clone();
    more.push("mcp__user_mcp__probe".into());
    assert!(check_claude_lead_applied(&lead_init(&more, &["vajra_run"], "dontAsk")).is_err(), "N3: no other server's tool");
    let mut bash = ok.clone();
    bash.push("Bash".into());
    assert!(check_claude_lead_applied(&lead_init(&bash, &["vajra_run"], "dontAsk")).is_err());
    assert!(check_claude_lead_applied(&lead_init(&ok, &["vajra_run", "user_mcp"], "dontAsk")).is_err());
    assert!(check_claude_lead_applied(&lead_init(&ok, &[], "dontAsk")).is_err(), "the vajra_run server must be up");
    assert!(check_claude_lead_applied(&lead_init(&ok, &["vajra_run"], "bypassPermissions")).is_err());
    assert!(check_claude_lead_applied("").is_err());
    assert!(check_claude_lead_applied(&lead_init_status(&ok, &["vajra_run"], "dontAsk", "failed")).is_err(), "the server must be connected");
    let no_run: Vec<String> = ["Read", "Grep", "Glob"].map(String::from).into();
    let e = check_claude_lead_applied(&lead_init(&no_run, &["vajra_run"], "dontAsk")).unwrap_err();
    assert!(e.0.contains("mcp__vajra_run__plan_tasks"), "a lead without the run tools can only exit: {e:?}");
}
