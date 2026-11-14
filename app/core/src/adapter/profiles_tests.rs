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

/// Owner 2026-11-13, "keep my extras, no key claim": no Codex launch switches off the user's own hooks, plugins, apps,
/// MCP servers or notify.
#[test]
fn a_codex_launch_keeps_the_users_own_codex_tools() {
    let lead = codex_run_lead_argv(Path::new("/opt/tools/bin/codex"), "x", PROFILE, deny(), Path::new(WT), Path::new(TMP)).unwrap();
    for a in [codex_worker_argv(&launch(RoleKind::Worker, false)).unwrap(), codex_login_worker_argv(&launch(RoleKind::Worker, false)).unwrap(), lead] {
        let c = pairs(&a);
        assert!(!c.iter().any(|x| x.starts_with("features.") || x.starts_with("mcp_servers") || x.starts_with("notify")), "{c:?}");
    }
}

/// bhishma #216 M1-b: Codex loads a worktree's own `.codex` and `.agents`; any non-empty one refuses a launch.
#[test]
fn a_worktree_with_its_own_codex_or_agents_folder_never_gets_a_codex_session() {
    let wt = tmpdir("wtcodex");
    assert_eq!(codex_project_outside(&wt), None, "neither folder");
    std::fs::create_dir_all(wt.join(".codex")).unwrap();
    std::fs::create_dir_all(wt.join(".agents")).unwrap();
    assert_eq!(codex_project_outside(&wt), None, "empty folders load nothing");
    std::fs::write(wt.join(".codex/hooks.json"), "{}").unwrap();
    assert_eq!(codex_project_outside(&wt).as_deref(), Some(CODEX_PROJECT_TOOLS), "a hooks.json");
    std::fs::remove_file(wt.join(".codex/hooks.json")).unwrap();
    std::fs::create_dir_all(wt.join(".agents/skills")).unwrap();
    assert!(codex_project_outside(&wt).is_some(), "a skills folder");
    let wt2 = tmpdir("wtcodex2");
    std::os::unix::fs::symlink(&wt, wt2.join(".codex")).unwrap();
    assert!(codex_project_outside(&wt2).is_some(), "a symlinked .codex counts as one");
    let wt3 = tmpdir("wtcodex3");
    std::fs::write(wt3.join(".agents"), "x").unwrap();
    assert!(codex_project_outside(&wt3).is_some(), "a file named .agents");
}

/// The three profile values every Codex worker gets (S11).
fn profile_kv(network: bool) -> [String; 3] {
    [
        format!("permissions.{PROFILE}.filesystem={{\"/\"=\"read\",\"{}\"=\"none\",\"{}\"=\"none\",\"{WT}\"=\"write\",\"{TMP}\"=\"write\"}}", DENY[0], DENY[1]),
        format!("permissions.{PROFILE}.network={{enabled={network}}}"),
        format!("default_permissions=\"{PROFILE}\""),
    ]
}

/// bhishma #216 H1 and M-Lead: the plain Codex run Lead gets a worker's profile and approvals, with the project and
/// temp dir writable and no network; web search is the user's own setting (vajra-1 2026-11-13).
#[test]
fn the_codex_run_lead_runs_like_a_worker_with_no_approvals() {
    let bin = Path::new("/opt/tools/bin/codex");
    let a = codex_run_lead_argv(bin, "exec this --flag", PROFILE, deny(), Path::new(WT), Path::new(TMP)).unwrap();
    // bhishma #216 Low 1: the binary Vajra found, by its absolute path.
    let mut want: Vec<String> = vec!["/opt/tools/bin/codex".into(), "-c".into(), "approval_policy=\"never\"".into()];
    for o in codex_profile_overrides(PROFILE, deny(), &[Path::new(WT), Path::new(TMP)], false).unwrap() {
        want.extend(["-c".into(), o]);
    }
    want.extend(["--".into(), "exec this --flag".into()]);
    assert_eq!(a, want);
    assert!(a.iter().any(|x| x.contains(&format!("\"{}\"=\"none\"", DENY[0]))), "{a:?}");
    assert!(a.iter().any(|x| x.ends_with(".network={enabled=false}")), "{a:?}");
    assert!(!a.iter().any(|x| x.starts_with("web_search")), "{a:?}");
    assert!(!a.iter().any(|x| x == "--sandbox"), "{a:?}");
    assert!(codex_run_lead_argv(bin, "x", PROFILE, &[], Path::new(WT), Path::new(TMP)).is_err(), "never without the deny set");
    assert!(codex_run_lead_argv(bin, "x", "vajra", deny(), Path::new(WT), Path::new(TMP)).is_err(), "never under a guessable name");
    assert!(codex_run_lead_argv(Path::new("codex"), "x", PROFILE, deny(), Path::new(WT), Path::new(TMP)).is_err(), "never the bare word");
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
        "--", "Do the task",
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
        "--", "Do the task",
    ].map(String::from).into();
    assert_eq!(a, want);
    // S3: `model_providers.openai.*` makes codex refuse to load, so the login route never passes one.
    assert!(!a.iter().any(|x| x.contains("model_providers.openai")));
}

