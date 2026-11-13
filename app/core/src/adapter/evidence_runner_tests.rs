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
        let p = Profile {
            worktree: self.0.join("work"),
            tmp: self.0.join("tmp"),
            deny: deny_dirs(&self.home(), &self.0.join("data")),
            caches: default_caches(&self.home()),
            network,
        };
        p.prepare().unwrap();
        p
    }
    /// `sh -c script` in the worktree under the profile, with HOME and TMPDIR set like a run.
    fn run(&self, p: &Profile, script: &str, extra: &[(&str, PathBuf)]) -> std::process::Output {
        let mut c = Command::new(SANDBOX_EXEC);
        c.args(p.argv("/bin/sh", &["-c".into(), script.into()]).unwrap()).current_dir(&p.worktree).env("HOME", self.home()).env("TMPDIR", &p.tmp);
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
    assert!(!t.profile(true).sbpl().unwrap().contains("(deny network-outbound)"), "a network seat keeps the network");
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
    ];
    for c in &cases {
        let o = t.run(&p, c, &[]);
        assert!(!o.status.success(), "{c} must be refused: {}", text(&o));
        assert!(text(&o).contains("Operation not permitted"), "{c}: {}", text(&o));
    }
    assert!(!String::from_utf8_lossy(&t.run(&p, &format!("cat '{}' 2>/dev/null; true", key.display()), &[]).stdout).contains("test-key"));
    for p2 in ["home/.claude/vajra/planted", "data/planted", "home/outside", "planted-next-to-work"] {
        assert!(!t.0.join(p2).exists(), "{p2}");
    }
    // The worktree, the tmp dir and a cache are writable.
    let o = t.run(&p, "echo ok > in-work && echo ok > \"$TMPDIR/in-tmp\" && mkdir -p \"$HOME/.cache/x\" && echo ok > \"$HOME/.cache/x/f\"", &[]);
    assert!(o.status.success(), "{}", text(&o));
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
        return eprintln!("skipped: pytest");
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
    let extra = [("RUSTUP_HOME", real_home.join(".rustup")), ("CARGO_HOME", t.home().join(".cargo")), ("CARGO_TARGET_DIR", w.join("target"))];
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
    let extra = [("GOCACHE", t.home().join("Library/Caches/go-build")), ("GOPATH", t.home().join("go")), ("GOFLAGS", PathBuf::from("-mod=mod")), ("GOTOOLCHAIN", PathBuf::from("local"))];
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

#[test]
fn the_deny_list_is_six_k_s_plus_the_configs_credentials() {
    let t = Tmp::new("list");
    let d = deny_dirs(&t.home(), &t.0.join("data"));
    for p in crate::adapter::profiles::vajra_deny_dirs(&t.home(), &t.0.join("data")) {
        assert!(d.contains(&p), "{}", p.display());
    }
    for rel in [".ssh", ".aws", ".config/gh", ".claude/skills/vajra", ".claude/hooks", "Library/Keychains"] {
        assert!(d.contains(&t.home().join(rel)), "{rel}");
    }
}
