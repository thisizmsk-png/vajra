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
        "--setting-sources", "", "--settings", "/d/profiles/claude-worker.json", "--permission-mode", "dontAsk",
        "--tools", "Read,Grep,Glob,Edit,Write,Bash", "--allowedTools", "Read,Grep,Glob,Edit,Write,Bash",
        "--model", "vendor/model-a", "--", "Do the task",
    ].map(String::from).into();
    assert_eq!(a, want);
    assert!(!a.iter().any(|x| x.contains("bypass") || x == "--strict-mcp-config"));
    // No settings file loads but the flag file: not the worktree's project or local settings (#147 review), and not the
    // user's, whose merged sandbox lists would reopen the key folder and the network (vajra-70 spike).
    assert_eq!(a.iter().filter(|x| *x == "--setting-sources").count(), 1);
    assert_eq!(a[a.iter().position(|x| x == "--setting-sources").unwrap() + 1], "");
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
    let s = claude_worker_flag(&home, &work, std::slice::from_ref(&repo), &data, &d.join("tmp")).unwrap();
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

/// The OS sandbox block (vajra-70 spike): on, fail closed, no escape hatch; writes only in the ticket tmp beyond the
/// worktree; the key folder and the data dir refused to read and write in both spellings; loopback only.
#[test]
fn the_flag_file_turns_the_os_sandbox_on_around_the_key_folder_and_data_dir() {
    let d = tmpdir("sandbox");
    let (home, work, repo, data, tmp) = (d.join("home"), d.join("work"), d.join("repo"), d.join("data"), d.join("tmp"));
    for p in [&home, &work, &repo, &data, &tmp] {
        std::fs::create_dir_all(p).unwrap();
    }
    let v: Value = serde_json::from_str(&claude_worker_flag(&home, &work, std::slice::from_ref(&repo), &data, &tmp).unwrap()).unwrap();
    let sb = &v["sandbox"];
    assert_eq!(sb["enabled"], true);
    assert_eq!(sb["failIfUnavailable"], true, "no sandbox, no launch");
    assert_eq!(sb["allowUnsandboxedCommands"], false, "dangerouslyDisableSandbox is ignored");
    assert_eq!(sb["autoAllowBashIfSandboxed"], false);
    assert_eq!(sb["excludedCommands"], json!([]));
    assert_eq!(sb["ignoreViolations"], json!({}));
    assert_eq!(sb["network"], json!({"allowLocalBinding": true}), "no allowedDomains: loopback only");
    let fs = &sb["filesystem"];
    assert_eq!(fs["disabled"], false);
    assert_eq!(fs["allowRead"], json!([]));
    assert_eq!(fs["allowWrite"], json!([tmp.to_str().unwrap()]));
    let locked = crate::control::lead_settings::sandbox_locked_dirs(&home, &data).unwrap();
    assert_eq!(fs["denyRead"], json!(locked));
    assert_eq!(fs["denyWrite"], json!(locked));
    for base in [home.join(".claude/vajra"), home.canonicalize().unwrap().join(".claude/vajra"), data.clone(), data.canonicalize().unwrap()] {
        assert!(locked.contains(&base.to_str().unwrap().to_string()), "{} missing from {locked:?}", base.display());
    }
    assert_eq!(crate::control::lead_settings::KEY_DIR, crate::control::lead_settings::HOME_RULES[0].trim_end_matches("/**"));
    // A ticket tmp that is relative, or inside or around a locked folder, refuses the flag.
    for bad in [PathBuf::from("rel/tmp"), data.join("t"), home.join(".claude/vajra/t"), home.clone(), d.clone()] {
        assert!(claude_worker_flag(&home, &work, std::slice::from_ref(&repo), &data, &bad).is_err(), "{}", bad.display());
    }
    // #212 bhishma Low: a ticket tmp that is a link into a locked folder, or passes through one, is caught by realpath.
    std::fs::create_dir_all(data.join("inner")).unwrap();
    std::os::unix::fs::symlink(&data, d.join("tmp-link")).unwrap();
    std::fs::create_dir_all(d.join("via")).unwrap();
    std::os::unix::fs::symlink(&data, d.join("via/up")).unwrap();
    for bad in [d.join("tmp-link"), d.join("via/up/inner")] {
        assert!(claude_worker_flag(&home, &work, std::slice::from_ref(&repo), &data, &bad).is_err(), "{}", bad.display());
    }
    assert!(claude_worker_flag(&home, &work, std::slice::from_ref(&repo), &data, &tmp).is_ok(), "a plain ticket tmp still works");
}

