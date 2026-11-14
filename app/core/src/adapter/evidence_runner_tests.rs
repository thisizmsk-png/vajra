//! The runner's profile, and its real behaviour under `/usr/bin/sandbox-exec` (S12, the parts #214's spike didn't
//! cover: pytest, real test runs, the key probe inside the profile; and S9's git). Every path is a temp dir: a fake
//! HOME with a fake Vajra state dir and a fake key file. The owner's real `~/.claude`, data dir and key are never
//! named or read. A test whose tool is missing on the machine prints `skipped: <tool>` and passes.

use super::*;
use std::fs;
use std::process::Command;

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("vajra-runner-{tag}-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        for sub in ["home/.claude/vajra", "data", "work", "tmp"] {
            fs::create_dir_all(d.join(sub)).unwrap();
        }
        // A fake key: the probe and the denies must keep every command from it.
        fs::write(d.join("home/.claude/vajra/.hmac-key"), "test-key-not-real").unwrap();
        Self(d.canonicalize().unwrap())
    }
    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
    fn profile(&self, network: bool) -> Profile {
        let p = self.fresh(network, "tmp");
        p.prepare().unwrap();
        p
    }
    /// A profile whose tmp dir (`name`) doesn't exist yet: what `run` needs, since it refuses a tmp that already does.
    fn fresh(&self, network: bool, name: &str) -> Profile {
        Profile { worktree: self.0.join("work"), tmp: self.0.join(name), deny: deny_dirs(&self.home(), &self.0.join("data")), network }
    }
    fn state(&self) -> PathBuf {
        self.home().join(".claude/vajra")
    }
    /// `sh -c script` in the worktree under the profile, with HOME, TMPDIR and the caches set like a run.
    fn run(&self, p: &Profile, script: &str, extra: &[(&str, PathBuf)]) -> std::process::Output {
        let mut c = Command::new(SANDBOX_EXEC);
        c.args(p.argv("/bin/sh", &["-c".into(), script.into()]).unwrap()).current_dir(&p.worktree).env("HOME", self.home()).env("TMPDIR", &p.tmp);
        for (k, v) in cache_env(&p.tmp) {
            c.env(k, v);
        }
        for (k, v) in extra {
            c.env(k, v);
        }
        c.output().unwrap()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn macos() -> bool {
    Path::new(SANDBOX_EXEC).is_file()
}

fn have(tool: &str) -> bool {
    let ok = Command::new("/bin/sh").args(["-c", &format!("command -v {tool}")]).output().is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("skipped: {tool}");
    }
    ok
}

fn text(o: &std::process::Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn the_profile_denies_the_vajra_dirs_last_and_limits_writes() {
    let t = Tmp::new("sbpl");
    let p = t.profile(false).sbpl().unwrap();
    let deny_at = p.rfind("(deny file-read* file-write*").unwrap();
    assert!(deny_at > p.find("(allow file-write*").unwrap(), "the deny comes last: {p}");
    assert!(p[deny_at..].contains(&format!("(subpath \"{}\")", t.home().join(".claude/vajra").display())));
    assert!(p[deny_at..].contains(&format!("(subpath \"{}\")", t.0.join("data").display())));
    assert!(p.contains("(deny file-write*)\n"));
    assert!(p.contains("(deny network-outbound)"));
    // Deny-default (#234 follow-up): nothing is allowed that no rule names.
    assert!(p.starts_with("(version 1)\n(deny default)\n") && !p.contains("(allow default)"), "{p}");
    let net = t.profile(true).sbpl().unwrap();
    assert!(!net.contains("(deny network-outbound)"), "a network seat keeps the network");
    for text in [&p, &net] {
        // Either setting: no unix socket beyond the resolver's and the run's own (M2), and the Mach denies (M1).
        assert!(text.lines().filter(|l| l.starts_with("(allow")).all(|l| !l.contains("unix-socket")), "{text}");
        assert!(text.contains("(literal \"/private/var/run/mDNSResponder\")"), "{text}");
        assert!(text.contains("(deny mach-lookup (global-name \"com.apple.coreservices.launchservicesd\")"), "{text}");
        assert!(text.contains("(deny job-creation)"), "{text}");
        // The Mach and network rules come before the final deny, which still comes last.
        assert!(text.rfind("(deny file-read* file-write*").unwrap() > text.rfind("(allow network-outbound").unwrap());
    }
}

/// bhishma's ruling B on #250: the trust-evaluation agent is allowed by its exact name only with a network seat. Either
/// way the security server's denies and the Keychains read deny stay, and the three other denials the https fetch
/// printed (the dtrace helper, the notify shared memory, the kernel-control socket) stay denied.
#[test]
fn the_trust_evaluation_agent_is_allowed_only_with_a_network_seat_and_keychain_denies_stay() {
    let t = Tmp::new("seat-mach");
    let agent = "(global-name \"com.apple.TrustEvaluationAgent\")";
    let (off, on) = (t.profile(false).sbpl().unwrap(), t.profile(true).sbpl().unwrap());
    assert!(!off.contains("TrustEvaluationAgent"), "no seat, no trust agent: {off}");
    assert_eq!(on.matches("TrustEvaluationAgent").count(), 1, "{on}");
    assert!(on.contains(&format!("(allow mach-lookup {agent})\n")), "exact name, not a prefix: {on}");
    let keychains = format!("(subpath \"{}\")", t.home().join("Library/Keychains").display());
    for text in [&off, &on] {
        let deny = text.lines().find(|l| l.starts_with("(deny mach-lookup")).unwrap();
        for name in ["(global-name \"com.apple.SecurityServer\")", "(global-name-prefix \"com.apple.securityd\")", "(global-name \"com.apple.secd\")"] {
            assert!(deny.contains(name), "{name} stays denied: {text}");
        }
        let last = text.rfind("(deny file-read* file-write*").unwrap();
        assert!(text[last..].contains(&keychains), "the Keychains read deny stays last: {text}");
        let allows: Vec<&str> = text.lines().filter(|l| l.starts_with("(allow")).collect();
        for kept in ["dtracehelper", "ipc-posix-shm", "system-socket"] {
            assert!(allows.iter().all(|l| !l.contains(kept)), "{kept} stays denied: {text}");
        }
    }
}

#[test]
fn a_path_that_could_break_out_of_the_profile_text_is_refused() {
    let t = Tmp::new("lit");
    for bad in ["relative/dir", "/x\"))(allow file-read* (subpath \"/", "/x\n(allow default)"] {
        let mut p = t.profile(false);
        p.worktree = PathBuf::from(bad);
        let r = p.sbpl();
        // A quote is escaped, never closes the literal; a relative or control-character path is refused.
        if let Ok(text) = r {
            assert!(!text.contains("(allow file-read* (subpath \"/\""), "{text}");
            assert!(text.contains("\\\""), "{text}");
        }
    }
    let mut p = t.profile(false);
    p.deny.clear();
    assert!(p.sbpl().is_err(), "no deny list, no profile");
}

#[test]
fn the_key_probe_passes_only_when_the_os_refuses_the_state_dir() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("probe");
    let state = t.home().join(".claude/vajra");
    assert!(key_probe(&t.profile(false), &state), "the profile refuses the state dir");
    // A profile that doesn't deny it fails the probe.
    let mut open = t.profile(false);
    open.deny = vec![t.0.join("nothing-here")];
    assert!(!key_probe(&open, &state));
    // No state dir: not proven.
    assert!(!key_probe(&t.profile(false), &t.0.join("absent")));
}

