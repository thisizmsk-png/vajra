use super::*;
use std::path::Path;

const TMP: &str = "/w/proj/run/tmp/t1-r1";
const WT: &str = "/w/proj/run/t1-r1";
/// A value that would be a key if one leaked. Never a real key.
const SENTINEL: &str = "sk-vajra-sentinel-0000";

/// The Vajra dirs a Codex session may not read (6k), as `vajra_deny_dirs` would give them.
const DENY: [&str; 2] = ["/h/.claude/vajra", "/h/data"];

fn deny() -> &'static [PathBuf] {
    static D: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    D.get_or_init(|| DENY.iter().map(PathBuf::from).collect())
}

/// A launch's permission profile name, as `codex_profile_name` would make it.
const PROFILE: &str = "vajra-0123456789abcdef";

fn launch(kind: RoleKind, network: bool) -> WorkerLaunch<'static> {
    WorkerLaunch {
        kind, prompt: "Do the task", model: Some("vendor/model-a"), effort: Some("high"), network, ticket_tmp: Path::new(TMP), worktree: Path::new(WT),
        deny: deny(), profile: PROFILE,
    }
}

/// The three profile values every Codex worker gets (S11).
fn profile_kv(network: bool) -> [String; 3] {
    [
        format!("permissions.{PROFILE}.filesystem={{\"/\"=\"read\",\"{}\"=\"none\",\"{}\"=\"none\",\"{WT}\"=\"write\",\"{TMP}\"=\"write\"}}", DENY[0], DENY[1]),
        format!("permissions.{PROFILE}.network={{enabled={network}}}"),
        format!("default_permissions=\"{PROFILE}\""),
    ]
}