/// A Claude worker's build caches live in its ticket tmp: the sandbox leaves HOME read-only and rewrites TMPDIR.
#[test]
fn a_claude_worker_keeps_its_build_caches_in_the_ticket_tmp() {
    let l = launch(RoleKind::Worker, false);
    let env = worker_env(Engine::Claude, &l, None);
    for (k, v) in &env {
        assert!(crate::sessions::EXTRA_ENV_ALLOW.contains(k), "{k}");
        assert!(Path::new(v).starts_with(l.ticket_tmp), "{k}={v:?}");
    }
    let names: Vec<&str> = env.iter().map(|(k, _)| *k).collect();
    assert_eq!(names, ["TMPDIR", "GOCACHE", "GOTMPDIR", "npm_config_cache"]);
    assert!(!worker_env(Engine::Codex, &l, None).iter().any(|(k, _)| *k == "GOCACHE"), "Codex is unchanged");
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
    let s = claude_worker_flag(&home, &work, std::slice::from_ref(&repo), &data, &d.join("tmp")).unwrap();
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
    let flag = claude_worker_flag(&home, &work, &[repo], &data, &d.join("tmp")).unwrap();
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
    // #212 review: a Claude worker's sandbox is loopback only, so network on never says "Internet on".
    use crate::desktop::seat_start::{CLAUDE_NOTE, CLAUDE_NO_NETWORK_NOTE, NETWORK_NOTE};
    for kind in [RoleKind::Worker, RoleKind::Verifier] {
        assert_eq!(disclosures(&profile(Engine::Claude, kind), true), vec![CLAUDE_NOTE, CLAUDE_NO_NETWORK_NOTE]);
    }
    assert_eq!(disclosures(&p, true)[1], NETWORK_NOTE, "Codex keeps its internet note");
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

/// Pins today's state: the owner's S4 run on batch-3 printed PASS, so both Claude worker kinds are supported. Setting
/// `CLAUDE_WORKER_S4_PASSED` back to false (a new Claude Code build that fails S4) must change this test with it.
/// #180 bhishma: today's worker flag shape must be one S4 proved. Any change to the deny-rule tables, the hooks or the
/// flag's fields changes the shape and fails this test until S4 passes on it and its pair is added to
/// `CLAUDE_WORKER_PROVEN` (the spike prints the pair). Only exact `claude --version` strings in that list pass.
#[test]
fn only_a_claude_build_s4_proved_with_todays_flag_shape_is_dispatched() {
    let shape = claude_worker_flag_shape(&repo_hooks()).unwrap();
    assert_eq!(shape.len(), 64);
    assert!(CLAUDE_WORKER_PROVEN.iter().any(|(_, s)| *s == shape), "today's flag shape {shape} is not one S4 proved: rerun S4 and add its pair");
    assert_eq!(claude_worker_flag_shape(&repo_hooks()).unwrap(), shape, "the shape is the same on every call");
    assert_eq!(claude_worker_proven("2.1.296 (Claude Code)", &shape), Ok(()));
    assert_eq!(claude_worker_proven("  2.1.296 (Claude Code)\n", &shape), Ok(()), "--version output is trimmed");
    for bad in ["2.1.322 (Claude Code)", "2.1.296", "2.1.2960 (Claude Code)", "", "SHAPE_PENDING"] {
        assert_eq!(claude_worker_proven(bad, &shape), Err(CLAUDE_UNPROVEN), "{bad:?}");
    }
    assert_eq!(claude_worker_proven("2.1.296 (Claude Code)", &"0".repeat(64)), Err(CLAUDE_UNPROVEN), "a proven version with another shape");
    assert_eq!(CLAUDE_UNPROVEN.reason, "claude_unproven");
}

fn repo_hooks() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hooks")
}