#[test]
fn a_command_can_neither_read_nor_write_the_vajra_dirs_nor_write_outside_its_worktree() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("deny");
    let p = t.profile(false);
    let key = t.home().join(".claude/vajra/.hmac-key");
    let cases = [
        format!("cat '{}'", key.display()),
        format!("ls '{}'", t.home().join(".claude/vajra").display()),
        format!("echo x > '{}'", t.home().join(".claude/vajra/planted").display()),
        format!("echo x > '{}'", t.0.join("data/planted").display()),
        format!("echo x > '{}'", t.home().join("outside").display()),
        format!("echo x > '{}'", t.0.join("planted-next-to-work").display()),
        // The shared caches in the home folder are read-only (M3): nothing planted there reaches a later build.
        "mkdir -p \"$HOME/.cache/x\"".to_string(),
        "mkdir -p \"$HOME/.cargo/registry/src/planted\"".to_string(),
        "mkdir -p \"$HOME/go/pkg/mod/planted\"".to_string(),
        "mkdir -p \"$HOME/.npm/planted\"".to_string(),
        // F1: the Codex sign-in folder, the Claude config file, cloud CLI credentials.
        "cat \"$HOME/.codex/auth.json\"".to_string(),
        "cat \"$HOME/.claude.json\"".to_string(),
        "ls \"$HOME/.config/gcloud\"".to_string(),
        "ls \"$HOME/.azure\"".to_string(),
    ];
    for d in [".codex", ".config/gcloud", ".azure"] {
        fs::create_dir_all(t.home().join(d)).unwrap();
    }
    fs::write(t.home().join(".codex/auth.json"), "fake").unwrap();
    fs::write(t.home().join(".claude.json"), "fake").unwrap();
    for c in &cases {
        let o = t.run(&p, c, &[]);
        assert!(!o.status.success(), "{c} must be refused: {}", text(&o));
        assert!(text(&o).contains("Operation not permitted"), "{c}: {}", text(&o));
    }
    assert!(!String::from_utf8_lossy(&t.run(&p, &format!("cat '{}' 2>/dev/null; true", key.display()), &[]).stdout).contains("test-key"));
    for p2 in ["home/.claude/vajra/planted", "data/planted", "home/outside", "planted-next-to-work", "home/.cache/x", "home/.cargo/registry/src/planted"] {
        assert!(!t.0.join(p2).exists(), "{p2}");
    }
    // The worktree, the tmp dir and the caches (in the tmp dir) are writable.
    let o = t.run(&p, "echo ok > in-work && echo ok > \"$TMPDIR/in-tmp\" && echo ok > \"$XDG_CACHE_HOME/f\" && echo ok > \"$GOCACHE/f\" && echo ok > \"$npm_config_cache/f\"", &[]);
    assert!(o.status.success(), "{}", text(&o));
    for (_, d) in cache_env(&p.tmp) {
        assert!(d.starts_with(&p.tmp), "every cache is in the run's tmp: {}", d.display());
    }
}

#[test]
fn a_loopback_server_binds_and_the_internet_is_refused_without_network() {
    if !macos() || !have("python3") {
        return;
    }
    let t = Tmp::new("net");
    let p = t.profile(false);
    let script = "python3 -I -c 'import socket\ns=socket.socket(); s.bind((\"127.0.0.1\",0)); s.listen(1); p=s.getsockname()[1]\nc=socket.create_connection((\"127.0.0.1\",p),2); a,_=s.accept(); c.send(b\"hi\"); assert a.recv(2)==b\"hi\"; print(\"loopback ok\")'";
    let o = t.run(&p, script, &[]);
    assert!(o.status.success() && text(&o).contains("loopback ok"), "{}", text(&o));
    let out = t.run(&p, "python3 -I -c 'import socket; socket.create_connection((\"1.1.1.1\",443),3); print(\"reached\")'", &[]);
    // stdout only: the traceback on stderr quotes the source line, print included.
    assert!(!out.status.success() && !String::from_utf8_lossy(&out.stdout).contains("reached"), "{}", text(&out));
    assert!(text(&out).contains("Operation not permitted"), "{}", text(&out));
}

#[test]
fn pytest_runs_a_real_suite_inside_the_runner() {
    if !macos() || !have("python3") {
        return;
    }
    let ok = Command::new("python3").args(["-I", "-c", "import pytest"]).output().is_ok_and(|o| o.status.success());
    if !ok {
        // pytest isn't a dependency of this repo, and no CI job installs it. Run locally with a throwaway venv's bin
        // first on PATH to cover it.
        return eprintln!("skipped: pytest is not installed for the python3 on PATH (not a repo dependency; no CI job installs it)");
    }
    let t = Tmp::new("pytest");
    fs::write(t.0.join("work/test_fixture.py"), "def test_adds():\n    assert 1 + 1 == 2\n\ndef test_writes(tmp_path):\n    (tmp_path / 'x').write_text('y')\n").unwrap();
    let o = t.run(&t.profile(false), "python3 -I -m pytest -q -p no:cacheprovider", &[]);
    assert!(o.status.success() && text(&o).contains("2 passed"), "{}", text(&o));
    // A failing test fails the run (the runner reports, never hides, a red suite).
    fs::write(t.0.join("work/test_fail.py"), "def test_no():\n    assert False\n").unwrap();
    assert!(!t.run(&t.profile(false), "python3 -I -m pytest -q -p no:cacheprovider", &[]).status.success());
}

#[test]
fn cargo_test_runs_a_real_crate_inside_the_runner() {
    if !macos() || !have("cargo") {
        return;
    }
    let t = Tmp::new("cargo");
    let w = t.0.join("work");
    fs::create_dir_all(w.join("src")).unwrap();
    fs::write(w.join("Cargo.toml"), "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    fs::write(w.join("src/lib.rs"), "#[test]\nfn adds() { assert_eq!(1 + 1, 2); }\n").unwrap();
    // The fake HOME has no toolchain: point rustup and cargo at the real ones, read-only (the build dir is the worktree).
    let real_home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
    let extra = [("RUSTUP_HOME", real_home.join(".rustup"))];
    let o = t.run(&t.profile(false), "cargo test --offline -q 2>&1", &extra);
    assert!(o.status.success() && text(&o).contains("1 passed"), "{}", text(&o));
}

#[test]
fn npm_test_runs_a_real_package_inside_the_runner() {
    if !macos() || !have("npm") || !have("node") {
        return;
    }
    let t = Tmp::new("npm");
    let w = t.0.join("work");
    fs::write(w.join("package.json"), r#"{"name":"fixture","version":"1.0.0","scripts":{"test":"node test.js"}}"#).unwrap();
    fs::write(w.join("test.js"), "const assert = require('assert'); assert.strictEqual(1 + 1, 2); console.log('npm fixture ok');\n").unwrap();
    let o = t.run(&t.profile(false), "npm test --silent --offline 2>&1", &[]);
    assert!(o.status.success() && text(&o).contains("npm fixture ok"), "{}", text(&o));
}

#[test]
fn go_test_runs_a_real_module_inside_the_runner() {
    if !macos() || !have("go") {
        return;
    }
    let t = Tmp::new("go");
    let w = t.0.join("work");
    fs::write(w.join("go.mod"), "module fixture\n\ngo 1.20\n").unwrap();
    fs::write(w.join("add_test.go"), "package fixture\n\nimport \"testing\"\n\nfunc TestAdds(t *testing.T) { if 1+1 != 2 { t.Fatal(\"no\") } }\n").unwrap();
    let extra = [("GOPATH", t.home().join("go")), ("GOFLAGS", PathBuf::from("-mod=mod")), ("GOTOOLCHAIN", PathBuf::from("local"))];
    let o = t.run(&t.profile(false), "go test ./... 2>&1", &extra);
    assert!(o.status.success() && text(&o).contains("ok"), "{}", text(&o));
}

#[test]
fn git_write_tree_works_with_the_private_object_dir_and_never_writes_the_shared_repo() {
    if !macos() || !have("git") {
        return;
    }
    let t = Tmp::new("git");
    // A repo outside the worktree (its objects are the shared ones), and a worktree of it.
    let repo = t.0.join("repo");
    let git = |dir: &Path, args: &[&str]| Command::new("git").args(args).current_dir(dir).env("GIT_CONFIG_NOSYSTEM", "1").env("HOME", t.home()).output().unwrap();
    fs::create_dir_all(&repo).unwrap();
    assert!(git(&repo, &["init", "-q", "-b", "main"]).status.success());
    fs::write(repo.join("a.txt"), "a\n").unwrap();
    assert!(git(&repo, &["add", "a.txt"]).status.success());
    assert!(git(&repo, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "a"]).status.success());
    let _ = fs::remove_dir_all(t.0.join("work"));
    assert!(git(&repo, &["worktree", "add", "-q", t.0.join("work").to_str().unwrap()]).status.success());
    let common = repo.join(".git");
    fs::copy(common.join("worktrees/work/index"), t.0.join("tmp/index")).unwrap();
    fs::create_dir_all(t.0.join("tmp/objects")).unwrap();
    let objects_before = fs::read_dir(common.join("objects")).unwrap().count();
    fs::write(t.0.join("work/b.txt"), "b\n").unwrap();
    // Plain write-tree under the runner can't write the shared objects (S9).
    let p = t.profile(false);
    let plain = t.run(&p, "git add b.txt && git write-tree", &[]);
    assert!(!plain.status.success(), "{}", text(&plain));
    // With the private object dir and the shared one as an alternate, it works.
    let env = git_env(&p.tmp, &common);
    let o = t.run(&p, "git add b.txt && git write-tree", &env);
    assert!(o.status.success(), "{}", text(&o));
    let tree = String::from_utf8_lossy(&o.stdout).trim().to_string();
    assert_eq!(tree.len(), 40, "{}", text(&o));
    assert_eq!(fs::read_dir(common.join("objects")).unwrap().count(), objects_before, "nothing written to the shared repo");
}

#[test]
fn the_walkthrough_base_is_the_canonical_private_form() {
    let t = Tmp::new("base");
    // The system temp dir through its link (/var or /tmp on macOS) resolves to /private, which is what sign accepts.
    let linked = std::env::temp_dir().join(t.0.file_name().unwrap()).join("work");
    let base = canonical_base(&linked).unwrap();
    assert_eq!(base, t.0.join("work"));
    if cfg!(target_os = "macos") {
        assert!(base.starts_with("/private"), "{}", base.display());
    }
    assert!(canonical_base(&t.0.join("absent")).is_none());
    fs::write(t.0.join("file"), "x").unwrap();
    assert!(canonical_base(&t.0.join("file")).is_none(), "a file isn't a base");
}

#[test]
fn a_nested_sandbox_fails_the_probe_closed() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    // The probe from inside another sandbox: macOS refuses the inner one, so the probe must not pass.
    let t = Tmp::new("nest");
    let inner = t.profile(false).argv("/bin/ls", &[t.home().join(".claude/vajra").to_string_lossy().into_owned()]).unwrap();
    let mut args = vec!["-p".to_string(), "(version 1)(allow default)".to_string(), "--".into(), SANDBOX_EXEC.into()];
    args.extend(inner);
    let o = Command::new(SANDBOX_EXEC).args(&args).output().unwrap();
    let out = text(&o);
    assert!(!o.status.success(), "{out}");
    assert!(!out.contains(".hmac-key"), "{out}");
}

