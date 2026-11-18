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
    assert_eq!((d["text"].as_str(), &d["added"], &d["removed"]), (Some("# one\ntwo\n"), &json!([2]), &json!([])));
    assert_eq!((d["base_hidden"].as_bool(), d["notice"].as_str()), (Some(true), Some("Earlier content is hidden until the key check ships.")));
    let n = fx.app.files_diff(&fx.pid, Some("w1"), "src/new.rs").unwrap();
    assert_eq!((n["text"].as_str(), &n["added"], n["untracked"].as_bool()), (Some("new\n"), &json!([1]), Some(true)));
    for (p, want) in [("/etc/hosts", "invalid_args"), ("../x", "invalid_args"), ("", "invalid_args"), (".git/HEAD", "protected")] {
        assert_eq!(code(&fx.app.files_diff(&fx.pid, Some("w1"), p).unwrap_err()), want, "{p:?}");
    }
    // A path is only ever a path: `:(glob)*` names no file, so nothing (no diff of the whole tree).
    assert_eq!(code(&fx.app.files_diff(&fx.pid, Some("w1"), ":(glob)*").unwrap_err()), "not_found");
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
fn a_diff_of_a_staged_hard_link_to_the_key_is_refused_and_never_carries_its_bytes() {
    let fx = Fx::new();
    let key = fx.dir.join("home/.claude/vajra").join(crate::vajra::allowlist::DENIED_NAME);
    fs::write(&key, b"DUMMY-KEY-CANARY-d1ff").unwrap();
    let wt = fx.repo().join(".claude/worktrees/w1");
    fs::hard_link(&key, wt.join("notes.txt")).unwrap();
    git(&wt, &["add", "notes.txt"]);
    let r = fx.app.files_diff(&fx.pid, Some("w1"), "notes.txt");
    assert!(!r.as_ref().map_or_else(|e| e.to_string(), |v| v.to_string()).contains("CANARY"), "{r:?}");
    assert_eq!(code(&r.unwrap_err()), "protected");
    // Committed too.
    git(&wt, &["commit", "-q", "-m", "k"]);
    assert_eq!(code(&fx.app.files_diff(&fx.pid, Some("w1"), "notes.txt").unwrap_err()), "protected");
}

#[test]
fn a_diff_inside_a_denied_folder_in_the_repo_is_refused_present_or_deleted() {
    let fx = Fx::new();
    // The agent home sits inside the project (the store refuses that at add time, so the app is opened with that home
    // after the project was registered): its files are tracked in the repo.
    let home = fx.repo().join("u");
    fs::create_dir_all(home.join(".claude/vajra")).unwrap();
    fs::write(home.join(".claude/vajra/state.json"), b"SECRET-STATE").unwrap();
    fs::write(home.join(".claude/vajra/gone.json"), b"SECRET-GONE").unwrap();
    git(&fx.repo(), &["add", "-f", "u"]);
    git(&fx.repo(), &["commit", "-q", "-m", "home"]);
    fs::write(home.join(".claude/vajra/state.json"), b"SECRET-STATE-2").unwrap();
    fs::remove_file(home.join(".claude/vajra/gone.json")).unwrap();
    let app = App::open(&fx.dir.join("data"), &home).unwrap();
    for p in ["u/.claude/vajra/state.json", "u/.claude/vajra/gone.json"] {
        let r = app.files_diff(&fx.pid, None, p);
        assert!(!r.as_ref().map_or_else(|e| e.to_string(), |v| v.to_string()).contains("SECRET"), "{p}: {r:?}");
        assert_eq!(code(&r.unwrap_err()), "protected", "{p}");
    }
    // With the whole folder gone, the deleted path is still refused by name.
    fs::remove_dir_all(home.join(".claude")).unwrap();
    assert_eq!(code(&app.files_diff(&fx.pid, None, "u/.claude/vajra/state.json").unwrap_err()), "protected");
    // A deleted file elsewhere is reported as deleted, without its old content.
    fs::remove_file(fx.repo().join("src/lib.rs")).unwrap();
    let d = app.files_diff(&fx.pid, None, "src/lib.rs").unwrap();
    assert_eq!((d["kind"].as_str(), d["base_hidden"].as_bool()), (Some("deleted"), Some(true)));
    assert!(!d.to_string().contains("pub fn a"), "{d}");
}