/// #186 codex: the flag file names the guard scripts, but S4 proved what they do, so their bytes are in the shape.
/// One changed byte in the installed pre-tool-use.sh is a different shape; a missing or symlinked script is no shape.
#[test]
fn the_flag_shape_covers_the_installed_guard_scripts_bytes() {
    let names = claude_worker_hook_scripts().unwrap();
    assert!(names.iter().any(|n| n == "pre-tool-use.sh"), "{names:?}");
    let d = tmpdir("guard-shape");
    let copy = |to: &Path| {
        std::fs::create_dir_all(to).unwrap();
        for n in &names {
            std::fs::copy(repo_hooks().join(n), to.join(n)).unwrap();
        }
    };
    let same = d.join("same");
    copy(&same);
    let want = claude_worker_flag_shape(&repo_hooks()).unwrap();
    assert_eq!(claude_worker_flag_shape(&same).unwrap(), want, "an exact install of the repo's hooks has the repo's shape");

    let edited = d.join("edited");
    copy(&edited);
    let p = edited.join("pre-tool-use.sh");
    let mut b = std::fs::read(&p).unwrap();
    let i = b.len() / 2;
    b[i] ^= 1;
    std::fs::write(&p, b).unwrap();
    assert_ne!(claude_worker_flag_shape(&edited).unwrap(), want, "one changed byte in the guard");

    let missing = d.join("missing");
    copy(&missing);
    std::fs::remove_file(missing.join("pre-tool-use.sh")).unwrap();
    assert!(claude_worker_flag_shape(&missing).is_err());

    let linked = d.join("linked");
    copy(&linked);
    std::fs::remove_file(linked.join("pre-tool-use.sh")).unwrap();
    std::os::unix::fs::symlink(repo_hooks().join("pre-tool-use.sh"), linked.join("pre-tool-use.sh")).unwrap();
    assert!(claude_worker_flag_shape(&linked).is_err(), "a symlinked guard script is not hashed through");
}

/// #186 bhishma Low: the shape covers the worker argv S4 relied on, not only the flag file, so dropping or changing
/// `--setting-sources ""`, `--permission-mode dontAsk` or the `--tools` pair needs a new S4 PASS too.
#[test]
fn the_flag_shape_covers_the_worker_argv_flags_s4_relied_on() {
    let input = claude_worker_shape_input().unwrap();
    let argv: Vec<String> = serde_json::from_str(input.rsplit_once('\n').unwrap().1).unwrap();
    for pair in [["--setting-sources", ""], ["--permission-mode", "dontAsk"], ["--tools", "Read,Grep,Glob,Bash,Edit,Write"], ["--allowedTools", "Read,Grep,Glob,Bash,Edit,Write"], ["--output-format", "stream-json"]] {
        assert!(argv.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]), "{pair:?} in {argv:?}");
    }
    assert!(argv.iter().any(|a| a == "-p") && argv.iter().any(|a| a == "--settings"), "{argv:?}");
    let flag: serde_json::Value = serde_json::from_str(input.rsplit_once('\n').unwrap().0).unwrap();
    assert_eq!(flag["permissions"]["defaultMode"], "dontAsk", "the flag file is the first part");
    assert!(claude_worker_flag_shape(&repo_hooks()).is_ok());
}

#[test]
fn the_claude_worker_profile_is_supported_once_s4_passed() {
    const { assert!(CLAUDE_WORKER_S4_PASSED) };
    for kind in [RoleKind::Worker, RoleKind::Verifier] {
        let p = profile(Engine::Claude, kind);
        assert_eq!(p.supported, Ok(()));
        assert!(!p.always.is_empty(), "every override the profile applies is listed");
    }
}

