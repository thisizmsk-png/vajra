//! bhishma L1 (17 Nov): the read-only git bridge never runs a program the repository names. Each test builds a repo
//! whose config and attributes define a clean and process filter, a textconv, an external diff, an fsmonitor hook and
//! repo hooks that each write a marker, runs every git call the core makes through the bridge, and checks that no
//! marker exists and the answers are still right.
#![allow(clippy::disallowed_methods)] // test code: the app's own spawns go through the spawn lock

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use vajra_desktop::providers::effective::diff_from_git;
use vajra_desktop::vajra::bridge::{Bridge, Output, Request};

static N: AtomicUsize = AtomicUsize::new(0);

struct T {
    root: PathBuf,
}

impl T {
    fn new() -> T {
        let root = std::env::temp_dir().join(format!("vajra-gitsafe-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home")).unwrap();
        fs::create_dir_all(root.join("marks")).unwrap();
        T { root: root.canonicalize().unwrap() }
    }
    fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    fn marks(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(self.root.join("marks")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }
    /// Plain git as a test fixture tool (never through the bridge), with this test's HOME and no system config.
    fn git(&self, dir: &Path, args: &[&str]) -> String {
        let o = self.git_raw(dir, args);
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    }
    fn git_raw(&self, dir: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"])
            .args(args)
            .current_dir(dir)
            .env("HOME", self.home())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .output()
            .unwrap()
    }
    /// A marker-writing program; `extra` runs after it (a filter must still pass its input through).
    fn hook(&self, name: &str, extra: &str) -> PathBuf {
        let p = self.root.join(format!("bin-{name}"));
        fs::write(&p, format!("#!/bin/sh\ntouch '{}/{name}'\n{extra}\n", self.root.join("marks").display())).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p
    }
    fn bridge(&self) -> Bridge {
        Bridge::new(self.root.join("scripts")).with_home(self.home())
    }
    /// Every hostile key on `dir`'s repo config (`file`: the config file to write, `--local` by default).
    fn poison(&self, dir: &Path, file: Option<&Path>) {
        let set = |k: &str, v: &str| {
            let mut a = vec!["config"];
            let f;
            if let Some(p) = file {
                f = p.to_string_lossy().into_owned();
                a.extend(["--file", &f]);
            }
            a.extend([k, v]);
            self.git(dir, &a);
        };
        let p = |n: &str, extra: &str| self.hook(n, extra).to_string_lossy().into_owned();
        set("filter.x.clean", &p("clean", "cat"));
        set("filter.x.smudge", &p("smudge", "cat"));
        set("filter.x.process", &p("process", ""));
        set("filter.x.required", "true");
        set("diff.y.textconv", &p("textconv", "cat \"$1\""));
        set("diff.y.command", &p("extdiff", ""));
        set("diff.external", &p("external", ""));
        set("core.fsmonitor", &p("fsmonitor", ""));
        set("core.pager", &p("pager", "cat"));
        set("gpg.program", &p("gpg", ""));
        set("log.showSignature", "true");
        set("core.sshCommand", &p("ssh", ""));
        let hooks = self.root.join("hooks");
        fs::create_dir_all(&hooks).unwrap();
        for h in ["post-index-change", "reference-transaction", "post-checkout", "pre-commit"] {
            fs::copy(self.hook(h, ""), hooks.join(h)).unwrap();
            fs::set_permissions(hooks.join(h), fs::Permissions::from_mode(0o755)).unwrap();
        }
        set("core.hooksPath", &hooks.to_string_lossy());
    }
}

impl Drop for T {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run(b: &Bridge, cwd: &Path, args: &[&str]) -> Output {
    b.run(&Request::Git { cwd: cwd.to_path_buf(), args: args.iter().map(|a| a.to_string()).collect() }).unwrap()
}

fn ok(b: &Bridge, cwd: &Path, args: &[&str]) -> String {
    let o = run(b, cwd, args);
    assert_eq!(o.exit(), Some(0), "git {args:?}: {}", o.stderr);
    o.stdout
}

/// A committed repo whose attributes route every file through filter `x` and diff driver `y`, with a modified
/// tracked file and an untracked one, and an index older than the files (so status must re-read them).
fn hostile_repo(t: &T) -> (PathBuf, String) {
    let repo = t.root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    t.git(&repo, &["init", "-q"]);
    fs::write(repo.join(".gitattributes"), "* filter=x diff=y\n").unwrap();
    fs::write(repo.join("f.txt"), "one\ntwo\n").unwrap();
    t.git(&repo, &["add", "."]);
    t.git(&repo, &["commit", "-qm", "base"]);
    let base = t.git(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    // A staged new file, so `diff --cached` has something to show from the index.
    fs::write(repo.join("staged.txt"), "fresh\n").unwrap();
    t.git(&repo, &["add", "staged.txt"]);
    t.poison(&repo, None);
    // `info/attributes` too: it outranks `.gitattributes`.
    fs::write(repo.join(".git/info/attributes"), "* filter=x diff=y\n").unwrap();
    fs::write(repo.join("f.txt"), "one\nTWO\nthree\n").unwrap();
    fs::write(repo.join("new.txt"), "fresh\n").unwrap();
    // Make the index stat info stale so git must read the working file to decide if it changed.
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    fs::File::options().write(true).open(repo.join(".git/index")).unwrap().set_modified(old).unwrap();
    assert!(t.marks().is_empty(), "setup itself ran a hook: {:?}", t.marks());
    (repo, base)
}

#[test]
fn control_plain_git_runs_the_planted_programs() {
    // The fixture really is hostile: git without the bridge runs them.
    let t = T::new();
    let (repo, base) = hostile_repo(&t);
    // (The fake `process` filter does not speak git's protocol, so these may fail; what they ran is the point.)
    t.git_raw(&repo, &["status", "--porcelain"]);
    // Without the process filter git falls back to `clean`, and gets as far as the diff drivers.
    t.git(&repo, &["config", "--unset", "filter.x.process"]);
    t.git_raw(&repo, &["diff", &base, "--"]);
    t.git_raw(&repo, &["diff", "--no-ext-diff", &base, "--"]);
    let m = t.marks();
    let has = |n: &str| m.iter().any(|x| x == n);
    assert!(has("fsmonitor"), "{m:?}");
    assert!(has("process") && has("clean"), "{m:?}");
    assert!(has("textconv"), "{m:?}");
    assert!(has("external") || has("extdiff"), "{m:?}");
}

#[test]
fn every_bridge_call_on_a_hostile_repo_runs_nothing_and_answers_right() {
    let t = T::new();
    let (repo, base) = hostile_repo(&t);
    let b = t.bridge();

    // `files_changed` and the status read before closing a worker.
    let st = ok(&b, &repo, &["status", "--porcelain", "-uall"]);
    assert_eq!(st, " M f.txt\nA  staged.txt\n?? new.txt\n");
    // The Diff tab: a full patch against the base.
    let d = ok(&b, &repo, &["diff", &base, "--"]);
    assert!(d.contains("-two\n+TWO\n+three\n"), "{d}");
    assert!(!d.contains("Binary"), "{d}");
    // The receipt's line counts.
    assert_eq!(ok(&b, &repo, &["diff", "--numstat", "-z", "--ignore-submodules=all", &base, "--"]), "2\t1\tf.txt\x001\t0\tstaged.txt\0");
    assert_eq!(ok(&b, &repo, &["diff", "--stat", "HEAD", "--"]).lines().count(), 3);
    assert_eq!(ok(&b, &repo, &["ls-files", "--others", "--exclude-standard", "-z"]), "new.txt\0");
    assert_eq!(ok(&b, &repo, &["rev-parse", "HEAD"]).trim(), base);
    assert_eq!(ok(&b, &repo, &["rev-parse", "--verify", "--quiet", "main^{commit}"]).trim(), base);
    assert_eq!(ok(&b, &repo, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "main");
    assert_eq!(ok(&b, &repo, &["merge-base", "HEAD", &base]).trim(), base);
    assert_eq!(ok(&b, &repo, &["log", "--format=%H", "HEAD", "--not", "--remotes"]).trim(), base);
    assert!(ok(&b, &repo, &["log", "-p", "-1"]).contains("+two"));
    // No upstream: a failure, as before.
    assert_ne!(run(&b, &repo, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).exit(), Some(0));
    // Staged content is diffed from the index's blobs.
    assert!(ok(&b, &repo, &["diff", "--cached", "HEAD", "--"]).contains("+fresh"));

    // The receipt's whole diff reader.
    let diff = diff_from_git(&b, &repo, &base).unwrap();
    assert_eq!((diff.files_total, diff.added, diff.removed), (3, 4, 1), "{diff:?}");

    assert!(t.marks().is_empty(), "a repo-named program ran: {:?}", t.marks());
    // The bridge leaves no private git dir behind.
    let left: Vec<_> = fs::read_dir(std::env::temp_dir()).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with(&format!("vajra-git-{}-", std::process::id()))).collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn a_filter_in_an_included_file_or_a_worktree_config_runs_nothing() {
    // A linked worktree (as a crew worker's), its config split between the common config, an included file and
    // `config.worktree`, and an upstream so `@{u}` still resolves.
    let t = T::new();
    let origin = t.root.join("origin");
    fs::create_dir_all(&origin).unwrap();
    t.git(&origin, &["init", "-q"]);
    fs::write(origin.join(".gitattributes"), "*.txt filter=x diff=y\n").unwrap();
    fs::write(origin.join("a.txt"), "a\n").unwrap();
    t.git(&origin, &["add", "."]);
    t.git(&origin, &["commit", "-qm", "base"]);
    let clone = t.root.join("clone");
    t.git(&t.root, &["clone", "-q", &origin.to_string_lossy(), &clone.to_string_lossy()]);
    let wt = t.root.join("wt");
    t.git(&clone, &["worktree", "add", "-q", "-b", "topic", &wt.to_string_lossy()]);
    t.git(&wt, &["branch", "-q", "--set-upstream-to=origin/main"]);
    t.git(&wt, &["config", "extensions.worktreeConfig", "true"]);
    let inc = t.root.join("included.cfg");
    t.poison(&wt, Some(&inc));
    t.git(&wt, &["config", "include.path", &inc.to_string_lossy()]);
    let wtcfg = t.root.join("clone/.git/worktrees/wt/config.worktree");
    t.poison(&wt, Some(&wtcfg));
    fs::write(wt.join("a.txt"), "a\nb\n").unwrap();
    fs::write(wt.join("n.txt"), "n\n").unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    fs::File::options().write(true).open(t.root.join("clone/.git/worktrees/wt/index")).unwrap().set_modified(old).unwrap();
    assert!(t.marks().is_empty(), "{:?}", t.marks());

    let b = t.bridge();
    let sub = wt.join("deeper");
    fs::create_dir_all(&sub).unwrap();
    for dir in [&wt, &sub] {
        assert_eq!(ok(&b, dir, &["status", "--porcelain", "-uall"]), " M a.txt\n?? n.txt\n");
        assert!(ok(&b, dir, &["diff", "HEAD", "--"]).contains("+b\n"));
    }
    assert_eq!(ok(&b, &wt, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).trim(), "origin/main");
    assert_eq!(ok(&b, &wt, &["log", "--format=%H", "@{u}..HEAD"]), "");
    assert_eq!(ok(&b, &wt, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "topic");
    let head = ok(&b, &clone, &["rev-parse", "HEAD"]);
    assert_eq!(ok(&b, &wt, &["merge-base", "HEAD", head.trim()]), head);
    assert!(t.marks().is_empty(), "a repo-named program ran: {:?}", t.marks());
    // The worktree's own index is not rewritten by a status read (no optional lock taken).
    let idx = fs::metadata(t.root.join("clone/.git/worktrees/wt/index")).unwrap().modified().unwrap();
    assert_eq!(idx, old);
}

#[test]
fn outside_a_repo_the_call_fails_and_runs_nothing() {
    let t = T::new();
    let plain = t.root.join("plain");
    fs::create_dir_all(&plain).unwrap();
    let o = run(&t.bridge(), &plain, &["status", "--porcelain"]);
    assert_eq!(o.exit(), Some(128), "{o:?}");
    assert!(o.stdout.is_empty());
}