#[test]
fn a_tracked_file_replaced_by_a_hard_link_to_the_key_is_refused_by_the_diff() {
    // Codex's case: `notes.txt` is tracked with innocent content, then the working file is swapped for a hard link to
    // the key without staging anything.
    let fx = Fx::new();
    let key = fx.dir.join("home/.claude/vajra").join(crate::vajra::allowlist::DENIED_NAME);
    fs::write(&key, b"DUMMY-KEY-CANARY-c0de\n").unwrap();
    fs::write(fx.repo().join("notes.txt"), "hello\n").unwrap();
    git(&fx.repo(), &["add", "notes.txt"]);
    git(&fx.repo(), &["commit", "-q", "-m", "notes"]);
    fs::remove_file(fx.repo().join("notes.txt")).unwrap();
    fs::hard_link(&key, fx.repo().join("notes.txt")).unwrap();
    let r = fx.app.files_diff(&fx.pid, None, "notes.txt");
    assert!(!format!("{r:?}").contains("CANARY"), "{r:?}");
    assert_eq!(code(&r.unwrap_err()), "protected");
}

/// The key committed into the repo through a hard link: its bytes now live in a git blob with its own identity.
fn key_in_base(fx: &Fx) {
    let key = fx.dir.join("home/.claude/vajra").join(crate::vajra::allowlist::DENIED_NAME);
    fs::write(&key, b"DUMMY-KEY-CANARY-ba5e0001\n").unwrap();
    fs::hard_link(&key, fx.repo().join("notes.txt")).unwrap();
    git(&fx.repo(), &["add", "notes.txt"]);
    git(&fx.repo(), &["commit", "-q", "-m", "k"]);
}

#[test]
fn a_key_committed_then_deleted_sends_no_base_bytes_at_all() {
    let fx = Fx::new();
    key_in_base(&fx);
    fs::remove_file(fx.repo().join("notes.txt")).unwrap();
    let d = fx.app.files_diff(&fx.pid, None, "notes.txt").unwrap();
    assert!(!d.to_string().contains("CANARY"), "{d}");
    assert_eq!(d, json!({"path": "notes.txt", "base": d["base"], "kind": "deleted", "base_hidden": true, "notice": "Earlier content is hidden until the key check ships."}));
}

#[test]
fn a_key_committed_then_replaced_sends_only_the_current_file() {
    let fx = Fx::new();
    key_in_base(&fx);
    // Break the link: an innocent file with its own inode replaces it.
    fs::remove_file(fx.repo().join("notes.txt")).unwrap();
    fs::write(fx.repo().join("notes.txt"), "hello\n").unwrap();
    let d = fx.app.files_diff(&fx.pid, None, "notes.txt").unwrap();
    assert!(!d.to_string().contains("CANARY"), "{d}");
    // Only the current text, with line numbers and counts for the rest.
    assert_eq!((d["text"].as_str(), &d["added"], &d["removed"]), (Some("hello\n"), &json!([1]), &json!([[0, 1]])));
}

#[test]
fn no_diff_answer_ever_carries_base_content() {
    let fx = Fx::new();
    fs::write(fx.repo().join("src/lib.rs"), "pub fn z() {}\n").unwrap();
    let d = fx.app.files_diff(&fx.pid, None, "src/lib.rs").unwrap();
    assert!(!d.to_string().contains("pub fn a"), "{d}");
}

#[test]
fn a_folder_is_not_diffed_as_many_unchecked_files() {
    let fx = Fx::new();
    fs::write(fx.repo().join("src/lib.rs"), "pub fn b() {}\n").unwrap();
    assert_eq!(code(&fx.app.files_diff(&fx.pid, None, "src").unwrap_err()), "not_a_file");
    // A folder deleted from the work tree but present at the base.
    fs::remove_dir_all(fx.repo().join("src")).unwrap();
    assert_eq!(code(&fx.app.files_diff(&fx.pid, None, "src").unwrap_err()), "not_a_file");
}