#[test]
fn the_s4_owner_script_runs_the_argv_the_builder_makes() {
    let script = include_str!("../../../bench/spikes/s4-claude-settings.sh");
    let a = claude_worker_argv(&launch(RoleKind::Verifier, false), &claude(CHECKER_TOOLS)).unwrap();
    for flag in ["-p", "--output-format", "stream-json", "--verbose", "--session-id", "--settings", "--permission-mode", "dontAsk", "--tools", "--allowedTools", "--setting-sources"] {
        assert!(a.iter().any(|x| x == flag) && script.contains(&format!(" {flag} ")), "{flag}");
    }
    assert!(a.windows(2).any(|w| w[0] == "--setting-sources" && w[1].is_empty()) && script.contains(" --setting-sources \"\" "));
    assert!(!script.contains("--setting-sources user"), "both S4 runs use the worker's setting sources");
    // The data-dir run proves the OS sandbox: name-free probes refused with the sandbox's EPERM.
    assert!(S4_CHECK.contains("SANDBOX_REFUSAL = \"operation not permitted\""));
    assert!(S4_CHECK.contains("the OS sandbox refused a key read that names no locked path"));
    assert!(script.contains("\"$KD\" \"$TT2\""));
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
    s4_verdict_with(tag, served, status, result, DataRun::Refused)
}

/// How the data-dir run's Edit (tu_4) ended.
#[derive(Clone, Copy, PartialEq)]
enum DataRun {
    /// Refused by the deny rules, as every data-dir call was.
    Refused,
    /// It went through and changed the seed file.
    Leaked,
    /// It errored for another reason (Claude's "read it first"), so nothing proves the deny rule refused it.
    OtherError,
    /// Every deny rule and the guard held, but no OS sandbox ran: the name-free key read returned the stand-in key and
    /// the name-free data-dir write went through.
    Unsandboxed,
}

/// `s4_verdict`, plus the data-dir run: a full writer run whose Bash call the guard refused and whose Write and Edit the
/// deny rules refused, with the real refusal texts; `edit` changes how the Edit ended.
fn s4_verdict_with(tag: &str, served: usize, status: &str, result: bool, edit: DataRun) -> (Option<i32>, String) {
    let leak = edit == DataRun::Leaked;
    let open = edit == DataRun::Unsandboxed;
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
    // Each call the stub's step k asked for reaches the guard before step k + 1 is requested, recorded as its whole
    // hook input.
    let guard: String = ["echo allowed > m/allowed-ran", "echo x > m/forbidden-ran"].iter().take(served.saturating_sub(1)).map(|c| s4_hook_input(c) + "\n").collect();
    if served >= 2 {
        std::fs::write(m.join("guard"), guard).unwrap();
        std::fs::write(m.join("allowed-ran"), "allowed\n").unwrap();
    }
    // The data-dir run: tu_0 (a worktree Write) succeeds; tu_1..tu_4 (Bash, Write, Read, Edit into the data dir) are
    // refused, or with `leak` the Edit (tu_4) went through and changed the seed.
    // tu_5 and tu_6 are the name-free key read and data-dir write (the sandbox's EPERM, or with `open` the key's text and a
    // write); tu_7 the Bash writes into the ticket tmp and worktree.
    let (d, wt, data, kd, tt) = (sp.join("data-run"), sp.join("wt2"), sp.join("data"), sp.join("kd"), sp.join("tt2"));
    for dir in [&d, &wt, &data, &kd, &tt] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(wt.join("ok-written"), "x").unwrap();
    std::fs::write(wt.join("bash-ok"), "x").unwrap();
    std::fs::write(tt.join("tmp-ok"), "x").unwrap();
    std::fs::write(kd.join("s4-key"), "s4-stand-in\n").unwrap();
    std::fs::write(data.join("seed.json"), if leak { "edited\n" } else { "seed\n" }).unwrap();
    if open {
        std::fs::write(data.join("sandbox-ran"), "x").unwrap();
    }
    let dev = [json!({"type": "system", "subtype": "init", "tools": ["Read", "Grep", "Glob", "Bash", "Edit", "Write"], "permissionMode": "dontAsk"}), json!({"type": "result", "subtype": "success", "is_error": false})];
    std::fs::write(d.join("events.jsonl"), dev.iter().map(|e| e.to_string() + "\n").collect::<String>()).unwrap();
    let guard_text = "PreToolUse:Bash hook error: [bash \"pre-tool-use.sh\"]: Vajra guard blocked this Bash call: redirect into protected path";
    let deny_text = "<tool_use_error>File is in a directory that is denied by your permission settings.</tool_use_error>";
    let edit_text = match edit {
        DataRun::Refused | DataRun::Unsandboxed => deny_text,
        DataRun::Leaked => "The file has been updated.",
        DataRun::OtherError => "<tool_use_error>File has not been read yet. Read it first before writing to it.</tool_use_error>",
    };
    // The texts Claude Code 2.1.296 returned in the vajra-70 spike.
    let (key_text, data_text) = if open {
        ("s4-stand-in", "")
    } else {
        ("Traceback (most recent call last):\n  File \"<string>\", line 1, in <module>\nPermissionError: [Errno 1] Operation not permitted: '/h/.claude/vajra/s4-key'",
         "PermissionError: [Errno 1] Operation not permitted: '/h/Library/Application Support/VajraDesktop/sandbox-ran'")
    };
    let texts = ["", guard_text, deny_text, deny_text, edit_text, key_text, data_text, ""];
    let dres: Vec<_> = (0..8).map(|i| json!({"id": format!("tu_{i}"), "is_error": (1..=4).contains(&i) && !(leak && i == 4), "text": texts[i]})).collect();
    let dlog: String = (0..9).map(|k| json!({"path": "/v1/messages", "tools": ["Bash"], "k": k, "results": dres[..k], "body": "Run the S4 plan"}).to_string() + "\n").collect();
    std::fs::write(d.join("log"), dlog).unwrap();
    let o = std::process::Command::new("/usr/bin/python3").arg("-I").arg("-c").arg(S4_CHECK.replace("if __name__ == \"__main__\":\n    main()", "main()"))
        .args([sp.to_str().unwrap(), m.to_str().unwrap(), "Read,Grep,Glob,Bash", status])
        .args([d.to_str().unwrap(), wt.to_str().unwrap(), data.to_str().unwrap(), "Read,Grep,Glob,Bash,Edit,Write", "0"])
        .args([kd.to_str().unwrap(), tt.to_str().unwrap()]).output().unwrap();
    (o.status.code(), String::from_utf8_lossy(&o.stdout).into_owned())
}