const SLUG: &str = "deliver-20261113-0800";
const BUNDLE: &str = r#"{"slug":"deliver-20261113-0800","results":[],"git":{"head":"x"}}\n"#;

/// A fake `vajra-verify-work.sh` acting out 6i-e's contract. `--run-only` checks it was handed fd 3, runs each command
/// with fd 3 closed, then `printf`s `line` (a printf format: `\n` is the newline) to fd 3; `--sign` runs `sign_tail`.
/// The behaviour is baked into the file: the runner clears the env, so a test can't steer the fake through it.
fn fake(t: &Tmp, name: &str, line: &str, sign_tail: &str) -> PathBuf {
    let p = t.0.join(format!("tmp/{name}.sh"));
    let body = format!(
        r#"#!/bin/bash
mode="$1"; shift
if [ "$mode" = "--run-only" ]; then
  slug="$1"; shift
  {{ : >&3; }} 2>/dev/null || {{ echo "no fd 3" >&2; exit 2; }}
  rc=0
  for c in "$@"; do bash -c "$c" 3>&- || rc=1; done
  printf '{line}' >&3
  exec 3>&-
  exit $rc
fi
if [ "$mode" = "--sign" ]; then
  {sign_tail}
fi
exit 2
"#
    );
    fs::write(&p, body).unwrap();
    p
}

fn fake_sign(t: &Tmp, n: usize, code: u8, first_line: &str) -> PathBuf {
    let tail = if first_line.is_empty() { format!("exit {code}") } else { format!("echo '{first_line}' >&2; echo 'more detail' >&2; exit {code}") };
    fake(t, &format!("sign-{n}"), BUNDLE, &tail)
}

fn base(t: &Tmp) -> PathBuf {
    let b = t.0.join("work/walkthroughs");
    fs::create_dir_all(&b).unwrap();
    canonical_base(&b).unwrap()
}

fn evidence_run<'a>(script: &'a Path, base: &'a Path, cmds: &'a [String], common: &'a Path, home: &'a Path) -> EvidenceRun<'a> {
    // The fake state dir under the fake HOME (leaked: a few bytes per test, so the borrow outlives the helper).
    let state_dir: &'a Path = Box::leak(home.join(".claude/vajra").into_boxed_path());
    EvidenceRun { script, slug: SLUG, commands: cmds, base, git_common: common, home, state_dir, tool_path: &[], limit: Duration::from_secs(20) }
}

/// `run` under a fresh profile: each call gets its own tmp dir, as `run` requires.
fn go(t: &Tmp, r: &EvidenceRun) -> RunOutcome {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    run(&t.fresh(false, &format!("run-{}", N.fetch_add(1, std::sync::atomic::Ordering::SeqCst))), r)
}

#[test]
fn a_run_saves_exactly_the_fd_3_line_whatever_a_command_wrote_to_the_bundle_folder() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("run");
    let (script, base, home, common) = (fake(&t, "ok", BUNDLE, "exit 2"), base(&t), t.home(), t.0.join("repo.git"));
    // A command plants its own evidence.json in the bundle folder: the core's write replaces it with the fd-3 line.
    let plant = format!("mkdir -p '{0}/{SLUG}' && echo '{{\"slug\":\"{SLUG}\",\"forged\":true}}' > '{0}/{SLUG}/evidence.json'", base.display());
    let cmds = vec![plant, "echo ok".to_string()];
    let out = go(&t, &evidence_run(&script, &base, &cmds, &common, &home));
    let RunOutcome::Ran { passed, sha256 } = out else { panic!("{out:?}") };
    assert!(passed);
    let saved = fs::read(base.join(SLUG).join("evidence.json")).unwrap();
    assert_eq!(saved, br#"{"slug":"deliver-20261113-0800","results":[],"git":{"head":"x"}}"#);
    assert_eq!(crate::control::sha256::hex_digest(&saved), sha256);
    // A failing command: still a bundle, not passed.
    let cmds = vec!["exit 1".to_string()];
    let out = go(&t, &evidence_run(&script, &base, &cmds, &common, &home));
    assert!(matches!(out, RunOutcome::Ran { passed: false, .. }), "{out:?}");
}