/// vajra-1 2026-11-13: network follows the seat's switch, and web search is always left to the user's own Codex
/// setting (no `web_search` override, on or off).
#[test]
fn network_follows_the_seat_and_web_search_is_left_to_the_user() {
    for net in [false, true] {
        for a in [codex_login_worker_argv(&launch(RoleKind::Worker, net)).unwrap(), codex_worker_argv(&launch(RoleKind::Verifier, net)).unwrap()] {
            let c = pairs(&a);
            let want = format!("permissions.vajra-0123456789abcdef.network={{enabled={net}}}");
            assert!(c.contains(&want.as_str()), "{c:?}");
            assert!(!c.iter().any(|x| x.starts_with("web_search")), "{c:?}");
        }
    }
}

/// The "always" settings are present whatever the seat. Spike S3 ran this argv's overrides with a real codex 0.166
/// against a hostile temp `config.toml` (`danger-full-access`, `on-request`, network on, a user writable root, web
/// search live) and every override won. The user's own hooks, plugins, MCP servers and notify stay on (owner
/// 2026-11-13); `a_codex_launch_keeps_the_users_own_codex_tools` pins that.
#[test]
fn codex_worker_forces_every_always_setting() {
    for a in [codex_worker_argv(&launch(RoleKind::Worker, false)).unwrap(), codex_login_worker_argv(&launch(RoleKind::Worker, false)).unwrap()] {
        let c = pairs(&a);
        let [fs, net, dp] = profile_kv(false);
        for want in ["approval_policy=\"never\"", fs.as_str(), net.as_str(), dp.as_str()] {
            assert!(c.contains(&want), "{want} missing from {c:?}");
        }
        // S12: any sandbox flag beats the profile, so none is passed in any form.
        assert!(!a.iter().any(|x| x == "--sandbox" || x == "-s" || x.starts_with("--sandbox=")), "{a:?}");
        assert!(!c.iter().any(|x| x.starts_with("sandbox_workspace_write.") || x.starts_with("sandbox_mode=")), "{c:?}");
        assert!(!a.iter().any(|x| x.contains("danger") || x.contains("bypass")));
        assert_eq!(a[a.len() - 2], "--");
    }
}

/// bhishma #216 L1 (S12, codex 0.166, live): `--sandbox X`, `-s X`, `-sX` and `--sandbox=X` each beat a per-launch
/// profile, so every form is dropped, with the `sandbox_mode` and legacy `-c` keys; the prompt after `--` is kept as is.
#[test]
fn every_form_of_the_sandbox_flag_is_dropped_but_never_the_prompt() {
    let mut a: Vec<String> = [
        "/bin/sh", "-c", argv::NULL_STDIN, "sh", "codex", "exec", "--json", "--sandbox", "workspace-write", "-s", "danger-full-access",
        "-sdanger-full-access", "--sandbox=danger-full-access", "-m", "m", "-c", "sandbox_mode=\"danger-full-access\"",
        "-c", "sandbox_workspace_write.network_access=true", "-c", "approval_policy=\"never\"", "--skip-git-repo-check", "--", "-s --sandbox x",
    ]
    .map(String::from)
    .into();
    drop_sandbox_flag(&mut a);
    let want: Vec<String> = ["/bin/sh", "-c", argv::NULL_STDIN, "sh", "codex", "exec", "--json", "-m", "m", "-c", "approval_policy=\"never\"", "--skip-git-repo-check", "--", "-s --sandbox x"]
        .map(String::from)
        .into();
    assert_eq!(a, want);
    // A prompt that is itself a sandbox flag, after `--`, stays.
    let mut b: Vec<String> = ["codex", "--", "--sandbox"].map(String::from).into();
    drop_sandbox_flag(&mut b);
    assert_eq!(b, ["codex", "--", "--sandbox"]);
}