/// A PreToolUse hook input for a Bash call, shaped as Claude sends it: session and transcript first, tool_input after.
/// The transcript path runs far past 300 characters, as a deep macOS temp dir makes it.
fn s4_hook_input(command: &str) -> String {
    let transcript = format!("/private/var/folders/cc/{}/T/vajra-s4.X/home/.claude/projects/-deep/3f1d7e8a.jsonl", "d".repeat(300));
    // Written by hand: serde_json's map sorts keys, which would move transcript_path behind tool_input.
    let s = |v: &str| serde_json::Value::from(v).to_string();
    format!("{{\"session_id\":\"3f1d7e8a\",\"transcript_path\":{},\"cwd\":\"/w\",\"hook_event_name\":\"PreToolUse\",\"tool_name\":\"Bash\",\"tool_input\":{{\"command\":{},\"description\":\"probe\"}}}}", s(&transcript), s(command))
}

/// S4 rerun on batch-3: the guard stand-in cut each record to 300 characters, the long transcript path filled them, and
/// the check never saw the blocked call. Run the stand-in as the script writes it on a hook input whose tool_input sits
/// past character 300: it must block the forbidden call, record both calls whole, and the check must find the blocked one.
#[test]
fn the_s4_guard_stand_in_records_a_call_behind_a_long_transcript_path() {
    use std::io::Write;
    let script = include_str!("../../../bench/spikes/s4-claude-settings.sh");
    let open = "cat > \"$SP/guard.sh\" <<EOF\n";
    let start = script.find(open).expect("the guard stand-in heredoc") + open.len();
    let body = &script[start..start + script[start..].find("\nEOF\n").expect("the heredoc's end") + 1];
    assert!(!body.contains("cut "), "the stand-in records the whole hook input: {body}");
    let sp = tmpdir("s4-guard-stand-in");
    let m = sp.join("m");
    std::fs::create_dir_all(&m).unwrap();
    // The unquoted heredoc expands $M and turns \$ into $; nothing else in the body is expanded.
    let guard_sh = sp.join("guard.sh");
    std::fs::write(&guard_sh, body.replace("\"$M/guard\"", &format!("\"{}/guard\"", m.display())).replace("\\$", "$")).unwrap();
    for (cmd, code) in [("echo allowed > m/allowed-ran", 0), ("echo x > m/forbidden-ran", 2)] {
        let input = s4_hook_input(cmd);
        assert!(input.find("tool_input").unwrap() > 300, "the probe puts tool_input past the old cut");
        let mut child = std::process::Command::new("/bin/sh").arg(&guard_sh).stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(code), "{cmd}: {}", String::from_utf8_lossy(&o.stderr));
    }
    let recorded = std::fs::read_to_string(m.join("guard")).unwrap();
    assert_eq!(recorded.lines().count(), 2, "{recorded}");
    for line in recorded.lines() {
        assert_eq!(serde_json::from_str::<serde_json::Value>(line).ok(), serde_json::from_str(&s4_hook_input(if line.contains("forbidden") { "echo x > m/forbidden-ran" } else { "echo allowed > m/allowed-ran" })).ok(), "a whole record: {line}");
    }
    // The check parses that record and finds the blocked call in tool_input.command, and only there.
    let probe = "import sys\ncalls = hook_inputs(sys.argv[1])\nsys.exit(0 if saw_bash(calls, \"/forbidden-ran\") and not saw_bash(calls, \"3f1d7e8a\") else 1)";
    let o = std::process::Command::new("/usr/bin/python3").arg("-I").arg("-c").arg(S4_CHECK.replace("if __name__ == \"__main__\":\n    main()", probe))
        .arg(m.join("guard")).output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
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
    // The owner's condition for the flip: an Edit that got into the data dir fails S4, though every other probe held.
    let (code, out) = s4_verdict_with("s4-leak", 4, "0", true, DataRun::Leaked);
    assert_eq!(code, Some(1), "{out}");
    for line in ["FAIL  data-dir run: the deny rules refused an Edit in the data dir", "FAIL  data-dir run: the data dir holds only its seed file, unchanged", "S4 FAIL"] {
        assert!(out.contains(line), "{line}: {out}");
    }
    assert!(!out.contains("FAIL  the "), "only the data-dir run failed: {out}");
    // #163 review: an Edit that errored for another reason is not proof the deny rule refused it, though the seed held.
    let (code, out) = s4_verdict_with("s4-other-error", 4, "0", true, DataRun::OtherError);
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("FAIL  data-dir run: the deny rules refused an Edit in the data dir") && out.contains("S4 FAIL"), "{out}");
    assert!(out.contains("ok    data-dir run: the data dir holds only its seed file, unchanged"), "{out}");
    assert!(out.contains("ok    data-dir run: the Vajra guard refused a Bash write into the data dir"), "{out}");
    // Key A: the guard and the deny rules held, but with no OS sandbox a command that names no locked path read the key
    // and wrote into the data dir. That fails S4.
    let (code, out) = s4_verdict_with("s4-unsandboxed", 4, "0", true, DataRun::Unsandboxed);
    assert_eq!(code, Some(1), "{out}");
    for line in [
        "FAIL  data-dir run: the OS sandbox refused a key read that names no locked path",
        "FAIL  data-dir run: the OS sandbox refused a data-dir write that names no locked path",
        "FAIL  data-dir run: the stand-in key reached no tool result and is unchanged",
        "FAIL  data-dir run: the data dir holds only its seed file, unchanged",
        "ok    data-dir run: the Vajra guard refused a Bash write into the data dir",
        "ok    data-dir run: Bash wrote in the ticket tmp and the worktree",
        "S4 FAIL",
    ] {
        assert!(out.contains(line), "{line}: {out}");
    }
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