#[test]
fn a_run_with_a_bad_evidence_line_fails_and_saves_nothing() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("badline");
    let (base, home, common) = (base(&t), t.home(), t.0.join("repo.git"));
    let cmds = vec!["true".to_string()];
    for (n, (line, why)) in [
        ("", "no line"),
        (r#"{"slug":"deliver-20261113-0800"}"#, "no newline"),
        (r#"{"slug":"deliver-20261113-0800"}\n{"slug":"deliver-20261113-0800"}\n"#, "two lines"),
        (r#"{"slug":"deliver-20261113-0800"}\n\n"#, "a trailing empty line"),
        ("not json\\n", "not JSON"),
        (r#"{"slug":"deliver-other"}\n"#, "another slug"),
        (r#"{"slug":"deliver-20261113-0800","hmac":"ab"}\n"#, "already signed"),
    ]
    .into_iter()
    .enumerate()
    {
        let script = fake(&t, &format!("bad-{n}"), line, "exit 2");
        let out = go(&t, &evidence_run(&script, &base, &cmds, &common, &home));
        assert!(matches!(out, RunOutcome::Failed(_)), "{why}: {out:?}");
        assert!(!base.join(SLUG).join("evidence.json").exists(), "{why}");
    }
    // A slug that isn't a stream id, or no commands: refused before anything starts.
    let script = fake(&t, "ok", BUNDLE, "exit 2");
    let mut r = evidence_run(&script, &base, &cmds, &common, &home);
    r.slug = "../escape";
    assert!(matches!(go(&t, &r), RunOutcome::Failed(_)));
    assert!(matches!(go(&t, &evidence_run(&script, &base, &[], &common, &home)), RunOutcome::Failed(_)));
}

#[test]
fn a_run_past_its_limit_is_killed_and_fails() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("limit");
    let (script, base, home, common) = (fake(&t, "slow", BUNDLE, "exit 2"), base(&t), t.home(), t.0.join("repo.git"));
    let cmds = vec!["sleep 30".to_string()];
    let mut r = evidence_run(&script, &base, &cmds, &common, &home);
    r.limit = Duration::from_millis(500);
    let start = std::time::Instant::now();
    assert_eq!(go(&t, &r), RunOutcome::Failed("the evidence check ran past its time limit".into()));
    assert!(start.elapsed() < Duration::from_secs(10), "the whole group is killed, not waited on");
    assert!(!base.join(SLUG).join("evidence.json").exists());
}

#[test]
fn a_run_and_a_sign_refuse_a_walkthrough_base_through_a_link() {
    let t = Tmp::new("link");
    let (script, home, common) = (fake_sign(&t, 0, 0, ""), t.home(), t.0.join("repo.git"));
    let real = base(&t);
    let link = t.0.join("tmp/base-link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let cmds = vec!["true".to_string()];
    assert!(matches!(go(&t, &evidence_run(&script, &link, &cmds, &common, &home)), RunOutcome::Failed(_)));
    assert!(matches!(sign(&script, SLUG, &"a".repeat(64), &link, &t.0.join("work"), &home), SignOutcome::Unknown(_)));
}

#[test]
fn sign_maps_each_contract_answer_and_fails_closed_on_anything_else() {
    let t = Tmp::new("sign");
    let (base, home, work) = (base(&t), t.home(), t.0.join("work"));
    let sha = "a".repeat(64);
    let known: Vec<(u8, String, SignOutcome)> = vec![
        (0, String::new(), SignOutcome::Signed),
        (4, "sign refused: no usable key".into(), SignOutcome::NoKey),
        (3, "sign refused: sha256 mismatch".into(), SignOutcome::Refused("sha256 mismatch".into())),
        (3, "sign refused: head mismatch".into(), SignOutcome::Refused("head mismatch".into())),
        (3, "sign refused: already signed".into(), SignOutcome::Refused("already signed".into())),
        (3, format!("sign refused: not a bundle for {SLUG}"), SignOutcome::Refused(format!("not a bundle for {SLUG}"))),
        (3, "sign refused: no evidence.json".into(), SignOutcome::Refused("no evidence.json".into())),
        (3, "sign refused: not JSON".into(), SignOutcome::Refused("not JSON".into())),
        (3, "sign refused: a symlinked path".into(), SignOutcome::Refused("a symlinked path".into())),
    ];
    for (n, (code, first, want)) in known.into_iter().enumerate() {
        let script = fake_sign(&t, n, code, &first);
        assert_eq!(sign(&script, SLUG, &sha, &base, &work, &home), want, "{code} {first}");
    }
    let unknown: [(u8, &str); 6] = [
        (3, "sign refused: some path leaked here"),
        (3, "sign refused: not a bundle for deliver-other"),
        (3, "something else"),
        (3, ""),
        (1, "sign refused: sha256 mismatch"),
        (9, ""),
    ];
    for (n, (code, first)) in unknown.into_iter().enumerate() {
        let script = fake_sign(&t, 100 + n, code, first);
        assert!(matches!(sign(&script, SLUG, &sha, &base, &work, &home), SignOutcome::Unknown(_)), "{code} {first}");
    }
}

fn script(t: &Tmp, name: &str, body: &str) -> PathBuf {
    let p = t.0.join(format!("tmp/{name}.sh"));
    fs::write(&p, format!("#!/bin/bash\n{body}\n")).unwrap();
    p
}

/// M1: a command can't reach a Mach service that starts or reaches a process outside the sandbox. Each lookup goes
/// through `bootstrap_look_up` from python's ctypes: nothing is opened, sent or read, and no Keychain is queried. A name
/// counts only if it answers outside the sandbox; the test needs the three main ones to.
#[test]
fn a_command_can_reach_no_mach_service_that_starts_an_unsandboxed_process() {
    if !macos() || !have("python3") {
        return eprintln!("skipped: sandbox-exec or python3");
    }
    let t = Tmp::new("mach");
    let probe = t.0.join("lookup.py");
    fs::write(
        &probe,
        "import ctypes,sys\nlib=ctypes.CDLL(None)\nbp=ctypes.c_uint.in_dll(lib,'bootstrap_port')\nf=lib.bootstrap_look_up\nf.argtypes=[ctypes.c_uint,ctypes.c_char_p,ctypes.POINTER(ctypes.c_uint)]\nfor n in sys.argv[1:]:\n    p=ctypes.c_uint(0)\n    print(n, f(bp.value,n.encode(),ctypes.byref(p)))\n",
    )
    .unwrap();
    let names = [
        "com.apple.coreservices.launchservicesd",
        "com.apple.lsd.mapdb",
        "com.apple.lsd.modifydb",
        "com.apple.coreservices.appleevents",
        "com.apple.SecurityServer",
        "com.apple.securityd.xpc",
        "com.apple.secd",
        "com.apple.pasteboard.1",
    ];
    let answers = |out: &str| -> Vec<(String, bool)> { out.lines().filter_map(|l| l.rsplit_once(' ').map(|(n, r)| (n.to_string(), r.trim() == "0"))).collect() };
    let args = names.join(" ");
    let outside = answers(&String::from_utf8_lossy(&Command::new("python3").arg(&probe).args(names).output().unwrap().stdout));
    let reachable: Vec<&String> = outside.iter().filter(|(_, ok)| *ok).map(|(n, _)| n).collect();
    for main in ["com.apple.coreservices.launchservicesd", "com.apple.coreservices.appleevents", "com.apple.SecurityServer"] {
        if !reachable.iter().any(|n| *n == main) {
            return eprintln!("skipped: {main} doesn't answer outside the sandbox here");
        }
    }
    let p = t.profile(false);
    let o = t.run(&p, &format!("python3 '{}' {args}", probe.display()), &[]);
    let inside = answers(&text(&o));
    for n in &reachable {
        assert_eq!(inside.iter().find(|(m, _)| m == *n).map(|(_, ok)| *ok), Some(false), "{n} answered inside: {}", text(&o));
    }
    // A service the tools need still answers: the allow-list works.
    let o = t.run(&p, &format!("python3 '{}' com.apple.system.notification_center", probe.display()), &[]);
    assert_eq!(answers(&text(&o)), vec![("com.apple.system.notification_center".to_string(), true)], "{}", text(&o));
    // Deny-default: a service on neither list (the window server, the Dock, privacy consent) answers outside but not
    // inside, so a service nobody thought to deny stays shut.
    let unlisted = ["com.apple.windowserver.active", "com.apple.dock.server", "com.apple.tccd"];
    let outside = answers(&String::from_utf8_lossy(&Command::new("python3").arg(&probe).args(unlisted).output().unwrap().stdout));
    let o = t.run(&p, &format!("python3 '{}' {}", probe.display(), unlisted.join(" ")), &[]);
    let inside = answers(&text(&o));
    for (n, _) in outside.iter().filter(|(_, ok)| *ok) {
        assert_eq!(inside.iter().find(|(m, _)| m == n).map(|(_, ok)| *ok), Some(false), "unlisted {n} answered inside: {}", text(&o));
    }
    // LaunchServices names no app from inside (the launch-time probe's check), but does outside.
    let front = |o: &std::process::Output| String::from_utf8_lossy(&o.stdout).contains("ASN:");
    if front(&Command::new("/usr/bin/lsappinfo").arg("front").output().unwrap()) {
        assert!(!front(&t.run(&p, "/usr/bin/lsappinfo front", &[])));
    }
}

/// Deny-default still lets a network seat do what a check needs: resolve a name and fetch over HTTPS with certificate
/// trust (curl's TLS goes through the trust daemons on the allow-list). Skipped when the Mac itself is offline.
#[test]
fn a_network_seat_resolves_and_fetches_over_https_under_deny_default() {
    if !macos() || !have("curl") {
        return eprintln!("skipped: sandbox-exec or curl");
    }
    let get = "/usr/bin/curl -sS -o /dev/null --max-time 15 https://example.com/";
    if !Command::new("/bin/sh").args(["-c", get]).output().is_ok_and(|o| o.status.success()) {
        return eprintln!("skipped: no internet here");
    }
    let t = Tmp::new("netseat");
    // curl is exec'd by the shell, so it keeps the pid the shell prints: on a failure the sandbox's own denial lines for
    // that pid are read from the system log and shown, naming exactly what the profile refused (it failed once on a CI
    // image and never locally). Strict either way: no skip, no retry.
    let o = t.run(&t.profile(true), &format!("echo VAJRA_PID=$$; exec {get}"), &[]);
    if !o.status.success() {
        let out = text(&o);
        let pid = out.lines().find_map(|l| l.strip_prefix("VAJRA_PID=")).unwrap_or("?").trim().to_string();
        std::thread::sleep(std::time::Duration::from_secs(2)); // the kernel's sandbox lines reach the log a moment later
        let log = Command::new("/usr/bin/log").args(["show", "--style", "compact", "--last", "3m", "--predicate", "eventMessage CONTAINS \"deny(\""]).output();
        let denials: Vec<String> = log
            .map(|l| String::from_utf8_lossy(&l.stdout).lines().filter(|l| l.contains(&format!("({pid})"))).map(str::to_string).collect())
            .unwrap_or_default();
        panic!("a network seat couldn't fetch: {out}\nsandbox denials for pid {pid} ({}):\n{}", denials.len(), denials.join("\n"));
    }
    // And without one, the same fetch is refused.
    assert!(!t.run(&t.profile(false), get, &[]).status.success());
}

/// M1: launchd starts no job for a sandboxed caller. The job would only run `/usr/bin/true`, and is removed either way.
#[test]
fn launchd_starts_no_job_for_a_command() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("launchd");
    let label = format!("com.vajra.test.runner-probe.{}", std::process::id());
    let o = t.run(&t.profile(false), &format!("/bin/launchctl submit -l {label} -- /usr/bin/true"), &[]);
    let listed = Command::new("/bin/launchctl").args(["list", &label]).output().unwrap().status.success();
    let _ = Command::new("/bin/launchctl").args(["remove", &label]).output();
    assert!(!o.status.success() && !listed, "launchctl submit started a job: {}", text(&o));
}

/// M2: a command reaches the system resolver and sockets of its own, never another daemon's socket.
#[test]
fn a_command_reaches_only_the_resolver_and_its_own_unix_sockets() {
    if !macos() || !have("python3") {
        return eprintln!("skipped: sandbox-exec or python3");
    }
    let t = Tmp::new("unix");
    let outside = t.0.join("daemon");
    fs::create_dir_all(&outside).unwrap();
    let serve = "import socket,sys,os\np=sys.argv[1]\ns=socket.socket(socket.AF_UNIX)\ns.bind(p)\ns.listen(4)\nprint('up',flush=True)\nwhile True:\n    c,_=s.accept(); c.sendall(b'hi'); c.close()\n";
    let connect = "import socket,sys\ns=socket.socket(socket.AF_UNIX)\ntry:\n    s.connect(sys.argv[1]); print('got', s.recv(2))\nexcept OSError as e:\n    print('refused', e.errno)\n";
    fs::write(t.0.join("serve.py"), serve).unwrap();
    fs::write(t.0.join("connect.py"), connect).unwrap();
    for network in [false, true] {
        let p = t.fresh(network, if network { "tmp-net" } else { "tmp-off" });
        p.prepare().unwrap();
        for (sock, reachable) in [(outside.join(format!("d{network}")), false), (p.tmp.join("s"), true), (p.worktree.join(format!("w{network}")), true)] {
            let mut srv = Command::new("python3").arg(t.0.join("serve.py")).arg(&sock).stdout(std::process::Stdio::piped()).spawn().unwrap();
            let mut line = String::new();
            std::io::BufRead::read_line(&mut std::io::BufReader::new(srv.stdout.take().unwrap()), &mut line).unwrap();
            let o = t.run(&p, &format!("python3 '{}' '{}'", t.0.join("connect.py").display(), sock.display()), &[]);
            let _ = srv.kill();
            let _ = srv.wait();
            assert_eq!(text(&o).contains("got b'hi'"), reachable, "network {network}, {}: {}", sock.display(), text(&o));
        }
        // Names still resolve through the system resolver's socket, when they do outside.
        let lookup = "python3 -c \"import socket; print(socket.getaddrinfo('apple.com', 443)[0][4][0])\"";
        if Command::new("/bin/sh").args(["-c", lookup]).output().unwrap().status.success() {
            let o = t.run(&p, lookup, &[]);
            assert!(o.status.success(), "network {network}: a name lookup failed: {}", text(&o));
        }
    }
}

/// M4: a hard link to the key is a name no deny rule covers, so neither the probe nor a run starts with one.
#[test]
fn a_key_with_a_second_link_stops_the_probe_and_the_run() {
    let t = Tmp::new("hardlink");
    let state = t.state();
    assert!(crate::vajra::allowlist::key_has_one_link(&state));
    let key = state.join(crate::vajra::allowlist::DENIED_NAME);
    let link = t.0.join("work/innocent-name");
    fs::hard_link(&key, &link).unwrap();
    assert!(!crate::vajra::allowlist::key_has_one_link(&state));
    if macos() {
        assert!(!key_probe(&t.profile(false), &state));
    }
    let (script, base, home, common) = (fake(&t, "ok", BUNDLE, "exit 2"), base(&t), t.home(), t.0.join("repo.git"));
    let cmds = vec!["true".to_string()];
    assert!(matches!(go(&t, &evidence_run(&script, &base, &cmds, &common, &home)), RunOutcome::Failed(why) if why.contains("second name")));
    fs::remove_file(&link).unwrap();
    // A key that is a symlink fails as well; no key at all passes (signing then ends "no key").
    let real = t.0.join("data/real-key");
    fs::rename(&key, &real).unwrap();
    assert!(crate::vajra::allowlist::key_has_one_link(&state));
    std::os::unix::fs::symlink(&real, &key).unwrap();
    assert!(!crate::vajra::allowlist::key_has_one_link(&state));
}

/// L1: a write end held open past the script's exit ends the run soon after, Failed; so does an oversized line.
#[test]
fn a_held_open_or_oversized_evidence_channel_fails_fast() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("held");
    let (base, home, common) = (base(&t), t.home(), t.0.join("repo.git"));
    let cmds = vec!["true".to_string()];
    let held = script(&t, "held", &format!("printf '{BUNDLE}' >&3\n(sleep 30) &\nexit 0"));
    let start = std::time::Instant::now();
    assert_eq!(go(&t, &evidence_run(&held, &base, &cmds, &common, &home)), RunOutcome::Failed("the evidence channel stayed open after the script ended".into()));
    assert!(start.elapsed() < Duration::from_secs(10));
    let big = script(&t, "big", "head -c 1100000 /dev/zero | tr '\\0' a >&3\necho >&3\nexit 0");
    assert!(matches!(go(&t, &evidence_run(&big, &base, &cmds, &common, &home)), RunOutcome::Failed(_)));
    assert!(!base.join(SLUG).join("evidence.json").exists());
}

/// The #234 follow-up (before key A): a process another thread starts while the runner is between `pipe()` and the
/// close-on-exec flags never holds the evidence write end. A child that did would keep the channel open after the
/// script ends, and the run would fail. The racer goes through the app's spawn lock, so it waits out the window; the
/// control run starts a bare child in the same window, which does inherit the write end and fails the run.
///
/// It runs alone, in a fresh run of this test binary: in the shared one, other tests start children with a bare
/// `spawn` (test code is exempt from the lock), and one landing in this window would hold the channel too.
#[test]
fn a_process_started_during_the_evidence_pipe_window_never_holds_the_write_end() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let name = "adapter::evidence_runner::tests::spawn_race_body";
    let o = Command::new(std::env::current_exe().unwrap()).args(["--exact", name, "--ignored", "--nocapture", "--test-threads=1"]).env("VAJRA_SPAWN_RACE_BODY", "1").output().unwrap();
    let out = text(&o);
    assert!(o.status.success() && out.contains("1 passed"), "{out}");
}

#[test]
#[ignore = "run alone by a_process_started_during_the_evidence_pipe_window_never_holds_the_write_end"]
fn spawn_race_body() {
    if std::env::var_os("VAJRA_SPAWN_RACE_BODY").is_none() {
        return eprintln!("skipped: runs only from its parent test");
    }
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let t = Tmp::new("spawnrace");
    let (script, base, home, common) = (fake(&t, "ok", BUNDLE, "exit 2"), base(&t), t.home(), t.0.join("repo.git"));
    let cmds = vec!["true".to_string()];
    let kids: Arc<Mutex<Vec<std::process::Child>>> = Arc::default();
    let sleeper = || {
        let mut c = Command::new("/bin/sleep");
        c.arg("8").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        c
    };
    let hook = |f: Box<dyn Fn([libc::c_int; 2]) + Send>| *crate::spawnlock::PIPE_HOOK.lock().unwrap() = Some((std::thread::current().id(), f));
    // The routed racer, from another thread, while the runner holds the window.
    let (k, ran_early) = (kids.clone(), Arc::new(AtomicBool::new(false)));
    let early = ran_early.clone();
    let racer: Arc<Mutex<Option<std::thread::JoinHandle<()>>>> = Arc::default();
    let r = racer.clone();
    hook(Box::new(move |_| {
        let (k, e) = (k.clone(), early.clone());
        let started = Arc::new(AtomicBool::new(false));
        let s = started.clone();
        *r.lock().unwrap() = Some(std::thread::spawn(move || {
            let c = crate::spawnlock::spawn(&mut sleeper()).unwrap();
            s.store(true, Ordering::SeqCst);
            k.lock().unwrap().push(c);
        }));
        std::thread::sleep(Duration::from_millis(300));
        e.store(started.load(Ordering::SeqCst), Ordering::SeqCst);
    }));
    let out = go(&t, &evidence_run(&script, &base, &cmds, &common, &home));
    *crate::spawnlock::PIPE_HOOK.lock().unwrap() = None;
    racer.lock().unwrap().take().unwrap().join().unwrap();
    assert!(!ran_early.load(Ordering::SeqCst), "a routed spawn ran inside the evidence pipe window");
    assert!(matches!(out, RunOutcome::Ran { passed: true, .. }), "a racing child held the evidence channel: {out:?}");
    // The control: a bare spawn in the same window inherits the write end, so the channel stays open and the run fails.
    let k = kids.clone();
    hook(Box::new(move |_| {
        let k = k.clone();
        std::thread::spawn(move || k.lock().unwrap().push(sleeper().spawn().unwrap())).join().unwrap();
    }));
    let out = go(&t, &evidence_run(&script, &base, &cmds, &common, &home));
    *crate::spawnlock::PIPE_HOOK.lock().unwrap() = None;
    for mut c in kids.lock().unwrap().drain(..) {
        let _ = c.kill();
        let _ = c.wait();
    }
    assert_eq!(out, RunOutcome::Failed("the evidence channel stayed open after the script ended".into()), "control: a bare spawn in the window should hold the channel");
}

/// The script runs from its real place in the engine folder, which the profile denies: bash gets its text, never its
/// path. A command that rewrites the script's file mid-run changes nothing, since bash read it all before any command.
#[test]
fn the_engine_script_runs_from_its_denied_folder_and_a_rewrite_mid_run_changes_nothing() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("engine");
    let (base, home, common) = (base(&t), t.home(), t.0.join("repo.git"));
    let engine = home.join(".claude/skills/vajra/scripts");
    fs::create_dir_all(&engine).unwrap();
    let real = engine.join("vajra-verify-work.sh");
    fs::copy(fake(&t, "src", BUNDLE, "exit 2"), &real).unwrap();
    assert!(deny_dirs(&home, &t.0.join("data")).iter().any(|d| real.starts_with(d)), "the engine folder is denied");
    let cmds = vec!["true".to_string()];
    let out = go(&t, &evidence_run(&real, &base, &cmds, &common, &home));
    assert!(matches!(out, RunOutcome::Ran { passed: true, .. }), "{out:?}");
    // The same script in the worktree, where a command can write: the first command replaces it with one that would
    // print a forged passing line; the evidence is still the original's.
    let in_work = t.0.join("work/vajra-verify-work.sh");
    fs::copy(&real, &in_work).unwrap();
    let forged = format!("printf '#!/bin/bash\\nprintf %s >&3\\n' '{{\"slug\":\"{SLUG}\",\"forged\":true}}\\n' > '{}'", in_work.display());
    let cmds = vec![forged, "true".to_string()];
    let out = go(&t, &evidence_run(&in_work, &base, &cmds, &common, &home));
    assert!(matches!(out, RunOutcome::Ran { .. }), "{out:?}");
    assert!(fs::read_to_string(&in_work).unwrap().contains("forged"), "the command did rewrite the file");
    assert_eq!(fs::read(base.join(SLUG).join("evidence.json")).unwrap(), br#"{"slug":"deliver-20261113-0800","results":[],"git":{"head":"x"}}"#);
    // A command that swaps the bundle folder for a link elsewhere: the core writes nothing through it.
    fs::remove_dir_all(base.join(SLUG)).unwrap();
    let elsewhere = t.0.join("work/elsewhere");
    let swap = format!("mkdir -p '{0}' && ln -s '{0}' '{1}'", elsewhere.display(), base.join(SLUG).display());
    let out = go(&t, &evidence_run(&real, &base, &[swap], &common, &home));
    assert_eq!(out, RunOutcome::Failed("the evidence folder was replaced during the run".into()));
    assert!(!elsewhere.join("evidence.json").exists());
    // A script that is a link isn't run.
    let link = t.0.join("tmp/linked.sh");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(matches!(go(&t, &evidence_run(&link, &base, &["true".to_string()], &common, &home)), RunOutcome::Failed(_)));
}

/// The evidence write goes through the pinned bundle folder (#234 codex P1): a swap of the folder, or of a folder above
/// it, for a link, by a process that outlived the kill, can't send the core's write outside the folder it opened.
#[test]
fn the_evidence_write_follows_the_pinned_folder_not_a_path_swapped_after_it() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("pinned");
    let (script, base, home, common) = (fake(&t, "ok", BUNDLE, "exit 2"), base(&t), t.home(), t.0.join("repo.git"));
    let cmds = vec!["true".to_string()];
    let elsewhere = t.0.join("data/elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    // After the folder is opened: it is moved away and a link to elsewhere takes its name.
    let (slug_dir, moved, target) = (base.join(SLUG), base.join("moved"), elsewhere.clone());
    super::AFTER_PIN.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || {
            fs::rename(&slug_dir, &moved).unwrap();
            std::os::unix::fs::symlink(&target, &slug_dir).unwrap();
        }))
    });
    let out = go(&t, &evidence_run(&script, &base, &cmds, &common, &home));
    super::AFTER_PIN.with(|h| *h.borrow_mut() = None);
    assert!(matches!(out, RunOutcome::Ran { .. }), "{out:?}");
    assert!(!elsewhere.join("evidence.json").exists(), "the write followed the swapped name");
    assert_eq!(fs::read(base.join("moved/evidence.json")).unwrap(), br#"{"slug":"deliver-20261113-0800","results":[],"git":{"head":"x"}}"#);
    // A command swaps a folder above the bundle (the base itself) for a link: the walk refuses it.
    let real_base = t.0.join("work/walkthroughs-real");
    let swap = format!("mv '{0}' '{1}' && mkdir -p '{2}' && ln -s '{2}' '{0}'", base.display(), real_base.display(), elsewhere.display());
    let out = go(&t, &evidence_run(&script, &base, &[swap], &common, &home));
    assert_eq!(out, RunOutcome::Failed("the evidence folder was replaced during the run".into()));
    assert!(!elsewhere.join(SLUG).exists() && !elsewhere.join("evidence.json").exists());
}

