//! The Files panel's commands end to end: an App over a temp home, one project that is a git repo with a run worktree.

use super::files::parse_name_status;
use super::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

fn git(cwd: &Path, args: &[&str]) {
    let o = std::process::Command::new("git").arg("-C").arg(cwd).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

struct Fx {
    dir: PathBuf,
    app: App,
    pid: String,
}

impl Fx {
    /// `repo` (the project): `README.md`, `src/lib.rs`, a `.gitignore` for `target/` and `*.log`, one commit, and a
    /// worktree `w1` on branch `worker-1` that modifies `README.md` and adds `src/new.rs`.
    fn new() -> Self {
        let d = std::env::temp_dir().join(format!("vajra-filescmd-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("home/.claude/vajra")).unwrap();
        let dir = d.canonicalize().unwrap();
        let repo = dir.join("repo");
        let pid = test_project(&dir.join("data"), &dir.join("home"), &repo);
        git(&repo, &["init", "-q", "-b", "main"]);
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("README.md"), "# one\n").unwrap();
        fs::write(repo.join("src/lib.rs"), "pub fn a() {}\n").unwrap();
        fs::write(repo.join(".gitignore"), "target/\n*.log\n.claude/\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        fs::create_dir_all(repo.join("target")).unwrap();
        fs::write(repo.join("target/out.o"), "x").unwrap();
        fs::write(repo.join("build.log"), "x").unwrap();
        let wt = repo.join(".claude/worktrees/w1");
        git(&repo, &["worktree", "add", "-q", "-b", "worker-1", wt.to_str().unwrap()]);
        fs::write(wt.join("README.md"), "# one\ntwo\n").unwrap();
        git(&wt, &["commit", "-q", "-am", "edit"]);
        fs::write(wt.join("src/new.rs"), "new\n").unwrap();
        let app = App::open(&dir.join("data"), &dir.join("home")).unwrap();
        Self { dir, app, pid }
    }
    fn repo(&self) -> PathBuf {
        self.dir.join("repo")
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn code(v: &Value) -> &str {
    v["error"].as_str().unwrap_or("")
}

fn names(v: &Value) -> Vec<String> {
    v["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn roots_name_the_project_and_its_run_worktrees_with_their_branches() {
    let fx = Fx::new();
    let r = fx.app.files_roots(&fx.pid).unwrap();
    assert_eq!(r["worktrees"], json!([{"name": "w1", "branch": "worker-1"}]));
    assert_eq!(r["root"], fx.repo().to_string_lossy().as_ref());
}

#[test]
fn an_unknown_or_malformed_project_id_is_refused() {
    let fx = Fx::new();
    assert_eq!(code(&fx.app.files_list("../etc", None, "", false).unwrap_err()), "invalid_args");
    assert_eq!(code(&fx.app.files_list("p_x", None, "", false).unwrap_err()), "invalid_args");
    let other = crate::projects::contract::make_id("p", 1, [7; 10]);
    assert_eq!(code(&fx.app.files_roots(&other).unwrap_err()), "not_found");
}

#[test]
fn listing_hides_ignored_entries_unless_asked_and_marks_them() {
    let fx = Fx::new();
    let l = fx.app.files_list(&fx.pid, None, "", false).unwrap();
    // `.claude` is ignored here, `.git` is never listed.
    assert_eq!(names(&l), vec!["src", ".gitignore", "README.md"]);
    let all = fx.app.files_list(&fx.pid, None, "", true).unwrap();
    assert_eq!(names(&all), vec![".claude", "src", "target", ".gitignore", "build.log", "README.md"]);
    let target = all["entries"].as_array().unwrap().iter().find(|e| e["name"] == "target").unwrap();
    assert_eq!((target["ignored"].as_bool(), target["kind"].as_str(), target["path"].as_str()), (Some(true), Some("dir"), Some("target")));
    let sub = fx.app.files_list(&fx.pid, None, "src", false).unwrap();
    assert_eq!(sub["entries"][0]["path"], "src/lib.rs");
}

#[test]
fn a_worktree_is_read_by_name_and_names_that_escape_are_refused() {
    let fx = Fx::new();
    let r = fx.app.files_read(&fx.pid, Some("w1"), "README.md").unwrap();
    assert_eq!((r["kind"].as_str(), r["text"].as_str()), (Some("text"), Some("# one\ntwo\n")));
    for bad in ["..", "../..", "w1/../..", "/tmp", "", "a/b"] {
        assert_eq!(code(&fx.app.files_list(&fx.pid, Some(bad), "", false).unwrap_err()), "invalid_args", "{bad:?}");
    }
    assert_eq!(code(&fx.app.files_list(&fx.pid, Some("w9"), "", false).unwrap_err()), "not_found");
    // A worktree name that is a link out of the project is refused.
    let out = fx.dir.join("elsewhere");
    fs::create_dir_all(&out).unwrap();
    symlink(&out, fx.repo().join(".claude/worktrees/w2")).unwrap();
    assert_eq!(code(&fx.app.files_list(&fx.pid, Some("w2"), "", false).unwrap_err()), "outside_root");
    assert!(!fx.app.files_roots(&fx.pid).unwrap()["worktrees"].as_array().unwrap().iter().any(|w| w["name"] == "w2"), "and not offered");
}

#[test]
fn every_refusal_reaches_the_command_answer() {
    let fx = Fx::new();
    symlink(fx.dir.join("home"), fx.repo().join("home-link")).unwrap();
    for (p, want) in [
        ("/etc/hosts", "invalid_args"),
        ("../repo/README.md", "invalid_args"),
        (".git/config", "protected"),
        ("home-link/.claude/vajra", "outside_root"),
        ("home-link/no-such-file", "outside_root"),
        ("nope", "not_found"),
        ("src", "not_a_file"),
    ] {
        assert_eq!(code(&fx.app.files_read(&fx.pid, None, p).unwrap_err()), want, "{p}");
    }
}

#[test]
fn the_agent_home_the_app_data_and_the_signing_key_are_out_of_reach_from_a_project() {
    // The store already refuses the home folder, a folder holding it, and one holding the app's data folder as a
    // project (a denied folder inside a root is covered in `files::tests`). What is left: links to them, and a hard
    // link to the key under an innocent name.
    let fx = Fx::new();
    let home = fx.dir.join("home");
    let key = home.join(".claude/vajra").join(crate::vajra::allowlist::DENIED_NAME);
    fs::write(&key, b"DUMMY-KEY-CANARY").unwrap();
    fs::write(home.join(".claude/vajra/x"), b"x").unwrap();
    symlink(home.join(".claude"), fx.repo().join("agent")).unwrap();
    symlink(fx.dir.join("data"), fx.repo().join("appdata")).unwrap();
    for p in ["agent/vajra/x", "agent/vajra/missing", "appdata/projects"] {
        assert_eq!(code(&fx.app.files_read(&fx.pid, None, p).unwrap_err()), "outside_root", "{p}");
    }
    assert_eq!(code(&fx.app.files_list(&fx.pid, None, "agent", true).unwrap_err()), "outside_root");
    fs::hard_link(&key, fx.repo().join("notes.txt")).unwrap();
    assert_eq!(code(&fx.app.files_read(&fx.pid, None, "notes.txt").unwrap_err()), "protected");
}

#[test]
fn images_come_as_base64_and_large_or_binary_files_as_a_note() {
    let fx = Fx::new();
    fs::write(fx.repo().join("a.png"), [0x89, b'P', b'N', b'G']).unwrap();
    fs::write(fx.repo().join("a.bin"), [0u8, 1, 2]).unwrap();
    fs::write(fx.repo().join("big.txt"), vec![b'a'; crate::files::TEXT_CAP as usize + 5]).unwrap();
    let img = fx.app.files_read(&fx.pid, None, "a.png").unwrap();
    assert_eq!((img["kind"].as_str(), img["mime"].as_str(), img["base64"].as_str()), (Some("image"), Some("image/png"), Some("iVBORw==")));
    let bin = fx.app.files_read(&fx.pid, None, "a.bin").unwrap();
    assert_eq!(bin, json!({"path": "a.bin", "size": 3, "kind": "binary"}));
    let big = fx.app.files_read(&fx.pid, None, "big.txt").unwrap();
    assert_eq!((big["kind"].as_str(), big["cap"].as_u64(), big.get("text")), (Some("too_large"), Some(crate::files::TEXT_CAP), None));
}

#[test]
fn changed_files_of_a_worktree_are_counted_from_where_it_left_the_project() {
    let fx = Fx::new();
    let c = fx.app.files_changed(&fx.pid, Some("w1")).unwrap();
    assert_eq!(c["files"], json!([{"path": "README.md", "status": "M", "from": null}, {"path": "src/new.rs", "status": "A", "from": null}]));
    // The project itself: nothing uncommitted yet (ignored files are not changes).
    assert_eq!(fx.app.files_changed(&fx.pid, None).unwrap()["files"], json!([]));
    fs::write(fx.repo().join("src/lib.rs"), "pub fn b() {}\n").unwrap();
    assert_eq!(fx.app.files_changed(&fx.pid, None).unwrap()["files"], json!([{"path": "src/lib.rs", "status": "M", "from": null}]));
}

#[test]
fn a_file_diff_is_against_the_base_and_a_new_file_is_marked_untracked() {
    let fx = Fx::new();
    let d = fx.app.files_diff(&fx.pid, Some("w1"), "README.md").unwrap();
    let text = d["diff"].as_str().unwrap();
    assert!(text.contains("+two") && text.contains("README.md"), "{text}");
    let n = fx.app.files_diff(&fx.pid, Some("w1"), "src/new.rs").unwrap();
    assert_eq!((n["diff"].as_str(), n["untracked"].as_bool()), (Some(""), Some(true)));
    for (p, want) in [("/etc/hosts", "invalid_args"), ("../x", "invalid_args"), ("", "invalid_args"), (".git/HEAD", "protected")] {
        assert_eq!(code(&fx.app.files_diff(&fx.pid, Some("w1"), p).unwrap_err()), want, "{p:?}");
    }
    // Pathspec magic is read literally: `:(glob)*` names no file, so no diff of the whole tree.
    let g = fx.app.files_diff(&fx.pid, Some("w1"), ":(glob)*").unwrap();
    assert_eq!(g["diff"], "");
}

#[test]
fn the_index_lists_tracked_and_new_files_but_not_ignored_or_git_ones() {
    let fx = Fx::new();
    let paths = |v: Value| v["paths"].as_array().unwrap().iter().map(|p| p.as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(paths(fx.app.files_index(&fx.pid, None).unwrap()), vec![".gitignore", "README.md", "src/lib.rs"]);
    assert_eq!(paths(fx.app.files_index(&fx.pid, Some("w1")).unwrap()), vec![".gitignore", "README.md", "src/lib.rs", "src/new.rs"]);
}

#[test]
fn outside_git_the_index_walks_folders_but_never_through_a_link() {
    let fx = Fx::new();
    let plain = fx.dir.join("plain");
    fs::create_dir_all(plain.join("a/b")).unwrap();
    fs::write(plain.join("a/b/c.txt"), "c").unwrap();
    fs::write(plain.join("top.md"), "t").unwrap();
    fs::create_dir_all(fx.dir.join("out")).unwrap();
    fs::write(fx.dir.join("out/secret.txt"), "s").unwrap();
    symlink(fx.dir.join("out"), plain.join("link")).unwrap();
    let pid = test_project(&fx.dir.join("data"), &fx.dir.join("home"), &plain);
    let app = App::open(&fx.dir.join("data"), &fx.dir.join("home")).unwrap();
    let v = app.files_index(&pid, None).unwrap();
    assert_eq!(v["paths"], json!(["top.md", "a/b/c.txt"]));
}

#[test]
fn name_status_parses_renames_copies_and_deletes() {
    let raw = b"M\0a.rs\0A\0b.rs\0D\0c.rs\0R100\0old.rs\0new.rs\0C075\0x.rs\0y.rs\0T\0t.rs\0";
    let got: Vec<(String, char)> = parse_name_status(raw).into_iter().map(|c| (c.path, c.status)).collect();
    assert_eq!(got, vec![("a.rs".into(), 'M'), ("b.rs".into(), 'A'), ("c.rs".into(), 'D'), ("new.rs".into(), 'R'), ("y.rs".into(), 'A'), ("t.rs".into(), 'M')]);
}