/// #241 L3: the stripper also drops every other flag that would widen or replace the profile, in each spelling, and
/// still keeps the shell's own `-c <script>`, an unrelated `-c` config, and anything after `--`.
#[test]
fn every_profile_widening_flag_is_dropped_in_every_spelling() {
    let mut a: Vec<String> = [
        "/bin/sh", "-c", argv::NULL_STDIN, "sh", "codex", "--full-auto", "--dangerously-bypass-approvals-and-sandbox", "--yolo",
        "--profile", "theirs", "-p", "theirs", "--profile=theirs", "-ptheirs",
        "--config", "sandbox_mode=\"danger-full-access\"", "--config=sandbox_mode=\"danger-full-access\"",
        "-csandbox_mode=\"danger-full-access\"", "-csandbox_workspace_write.network_access=true",
        "-c", "model=\"x\"", "--config", "approval_policy=\"never\"", "--", "--yolo -p x",
    ]
    .map(String::from)
    .into();
    drop_sandbox_flag(&mut a);
    let want: Vec<String> = ["/bin/sh", "-c", argv::NULL_STDIN, "sh", "codex", "-c", "model=\"x\"", "--config", "approval_policy=\"never\"", "--", "--yolo -p x"].map(String::from).into();
    assert_eq!(a, want);
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

/// Sandbox B: the user's default model and named model-routing env vars carry over into the flag file; no permission,
/// sandbox, hook, apiKeyHelper or MCP key, no key or token, and no unnamed env var ever does.
#[test]
fn only_the_default_model_and_named_routing_env_carry_over_from_the_users_settings() {
    let hostile = r#"{
        "model": "opus-plan",
        "env": {"ANTHROPIC_BASE_URL": "https://gw.example/v1", "CLAUDE_CODE_USE_BEDROCK": "1", "AWS_REGION": "us-west-2",
                "HTTPS_PROXY": "http://proxy:3128", "ANTHROPIC_API_KEY": "sk-not-carried", "ANTHROPIC_AUTH_TOKEN": "t",
                "ANTHROPIC_CUSTOM_HEADERS": "x-key: v", "AWS_PROFILE": "p", "CLAUDE_CODE_SANDBOX": "off", "PATH": "/evil",
                "ANTHROPIC_MODEL": "bad\nline", "ANTHROPIC_SMALL_FAST_MODEL": 7, "NO_PROXY": "localhost",
                "HTTP_PROXY": "http://127.0.0.1:8080", "ALL_PROXY": "socks5://127.0.0.1:1080", "https_proxy": "http://127.0.0.1:3128",
                "http_proxy": "x", "all_proxy": "x", "no_proxy": "x"},
        "permissions": {"allow": ["Bash(*)"], "defaultMode": "bypassPermissions"},
        "sandbox": {"enabled": false, "filesystem": {"allowRead": ["/"]}, "excludedCommands": ["*"]},
        "hooks": {"PreToolUse": []}, "disableAllHooks": true, "apiKeyHelper": "/bin/echo k",
        "enabledMcpjsonServers": ["x"], "mcpServers": {"x": {}}
    }"#;
    let c = claude_carry_over(Some(hostile));
    assert_eq!(c.model.as_deref(), Some("opus-plan"));
    assert_eq!(c.env, vec![
        ("ANTHROPIC_BASE_URL", "https://gw.example/v1".to_string()),
        ("CLAUDE_CODE_USE_BEDROCK", "1".to_string()),
        ("AWS_REGION", "us-west-2".to_string()),
    ], "only named, single-line, non-blank strings; no proxy variable");
    for k in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_CUSTOM_HEADERS", "AWS_PROFILE", "PATH"] {
        assert!(!CLAUDE_CARRY_ENV.contains(&k), "{k}");
    }
    // #224 bhishma Low: settings env reaches the sandboxed Bash, and a loopback proxy would be a way off the machine.
    for k in CLAUDE_CARRY_NEVER {
        assert!(!CLAUDE_CARRY_ENV.iter().any(|c| c.eq_ignore_ascii_case(k)), "{k}");
    }
    assert!(CLAUDE_CARRY_ENV.iter().all(|k| !k.to_ascii_uppercase().contains("PROXY")));
    assert_eq!(CLAUDE_CARRY_NEVER.len(), 8, "the four proxy variables in both cases");
    assert!(CLAUDE_CARRY_ENV.iter().all(|k| !k.contains("KEY") && !k.contains("TOKEN") && !k.contains("HEADER")));
    for t in [None, Some(""), Some("not json"), Some("[1]"), Some(r#"{"model": 3, "env": "x"}"#)] {
        assert_eq!(claude_carry_over(t), CarryOver::default(), "{t:?}");
    }
    assert_eq!(claude_carry_over(Some(&format!(r#"{{"model":"{}"}}"#, "m".repeat(600)))).model, None, "too long");

    // In the flag: only `model` and `env` change; the sandbox, permissions and hooks are the template's, byte for byte.
    let d = tmpdir("carry");
    let (home, work, repo, data, tmp) = (d.join("home"), d.join("work"), d.join("repo"), d.join("data"), d.join("tmp"));
    for p in [&home, &work, &repo, &data, &tmp] {
        std::fs::create_dir_all(p).unwrap();
    }
    let base: Value = serde_json::from_str(&claude_worker_flag(&home, &work, std::slice::from_ref(&repo), &data, &tmp).unwrap()).unwrap();
    let mut with: Value = serde_json::from_str(&claude_worker_flag_with(&home, &work, std::slice::from_ref(&repo), &data, &tmp, &c).unwrap()).unwrap();
    assert_eq!(with["model"], "opus-plan");
    assert_eq!(with["env"], json!({"ANTHROPIC_BASE_URL": "https://gw.example/v1", "AWS_REGION": "us-west-2", "CLAUDE_CODE_USE_BEDROCK": "1"}));
    assert!(base.get("model").is_none() && base.get("env").is_none(), "the template carries nothing");
    let o = with.as_object_mut().unwrap();
    o.remove("model");
    o.remove("env");
    assert_eq!(with, base);
    // The proven shape hashes the template, whatever the user's settings hold.
    let shape = claude_worker_flag_shape(&repo_hooks()).unwrap();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(home.join(".claude/settings.json"), hostile).unwrap();
    assert_eq!(read_claude_carry_over(&home), c, "read through the provider check's reader");
    assert_eq!(claude_worker_flag_shape(&repo_hooks()).unwrap(), shape);
}

/// The carry-over reader follows no link: a settings.json that is a symlink carries nothing.
#[test]
fn a_linked_user_settings_file_carries_nothing() {
    let d = tmpdir("carry-link");
    let home = d.join("home");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(d.join("elsewhere.json"), r#"{"model":"m"}"#).unwrap();
    std::os::unix::fs::symlink(d.join("elsewhere.json"), home.join(".claude/settings.json")).unwrap();
    assert_eq!(read_claude_carry_over(&home), CarryOver::default());
    assert_eq!(read_claude_carry_over(&d.join("no-home")), CarryOver::default());
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
    // #212 review: a Claude worker's sandbox is loopback only, so network on never says "Internet on".
    use crate::desktop::seat_start::{CLAUDE_NOTE, CLAUDE_NO_NETWORK_NOTE, NETWORK_NOTE};
    for kind in [RoleKind::Worker, RoleKind::Verifier] {
        assert_eq!(disclosures(&profile(Engine::Claude, kind), true), vec![CLAUDE_NOTE, CLAUDE_NO_NETWORK_NOTE]);
    }
    assert_eq!(disclosures(&p, true)[1], NETWORK_NOTE, "Codex keeps its internet note");
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

/// A Claude worker is guarded only when its init event shows no MCP server loaded and no `mcp__` tool; otherwise the
/// reason names every server, from either place.
#[test]
fn a_claude_worker_is_guarded_only_when_no_mcp_server_loaded() {
    let ev = |extra: Value| {
        let mut init = json!({"type": "system", "subtype": "init", "tools": ["Read"]});
        for (k, v) in extra.as_object().unwrap() {
            init[k] = v.clone();
        }
        format!("{{\"type\":\"assistant\"}}\n{init}\n")
    };
    assert_eq!(claude_key_guard(&ev(json!({"mcp_servers": []}))), json!({"guarded": true}));
    assert_eq!(claude_key_guard(&ev(json!({}))), json!({"guarded": true}));
    let one = claude_key_guard(&ev(json!({"mcp_servers": [{"name": "files", "status": "failed"}]})));
    assert_eq!((one["guarded"].as_bool(), one["mcp_servers"].clone()), (Some(false), json!(["files"])), "a failed server may have started");
    assert_eq!(one["reason"], "Your MCP server files runs outside the sandbox and can read your files, including Vajra's signing key.");
    let two = claude_key_guard(&ev(json!({"mcp_servers": [{"name": "shell"}], "tools": ["Read", "mcp__files__read", "mcp__shell__run"]})));
    assert_eq!(two["mcp_servers"], json!(["files", "shell"]), "{two}");
    assert!(two["reason"].as_str().unwrap().starts_with("Your MCP servers files, shell run outside"), "{two}");
    // #215 bhishma: a format the parser can't read fails closed, as an unnamed server, never as "none loaded".
    for odd in [json!({"mcp_servers": {"files": {}}}), json!({"mcp_servers": "files"}), json!({"mcp_servers": [{"status": "connected"}]}),
        json!({"mcp_servers": [{"name": ""}]}), json!({"mcp_servers": [7]}), json!({"tools": "Read,mcp__x__y"}), json!({"tools": ["Read", {"name": "mcp__x__y"}]}),
        json!({"tools": ["Read", "mcp____y"]})]
    {
        let g = claude_key_guard(&ev(odd.clone()));
        assert_eq!((g["guarded"].as_bool(), g["mcp_servers"].clone()), (Some(false), json!([UNNAMED_MCP_SERVER])), "{odd}: {g}");
    }
    // Every init event counts, not only the first.
    let two_inits = format!("{}{}", ev(json!({"mcp_servers": []})), ev(json!({"mcp_servers": [{"name": "late"}]})));
    assert_eq!(claude_key_guard(&two_inits)["mcp_servers"], json!(["late"]));
    // No init event: unknown, so not guarded.
    assert_eq!(claude_mcp_loaded("{}\n"), None);
    let none = claude_key_guard("{}\n");
    assert_eq!(none["guarded"], false, "{none}");
    assert!(none["reason"].as_str().unwrap().contains("couldn't read which MCP servers loaded"), "{none}");
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

// ------------------------------------------------ the plain Codex Lead's first-turn read-back (bhishma MUST, S12)

/// The project, temp dir and marker the pinned Lead sessions below use (paths that don't exist, so none resolves).
const LP: &str = "/w/lead/project";
const LT: &str = "/w/lead/tmp";
const LM: &str = "/h/data/.lead-vajra-0123456789abcdef";

fn lead_want() -> CodexWant {
    let mut deny = deny().to_vec();
    deny.push(PathBuf::from(LM));
    CodexWant { cwd: LP.into(), ticket_tmp: LT.into(), network: false, deny }
}

/// A Lead session file: `session_meta` with `cwd`, then (when given) one `turn_context` whose sandbox and file-system
/// policy are as given, in the shape codex 0.166 wrote them in S12 (`codex-tui`, the interactive Codex the Lead is).
fn lead_file(cwd: &str, sandbox: &str, fs: &str) -> String {
    let meta = format!(r#"{{"type":"session_meta","payload":{{"id":"01a1d77d-62c1-74c3-8a27-9edd284a8505","cwd":"{cwd}","originator":"codex-tui","source":"cli","cli_version":"0.166.0"}}}}"#);
    if sandbox.is_empty() {
        return format!("{meta}\n");
    }
    format!("{meta}\n{{\"type\":\"turn_context\",\"payload\":{{\"cwd\":\"{cwd}\",\"approval_policy\":\"never\",\"sandbox_policy\":{sandbox},\"file_system_sandbox_policy\":{fs}}}}}\n")
}

/// S12's `sandbox_policy` under the profile (both with no user setting and with the user's own top-level
/// `sandbox_mode = "danger-full-access"`: the profile won, and the two were identical).
const S12_PROFILE_SANDBOX: &str = r#"{"type":"workspace-write","network_access":false,"exclude_tmpdir_env_var":true,"exclude_slash_tmp":true}"#;
/// S12's `sandbox_policy` with `-s danger-full-access` (also `--sandbox X` and `--sandbox=X`): the flag beat the profile,
/// the file-system policy was `null`, and the denied file was read.
const S12_FLAG_SANDBOX: &str = r#"{"type":"danger-full-access"}"#;

fn s12_profile_fs(marker: bool) -> String {
    let mut es = vec![r#"{"path":{"type":"path","path":"/"},"access":"read"}"#.to_string()];
    for d in DENY {
        es.push(format!(r#"{{"path":{{"type":"path","path":"{d}"}},"access":"deny"}}"#));
    }
    if marker {
        es.push(format!(r#"{{"path":{{"type":"path","path":"{LM}"}},"access":"deny"}}"#));
    }
    es.push(format!(r#"{{"path":{{"type":"path","path":"{LP}"}},"access":"write"}}"#));
    es.push(format!(r#"{{"path":{{"type":"path","path":"{LT}"}},"access":"write"}}"#));
    format!(r#"{{"kind":"restricted","entries":[{}]}}"#, es.join(","))
}

/// The pinned S12 observations, through the read-back: the profile in force (with or without the user's own
/// `sandbox_mode`) passes; a sandbox flag that beat it ends the Lead, whether its session file is told apart by the
/// marker or not (a flag drops every file-system entry, the marker with them).
#[test]
fn the_lead_read_back_passes_the_profile_and_ends_a_lead_a_sandbox_flag_beat() {
    let (w, m) = (lead_want(), Path::new(LM));
    let applied = lead_file(LP, S12_PROFILE_SANDBOX, &s12_profile_fs(true));
    assert_eq!(codex_lead_verdict(std::slice::from_ref(&applied), &w, m), LeadVerdict::Applied);
    let flag = lead_file(LP, S12_FLAG_SANDBOX, "null");
    assert!(check_codex_applied(&flag, &w).is_err(), "the flag's turn_context fails the worker check too");
    assert_eq!(codex_lead_verdict(&[flag], &w, m), LeadVerdict::NotApplied(LEAD_UNMARKED.into()));
    // The marker is shown but the Vajra state dir isn't denied: the Lead ends with the check's own reason.
    let open = lead_file(LP, S12_PROFILE_SANDBOX, &s12_profile_fs(true).replace(r#""/h/.claude/vajra"},"access":"deny""#, r#""/h/.claude/vajra"},"access":"read""#));
    assert!(matches!(codex_lead_verdict(&[open], &w, m), LeadVerdict::NotApplied(why) if why.contains("/h/.claude/vajra")));
    // #175's L1, shared: a read entry inside a Vajra dir reopens it, so the Lead ends even with every dir denied.
    let reopened = lead_file(LP, S12_PROFILE_SANDBOX, &s12_profile_fs(true).replace(r#"{"kind":"restricted","entries":["#, r#"{"kind":"restricted","entries":[{"path":{"type":"path","path":"/h/data/runs"},"access":"read"},"#));
    assert!(matches!(codex_lead_verdict(&[reopened], &w, m), LeadVerdict::NotApplied(why) if why.contains("/h/data/runs")));
    // The marker is shown with a wider sandbox: ended.
    let wide = lead_file(LP, r#"{"type":"danger-full-access"}"#, &s12_profile_fs(true));
    assert!(matches!(codex_lead_verdict(&[wide], &w, m), LeadVerdict::NotApplied(_)));
}

/// The read-back waits while the Lead's first turn hasn't run and ignores another folder's sessions. bhishma L1 on
/// #241: any unmarked first turn in the project ends the Lead, even beside a marked file that passes, since a sandbox
/// flag drops the marker and that file may be the Lead's own.
#[test]
fn the_lead_read_back_waits_for_the_first_turn_and_tells_its_own_file_apart() {
    let (w, m) = (lead_want(), Path::new(LM));
    assert_eq!(codex_lead_verdict(&[], &w, m), LeadVerdict::Pending);
    let before = lead_file(LP, "", "");
    assert_eq!(codex_lead_verdict(std::slice::from_ref(&before), &w, m), LeadVerdict::Pending, "no turn yet");
    let elsewhere = lead_file("/w/other", S12_FLAG_SANDBOX, "null");
    assert_eq!(codex_lead_verdict(std::slice::from_ref(&elsewhere), &w, m), LeadVerdict::Pending, "another folder's session is never the Lead's");
    let users_own = lead_file(LP, S12_PROFILE_SANDBOX, &s12_profile_fs(false));
    let ours = lead_file(LP, S12_PROFILE_SANDBOX, &s12_profile_fs(true));
    assert_eq!(codex_lead_verdict(&[ours.clone(), elsewhere.clone(), before.clone()], &w, m), LeadVerdict::Applied);
    // An unmarked first turn ends the Lead in any order: beside a passing marked file, beside one before its first
    // turn, or alone.
    let unmarked = LeadVerdict::NotApplied(LEAD_UNMARKED.into());
    assert_eq!(codex_lead_verdict(&[ours.clone(), users_own.clone()], &w, m), unmarked, "marked first");
    assert_eq!(codex_lead_verdict(&[users_own.clone(), ours, elsewhere], &w, m), unmarked, "unmarked first");
    assert_eq!(codex_lead_verdict(&[users_own.clone(), before], &w, m), unmarked);
    assert_eq!(codex_lead_verdict(&[users_own], &w, m), unmarked);
    // A layout with no cwd in either record still counts as the project's (fail closed).
    let no_cwd = lead_file(LP, S12_FLAG_SANDBOX, "null").replace(&format!(r#""cwd":"{LP}","#), "");
    assert!(!no_cwd.contains(LP));
    assert_eq!(codex_lead_verdict(&[no_cwd], &w, m), LeadVerdict::NotApplied(LEAD_UNMARKED.into()));
}

/// The Lead's argv carries the marker as a deny entry when the caller adds it, and never a sandbox flag.
#[test]
fn the_lead_argv_denies_the_marker_and_has_no_sandbox_flag() {
    let mut d = deny().to_vec();
    d.push(codex_lead_marker(Path::new("/h/data"), PROFILE));
    assert_eq!(d.last().unwrap(), Path::new(LM));
    let a = codex_run_lead_argv(Path::new("/opt/tools/bin/codex"), "x", PROFILE, &d, Path::new(LP), Path::new(LT)).unwrap();
    assert!(a.iter().any(|x| x.contains(&format!("\"{LM}\"=\"none\""))), "{a:?}");
    let mut stripped = a.clone();
    drop_sandbox_flag(&mut stripped);
    assert_eq!(stripped, a, "nothing to strip");
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

/// A lead's deny dirs: the state dir and the data dir `/d`, which holds the lead dir (so the profile reads it back in).
fn lead_deny() -> &'static [PathBuf] {
    static D: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    D.get_or_init(|| ["/h/.claude/vajra", "/d"].iter().map(PathBuf::from).collect())
}

fn lead(model: Option<&'static str>, effort: Option<&'static str>) -> LeadLaunch<'static> {
    LeadLaunch {
        prompt: "the charter", model, effort, lead_dir: Path::new("/d/runs/20261112-0930/lead"), project: Path::new("/p/repo"), server: server(),
        deny: lead_deny(), profile: PROFILE,
    }
}

/// The lead's 6k profile `-c` values, as `codex_lead_launch` writes them.
fn lead_profile_kv() -> [String; 3] {
    [
        format!("permissions.{PROFILE}.filesystem={{\"/\"=\"read\",\"/h/.claude/vajra\"=\"none\",\"/d\"=\"none\",\"/d/runs/20261112-0930/lead\"=\"read\"}}"),
        format!("permissions.{PROFILE}.network={{enabled=false}}"),
        format!("default_permissions=\"{PROFILE}\""),
    ]
}

/// 6k on the core lead: a fresh profile with the Vajra dirs unreadable, the lead dir alone read back in, no write
/// entry and no network, on both routes; N3 stays (only `vajra_run`, `notify` off). A launch with no deny dirs, a
/// guessable profile name, or a read-back entry that would reopen a denied dir is refused.
#[test]
fn the_codex_lead_runs_under_a_fresh_profile_with_the_vajra_dirs_denied_and_no_writes() {
    for route in [CodexRoute::Login, CodexRoute::Gateway { home: Path::new(GW_HOME) }] {
        let a = codex_lead_argv_of(&lead(Some("m"), None), route).unwrap();
        let kv = pairs(&a);
        for p in lead_profile_kv() {
            assert_eq!(kv.iter().filter(|k| **k == p).count(), 1, "{p} in {kv:?}");
        }
        assert!(!kv.iter().any(|k| k.contains("\"write\"")), "a lead's profile has no write entry: {kv:?}");
        // On 0.166 an explicit --sandbox wins over default_permissions and the deny entries never apply (2026-11-13).
        assert!(!a.iter().any(|x| x == "--sandbox"), "no --sandbox flag, which would switch the profile off: {a:?}");
        assert!(kv.contains(&"notify=[]") && kv.contains(&run_server_kv().as_str()), "N3 holds alongside the profile");
    }
    let mut l = lead(Some("m"), None);
    l.deny = &[];
    assert_eq!(codex_lead_launch(&l, CodexRoute::Login).unwrap_err().field, "deny");
    let mut l = lead(Some("m"), None);
    l.profile = "vajra";
    assert_eq!(codex_lead_launch(&l, CodexRoute::Login).unwrap_err().field, "profile");
    // A lead dir that is, or holds, a denied dir would reopen it.
    for dir in ["/d", "/"] {
        let mut l = lead(Some("m"), None);
        l.lead_dir = Path::new(dir);
        assert_eq!(codex_lead_launch(&l, CodexRoute::Login).unwrap_err().field, "read", "{dir}");
    }
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
        "/bin/sh", "-c", r#"exec "$@" </dev/null"#, "sh", "codex", "exec", "--json", "--skip-git-repo-check",
        "-C", "/d/runs/20261112-0930/lead", "--ignore-rules", "--ignore-user-config", "-m", "model-a",
        "-c", "approval_policy=\"never\"", "-c", "notify=[]", "-c", "web_search=\"disabled\"",
        "-c", "features.multi_agent=false", "-c", "features.goals=false",
        "-c", &lead_profile_kv()[0], "-c", &lead_profile_kv()[1], "-c", &lead_profile_kv()[2],
        "-c", "model_reasoning_effort=\"high\"", "-c", &run_server_kv(), "--", "the charter",
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
    let (wt, d) = (Path::new(WT), deny());
    assert!(check_codex_lead_applied(&lead_rollout(json!({"type": "read-only"}), &[]), wt, d).is_ok());
    let ws = json!({"type": "workspace-write", "network_access": false, "exclude_slash_tmp": true, "exclude_tmpdir_env_var": true, "writable_roots": []});
    assert!(check_codex_lead_applied(&lead_rollout(ws, &[]), wt, d).is_err(), "workspace-write is wider than a lead's profile");
    assert!(check_codex_lead_applied(&lead_rollout(json!({"type": "read-only", "network_access": true}), &[]), wt, d).is_err());
    assert!(check_codex_lead_applied(&lead_rollout(json!({"type": "read-only"}), &[WT]), wt, d).is_err(), "no write entry at all");
    assert!(check_codex_lead_applied(&lead_rollout(json!({"type": "read-only"}), &[]), Path::new("/elsewhere"), d).is_err());
    assert!(check_codex_lead_applied(&rollout(json!({"type": "read-only"}), &[], "on-request"), wt, d).is_err());
    assert!(check_codex_lead_applied("{}\n", wt, d).is_err());
}

/// The `turn_context` codex 0.166 wrote on 2026-11-13 for a lead launched as `codex_lead_launch` builds it (a no-write
/// profile, no `--sandbox`): a `read-only` sandbox and the profile's entries, `deny` for the data dir and `read` for
/// the lead dir inside it. Run with a temp CODEX_HOME and a placeholder provider on a closed loopback port; no key.
const OBSERVED_LEAD_TC: &str = r#"{"type":"turn_context","payload":{"cwd":"/private/tmp/v52-nowrite/data/runs/r1/lead","approval_policy":"never","sandbox_policy":{"type":"read-only"},"file_system_sandbox_policy":{"kind":"restricted","entries":[{"path":{"type":"path","path":"/"},"access":"read"},{"path":{"type":"path","path":"/private/tmp/v52-nowrite/data"},"access":"deny"},{"path":{"type":"path","path":"/private/tmp/v52-nowrite/data/runs/r1/lead"},"access":"read"}]}}}"#;

/// The same launch with `--sandbox read-only` added: the flag wins and the profile is gone (only the root reads).
const OBSERVED_FLAG_TC: &str = r#"{"type":"turn_context","payload":{"cwd":"/private/tmp/v52-nowrite/data/runs/r1/lead","approval_policy":"never","sandbox_policy":{"type":"read-only"},"permission_profile":{"type":"managed","file_system":{"type":"restricted","entries":[{"path":{"type":"special","value":{"kind":"root"}},"access":"read"}]},"network":"restricted"}}}"#;

#[test]
fn the_codex_lead_read_back_passes_what_real_codex_reports_and_refuses_a_profile_the_flag_switched_off() {
    let lead = Path::new("/private/tmp/v52-nowrite/data/runs/r1/lead");
    let deny = [PathBuf::from("/private/tmp/v52-nowrite/data")];
    assert!(check_codex_lead_applied(OBSERVED_LEAD_TC, lead, &deny).is_ok());
    assert!(check_codex_lead_applied(OBSERVED_FLAG_TC, lead, &deny).is_err(), "--sandbox switched the profile off");
}

/// 6k: a lead whose profile didn't apply never runs. A Vajra dir with no deny entry fails, an older layout with no
/// file-system policy can't show the dirs are locked so it fails, and an empty deny list is never a pass.
#[test]
fn the_codex_lead_read_back_fails_unless_every_vajra_dir_shows_denied() {
    let wt = Path::new(WT);
    let ok = lead_rollout(json!({"type": "read-only"}), &[]);
    let one_more = [deny().to_vec(), vec![PathBuf::from("/h/other")]].concat();
    assert!(check_codex_lead_applied(&ok, wt, &one_more).unwrap_err().0.contains("/h/other"), "a dir with no deny entry is readable");
    let mut v: Vec<Value> = ok.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    v[1]["payload"].as_object_mut().unwrap().remove("file_system_sandbox_policy");
    let old = v.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
    assert!(check_codex_lead_applied(&old, wt, deny()).is_err(), "no file-system policy can't show the dirs are denied");
    assert!(check_codex_lead_applied(&ok, wt, &[]).is_err(), "nothing to check is never a pass");
}

/// Plan #175 L1: the read-entry check compares canonical forms on both sides. A deny dir spelled through a link, and a
/// read entry spelled through a link or with `..`, still meet; the lead dir itself stays readable in either spelling.
#[test]
fn the_lead_read_entry_check_compares_canonical_forms_of_the_deny_dirs() {
    let t = std::env::temp_dir().join(format!("vajra-canon-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&t);
    let real = t.join("real");
    std::fs::create_dir_all(real.join("data/control/lead/run1")).unwrap();
    std::os::unix::fs::symlink(&real, t.join("alias")).unwrap();
    let real = real.canonicalize().unwrap();
    let alias = t.join("alias");
    let tc = |p: &Path| json!({"payload": {"file_system_sandbox_policy": {"entries": [{"access": "read", "path": {"type": "path", "path": p.to_str().unwrap()}}]}}});
    let lead = real.join("data/control/lead/run1");
    let allowed = std::slice::from_ref(&lead);
    // The deny dir spelled through the link; the entry in the real form.
    let deny_alias = vec![alias.join("data")];
    assert!(check_no_read_reopens(&tc(&real.join("data/control")), &deny_alias, allowed).is_err(), "a real-path entry inside a deny dir spelled through a link");
    // The deny dir real; entries through the link, and with `..`.
    let deny = vec![real.join("data")];
    assert!(check_no_read_reopens(&tc(&alias.join("data/control")), &deny, allowed).is_err(), "an entry through a link");
    assert!(check_no_read_reopens(&tc(&real.join("elsewhere/../data/control")), &deny, allowed).is_err(), "an entry with ..");
    // A `..` spelling that only looks like the lead dir is not the lead dir.
    assert!(check_no_read_reopens(&tc(&lead.join("../run1/..")), &deny, allowed).is_err(), "a .. spelling of the lead's parent");
    // The lead dir, as written or through the link, stays allowed; an entry outside every deny dir is fine.
    assert!(check_no_read_reopens(&tc(&lead), &deny, allowed).is_ok());
    assert!(check_no_read_reopens(&tc(&alias.join("data/control/lead/run1")), &deny_alias, allowed).is_ok(), "the lead dir through the link");
    assert!(check_no_read_reopens(&tc(&real.join("other")), &deny, allowed).is_ok());
    assert!(check_no_read_reopens(&tc(Path::new("/")), &deny, allowed).is_ok(), "the root entry is exempt");
    // An entry that doesn't exist yet is still placed by its existing ancestor.
    assert!(check_no_read_reopens(&tc(&alias.join("data/not-yet")), &deny, allowed).is_err(), "a missing entry under a linked deny dir");
    let _ = std::fs::remove_dir_all(&t);
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