/// A tool installed outside the system dirs (a rustup `cargo`, a version manager's `node`) is found through the run's
/// own environment when the user's PATH names its dir, and only then (#234 codex P2). Relative entries are dropped.
#[test]
fn a_run_finds_a_tool_on_the_users_path_outside_the_system_dirs() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("toolpath");
    let (script, base, home, common) = (fake(&t, "ok", BUNDLE, "exit 2"), base(&t), t.home(), t.0.join("repo.git"));
    let bin = home.join(".cargo/bin");
    fs::create_dir_all(&bin).unwrap();
    let tool = bin.join("vajra-fake-tool");
    fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    let cmds = vec!["vajra-fake-tool".to_string()];
    let path = vec![PathBuf::from("relative/bin"), bin.clone()];
    let mut r = evidence_run(&script, &base, &cmds, &common, &home);
    r.tool_path = &path;
    assert!(matches!(go(&t, &r), RunOutcome::Ran { passed: true, .. }), "found on the user's PATH");
    let r = evidence_run(&script, &base, &cmds, &common, &home);
    assert!(matches!(go(&t, &r), RunOutcome::Ran { passed: false, .. }), "not found without it");
    assert_eq!(run_path(&path), format!("{}:{}", bin.display(), SYSTEM_PATH.join(":")));
    assert_eq!(run_path(&[PathBuf::from("/a:b"), PathBuf::from("/usr/bin")]), SYSTEM_PATH.join(":"));
}