/// bhishma #216 H2: what in a Codex config runs outside the sandbox, named; anything unreadable fails closed.
#[test]
fn a_codex_config_with_an_mcp_server_or_notify_is_named_and_a_clean_one_is_not() {
    let named = codex_config_outside;
    assert_eq!(named("model = \"x\"\n[tui]\nnotifications = true\n"), None);
    assert_eq!(named("[mcp_servers.files]\ncommand = \"npx\"\nenabled = false\n"), None, "a switched-off server doesn't run");
    assert!(named("[mcp_servers.files]\ncommand = \"npx\"\n").unwrap().contains("MCP server \"files\""));
    assert!(named("[profiles.work.mcp_servers.fs]\ncommand = \"x\"\n").unwrap().contains("\"fs\""), "inside a profile too");
    assert!(named("[mcp_servers.\"we ird\"]\ncommand = \"x\"\n").unwrap().contains("an MCP server"), "an odd name isn't echoed");
    assert!(named("notify = [\"/bin/sh\", \"-c\", \"x\"]\n").unwrap().contains("notify command"));
    assert_eq!(named("notify = []\n"), None);
    assert!(named("[[[ not toml").unwrap().contains("couldn't read"), "an unparsable config fails closed");

    let d = tmpdir("tools");
    let (none, mcp, bad) = (d.join("none.toml"), d.join("mcp.toml"), d.join("bad.toml"));
    std::fs::write(&mcp, "[mcp_servers.files]\ncommand = \"npx\"\n").unwrap();
    std::fs::write(&bad, "[[[").unwrap();
    let only = std::slice::from_ref(&none);
    let no = FakeCodex { found: false, answer: None, asked: Default::default() };
    assert_eq!(codex_tools_outside(&no, only, false), None, "a missing file loads nothing");
    assert!(codex_tools_outside(&no, &[none.clone(), mcp], false).unwrap().contains("\"files\""));
    assert!(codex_tools_outside(&no, &[bad], false).unwrap().contains("couldn't read"));
    // The login route also asks codex itself, which counts plugin servers too.
    let listed = FakeCodex { found: true, answer: out(0, r#"[{"name":"plug","enabled":true}]"#, ""), asked: Default::default() };
    assert!(codex_tools_outside(&listed, only, true).unwrap().contains("\"plug\""));
    assert_eq!(*listed.asked.lock().unwrap(), ["mcp", "list", "--json"]);
    let off = FakeCodex { found: true, answer: out(0, r#"[{"name":"plug","enabled":false}]"#, ""), asked: Default::default() };
    assert_eq!(codex_tools_outside(&off, only, true), None);
    let broken = FakeCodex { found: true, answer: out(1, "", "boom"), asked: Default::default() };
    assert!(codex_tools_outside(&broken, only, true).unwrap().contains("couldn't read"), "a failed list fails closed");
    assert!(codex_tools_outside(&no, only, true).unwrap().contains("couldn't read"), "no codex to ask fails closed");
}

/// bhishma #216 M1: a worktree's own `.codex` config is judged at each launch.
#[test]
fn a_worktree_codex_config_that_would_load_tools_outside_the_sandbox_is_found() {
    let wt = tmpdir("wtcodex");
    assert_eq!(codex_project_outside(&wt), None, "no .codex");
    std::fs::create_dir_all(wt.join(".codex")).unwrap();
    std::fs::write(wt.join(".codex/config.toml"), "model = \"x\"\n").unwrap();
    assert_eq!(codex_project_outside(&wt), None, "a committed config with no tools is fine");
    std::fs::write(wt.join(".codex/config.toml"), "notify = [\"/bin/sh\"]\n").unwrap();
    assert!(codex_project_outside(&wt).is_some());
    let wt2 = tmpdir("wtcodex2");
    std::os::unix::fs::symlink(&wt, wt2.join(".codex")).unwrap();
    assert!(codex_project_outside(&wt2).is_some(), "a symlinked .codex counts as one");
}

/// bhishma #216 H1: the plain Codex run Lead gets the same profile as a worker, with the project and temp dir writable.
#[test]
fn the_codex_run_lead_runs_under_the_profile_with_the_vajra_dirs_denied() {
    let a = codex_run_lead_argv("exec this --flag", PROFILE, deny(), Path::new(WT), Path::new(TMP)).unwrap();
    let mut want: Vec<String> = vec!["codex".into()];
    for o in codex_profile_overrides(PROFILE, deny(), &[Path::new(WT), Path::new(TMP)], true).unwrap() {
        want.extend(["-c".into(), o]);
    }
    want.extend(["--".into(), "exec this --flag".into()]);
    assert_eq!(a, want);
    assert!(a.iter().any(|x| x.contains(&format!("\"{}\"=\"none\"", DENY[0]))), "{a:?}");
    assert!(!a.iter().any(|x| x == "--sandbox" || x.contains("approval_policy")), "approvals stay the user's: {a:?}");
    assert!(codex_run_lead_argv("x", PROFILE, &[], Path::new(WT), Path::new(TMP)).is_err(), "never without the deny set");
    assert!(codex_run_lead_argv("x", "vajra", deny(), Path::new(WT), Path::new(TMP)).is_err(), "never under a guessable name");
}

/// S11 hostile-config check: a user table named like ours merges into it, so each launch gets a name nobody can guess,
/// and a fixed or malformed name never goes out.
#[test]
fn each_launch_gets_a_fresh_profile_name_and_a_guessable_one_is_refused() {
    let (a, b) = (codex_profile_name().unwrap(), codex_profile_name().unwrap());
    assert_ne!(a, b);
    for n in [&a, &b] {
        assert!(valid_profile_name(n), "{n}");
        assert!(codex_profile_overrides(n, deny(), &[], false).is_ok());
    }
    for n in ["vajra", "default", "vajra-0123", "vajra-0123456789ABCDEF", "vajra-0123456789abcdeg", "x-0123456789abcdef", "vajra-0123456789abcdef0"] {
        assert_eq!(codex_profile_overrides(n, deny(), &[], false).unwrap_err().field, "profile", "{n}");
    }
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
    let [fs, net, dp] = profile_kv(false);
    let want: Vec<String> = [
        "/bin/sh", "-c", r#"exec "$@" </dev/null"#, "sh", "codex", "exec", "--json", "-m", "vendor/model-a",
        "-c", "model_reasoning_effort=\"high\"",
        "-c", &format!("model_providers.{id}.request_max_retries=0"), "-c", &format!("model_providers.{id}.stream_max_retries=0"),
        "-c", "approval_policy=\"never\"", "-c", &fs, "-c", &net, "-c", &dp,
        "-c", "web_search=\"disabled\"", "--", "Do the task",
    ].map(String::from).into();
    assert_eq!(a, want);
}

#[test]
fn codex_login_worker_argv_snapshot_has_no_builtin_provider_override() {
    let a = codex_login_worker_argv(&launch(RoleKind::Verifier, false)).unwrap();
    let [fs, net, dp] = profile_kv(false);
    let want: Vec<String> = [
        "/bin/sh", "-c", r#"exec "$@" </dev/null"#, "sh", "codex", "exec", "--json", "-m", "vendor/model-a",
        "-c", "model_reasoning_effort=\"high\"",
        "-c", "approval_policy=\"never\"", "-c", &fs, "-c", &net, "-c", &dp,
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
    assert!(c.contains(&"permissions.vajra-0123456789abcdef.network={enabled=true}"));
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
        let [fs, net, dp] = profile_kv(false);
        for want in ["approval_policy=\"never\"", fs.as_str(), net.as_str(), dp.as_str(), "web_search=\"disabled\""] {
            assert!(c.contains(&want), "{want} missing from {c:?}");
        }
        // S11: `--sandbox` would switch the profile off, and the legacy keys are ignored under one.
        assert!(!a.iter().any(|x| x == "--sandbox"), "{a:?}");
        assert!(!c.iter().any(|x| x.starts_with("sandbox_workspace_write.")), "{c:?}");
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
    // 6k: the profile's deny entries, as codex 0.166 reports them in S11 (`none` comes back as `deny`).
    let entries: Vec<Value> = std::iter::once(json!({"path": {"type": "special", "value": {"kind": "root"}}, "access": "read"}))
        .chain(DENY.iter().map(|d| json!({"path": {"type": "path", "path": d}, "access": "deny"})))
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
    CodexWant { cwd: WT.into(), ticket_tmp: TMP.into(), network: false, deny: deny().to_vec() }
}

/// 6k: a session where a Vajra dir is readable never runs: no entry, a `read` entry or a `write` entry all fail.
#[test]
fn the_read_back_refuses_a_session_that_can_read_a_vajra_dir() {
    let good = rollout(good_sandbox(), &[WT, TMP], "never");
    assert_eq!(check_codex_applied(&good, &want()), Ok(()));
    let entry = |a: &str| format!("{{\"access\":\"{a}\",\"path\":{{\"path\":\"{}\",\"type\":\"path\"}}}}", DENY[0]);
    let deny_entry = entry("deny");
    assert!(good.contains(&deny_entry), "{good}");
    for (bad, why) in [(good.replace(&format!("{deny_entry},"), ""), "no entry"), (good.replace(&deny_entry, &entry("read")), "read"), (good.replace(&deny_entry, &entry("write")), "write")] {
        let e = check_codex_applied(&bad, &want()).unwrap_err();
        assert!(e.0.contains("Vajra dir") || e.0.contains("write entry"), "{why}: {}", e.0);
    }
    assert_eq!(check_codex_applied(&good.replace(&deny_entry, &entry("none")), &want()), Ok(()), "none is as good as deny");
    // An older layout with no file-system entries can't show the dirs are locked.
    let mut v: Value = serde_json::from_str(good.lines().nth(1).unwrap()).unwrap();
    v["payload"].as_object_mut().unwrap().remove("file_system_sandbox_policy");
    assert!(check_codex_applied(&format!("{v}\n"), &want()).is_err());
}

#[test]
fn the_deny_dirs_are_the_state_dir_and_the_data_dir_in_written_and_real_form() {
    let d = tmpdir("deny");
    let real = d.join("real");
    std::fs::create_dir_all(real.join(".claude/vajra")).unwrap();
    std::os::unix::fs::symlink(&real, d.join("link")).unwrap();
    let home = d.join("link");
    let got = vajra_deny_dirs(&home, &home.join("data-not-made-yet"));
    assert_eq!(got, vec![home.join(".claude/vajra"), real.join(".claude/vajra"), home.join("data-not-made-yet"), real.join("data-not-made-yet")]);
    let flat = vajra_deny_dirs(&real, &real.join("data"));
    assert_eq!(flat, vec![real.join(".claude/vajra"), real.join("data")], "no duplicate when the path is already real");
    assert!(codex_profile_overrides(PROFILE, &[], &[], false).is_err(), "a Codex profile never goes out with no Vajra dirs denied");
    assert_eq!(codex_profile_overrides(PROFILE, &[PathBuf::from("/a\"b")], &[], false).unwrap_err().field, "deny");
}

/// A `Probe` whose codex answers the launch-time check the way it is told to, and records what it was asked.
struct FakeCodex {
    found: bool,
    answer: Option<(i32, String, String)>,
    asked: std::sync::Mutex<Vec<String>>,
}

impl crate::providers::detect::Probe for FakeCodex {
    fn find(&self, name: &str) -> Option<PathBuf> {
        (self.found && name == "codex").then(|| PathBuf::from("/bin/codex"))
    }
    fn run(&self, _path: &Path, args: &[&str]) -> Option<crate::providers::detect::Out> {
        *self.asked.lock().unwrap() = args.iter().map(|s| s.to_string()).collect();
        self.answer.clone().map(|(c, stdout, stderr)| crate::providers::detect::Out { code: Some(c), stdout, stderr })
    }
    fn env(&self, _key: &str) -> Option<OsString> {
        None
    }
}

fn out(code: i32, stdout: &str, stderr: &str) -> Option<(i32, String, String)> {
    Some((code, stdout.into(), stderr.into()))
}

#[test]
fn the_launch_time_check_passes_only_when_the_os_refuses_the_listing() {
    let home = tmpdir("probe");
    let state = home.join(".claude/vajra");
    std::fs::create_dir_all(&state).unwrap();
    let data = home.join("data");
    let fake = |found: bool, answer| FakeCodex { found, answer, asked: Default::default() };
    let eperm = format!("ls: {}: Operation not permitted\n", state.display());
    let ok = fake(true, out(1, "", &eperm));
    assert!(codex_key_probe(&ok, &home, &data));
    // It runs `codex sandbox` with the very profile a session gets, then lists the state dir; it never names the key.
    let asked = ok.asked.lock().unwrap().clone();
    assert_eq!(asked[0], "sandbox");
    // The probe makes its own fresh name; the rest of the profile is the one a session gets under that name.
    let name = asked.iter().find_map(|a| a.strip_prefix("default_permissions=\"")?.strip_suffix('"')).unwrap().to_string();
    assert!(valid_profile_name(&name), "{name}");
    let ov = codex_profile_overrides(&name, &vajra_deny_dirs(&home, &data), &[], false).unwrap();
    for o in &ov {
        assert!(asked.windows(2).any(|w| w[0] == "-c" && &w[1] == o), "{o} missing from {asked:?}");
    }
    assert_eq!(asked[asked.len() - 3..], ["--".to_string(), "/bin/ls".into(), state.to_str().unwrap().into()]);
    assert!(!asked.iter().any(|a| a.contains("hmac")), "{asked:?}");
    for (f, why) in [
        (fake(true, out(0, ".hmac-key\ncampaigns\n", "")), "the listing worked"),
        (fake(true, out(1, "", "ls: x: No such file or directory\n")), "another error"),
        (fake(true, out(2, "", "Error: unknown option\n")), "codex refused the profile"),
        (fake(true, None), "timed out"),
        (fake(false, out(1, "", &eperm)), "no codex"),
    ] {
        assert!(!codex_key_probe(&f, &home, &data), "{why}");
    }
    let empty = tmpdir("probe-nostate");
    assert!(!codex_key_probe(&fake(true, out(1, "", &eperm)), &empty, &data), "no state dir: nothing proven");
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
}

/// `s4_verdict`, plus the data-dir run: a full writer run whose Bash call the guard refused and whose Write and Edit the
/// deny rules refused, with the real refusal texts; `edit` changes how the Edit ended.
fn s4_verdict_with(tag: &str, served: usize, status: &str, result: bool, edit: DataRun) -> (Option<i32>, String) {
    let leak = edit == DataRun::Leaked;
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
    let (d, wt, data) = (sp.join("data-run"), sp.join("wt2"), sp.join("data"));
    for dir in [&d, &wt, &data] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(wt.join("ok-written"), "x").unwrap();
    std::fs::write(data.join("seed.json"), if leak { "edited\n" } else { "seed\n" }).unwrap();
    let dev = [json!({"type": "system", "subtype": "init", "tools": ["Read", "Grep", "Glob", "Bash", "Edit", "Write"], "permissionMode": "dontAsk"}), json!({"type": "result", "subtype": "success", "is_error": false})];
    std::fs::write(d.join("events.jsonl"), dev.iter().map(|e| e.to_string() + "\n").collect::<String>()).unwrap();
    let guard_text = "PreToolUse:Bash hook error: [bash \"pre-tool-use.sh\"]: Vajra guard blocked this Bash call: redirect into protected path";
    let deny_text = "<tool_use_error>File is in a directory that is denied by your permission settings.</tool_use_error>";
    let edit_text = match edit {
        DataRun::Refused => deny_text,
        DataRun::Leaked => "The file has been updated.",
        DataRun::OtherError => "<tool_use_error>File has not been read yet. Read it first before writing to it.</tool_use_error>",
    };
    let texts = ["", guard_text, deny_text, deny_text, edit_text];
    let dres: Vec<_> = (0..5).map(|i| json!({"id": format!("tu_{i}"), "is_error": i != 0 && !(leak && i == 4), "text": texts[i]})).collect();
    let dlog: String = (0..6).map(|k| json!({"path": "/v1/messages", "tools": ["Bash"], "k": k, "results": dres[..k], "body": "Run the S4 plan"}).to_string() + "\n").collect();
    std::fs::write(d.join("log"), dlog).unwrap();
    let o = std::process::Command::new("/usr/bin/python3").arg("-I").arg("-c").arg(S4_CHECK.replace("if __name__ == \"__main__\":\n    main()", "main()"))
        .args([sp.to_str().unwrap(), m.to_str().unwrap(), "Read,Grep,Glob,Bash", status])
        .args([d.to_str().unwrap(), wt.to_str().unwrap(), data.to_str().unwrap(), "Read,Grep,Glob,Bash,Edit,Write", "0"]).output().unwrap();
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
