//! C4 script bridge and preflight, using fake scripts in temp dirs.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use vajra_desktop::vajra::allowlist::Allowlist;
use vajra_desktop::vajra::bridge::{Bridge, BridgeError, OUTPUT_CAP, Request, parse_kb_hits, parse_kb_line};
use vajra_desktop::vajra::preflight::{self, Row, start_enabled};

static N: AtomicUsize = AtomicUsize::new(0);

fn tmp() -> PathBuf {
    let d = std::env::temp_dir().join(format!("vajra-bridge-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, format!("#!/bin/bash\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p
}

/// A bridge whose python and git are fake scripts that echo their argv.
fn echo_bridge(dir: &Path) -> Bridge {
    let mut b = Bridge::new(dir.to_path_buf());
    b.python = script(dir, "fake-python", r#"for a in "$@"; do echo "[$a]"; done"#);
    b.git = script(dir, "fake-git", r#"for a in "$@"; do echo "[$a]"; done"#);
    script(dir, "vajra-campaign.sh", r#"for a in "$@"; do echo "[$a]"; done; exit 3"#);
    script(dir, "vajra-doctor.sh", "echo PASS ok");
    b
}

#[test]
fn argv_for_each_fixed_request() {
    let d = tmp();
    let b = echo_bridge(&d);
    let v = b.run(&Request::CampaignVerify { slug: "deliver-1".into() }).unwrap();
    assert_eq!(v.stdout, "[verify]\n[deliver-1]\n");
    assert_eq!(v.code, Some(3));

    let p = b.run(&Request::CampaignProgress { slug: "deliver-1".into(), text: "hi; rm -rf /".into() }).unwrap();
    assert_eq!(p.stdout, "[progress]\n[hi; rm -rf /]\n[--campaign]\n[deliver-1]\n");

    let ev = |head: Option<&str>, archived: Option<&str>, path: &str| Request::CheckEvidence {
        path: path.into(), head: head.map(Into::into), archived: archived.map(Into::into), repo: None,
    };
    let e = b.run(&ev(Some("abc1234"), None, "/r/evidence.json")).unwrap();
    assert!(e.stdout.contains("vajra_tasks.py]\n[check-evidence]\n[/r/evidence.json]\n[--head]\n[abc1234]\n[--json]\n"), "{}", e.stdout);
    let h40 = "ea73cb05d639b945e21c667e17103e56404861a8";
    let e = b.run(&ev(Some(h40), Some("c1"), "/r/c1/evidence/e1/evidence.json")).unwrap();
    assert!(e.stdout.contains(&format!("[--head]\n[{h40}]\n[--archived]\n[c1]\n[--json]\n")), "{}", e.stdout);
    for bad in [ev(None, None, "relative.json"), ev(Some("--evil"), None, "/r/e.json"), ev(None, Some("../x"), "/r/e.json")] {
        assert!(matches!(b.run(&bad), Err(BridgeError::Invalid(_))));
    }

    let repo = tmp();
    let k = b.run(&Request::KbSearch { query: "--help $(x)".into(), k: 5, repo: repo.clone() }).unwrap();
    assert!(k.stdout.contains("[search]\n[--format]\n[ids]\n[--k]\n[5]\n[--]\n[--help $(x)]\n"));

    let dr = b.run(&Request::Doctor).unwrap();
    assert_eq!(dr.stdout, "PASS ok\n");

    let g = b.run(&Request::Git { cwd: repo, args: vec!["rev-parse".into(), "--".into(), "a b".into()] }).unwrap();
    assert_eq!(g.stdout, "[--no-pager]\n[-c]\n[core.fsmonitor=]\n[rev-parse]\n[--]\n[a b]\n");
    // diff never runs a repo-configured external diff or textconv driver.
    let g = b.run(&Request::Git { cwd: tmp(), args: vec!["diff".into(), "main".into(), "--".into()] }).unwrap();
    assert_eq!(g.stdout, "[--no-pager]\n[-c]\n[core.fsmonitor=]\n[diff]\n[--no-ext-diff]\n[--no-textconv]\n[--ignore-submodules=all]\n[main]\n[--]\n");
}

#[test]
fn real_git_diff_ignores_repo_diff_external() {
    let repo = tmp();
    let marker = repo.join("pwned");
    let git = |args: &[&str]| {
        let o = std::process::Command::new("/usr/bin/git").args(args).current_dir(&repo).env("HOME", &repo).output().unwrap();
        assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
    };
    git(&["init", "-q"]);
    git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "base"]);
    fs::write(repo.join("f.txt"), "a\n").unwrap();
    git(&["add", "f.txt"]);
    git(&["config", "diff.external", &format!("/usr/bin/touch {}", marker.display())]);
    let b = Bridge::new(tmp());
    let o = b.run(&Request::Git { cwd: repo.clone(), args: vec!["diff".into(), "--cached".into(), "HEAD".into(), "--".into()] }).unwrap();
    assert!(!marker.exists(), "diff.external ran");
    assert!(o.stdout.contains("+a"), "{}", o.stdout);
}

#[test]
fn invalid_requests_run_nothing() {
    let d = tmp();
    let b = echo_bridge(&d);
    for slug in ["", "-rf", "a b", "a/b", "../x", "a;b", "$(x)"] {
        let r = b.plan(&Request::CampaignVerify { slug: slug.into() });
        assert!(matches!(r, Err(BridgeError::Invalid(_))), "{slug:?}");
    }
    let cwd = d.clone();
    for args in [
        vec!["push"],
        vec!["config", "x"],
        vec![],
        vec!["diff", "--output=/tmp/x"],
        vec!["log", "--output", "/tmp/x"],
        vec!["status", "-c"],
        vec!["diff", "--ext-diff"],
    ] {
        let args: Vec<String> = args.into_iter().map(String::from).collect();
        let r = b.plan(&Request::Git { cwd: cwd.clone(), args });
        assert!(matches!(r, Err(BridgeError::Invalid(_))));
    }
    let r = b.plan(&Request::KbSearch { query: "  ".into(), k: 1, repo: cwd });
    assert!(matches!(r, Err(BridgeError::Invalid(_))));
}

#[test]
fn environment_is_scrubbed() {
    let d = tmp();
    let mut b = echo_bridge(&d);
    b.git = script(&d, "env-git", "env | sort");
    // SAFETY: this test binary sets the variable before any bridge call and
    // no other test reads it.
    unsafe { std::env::set_var("VAJRA_BRIDGE_SECRET", "leak-me") };
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "leak-me-too") };
    let o = b.run(&Request::Git { cwd: d.clone(), args: vec!["--version".into()] }).unwrap();
    assert!(!o.stdout.contains("leak-me"), "{}", o.stdout);
    assert!(o.stdout.contains("PATH="));
    assert!(o.stdout.contains("GIT_TERMINAL_PROMPT=0"));
}

#[test]
fn timeout_kills_the_whole_process_group() {
    let d = tmp();
    let mut b = echo_bridge(&d);
    let marker = d.join("grandchild-survived");
    b.git = script(
        &d,
        "slow-git",
        &format!("(sleep 2; touch '{}') &\nsleep 30", marker.display()),
    );
    b.timeout = Duration::from_millis(300);
    let start = Instant::now();
    let o = b.run(&Request::Git { cwd: d.clone(), args: vec!["--version".into()] }).unwrap();
    assert!(o.timed_out);
    assert_eq!(o.code, None);
    assert!(start.elapsed() < Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!marker.exists(), "grandchild outlived the timeout");
}

#[test]
fn output_is_capped_at_1_mib_and_child_still_finishes() {
    let d = tmp();
    let mut b = echo_bridge(&d);
    b.git = script(&d, "loud-git", "head -c 3000000 /dev/zero | tr '\\0' 'a'; echo done >&2");
    let o = b.run(&Request::Git { cwd: d.clone(), args: vec!["--version".into()] }).unwrap();
    assert_eq!(o.stdout.len(), OUTPUT_CAP);
    assert!(o.truncated);
    assert_eq!(o.code, Some(0));
    assert_eq!(o.stderr, "done\n");
}

#[test]
fn spawn_failure_is_an_error() {
    let d = tmp();
    let b = Bridge::new(d.join("missing"));
    assert!(matches!(b.run(&Request::Doctor), Err(BridgeError::Spawn(_))));
}

#[test]
fn kb_line_regex_matches_design_4_1() {
    let hit = parse_kb_line("kb-0a1b2c decision Use Tauri v2 for the shell").unwrap();
    assert_eq!((hit.id.as_str(), hit.kind.as_str(), hit.title.as_str()), ("kb-0a1b2c", "decision", "Use Tauri v2 for the shell"));
    for kind in ["decision", "fact", "lesson", "delivery", "pitfall"] {
        assert!(parse_kb_line(&format!("kb-ffffff\t{kind}  t")).is_some(), "{kind}");
    }
    for bad in [
        "",
        "kb-0a1b2 decision short id",
        "kb-0A1B2C decision upper hex",
        "kb-0a1b2cdecision no space",
        "kb-0a1b2c rumor unknown kind",
        "kb-0a1b2c decision",
        "kb-0a1b2c decision   ",
        " kb-0a1b2c decision leading space",
        "xx-0a1b2c fact wrong prefix",
    ] {
        assert!(parse_kb_line(bad).is_none(), "{bad:?}");
    }
    let hits = parse_kb_hits("noise\nkb-000001 fact a\n\nkb-000002 lesson b\n");
    assert_eq!(hits.len(), 2);
    assert!(parse_kb_hits("").is_empty());
}

// ---- preflight ----

struct Env {
    allow: Allowlist,
    bridge: Bridge,
    bin: PathBuf,
    claude: PathBuf,
}

const DOCTOR_OK: &str = "echo 'PASS  dependency: git'\necho 'PASS  hook registered: PreToolUse'\necho 'PASS  hook registered: PostToolUse'\necho 'WARN  OS sandbox is off (x)'\nexit 0";

fn env_with(doctor: &str, with_workflow: bool) -> Env {
    let home = tmp();
    fs::create_dir_all(home.join(".claude/workflows")).unwrap();
    fs::create_dir_all(home.join("appdata")).unwrap();
    if with_workflow {
        fs::write(home.join(".claude/workflows/vajra-deliver.js"), "").unwrap();
    }
    let allow = Allowlist::new(&home.join(".claude"), &home.join("appdata")).unwrap();
    let scripts = home.join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    script(&scripts, "vajra-doctor.sh", doctor);
    let bin = home.join("bin");
    fs::create_dir_all(&bin).unwrap();
    for t in ["git", "python3", "jq", "openssl"] {
        script(&bin, t, "true");
    }
    let claude = script(&bin, "claude", "echo '2.1.0 (Claude Code)'");
    Env { allow, bridge: Bridge::new(scripts), bin, claude }
}

fn get<'a>(rows: &'a [Row], name: &str) -> &'a Row {
    rows.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no row {name}"))
}