/// The engine script's own tools come only from the system dirs, while the check commands see the user's PATH
/// (bhishma F1 on 2e7e304f). A planted `bash` that reports every command passed, and a planted `jq` that prints a
/// forged bundle, both sit in a folder on the user's PATH: neither is used by the script.
#[test]
fn a_planted_bash_or_jq_on_the_users_path_never_runs_as_the_scripts_own() {
    if !macos() || !Path::new("/usr/bin/jq").is_file() {
        return eprintln!("skipped: sandbox-exec or the system jq");
    }
    let t = Tmp::new("planted");
    let (base, home, common) = (base(&t), t.home(), t.0.join("repo.git"));
    let bin = home.join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let plant = |name: &str, body: &str| {
        fs::write(bin.join(name), body).unwrap();
        fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    };
    plant("bash", "#!/bin/sh\nexit 0\n");
    plant("jq", &format!("#!/bin/sh\necho '{{\"slug\":\"{SLUG}\",\"forged\":true}}'\n"));
    plant("vajra-fake-tool", "#!/bin/sh\nexit 0\n");
    // The script builds its line with jq, like the engine's, and runs each command with bash.
    let script = script(
        &t,
        "jq-line",
        r#"shift; slug="$1"; shift
rc=0
for c in "$@"; do bash -c "$c" 3>&- || rc=1; done
jq -nc --arg s "$slug" '{slug:$s,results:[],git:{head:"x"}}' >&3
exec 3>&-
exit $rc"#,
    );
    let path = vec![bin.clone()];
    // The line is the system jq's, not the planted one's.
    let cmds = vec!["true".to_string()];
    let mut r = evidence_run(&script, &base, &cmds, &common, &home);
    r.tool_path = &path;
    assert!(matches!(go(&t, &r), RunOutcome::Ran { .. }));
    assert_eq!(fs::read(base.join(SLUG).join("evidence.json")).unwrap(), br#"{"slug":"deliver-20261113-0800","results":[],"git":{"head":"x"}}"#, "the planted jq wrote the line");
    // A failing command stays failed: the script's bash is /bin/bash, not the planted one.
    let cmds = vec!["exit 1".to_string()];
    let mut r = evidence_run(&script, &base, &cmds, &common, &home);
    r.tool_path = &path;
    assert!(matches!(go(&t, &r), RunOutcome::Ran { passed: false, .. }), "the planted bash ran the command");
    // The check commands themselves still find the user's tools.
    let cmds = vec!["vajra-fake-tool && [ \"$(command -v jq)\" = \"$HOME/.local/bin/jq\" ]".to_string()];
    let mut r = evidence_run(&script, &base, &cmds, &common, &home);
    r.tool_path = &path;
    assert!(matches!(go(&t, &r), RunOutcome::Ran { passed: true, .. }), "a check command sees the user's PATH");
}

/// The BASH_ENV file every check's bash sources is out of every check's reach (codex P1 on d85a04a2): a first command
/// that tries to rewrite it to `exit 0`, replace its folder or chmod it changes nothing, and a later failing command
/// still runs and fails. The folder is gone after the run.
#[test]
fn a_check_cannot_rewrite_the_startup_file_so_a_later_failing_check_still_fails() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("bashenv");
    let (script, base, home, common) = (fake(&t, "ok", BUNDLE, "exit 2"), base(&t), t.home(), t.0.join("repo.git"));
    let ran = t.0.join("work/later-ran");
    let cmds = vec![
        // Link, unlock, then rewrite: each must be refused, and the rewrite is what would pass the later check.
        "ln \"$BASH_ENV\" \"$PWD/linked-env\"; chmod 666 \"$BASH_ENV\"; chmod 777 \"$(dirname \"$BASH_ENV\")\"; printf 'exit 0\\n' > \"$BASH_ENV\"; true".to_string(),
        format!("touch '{}'; exit 1", ran.display()),
    ];
    let p = t.fresh(false, "run-bashenv");
    let env_dir = t.0.join("run-bashenv.bash-env");
    let out = run(&p, &evidence_run(&script, &base, &cmds, &common, &home));
    assert!(matches!(out, RunOutcome::Ran { passed: false, .. }), "the later failing check passed: {out:?}");
    assert!(ran.exists(), "the later check ran");
    assert!(!t.0.join("work/linked-env").exists(), "a hard link to the startup file was made");
    assert!(!env_dir.exists(), "the startup folder is removed after the run");
    // A startup folder already there (left by anything) stops the run.
    let p = t.fresh(false, "run-stale");
    fs::create_dir(t.0.join("run-stale.bash-env")).unwrap();
    assert!(matches!(run(&p, &evidence_run(&script, &base, &cmds, &common, &home)), RunOutcome::Failed(why) if why.contains("fresh")));
}