#[test]
fn a_diff_is_built_from_the_checked_read_and_the_base_object() {
    let fx = Fx::new();
    // Modified, with the base read from git's objects even though the working file differs.
    fs::write(fx.repo().join("src/lib.rs"), "pub fn a() {}\npub fn b() {}\n").unwrap();
    let d = fx.app.files_diff(&fx.pid, None, "src/lib.rs").unwrap();
    assert_eq!((&d["added"], &d["removed"]), (&json!([2]), &json!([])));
    // Unchanged.
    let u = fx.app.files_diff(&fx.pid, None, "README.md").unwrap();
    assert_eq!((&u["added"], &u["removed"]), (&json!([]), &json!([])));
    // Binary at the working side.
    fs::write(fx.repo().join("README.md"), b"\0\x01").unwrap();
    assert_eq!(fx.app.files_diff(&fx.pid, None, "README.md").unwrap()["kind"], "binary");
    // Missing on both sides.
    assert_eq!(code(&fx.app.files_diff(&fx.pid, None, "nope.txt").unwrap_err()), "not_found");
}

#[test]
fn an_index_over_the_bridge_cap_stays_a_git_list_flagged_partial() {
    let fx = Fx::new();
    // 6000 tracked paths of ~200 bytes each: over the bridge's 1 MiB output cap.
    let deep = format!("{}/", "d".repeat(190));
    let dir = fx.repo().join(&deep);
    fs::create_dir_all(&dir).unwrap();
    for i in 0..6000 {
        fs::write(dir.join(format!("f{i:04}.txt")), "").unwrap();
    }
    git(&fx.repo(), &["add", "."]);
    git(&fx.repo(), &["commit", "-q", "-m", "many"]);
    // Ignored files the plain walk would have listed.
    fs::write(fx.repo().join("target/z.o"), "x").unwrap();
    let v = fx.app.files_index(&fx.pid, None).unwrap();
    assert_eq!(v["truncated"], true);
    let paths: Vec<&str> = v["paths"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
    assert!(paths.len() > 500, "{}", paths.len());
    assert!(paths.iter().all(|p| !p.starts_with("target/") && !p.ends_with(".log") && !p.starts_with(".claude/")), "no ignored file");
    assert!(paths.iter().all(|p| p == &".gitignore" || p == &"README.md" || p == &"src/lib.rs" || (p.starts_with(&deep) && p.ends_with(".txt") && p.len() == deep.len() + 9)), "no cut path");
}

#[test]
fn a_project_below_the_repository_top_uses_its_own_paths_for_git() {
    let fx = Fx::new();
    // The repo's top holds a README.md too; the project is only `sub`.
    let sub = fx.repo().join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("README.md"), "sub one\nsub two\n").unwrap();
    fs::write(sub.join("Cargo.toml"), "[package]\n").unwrap();
    git(&fx.repo(), &["add", "sub"]);
    git(&fx.repo(), &["commit", "-q", "-m", "sub"]);
    let pid = test_project(&fx.dir.join("data"), &fx.dir.join("home"), &sub);
    let app = App::open(&fx.dir.join("data"), &fx.dir.join("home")).unwrap();
    // Unchanged: no marks (not "every line added", not compared with the top-level README.md).
    for p in ["Cargo.toml", "README.md"] {
        let d = app.files_diff(&pid, None, p).unwrap();
        assert_eq!((&d["added"], &d["removed"], d["untracked"].as_bool()), (&json!([]), &json!([]), Some(false)), "{p}: {d}");
    }
    fs::write(sub.join("README.md"), "sub one\nsub 2\n").unwrap();
    let d = app.files_diff(&pid, None, "README.md").unwrap();
    assert_eq!((&d["added"], &d["removed"]), (&json!([2]), &json!([[1, 1]])));
    // Changed files: only this project's, relative to it, each readable back through the checked reader.
    fs::write(fx.repo().join("src/lib.rs"), "outside the project\n").unwrap();
    let c = app.files_changed(&pid, None).unwrap();
    assert_eq!(c["files"], json!([{"path": "README.md", "status": "M", "from": null}]));
    assert_eq!(app.files_read(&pid, None, "README.md").unwrap()["text"], "sub one\nsub 2\n");
}

#[test]
fn name_status_parses_renames_copies_and_deletes() {
    let raw = b"M\0a.rs\0A\0b.rs\0D\0c.rs\0R100\0old.rs\0new.rs\0C075\0x.rs\0y.rs\0T\0t.rs\0";
    let got: Vec<(String, char)> = parse_name_status(raw).into_iter().map(|c| (c.path, c.status)).collect();
    assert_eq!(got, vec![("a.rs".into(), 'M'), ("b.rs".into(), 'A'), ("c.rs".into(), 'D'), ("new.rs".into(), 'R'), ("y.rs".into(), 'A'), ("t.rs".into(), 'M')]);
}