#[test]
fn preflight_all_green() {
    let e = env_with(DOCTOR_OK, true);
    let rows = preflight::run(&e.allow, &e.bridge, std::slice::from_ref(&e.bin));
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["claude", "vajra doctor", "hooks registered", "sandbox", "git", "python3", "jq", "openssl", "workflow"]);
    assert!(start_enabled(&rows), "{rows:?}");
    assert_eq!(get(&rows, "claude").detail, "2.1.0 (Claude Code)");
    assert_eq!(get(&rows, "sandbox").detail, "off");
    assert!(get(&rows, "sandbox").ok);
    assert_eq!(get(&rows, "hooks registered").detail, "yes (2 events)");
    assert!(rows.iter().all(|r| r.fix.is_empty()));
}

#[test]
fn preflight_missing_claude_blocks_start_with_fix() {
    let e = env_with(DOCTOR_OK, true);
    fs::remove_file(&e.claude).unwrap();
    let rows = preflight::run(&e.allow, &e.bridge, std::slice::from_ref(&e.bin));
    assert!(!start_enabled(&rows));
    let r = get(&rows, "claude");
    assert!(!r.ok);
    assert!(r.fix.starts_with("Install Claude Code"));
}

#[test]
fn preflight_doctor_fail_lists_failures() {
    let e = env_with("echo 'FAIL  integrity: manifest'\nexit 1", true);
    let rows = preflight::run(&e.allow, &e.bridge, std::slice::from_ref(&e.bin));
    let d = get(&rows, "vajra doctor");
    assert!(!d.ok);
    assert!(d.detail.contains("integrity: manifest"));
    assert!(!d.fix.is_empty());
    assert!(!get(&rows, "hooks registered").ok);
    assert_eq!(get(&rows, "sandbox").detail, "unknown");
}