/// The startup folder's overlap check is exact only on resolved paths (codex P1 on d660bb62): a tmp named through an
/// alias (`/var` for `/private/var`) inside the worktree would put the startup folder in a writable root the check
/// can't see. Such a profile is refused before any command runs; so is a resolved tmp whose startup folder would sit
/// in the worktree.
#[test]
fn a_profile_whose_startup_folder_would_be_writable_through_an_alias_is_refused() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("alias");
    let (script, base, home, common) = (fake(&t, "ok", BUNDLE, "exit 2"), base(&t), t.home(), t.0.join("repo.git"));
    let Ok(alias) = t.0.strip_prefix("/private").map(|p| Path::new("/").join(p)) else { return eprintln!("skipped: temp dir has no /private alias") };
    assert_ne!(alias, t.0);
    assert_eq!(alias.canonicalize().unwrap(), t.0, "the alias names the same folder");
    fs::create_dir_all(t.0.join("work/node_modules")).unwrap();
    let ran = t.0.join("work/later-ran");
    let cmds = vec!["printf 'exit 0\\n' > \"$BASH_ENV\"; true".to_string(), format!("touch '{}'; exit 1", ran.display())];
    // The worktree resolved, the tmp under it through the alias.
    let p = Profile { worktree: t.0.join("work"), tmp: alias.join("work/node_modules/run"), deny: deny_dirs(&home, &t.0.join("data")), network: false };
    let out = run(&p, &evidence_run(&script, &base, &cmds, &common, &home));
    assert!(matches!(&out, RunOutcome::Failed(why) if why.contains("resolved")), "{out:?}");
    assert!(!ran.exists(), "no command ran");
    assert!(!t.0.join("work/node_modules/run.bash-env").exists());
    // The worktree through the alias: refused too.
    let p = Profile { worktree: alias.join("work"), tmp: t.0.join("run-a"), deny: deny_dirs(&home, &t.0.join("data")), network: false };
    assert!(matches!(run(&p, &evidence_run(&script, &base, &cmds, &common, &home)), RunOutcome::Failed(why) if why.contains("resolved")));
    // Both resolved, the tmp inside the worktree: the startup folder would be writable, so refused.
    let p = Profile { worktree: t.0.join("work"), tmp: t.0.join("work/node_modules/run-b"), deny: deny_dirs(&home, &t.0.join("data")), network: false };
    assert!(matches!(run(&p, &evidence_run(&script, &base, &cmds, &common, &home)), RunOutcome::Failed(why) if why.contains("writable")));
    assert!(!ran.exists() && !t.0.join("work/node_modules/run-b.bash-env").exists());
}

/// The run's tmp is made exclusively, never through a link, and pinned (codex P1 on 3dc1a7bf, bhishma Low): a link
/// to its parent put in its place before it's made, or swapped in right after, refuses the run, and no command runs.
/// The profile names only the literal tmp, so a link there can't add the folder it points at.
#[test]
fn a_tmp_swapped_for_a_link_while_it_is_made_refuses_the_run() {
    if !macos() {
        return eprintln!("skipped: sandbox-exec");
    }
    let t = Tmp::new("tmplink");
    let (script, base, home, common) = (fake(&t, "ok", BUNDLE, "exit 2"), base(&t), t.home(), t.0.join("repo.git"));
    let ran = t.0.join("work/ran");
    let cmds = vec![format!("touch '{}'", ran.display())];
    for stage in ["before-create", "after-create"] {
        let p = t.fresh(false, &format!("run-{stage}"));
        let (tmp, parent, want) = (p.tmp.clone(), t.0.clone(), stage.to_string());
        super::TMP_HOOK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move |at: &str| {
                if at == want {
                    let _ = fs::remove_dir(&tmp);
                    std::os::unix::fs::symlink(&parent, &tmp).unwrap();
                }
            }))
        });
        let out = run(&p, &evidence_run(&script, &base, &cmds, &common, &home));
        super::TMP_HOOK.with(|h| *h.borrow_mut() = None);
        assert!(matches!(&out, RunOutcome::Failed(why) if why.contains("already exists") || why.contains("replaced")), "{stage}: {out:?}");
        assert!(!ran.exists(), "{stage}: a command ran");
        assert!(!t.0.join("objects").exists() && !t.0.join("cache").exists(), "{stage}: folders were made through the link");
        assert!(!t.0.join(format!("run-{stage}.bash-env")).exists());
    }
    // The profile text names each writable root as given, never a resolved form of it.
    let link = t.0.join("tmp-as-link");
    std::os::unix::fs::symlink(&t.0, &link).unwrap();
    let p = Profile { worktree: t.0.join("work"), tmp: link, deny: deny_dirs(&home, &t.0.join("data")), network: false };
    let text = p.sbpl().unwrap();
    let allow = text.lines().find(|l| l.starts_with("(allow file-write*")).unwrap();
    assert!(!allow.contains(&format!("(subpath \"{}\")", t.0.display())), "{allow}");
}