#[test]
fn preflight_missing_workflow_shows_install_then_doctor() {
    let e = env_with(DOCTOR_OK, false);
    let rows = preflight::run(&e.allow, &e.bridge, std::slice::from_ref(&e.bin));
    let w = get(&rows, "workflow");
    assert!(!w.ok);
    assert!(w.detail.contains("vajra-deliver.js"));
    assert_eq!(w.fix, "bash ~/.claude/skills/vajra/scripts/install.sh then /vajra doctor");
}

#[test]
fn preflight_missing_tool_row_fails() {
    let e = env_with(DOCTOR_OK, true);
    fs::remove_file(e.bin.join("jq")).unwrap();
    let rows = preflight::run(&e.allow, &e.bridge, std::slice::from_ref(&e.bin));
    assert!(!get(&rows, "jq").ok);
    assert_eq!(get(&rows, "jq").fix, "brew install jq");
    assert!(!start_enabled(&rows));
}

/// The real checker, not a fake python. Skipped on a base without
/// `check-evidence` (it arrives with the M1 engine).
#[test]
fn check_evidence_runs_against_the_real_vajra_tasks() {
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
    let src = fs::read_to_string(scripts.join("vajra_tasks.py")).unwrap();
    if !src.contains("def cmd_check_evidence") {
        eprintln!("skipped: this tree's vajra_tasks.py has no check-evidence");
        return;
    }
    let d = tmp();
    let ev = d.join("evidence.json");
    fs::write(&ev, "{}").unwrap();
    let b = Bridge::new(scripts);
    let o = b.run(&Request::CheckEvidence { path: ev, head: None, archived: None, repo: Some(d.clone()) }).unwrap();
    let v: serde_json::Value = serde_json::from_str(o.stdout.trim()).unwrap_or_else(|_| panic!("{} / {}", o.stdout, o.stderr));
    assert_eq!(o.code, Some(0), "{}", o.stderr);
    assert_eq!(v["green"], false);
    assert!(v["reasons"].as_array().is_some_and(|r| !r.is_empty()));
}

/// #13 item 6: an archived bundle is checked with `--archived <slug> --head <40-hex>`; the script refuses anything else
/// (exit 2), so the bridge refuses it before spawning instead of learning from the exit code.
#[test]
fn check_evidence_archived_needs_a_full_40_hex_head() {
    let d = tmp();
    let b = echo_bridge(&d);
    let ev = |head: Option<&str>| Request::CheckEvidence {
        path: "/r/c1/evidence/e1/evidence.json".into(), head: head.map(Into::into), archived: Some("c1".into()), repo: None,
    };
    for bad in [None, Some("abc1234"), Some("ea73cb05d639b945e21c667e17103e56404861a")] {
        assert!(matches!(b.run(&ev(bad)), Err(BridgeError::Invalid(_))), "{bad:?}");
    }
    let h40 = "ea73cb05d639b945e21c667e17103e56404861a8";
    assert!(b.run(&ev(Some(h40))).is_ok());
}