/// A run's private git index starts as the worktree's own (codex P2 on f95df103): on a clean linked worktree, a clean
/// tracked file is neither deleted nor untracked, through the production `run()`.
#[test]
fn a_run_sees_a_clean_worktree_as_clean_in_git() {
    if !macos() || !have("git") {
        return eprintln!("skipped: sandbox-exec or git");
    }
    let t = Tmp::new("gitindex");
    let repo = t.0.join("repo");
    let git = |dir: &Path, args: &[&str]| Command::new("/usr/bin/git").args(args).current_dir(dir).env("GIT_CONFIG_NOSYSTEM", "1").env("HOME", t.home()).output().unwrap();
    fs::create_dir_all(&repo).unwrap();
    assert!(git(&repo, &["init", "-q", "-b", "main"]).status.success());
    fs::write(repo.join("README.md"), "one\ntwo\n").unwrap();
    assert!(git(&repo, &["add", "README.md"]).status.success());
    assert!(git(&repo, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "a"]).status.success());
    let _ = fs::remove_dir_all(t.0.join("work"));
    assert!(git(&repo, &["worktree", "add", "-q", t.0.join("work").to_str().unwrap()]).status.success());
    let (script, home, common) = (fake(&t, "ok", BUNDLE, "exit 2"), t.home(), repo.join(".git"));
    let walk_base = t.0.join("tmp/walkthroughs");
    fs::create_dir_all(&walk_base).unwrap();
    let cmds = vec!["git diff --exit-code HEAD -- README.md && test -z \"$(git ls-files --others)\" && git ls-files --error-unmatch README.md".to_string()];
    let out = go(&t, &evidence_run(&script, &walk_base, &cmds, &common, &home));
    assert!(matches!(out, RunOutcome::Ran { passed: true, .. }), "a clean tracked file read as changed: {out:?}");
    // Not in git at all: no index to copy, and the run still goes.
    let t2 = Tmp::new("noindex");
    let (script2, home2) = (fake(&t2, "ok", BUNDLE, "exit 2"), t2.home());
    let base2 = base(&t2);
    assert!(matches!(go(&t2, &evidence_run(&script2, &base2, &["true".to_string()], &t2.0.join("none.git"), &home2)), RunOutcome::Ran { passed: true, .. }));
}

/// The index the core copies comes only from the run's repository (bhishma F1 on b04316cb): a worktree whose `.git`
/// file points elsewhere still gets its own admin folder's index, a worktree of another repository is refused, and an
/// index that is a hard link to a file kept elsewhere is refused.
#[test]
fn the_copied_index_is_only_ever_the_run_repositorys_own() {
    if !macos() || !have("git") {
        return eprintln!("skipped: sandbox-exec or git");
    }
    let t = Tmp::new("indexsrc");
    let git = |dir: &Path, args: &[&str]| Command::new("/usr/bin/git").args(args).current_dir(dir).env("GIT_CONFIG_NOSYSTEM", "1").env("HOME", t.home()).output().unwrap();
    let mk_repo = |name: &str| {
        let repo = t.0.join(name);
        fs::create_dir_all(&repo).unwrap();
        assert!(git(&repo, &["init", "-q", "-b", "main"]).status.success());
        fs::write(repo.join("a.txt"), "a\n").unwrap();
        assert!(git(&repo, &["add", "a.txt"]).status.success());
        assert!(git(&repo, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "a"]).status.success());
        repo
    };
    let repo = mk_repo("repo");
    let other = mk_repo("other");
    let _ = fs::remove_dir_all(t.0.join("work"));
    assert!(git(&repo, &["worktree", "add", "-q", t.0.join("work").to_str().unwrap()]).status.success());
    let (script, home, common) = (fake(&t, "ok", BUNDLE, "exit 2"), t.home(), repo.join(".git"));
    let walk_base = t.0.join("tmp/walkthroughs");
    fs::create_dir_all(&walk_base).unwrap();
    // A fake gitdir elsewhere, holding a marked index, and the worktree's .git file pointed at it.
    let elsewhere = t.0.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join("index"), "PLANTED-INDEX").unwrap();
    let real_dotgit = fs::read_to_string(t.0.join("work/.git")).unwrap();
    fs::write(t.0.join("work/.git"), format!("gitdir: {}\n", elsewhere.display())).unwrap();
    let cmds = vec!["! grep -q PLANTED-INDEX \"$GIT_INDEX_FILE\" && test -s \"$GIT_INDEX_FILE\"".to_string()];
    let out = go(&t, &evidence_run(&script, &walk_base, &cmds, &common, &home));
    assert!(matches!(out, RunOutcome::Ran { passed: true, .. }), "the planted index was copied: {out:?}");
    fs::write(t.0.join("work/.git"), real_dotgit).unwrap();
    // The run's repository isn't the worktree's: refused, nothing runs.
    let out = go(&t, &evidence_run(&script, &walk_base, &cmds, &other.join(".git"), &home));
    assert_eq!(out, RunOutcome::Failed("the worktree's git folder isn't the run's repository".into()));
    // The admin folder's index replaced by a hard link to a file kept elsewhere: refused.
    let admin_index = common.join("worktrees/work/index");
    fs::remove_file(&admin_index).unwrap();
    fs::hard_link(elsewhere.join("index"), &admin_index).unwrap();
    let out = go(&t, &evidence_run(&script, &walk_base, &cmds, &common, &home));
    assert_eq!(out, RunOutcome::Failed("the worktree's git index isn't a plain file with one link".into()));
    // #234 follow-up: the index must be a regular file. A symlink (even to a real index), a directory and a FIFO are
    // each refused, and the FIFO doesn't hang the run (the open never blocks).
    let not_plain = RunOutcome::Failed("the worktree's git index isn't a plain file with one link".into());
    let real_index = t.0.join("real-index");
    fs::copy(repo.join(".git/index"), &real_index).unwrap();
    fs::remove_file(&admin_index).unwrap();
    std::os::unix::fs::symlink(&real_index, &admin_index).unwrap();
    assert_eq!(go(&t, &evidence_run(&script, &walk_base, &cmds, &common, &home)), not_plain, "a symlinked index");
    fs::remove_file(&admin_index).unwrap();
    fs::create_dir(&admin_index).unwrap();
    assert_eq!(go(&t, &evidence_run(&script, &walk_base, &cmds, &common, &home)), not_plain, "a directory index");
    fs::remove_dir(&admin_index).unwrap();
    let c = std::ffi::CString::new(admin_index.to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let start = std::time::Instant::now();
    assert_eq!(go(&t, &evidence_run(&script, &walk_base, &cmds, &common, &home)), not_plain, "a FIFO index");
    assert!(start.elapsed() < Duration::from_secs(5), "a FIFO index stalled the run");
}

/// L2: a run's tmp must not exist yet, so nothing an earlier run left behind can have prepared it.
#[test]
fn a_run_refuses_a_tmp_that_already_exists() {
    let t = Tmp::new("staletmp");
    let (script, base, home, common) = (fake(&t, "ok", BUNDLE, "exit 2"), base(&t), t.home(), t.0.join("repo.git"));
    let cmds = vec!["true".to_string()];
    let p = t.fresh(false, "used");
    fs::create_dir_all(&p.tmp).unwrap();
    assert!(matches!(run(&p, &evidence_run(&script, &base, &cmds, &common, &home)), RunOutcome::Failed(why) if why.contains("fresh")));
}

#[test]
fn the_deny_list_is_six_k_s_plus_the_configs_credentials() {
    let t = Tmp::new("list");
    let d = deny_dirs(&t.home(), &t.0.join("data"));
    for p in crate::adapter::profiles::vajra_deny_dirs(&t.home(), &t.0.join("data")) {
        assert!(d.contains(&p), "{}", p.display());
    }
    for rel in [".ssh", ".aws", ".config/gh", ".claude/skills/vajra", ".claude/hooks", "Library/Keychains", ".codex", ".claude.json", ".config/gcloud", ".azure"] {
        assert!(d.contains(&t.home().join(rel)), "{rel}");
    }
}
