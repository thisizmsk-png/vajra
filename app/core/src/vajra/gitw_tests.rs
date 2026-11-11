use super::*;
use std::process::Command;
use std::sync::atomic::Ordering;

const GIT: &str = "/usr/bin/git";

/// No other live session on any repo.
#[derive(Debug)]
struct NoAgents;

impl Quiesce for NoAgents {
    fn pause(&self, _: &Path) -> Pause {
        Pause::new(|| {})
    }
}

struct Fx {
    d: PathBuf,
    home: PathBuf,
    repo: PathBuf,
    gw: Gitw,
    base: String,
    marker: PathBuf,
}

fn tmpdir(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("vajra-gitw-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

/// The fixture's own git, as a user would run it: temp HOME, no system config.
fn ugit(home: &Path, cwd: &Path, args: &[&str]) -> std::process::Output {
    let o = Command::new(GIT).args(args).current_dir(cwd).env_clear().env("PATH", "/usr/bin:/bin").env("HOME", home).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    o
}

fn out(o: std::process::Output) -> String {
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

/// A script that records it ran, then passes its stdin through (a filter that would be harmless if it ran).
fn marker_script(d: &Path, marker: &Path) -> PathBuf {
    let s = d.join("mark.sh");
    std::fs::write(&s, format!("#!/bin/sh\necho ran >> '{}'\ncat\n", marker.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o755)).unwrap();
    s
}

fn fx_with(tag: &str, gitconfig: &str, files: &[(&str, &str)]) -> Fx {
    fx_q(tag, gitconfig, files, Arc::new(NoAgents))
}

fn fx_q(tag: &str, gitconfig: &str, files: &[(&str, &str)], q: Arc<dyn Quiesce>) -> Fx {
    let d = tmpdir(tag);
    let (home, data, work, repo) = (d.join("home"), d.join("data"), d.join("work"), d.join("repo"));
    for p in [&home, &data, &repo] {
        std::fs::create_dir_all(p).unwrap();
    }
    let marker = d.join("MARKER");
    std::fs::write(home.join(".gitconfig"), gitconfig).unwrap();
    ugit(&home, &repo, &["init", "-q", "-b", "main"]);
    for (p, c) in [("a.txt", "a\n"), ("src/lib.rs", "fn a() {}\n"), ("other/x.rs", "x\n"), ("AGENTS.md", "agents\n"), (".gitignore", "target/\n")].iter().chain(files) {
        let f = repo.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    ugit(&home, &repo, &["add", "-A"]);
    ugit(&home, &repo, &["-c", "user.name=U", "-c", "user.email=u@e", "commit", "-q", "-m", "base"]);
    let base = out(ugit(&home, &repo, &["rev-parse", "HEAD"]));
    // Hooks a repo could carry: none may ever run in the core's git.
    for h in ["post-checkout", "post-commit", "post-merge", "pre-commit", "reference-transaction", "post-index-change", "pre-merge-commit"] {
        let p = repo.join(".git/hooks").join(h);
        std::fs::write(&p, format!("#!/bin/sh\necho hook-{h} >> '{}'\n", marker.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let g = GlobalBaseline::take_or_load(Path::new(GIT), &data, &home, None).unwrap();
    check_roots(&data, &work).unwrap();
    let gw = Gitw::new(GIT.into(), home.clone(), data.canonicalize().unwrap(), work.canonicalize().unwrap(), &g, q);
    let common = gw.common_of(&repo).unwrap();
    gw.repos.acquire(&gw, &common, false).unwrap();
    Fx { d, home, repo: repo.canonicalize().unwrap(), gw, base, marker }
}

fn fx(tag: &str) -> Fx {
    fx_with(tag, "[user]\n\tname = U\n\temail = u@e\n", &[])
}

fn globs() -> Vec<String> {
    vec!["src/**".into()]
}

fn edit(t: &Ticket, p: &str, c: &str) {
    let f = t.wt.join(p);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(f, c).unwrap();
}

fn no_marker(f: &Fx) {
    assert!(!f.marker.exists(), "something ran: {}", std::fs::read_to_string(&f.marker).unwrap_or_default());
}

fn rev(f: &Fx, r: &str) -> String {
    out(ugit(&f.home, &f.repo, &["rev-parse", r]))
}

#[test]
fn refs_outside_vajra_and_bad_names_are_refused() {
    assert!(vajra_ref("refs/heads/vajra/r1/task-t1-r1").is_ok());
    for bad in ["refs/heads/main", "refs/heads/vajra/../main", "refs/tags/vajra/x", "refs/heads/vajra/a..b", "refs/heads/vajra/x.lock", "refs/heads/vajra/-x"] {
        assert!(vajra_ref(bad).is_err(), "{bad}");
    }
    let f = fx("names");
    for (p, r, t) in [("../x", "r1", "t1"), ("p", "r 1", "t1"), ("p", "r1", "t/1")] {
        assert_eq!(f.gw.worktree_add(&f.repo, p, r, t, 1, &f.base).unwrap_err().code(), "invalid");
    }
    assert_eq!(f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, "main").unwrap_err().code(), "invalid", "only a full commit id");
    assert_eq!(f.gw.branch_delete(&f.repo, "refs/heads/main").unwrap_err().code(), "invalid");
}

#[test]
fn overlapping_roots_refuse_worker_launch() {
    let d = tmpdir("roots");
    let data = d.join("data");
    std::fs::create_dir_all(&data).unwrap();
    assert_eq!(check_roots(&data, &data.join("work")), Err(GitwError::WorktreeRootInvalid));
    let work = d.join("work");
    std::fs::create_dir_all(work.join("data")).unwrap();
    assert_eq!(check_roots(&work.join("data"), &work), Err(GitwError::WorktreeRootInvalid));
    assert_eq!(check_roots(&data, &d.join("sibling")), Ok(()));
    assert!(work_root(Path::new("/Users/u")).ends_with("Library/Application Support/VajraWorktrees"));
}

/// End to end: a normal edit is snapshotted, integrated and removed; no planted hook runs at any step; the worker's
/// HEAD and index never move, so the tree is dirty until the capture check lets it go.
#[test]
fn a_normal_edit_is_snapshotted_integrated_and_removed_and_no_hook_runs() {
    let f = fx("e2e");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    assert!(t.wt.starts_with(&f.gw.work) && t.private.starts_with(&f.gw.data) && t.tmp.starts_with(&f.gw.work));
    edit(&t, "src/lib.rs", "fn a() { 1 }\n");
    edit(&t, "target/build.o", "ignored\n");
    let s = f.gw.snapshot(&t, &globs(), &[], "codex vendor/model-a").unwrap();
    let commit = s.commit.clone().unwrap();
    assert_eq!(s.files, vec!["src/lib.rs".to_string()]);
    assert!(s.leftovers.is_empty(), "{:?}", s.leftovers);
    assert_eq!(rev(&f, "refs/heads/vajra/r1/task-t1-r1"), commit);
    let msg = out(ugit(&f.home, &f.repo, &["log", "-1", "--format=%B", &commit]));
    assert!(msg.contains("Vajra-Worker: codex vendor/model-a"), "{msg}");
    // The worker's HEAD and index are untouched: still detached at dispatch_head, and dirty.
    assert_eq!(out(ugit(&f.home, &t.wt, &["rev-parse", "HEAD"])), f.base);
    assert!(!out(ugit(&f.home, &t.wt, &["-c", "core.hooksPath=/dev/null", "status", "--porcelain"])).is_empty());
    // Not removed before integrate.
    let it = f.gw.integration_tree(&f.repo, "p", "r1", 1, &f.base).unwrap();
    let iref = Gitw::integration_ref(&it);
    assert_eq!(f.gw.worktree_remove(&t, &commit, &iref).unwrap_err().code(), "kept", "not integrated yet");
    assert!(t.wt.exists());
    let head = f.gw.integrate(&it, &f.base, std::slice::from_ref(&commit)).unwrap();
    assert_eq!(rev(&f, "refs/heads/vajra/r1/integration"), head);
    f.gw.worktree_remove(&t, &commit, &iref).unwrap();
    assert!(!t.wt.exists() && !t.gitdir.exists() && !t.private.exists());
    f.gw.branch_delete(&f.repo, &t.task_ref()).unwrap();
    assert_eq!(rev(&f, "main"), f.base, "main never moves");
    no_marker(&f);
}

#[test]
fn the_capture_check_keeps_a_tree_that_differs_from_its_commit() {
    let f = fx("capture");
    let it = f.gw.integration_tree(&f.repo, "p", "r1", 1, &f.base).unwrap();
    let iref = Gitw::integration_ref(&it);
    // (label, change after snapshot, the path the card names)
    type After = fn(&Ticket);
    let cases: [(&str, After, &str); 5] = [
        ("post-snapshot edit", |t| edit(t, "src/lib.rs", "changed again\n"), "src/lib.rs"),
        ("restored to dispatch", |t| edit(t, "src/lib.rs", "fn a() {}\n"), "src/lib.rs"),
        ("mode only", |t| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(t.wt.join("src/lib.rs"), std::fs::Permissions::from_mode(0o755)).unwrap()
        }, "src/lib.rs"),
        ("deleted", |t| std::fs::remove_file(t.wt.join("src/lib.rs")).unwrap(), "src/lib.rs"),
        ("leftover", |t| edit(t, "other/x.rs", "leftover\n"), "other/x.rs"),
    ];
    let mut commits = Vec::new();
    let mut tickets = Vec::new();
    for (i, (_, _, _)) in cases.iter().enumerate() {
        let t = f.gw.worktree_add(&f.repo, "p", "r1", &format!("c{i}"), 1, &f.base).unwrap();
        edit(&t, "src/lib.rs", &format!("snap {i}\n"));
        let c = f.gw.snapshot(&t, &globs(), &[], "w m").unwrap().commit.unwrap();
        commits.push(c);
        tickets.push(t);
    }
    // Each touches the same file differently; integrate one at a time into its own round.
    for (i, (label, after, path)) in cases.iter().enumerate() {
        let it = if i == 0 { it.clone() } else { f.gw.integration_tree(&f.repo, "p", "r1", i as u32 + 1, &f.base).unwrap() };
        let iref = if i == 0 { iref.clone() } else { Gitw::integration_ref(&it) };
        f.gw.integrate(&it, &f.base, &[commits[i].clone()]).unwrap();
        after(&tickets[i]);
        match f.gw.worktree_remove(&tickets[i], &commits[i], &iref) {
            Err(GitwError::Kept { paths }) => assert_eq!(paths, vec![path.to_string()], "{label}"),
            other => panic!("{label}: {other:?}"),
        }
        assert!(tickets[i].wt.exists(), "{label}: kept");
    }
    no_marker(&f);
}

#[test]
fn snapshot_stages_only_the_task_files_and_ignores_what_the_worker_staged() {
    let f = fx("staged");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "allowed\n");
    edit(&t, "other/x.rs", "out of task\n");
    edit(&t, "AGENTS.md", "injected\n");
    edit(&t, "src/.claude/settings.json", "{}\n");
    edit(&t, "src/CLAUDE.md", "x\n");
    // The worker pre-stages the out-of-task file and the AGENTS.md edit in its own index.
    ugit(&f.home, &t.wt, &["add", "other/x.rs", "AGENTS.md"]);
    let s = f.gw.snapshot(&t, &["src/**".into(), "AGENTS.md".into()], &[], "w m").unwrap();
    assert_eq!(s.files, vec!["src/lib.rs".to_string()]);
    let mut l = s.leftovers.clone();
    l.sort();
    assert_eq!(l, vec!["AGENTS.md", "other/x.rs", "src/.claude/settings.json", "src/CLAUDE.md"]);
    let names = out(ugit(&f.home, &f.repo, &["diff-tree", "-r", "--name-only", &f.base, &s.commit.unwrap()]));
    assert_eq!(names, "src/lib.rs");
    // The ask names AGENTS.md: then it is committed.
    let t2 = f.gw.worktree_add(&f.repo, "p", "r1", "t2", 1, &f.base).unwrap();
    edit(&t2, "AGENTS.md", "named\n");
    let s2 = f.gw.snapshot(&t2, &["AGENTS.md".into()], &["AGENTS.md".into()], "w m").unwrap();
    assert_eq!(s2.files, vec!["AGENTS.md".to_string()]);
}

#[test]
fn a_tree_holding_a_path_outside_the_list_is_tampered() {
    let f = fx("forced");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "allowed\n");
    edit(&t, "other/x.rs", "sneaked\n");
    EXTRA_ADD.with(|e| *e.borrow_mut() = Some("other/x.rs".into()));
    let r = f.gw.snapshot(&t, &globs(), &[], "w m");
    EXTRA_ADD.with(|e| *e.borrow_mut() = None);
    assert_eq!(r.unwrap_err().code(), "tampered");
    assert!(ugit(&f.home, &f.repo, &["for-each-ref", "refs/heads/vajra/"]).stdout.is_empty(), "nothing committed");
}

#[test]
fn update_ref_never_overwrites_a_task_ref() {
    let f = fx("noover");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "one\n");
    let first = f.gw.snapshot(&t, &globs(), &[], "w m").unwrap().commit.unwrap();
    edit(&t, "src/lib.rs", "two\n");
    assert_eq!(f.gw.snapshot(&t, &globs(), &[], "w m"), Err(GitwError::Exists));
    assert_eq!(rev(&f, "refs/heads/vajra/r1/task-t1-r1"), first);
}

#[test]
fn a_rewritten_git_file_a_worker_commit_and_a_nested_repo_are_tampered() {
    let f = fx("tamper");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    std::fs::write(t.wt.join(".git"), format!("gitdir: {}\n\n", t.gitdir.display())).unwrap();
    assert_eq!(f.gw.snapshot(&t, &globs(), &[], "w m").unwrap_err(), GitwError::Tampered { what: ".git".into() });

    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t2", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    // The worker's own git (hooks off, so the marker only ever means the core ran one).
    ugit(&f.home, &t.wt, &["-c", "core.hooksPath=/dev/null", "-c", "user.name=W", "-c", "user.email=w@e", "commit", "-qam", "worker commit"]);
    assert_eq!(f.gw.snapshot(&t, &globs(), &[], "w m").unwrap_err(), GitwError::Tampered { what: "HEAD".into() });

    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t3", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    std::fs::create_dir_all(t.wt.join("src/inner")).unwrap();
    ugit(&f.home, &t.wt.join("src/inner"), &["init", "-q"]);
    edit(&t, "src/inner/f", "x\n");
    assert_eq!(f.gw.snapshot(&t, &globs(), &[], "w m").unwrap_err().code(), "tampered");
    no_marker(&f);
}

#[test]
fn gitattributes_at_any_depth_and_gitmodules_are_tampered_before_add_and_no_filter_runs() {
    let f = fx("attrs");
    let mark = marker_script(&f.d, &f.marker);
    // The user's own trusted filter, defined at app start; a worker only adds the attribute that would apply it.
    std::fs::write(f.home.join(".gitconfig"), format!("[user]\n\tname = U\n\temail = u@e\n[filter \"x\"]\n\tclean = {}\n", mark.display())).unwrap();
    for (task, path) in [("t1", "src/sub/dir/.gitattributes"), ("t2", ".gitattributes"), ("t3", ".gitmodules")] {
        let t = f.gw.worktree_add(&f.repo, "p", "r1", task, 1, &f.base).unwrap();
        edit(&t, "src/lib.rs", "x\n");
        edit(&t, path, "* filter=x\n");
        assert_eq!(f.gw.snapshot(&t, &["**".into()], &[], "w m").unwrap_err(), GitwError::Tampered { what: path.into() }, "{path}");
    }
    no_marker(&f);
}

#[test]
fn a_repo_config_change_is_tampered_and_refuses_new_worktrees() {
    let f = fx("repocfg");
    let mark = marker_script(&f.d, &f.marker);
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    edit(&t, "src/.keep", "x\n");
    ugit(&f.home, &f.repo, &["config", "filter.x.clean", mark.to_str().unwrap()]);
    ugit(&f.home, &f.repo, &["config", "gpg.program", mark.to_str().unwrap()]);
    match f.gw.snapshot(&t, &globs(), &[], "w m") {
        Err(GitwError::Tampered { what }) => assert!(what.contains("filter.x.clean") && what.contains("gpg.program") && !what.contains(mark.to_str().unwrap()), "names only: {what}"),
        other => panic!("{other:?}"),
    }
    match f.gw.worktree_add(&f.repo, "p", "r1", "t2", 1, &f.base) {
        Err(GitwError::RepoConfigChanged { keys }) => assert!(keys.contains(&"filter.x.clean".to_string())),
        other => panic!("{other:?}"),
    }
    // A second run on the repo reuses the held baseline and never takes the changed file as trusted.
    let common = f.gw.common_of(&f.repo).unwrap();
    f.gw.repos.acquire(&f.gw, &common, false).unwrap();
    assert_eq!(f.gw.worktree_add(&f.repo, "p", "r2", "t1", 1, &f.base).unwrap_err().code(), "repo_config_changed");
    // The human accepts it only with no agent session on the repo.
    assert!(f.gw.repos.accept_new(&f.gw, &common, true).is_err());
    f.gw.repos.accept_new(&f.gw, &common, false).unwrap();
    f.gw.repos.accept_programs(&common, false).unwrap();
    assert!(f.gw.worktree_add(&f.repo, "p", "r2", "t2", 1, &f.base).is_ok());
    no_marker(&f);
}

#[test]
fn a_gitattributes_created_after_snapshot_is_refused_before_the_capture_add() {
    let f = fx("capattr");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    let c = f.gw.snapshot(&t, &globs(), &[], "w m").unwrap().commit.unwrap();
    let it = f.gw.integration_tree(&f.repo, "p", "r1", 1, &f.base).unwrap();
    f.gw.integrate(&it, &f.base, std::slice::from_ref(&c)).unwrap();
    edit(&t, "src/.gitattributes", "* filter=x\n");
    assert_eq!(f.gw.worktree_remove(&t, &c, &Gitw::integration_ref(&it)).unwrap_err().code(), "tampered");
    assert!(t.wt.exists());
}

#[test]
fn info_attributes_in_the_common_dir_is_a_tamper_signal() {
    let f = fx("infoattr");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    std::fs::write(f.repo.join(".git/info/attributes"), "* filter=x\n").unwrap();
    match f.gw.snapshot(&t, &globs(), &[], "w m").unwrap_err() {
        GitwError::Tampered { what } => assert!(what.contains("info/attributes"), "{what}"),
        e => panic!("{e:?}"),
    }
}

/// Two tickets and the frozen global config: the base commit has `* filter=x`, and `~/.gitconfig` at app start
/// defines a harmless `filter.x`. Task A rewrites it to a command that writes a marker; task B is then created,
/// snapshotted and integrated with the baseline copy, and the marker never appears. The global change is a card note,
/// never a tamper signal.
#[test]
fn a_global_filter_changed_mid_run_never_runs_and_is_only_a_note() {
    let f = fx_with("global", "[user]\n\tname = U\n\temail = u@e\n[filter \"x\"]\n\tclean = cat\n\tsmudge = cat\n", &[(".gitattributes", "*.rs filter=x\n")]);
    let mark = marker_script(&f.d, &f.marker);
    // The baseline's filter is not git-lfs: the human accepts it before the run (#146 re-check N2).
    f.gw.repos.accept_programs(&f.gw.common_of(&f.repo).unwrap(), false).unwrap();
    let a = f.gw.worktree_add(&f.repo, "p", "r1", "ta", 1, &f.base).unwrap();
    // Task A (a Claude worker has no OS sandbox) edits the user's global config, and an include it adds.
    std::fs::write(f.home.join(".gitconfig"), format!("[user]\n\tname = U\n\temail = u@e\n[include]\n\tpath = ~/.extra\n[filter \"x\"]\n\tclean = {m}\n\tsmudge = {m}\n", m = mark.display())).unwrap();
    std::fs::write(f.home.join(".extra"), format!("[filter \"y\"]\n\tclean = {}\n", mark.display())).unwrap();
    std::fs::create_dir_all(f.home.join(".config/git")).unwrap();
    std::fs::write(f.home.join(".config/git/config"), "[core]\n\tpager = cat\n").unwrap();
    edit(&a, "src/lib.rs", "a\n");
    let ca = f.gw.snapshot(&a, &globs(), &[], "w m").unwrap().commit.unwrap();
    let mut b = f.gw.worktree_add(&f.repo, "p", "r1", "tb", 1, &f.base).unwrap();
    f.gw.merge_deps(&mut b, std::slice::from_ref(&ca)).unwrap();
    edit(&b, "src/lib.rs", "b\n");
    edit(&b, "src/new.rs", "new\n");
    let cb = f.gw.snapshot(&b, &globs(), &[], "w m").unwrap().commit.unwrap();
    let it = f.gw.integration_tree(&f.repo, "p", "r1", 1, &f.base).unwrap();
    f.gw.integrate(&it, &f.base, &[ca, cb.clone()]).unwrap();
    f.gw.worktree_remove(&b, &cb, &Gitw::integration_ref(&it)).unwrap();
    no_marker(&f);
    let g = GlobalBaseline::take_or_load(Path::new(GIT), &f.gw.data, &f.home, None).unwrap();
    let changed = g.changed_files(Path::new(GIT), &f.home, None);
    assert!(changed.contains(&f.home.join(".gitconfig")) && changed.contains(&f.home.join(".extra")) && changed.contains(&f.home.join(".config/git/config")), "{changed:?}");
    let keys = g.changed_keys(Path::new(GIT), &f.home, None);
    assert!(keys.contains(&"filter.x.clean".to_string()) && keys.contains(&"filter.y.clean".to_string()), "{keys:?}");
    assert!(keys.iter().any(|k| highlighted(k)));
}

#[test]
fn the_global_baseline_survives_a_restart_and_is_replaced_only_by_a_human_with_no_agents() {
    let f = fx("restart");
    let copy = std::fs::read_to_string(&f.gw.global).unwrap();
    assert!(copy.contains("email = \"u@e\""), "{copy}");
    std::fs::write(f.home.join(".gitconfig"), "[user]\n\tname = New\n\temail = new@e\n").unwrap();
    let mut g = GlobalBaseline::take_or_load(Path::new(GIT), &f.gw.data, &f.home, None).unwrap();
    assert_eq!(std::fs::read_to_string(&g.copy).unwrap(), copy, "a restart keeps the stored baseline");
    assert!(g.replace(Path::new(GIT), &f.home, None, true).is_err(), "disabled while an agent session runs");
    g.replace(Path::new(GIT), &f.home, None, false).unwrap();
    assert!(std::fs::read_to_string(&g.copy).unwrap().contains("new@e"));
    assert!(g.changed_files(Path::new(GIT), &f.home, None).is_empty());
}

/// A worker points `commondir` at its own directory: the original objects plus a `config` whose `filter.x.clean`
/// writes a marker and an `info/attributes` that applies it. HEAD still reads `dispatch_head` and no `.gitattributes`
/// changes: the plain-filesystem check catches it before any `add`. With that check off, the pinned `GIT_COMMON_DIR`
/// still keeps git from reading the planted config.
#[test]
fn a_redirected_commondir_is_tampered_and_the_pin_still_stops_the_filter() {
    let f = fx("commondir");
    let mark = marker_script(&f.d, &f.marker);
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    let evil = f.d.join("evil");
    std::fs::create_dir_all(evil.join("info")).unwrap();
    std::os::unix::fs::symlink(f.repo.join(".git/objects"), evil.join("objects")).unwrap();
    std::os::unix::fs::symlink(f.repo.join(".git/refs"), evil.join("refs")).unwrap();
    std::fs::copy(f.repo.join(".git/HEAD"), evil.join("HEAD")).unwrap();
    std::fs::create_dir_all(evil.join("worktrees")).unwrap();
    std::os::unix::fs::symlink(&t.gitdir, evil.join("worktrees/t1-r1")).unwrap();
    std::fs::write(evil.join("config"), format!("[core]\n\trepositoryformatversion = 0\n[filter \"x\"]\n\tclean = {}\n", mark.display())).unwrap();
    std::fs::write(evil.join("info/attributes"), "* filter=x\n").unwrap();
    std::fs::write(t.gitdir.join("commondir"), format!("{}\n", evil.display())).unwrap();
    assert_eq!(f.gw.snapshot(&t, &globs(), &[], "w m").unwrap_err(), GitwError::Tampered { what: "commondir".into() });
    no_marker(&f);
    f.gw.checks.store(false, Ordering::SeqCst);
    let s = f.gw.snapshot(&t, &globs(), &[], "w m").unwrap();
    assert!(s.commit.is_some());
    no_marker(&f);
}

/// A worker runs `git symbolic-ref HEAD refs/heads/main` while `main` sits at `dispatch_head`: the sha alone can't see
/// it, the detached check can. With that check off, `snapshot`'s plumbing still never moves `main`.
#[test]
fn a_symbolic_head_is_tampered_and_main_never_moves() {
    let f = fx("symref");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    std::fs::write(t.gitdir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    assert_eq!(f.gw.snapshot(&t, &globs(), &[], "w m").unwrap_err(), GitwError::Tampered { what: "HEAD".into() });
    f.gw.checks.store(false, Ordering::SeqCst);
    let s = f.gw.snapshot(&t, &globs(), &[], "w m").unwrap();
    assert!(s.commit.is_some());
    assert_eq!(rev(&f, "main"), f.base, "main never moves");
}

#[test]
fn a_missing_user_email_falls_back_and_a_hanging_gpg_never_blocks() {
    let f = fx_with("ident", "", &[]);
    let hang = f.d.join("gpg.sh");
    std::fs::write(&hang, format!("#!/bin/sh\necho gpg >> '{}'\nsleep 600\n", f.marker.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hang, std::fs::Permissions::from_mode(0o755)).unwrap();
    // The user's own setup, before the run: signing on, with a gpg that hangs.
    ugit(&f.home, &f.repo, &["config", "commit.gpgsign", "true"]);
    ugit(&f.home, &f.repo, &["config", "gpg.program", hang.to_str().unwrap()]);
    let common = f.gw.common_of(&f.repo).unwrap();
    f.gw.repos.accept_new(&f.gw, &common, false).unwrap();
    f.gw.repos.accept_programs(&common, false).unwrap();
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    let started = std::time::Instant::now();
    let c = f.gw.snapshot(&t, &globs(), &[], "w m").unwrap().commit.unwrap();
    assert!(started.elapsed() < Duration::from_secs(20));
    let who = out(ugit(&f.home, &f.repo, &["log", "-1", "--format=%an <%ae>", &c]));
    assert_eq!(who, "Vajra <vajra@localhost>");
    let it = f.gw.integration_tree(&f.repo, "p", "r1", 1, &f.base).unwrap();
    f.gw.integrate(&it, &f.base, &[c]).unwrap();
    no_marker(&f);
}

#[test]
fn deps_merge_at_dispatch_and_a_conflict_is_typed() {
    let f = fx("deps");
    let a = f.gw.worktree_add(&f.repo, "p", "r1", "ta", 1, &f.base).unwrap();
    edit(&a, "src/lib.rs", "from a\n");
    let ca = f.gw.snapshot(&a, &globs(), &[], "w m").unwrap().commit.unwrap();
    let b = f.gw.worktree_add(&f.repo, "p", "r1", "tb", 1, &f.base).unwrap();
    edit(&b, "src/lib.rs", "from b\n");
    let cb = f.gw.snapshot(&b, &globs(), &[], "w m").unwrap().commit.unwrap();
    let mut c = f.gw.worktree_add(&f.repo, "p", "r1", "tc", 1, &f.base).unwrap();
    f.gw.merge_deps(&mut c, std::slice::from_ref(&ca)).unwrap();
    assert_ne!(c.dispatch_head, f.base);
    f.gw.preflight(&c).unwrap();
    let mut d = f.gw.worktree_add(&f.repo, "p", "r1", "td", 1, &f.base).unwrap();
    assert_eq!(f.gw.merge_deps(&mut d, &[ca, cb]).unwrap_err().code(), "deps_conflict");
    no_marker(&f);
}

#[test]
fn agent_config_and_globs_match_as_specified() {
    for p in [".claude/settings.json", "a/.claude/x", ".mcp.json", "CLAUDE.md", "sub/AGENTS.md", ".codex/c.toml", ".envrc", ".husky/pre-commit", ".github/workflows/ci.yml"] {
        assert!(agent_config(p), "{p}");
    }
    // The worktree volume is case-insensitive (#146 gate H1).
    for p in [".Claude/settings.json", "a/.CLAUDE/x", ".MCP.json", "Claude.md", "docs/claude.md", "sub/Agents.md", ".Codex/c.toml", ".ENVRC", ".Husky/pre-commit", ".GitHub/Workflows/ci.yml"] {
        assert!(agent_config(p), "{p}");
    }
    for p in ["src/lib.rs", "docs/claude.txt", ".github/CODEOWNERS"] {
        assert!(!agent_config(p), "{p}");
    }
    assert!(glob_match("src/**", "src/a/b.rs") && glob_match("src/", "src/x") && glob_match("**/*.rs", "a.rs") && glob_match("a?c", "abc"));
    assert!(!glob_match("src/*", "src/a/b.rs") && !glob_match("../x", "../x") && !glob_match("src/**", "srcx/a"));
    // Accepted patterns against long adversarial names finish in bounded time (#144 review).
    let name = "a".repeat(200);
    let deep = "a/".repeat(500) + "b";
    let started = std::time::Instant::now();
    assert!(!glob_match("*a*a*a*a*a*a*a*a*a*a*a*b", &name));
    assert!(glob_match("*a*a*a*a*a*a*a*a*a*a*a*", &name));
    assert!(!glob_match(&("*".repeat(40) + "b"), &name));
    assert!(!glob_match("**/**/**/**/**/**/**/**/x", &deep));
    assert!(glob_match("**/*a*/**/*a*/**/b", &deep));
    assert!(!glob_match("**/*a*a*a*a*/**/*a*a*a*/**/c", &format!("{deep}/{name}")));
    assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    assert_eq!(parse_list_z("core.bare\0user.email\nu@e\0"), vec![("core.bare".into(), "true".into()), ("user.email".into(), "u@e".into())]);
}

/// The shipped sandbox policy, unchanged: it denies agent writes to `<data>`, and no deny entry covers `<work>`, so a
/// Claude seat under it can build in its worktree while the core's state stays out of reach.
#[test]
fn the_shipped_sandbox_denies_data_and_leaves_work_writable() {
    let s: serde_json::Value = serde_json::from_str(include_str!("../../../../config/sandbox.json")).unwrap();
    let home = Path::new("/Users/u");
    let fs = &s["settings"]["sandbox"]["filesystem"];
    let expand = |v: &serde_json::Value| -> Vec<PathBuf> {
        v.as_array().unwrap().iter().map(|x| home.join(x.as_str().unwrap().trim_start_matches("~/"))).collect()
    };
    let (deny_write, deny_read) = (expand(&fs["denyWrite"]), expand(&fs["denyRead"]));
    let data = crate::paths::resolve_data_dir(None, home);
    let work = work_root(home);
    assert!(deny_write.iter().any(|d| data.starts_with(d)), "the shipped policy must keep denying <data>");
    assert!(!deny_write.iter().chain(&deny_read).any(|d| work.starts_with(d) || d.starts_with(&work)), "<work> must stay writable");
    assert!(!data.starts_with(&work) && !work.starts_with(&data));
}

/// A failed `worktree add` (an id that is not a commit) and a failure after it succeeded both leave nothing behind:
/// no registered worktree, no private or tmp dir, so the same ticket can be retried (#144 review).
#[test]
fn a_failed_worktree_add_rolls_back_and_the_ticket_can_be_retried() {
    let f = fx("rollback");
    let blob = out(ugit(&f.home, &f.repo, &["rev-parse", "HEAD:a.txt"]));
    assert_eq!(f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &blob).unwrap_err().code(), "git_failed");
    FAIL_AFTER_ADD.with(|x| x.set(true));
    let r = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base);
    FAIL_AFTER_ADD.with(|x| x.set(false));
    assert_eq!(r.unwrap_err().code(), "tampered");
    let list = out(ugit(&f.home, &f.repo, &["worktree", "list", "--porcelain"]));
    assert_eq!(list.matches("worktree ").count(), 1, "only the main worktree is registered: {list}");
    assert!(!f.gw.work.join("p/r1/t1-r1").exists() && !f.gw.work.join("p/r1/tmp/t1-r1").exists() && !f.gw.data.join("tickets/p/r1/t1-r1").exists());
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    assert!(t.wt.exists() && t.private.exists() && t.tmp.exists());
    no_marker(&f);
}

/// #146 gate H1: on a case-insensitive volume `.Claude/settings.json` is `.claude/settings.json`, and `.GitAttributes`
/// is `.gitattributes`. None of them is committed, and no filter runs.
#[test]
fn case_variants_of_agent_config_and_git_names_are_caught() {
    let f = fx("case");
    let mark = marker_script(&f.d, &f.marker);
    std::fs::write(f.home.join(".gitconfig"), format!("[user]\n\tname = U\n\temail = u@e\n[filter \"x\"]\n\tclean = {}\n", mark.display())).unwrap();
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t0", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    edit(&t, ".Claude/settings.json", "{\"hooks\":{}}\n");
    edit(&t, "src/claude.md", "x\n");
    edit(&t, ".MCP.json", "{}\n");
    let s = f.gw.snapshot(&t, &["**".into()], &[], "w m").unwrap();
    assert_eq!(s.files, vec!["src/lib.rs".to_string()]);
    let mut l = s.leftovers.clone();
    l.sort();
    assert_eq!(l, vec![".Claude/settings.json", ".MCP.json", "src/claude.md"]);
    for (task, path) in [("t1", "src/.GitAttributes"), ("t2", ".GITMODULES"), ("t3", "src/.GIT/config")] {
        let t = f.gw.worktree_add(&f.repo, "p", "r1", task, 1, &f.base).unwrap();
        edit(&t, "src/lib.rs", "x\n");
        edit(&t, path, "* filter=x\n");
        assert_eq!(f.gw.snapshot(&t, &["**".into()], &[], "w m").unwrap_err().code(), "tampered", "{path}");
    }
    no_marker(&f);
}

/// #146 gate H2: the frozen copy in `<data>` is checked against the hash taken at load before every call, and across a
/// restart against the anchor in `<control>`. A changed copy is never used, and only the human re-takes it.
#[test]
fn a_rewritten_global_baseline_copy_is_tampered_and_never_runs() {
    let f = fx_with("baseline", "[user]\n\tname = U\n\temail = u@e\n[filter \"x\"]\n\tclean = cat\n", &[(".gitattributes", "*.rs filter=x\n")]);
    let mark = marker_script(&f.d, &f.marker);
    // The baseline's filter is not git-lfs: the human accepts it before the run (#146 re-check N2).
    f.gw.repos.accept_programs(&f.gw.common_of(&f.repo).unwrap(), false).unwrap();
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    let good = std::fs::read(&f.gw.global).unwrap();
    let mut evil = good.clone();
    evil.extend(format!("[filter \"x\"]\n\tclean = {}\n", mark.display()).as_bytes());
    std::fs::write(&f.gw.global, &evil).unwrap();
    assert_eq!(f.gw.snapshot(&t, &globs(), &[], "w m").unwrap_err(), GitwError::Tampered { what: "the global git baseline".into() });
    assert_eq!(f.gw.worktree_add(&f.repo, "p", "r1", "t2", 1, &f.base).unwrap_err().code(), "tampered");
    no_marker(&f);
    let load = || GlobalBaseline::take_or_load(Path::new(GIT), &f.gw.data, &f.home, None);
    assert_eq!(load().unwrap_err().code(), "tampered", "a restart refuses it too");
    std::fs::write(&f.gw.global, &good).unwrap();
    assert!(load().is_ok(), "the original bytes load");
    // A deleted anchor, and a deleted copy with the anchor left behind.
    let anchor = f.gw.data.join("control/git-baseline.sha256");
    let a = std::fs::read(&anchor).unwrap();
    std::fs::remove_file(&anchor).unwrap();
    assert_eq!(load().unwrap_err().code(), "tampered");
    std::fs::write(&anchor, &a).unwrap();
    std::fs::remove_file(&f.gw.global).unwrap();
    assert_eq!(load().unwrap_err().code(), "tampered");
    // The human re-takes it, only with no agent session running.
    assert!(GlobalBaseline::retake(Path::new(GIT), &f.gw.data, &f.home, None, true).is_err());
    let g = GlobalBaseline::retake(Path::new(GIT), &f.gw.data, &f.home, None, false).unwrap();
    f.gw.use_global(&g).unwrap();
    assert!(load().is_ok());
    assert!(f.gw.snapshot(&t, &globs(), &[], "w m").unwrap().commit.is_some());
    no_marker(&f);
}

/// #146 gate M1: a `.gitattributes` hidden by a `.gitignore`, and a tracked one rewritten to apply a filter the user
/// trusts, are caught by the filesystem walk before git reads any file content, with `GIT_ATTR_SOURCE` off (git
/// before 2.40). A tracked, unchanged `.gitattributes` passes.
#[test]
fn an_ignored_or_rewritten_gitattributes_is_caught_before_git_reads_content() {
    let mut f = fx_with("ignattr", "[user]\n\tname = U\n\temail = u@e\n", &[("keep/.gitattributes", "*.bin binary\n")]);
    f.gw.attr_source = false;
    let mark = marker_script(&f.d, &f.marker);
    std::fs::write(f.home.join(".gitconfig"), format!("[user]\n\tname = U\n\temail = u@e\n[filter \"x\"]\n\tclean = {}\n", mark.display())).unwrap();
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    assert!(f.gw.snapshot(&t, &globs(), &[], "w m").unwrap().commit.is_some(), "the tracked one passes");
    let cases: [(&str, &[(&str, &str)]); 2] = [
        ("t2", &[("src/sub/.gitignore", "*\n"), ("src/sub/.gitattributes", "* filter=x\n")]),
        ("t3", &[("keep/.gitattributes", "* filter=x\n")]),
    ];
    for (task, files) in cases {
        let t = f.gw.worktree_add(&f.repo, "p", "r1", task, 1, &f.base).unwrap();
        edit(&t, "src/lib.rs", "x\n");
        for (p, c) in files {
            edit(&t, p, c);
        }
        assert_eq!(f.gw.snapshot(&t, &globs(), &[], "w m").unwrap_err().code(), "tampered", "{task}");
    }
    no_marker(&f);
}

/// #146 gate M2: a filter planted in a file the repo config includes, or in the main worktree's `config.worktree`, is
/// a repo config change.
#[test]
fn a_changed_repo_config_include_or_config_worktree_is_tampered() {
    let f = fx("include");
    let mark = marker_script(&f.d, &f.marker);
    let inc = f.d.join("shared.gitconfig");
    std::fs::write(&inc, "[core]\n\tpager = cat\n").unwrap();
    ugit(&f.home, &f.repo, &["config", "include.path", inc.to_str().unwrap()]);
    let common = f.gw.common_of(&f.repo).unwrap();
    f.gw.repos.accept_new(&f.gw, &common, false).unwrap();
    f.gw.repos.accept_programs(&common, false).unwrap();
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    std::fs::write(&inc, format!("[filter \"x\"]\n\tclean = {}\n", mark.display())).unwrap();
    match f.gw.snapshot(&t, &globs(), &[], "w m") {
        Err(GitwError::Tampered { what }) => assert!(what.contains("include.path"), "{what}"),
        other => panic!("{other:?}"),
    }
    std::fs::write(&inc, "[core]\n\tpager = cat\n").unwrap();
    std::fs::write(common.join("config.worktree"), format!("[filter \"x\"]\n\tclean = {}\n", mark.display())).unwrap();
    match f.gw.worktree_add(&f.repo, "p", "r1", "t2", 1, &f.base) {
        Err(GitwError::RepoConfigChanged { keys }) => assert_eq!(keys, vec!["config.worktree".to_string()]),
        other => panic!("{other:?}"),
    }
    no_marker(&f);
}

/// #146 gate M4: a `tmp` dir planted as a symlink into `<data>` never becomes a ticket's tmp, and the rollback never
/// deletes through it. A run dir that is a link is refused before git runs.
#[test]
fn a_planted_dir_symlink_is_refused_and_nothing_is_deleted_through_it() {
    let f = fx("tmplink");
    let target = f.gw.data.join("elsewhere");
    std::fs::create_dir_all(target.join("t1-r1")).unwrap();
    std::fs::write(target.join("t1-r1/keep"), "x").unwrap();
    std::fs::create_dir_all(f.gw.work.join("p/r1")).unwrap();
    std::os::unix::fs::symlink(&target, f.gw.work.join("p/r1/tmp")).unwrap();
    assert_eq!(f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap_err().code(), "tampered");
    assert!(target.join("t1-r1/keep").exists(), "nothing deleted through the link");
    let list = out(ugit(&f.home, &f.repo, &["worktree", "list", "--porcelain"]));
    assert_eq!(list.matches("worktree ").count(), 1, "rolled back: {list}");
    std::fs::create_dir_all(f.gw.work.join("q")).unwrap();
    std::os::unix::fs::symlink(&f.repo, f.gw.work.join("q/r1")).unwrap();
    assert_eq!(f.gw.worktree_add(&f.repo, "q", "r1", "t1", 1, &f.base).unwrap_err().code(), "tampered");
    assert!(!f.repo.join("t1-r1").exists());
}

/// #146 gate L1: a worker moves the integration branch (refs live in the common dir) between merges.
#[test]
fn a_moved_integration_branch_is_tampered() {
    let f = fx("intmove");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    let c = f.gw.snapshot(&t, &globs(), &[], "w m").unwrap().commit.unwrap();
    let evil = out(ugit(&f.home, &f.repo, &["-c", "user.name=W", "-c", "user.email=w@e", "commit-tree", &format!("{}^{{tree}}", f.base), "-p", &f.base, "-m", "evil"]));
    let it = f.gw.integration_tree(&f.repo, "p", "r1", 1, &f.base).unwrap();
    let (home, repo, e) = (f.home.clone(), f.repo.clone(), evil.clone());
    BEFORE_MERGE.with(|b| *b.borrow_mut() = Some(Box::new(move || {
        ugit(&home, &repo, &["-c", "core.hooksPath=/dev/null", "update-ref", "refs/heads/vajra/r1/integration", &e]);
    })));
    let r = f.gw.integrate(&it, &f.base, std::slice::from_ref(&c));
    BEFORE_MERGE.with(|b| *b.borrow_mut() = None);
    assert_eq!(r.unwrap_err(), GitwError::Tampered { what: "the integration branch".into() });
    assert_eq!(rev(&f, "refs/heads/vajra/r1/integration"), evil, "refused, never overwritten");
    no_marker(&f);
}

/// #146 gate L3: a symlink to an absolute path or out of the worktree is a leftover, never committed.
#[test]
fn escaping_symlinks_are_leftovers() {
    let f = fx("links");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    std::os::unix::fs::symlink("/etc/hosts", t.wt.join("src/abs")).unwrap();
    std::os::unix::fs::symlink("../../outside", t.wt.join("src/up")).unwrap();
    std::os::unix::fs::symlink("lib.rs", t.wt.join("src/ok")).unwrap();
    std::os::unix::fs::symlink("../a.txt", t.wt.join("src/ok2")).unwrap();
    let s = f.gw.snapshot(&t, &globs(), &[], "w m").unwrap();
    assert_eq!(s.files, vec!["src/lib.rs".to_string(), "src/ok".into(), "src/ok2".into()]);
    let mut l = s.leftovers.clone();
    l.sort();
    assert_eq!(l, vec!["src/abs", "src/up"]);
}

/// #146 gate L2: a link planted at the old predictable temp name is never followed.
#[test]
fn write_private_never_follows_a_planted_temp_name() {
    let d = tmpdir("wp");
    let victim = d.join("victim");
    std::fs::write(&victim, "keep").unwrap();
    std::os::unix::fs::symlink(&victim, d.join(format!("f.tmp{}", std::process::id()))).unwrap();
    write_private(&d.join("f"), b"new").unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
    assert_eq!(std::fs::read_to_string(d.join("f")).unwrap(), "new");
}

/// #146 gate M3: while the core's git touches a repo's content, every other live session on the repo is stopped
/// (its whole process group), so it can't change config or attributes between the check and the use; it resumes after.
#[test]
fn other_live_sessions_on_the_repo_are_stopped_during_content_calls() {
    use crate::bus::{Bus, Config};
    let d = tmpdir("quiesce");
    let bus = Arc::new(Bus::open(&d.join("spill"), Config::default()).unwrap());
    let sup = Arc::new(crate::sessions::Supervisor::new(bus, None));
    let ticks = d.join("ticks");
    let script = format!("while :; do echo x >> '{}'; sleep 0.02; done", ticks.display());
    let spec = crate::sessions::SpawnSpec { argv: vec!["sh".into(), "-c".into(), script], cwd: d.clone(), rows: 24, cols: 80, redact: Default::default(), extra_env: Vec::new() };
    sup.spawn("w-live", spec).unwrap();
    let q = SessionQuiesce::new(sup.clone(), |_| vec!["w-live".to_string(), "w-gone".to_string()]);
    let f = fx_q("quiesce-fx", "[user]\n\tname = U\n\temail = u@e\n", &[], Arc::new(q));
    let size = || std::fs::metadata(&ticks).map(|m| m.len()).unwrap_or(0);
    let grows = || {
        let a = size();
        std::thread::sleep(Duration::from_millis(300));
        size() > a
    };
    assert!(grows(), "the session runs before");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    let c = f.gw.snapshot(&t, &globs(), &[], "w m").unwrap().commit.unwrap();
    let it = f.gw.integration_tree(&f.repo, "p", "r1", 1, &f.base).unwrap();
    let seen = Arc::new(Mutex::new(None));
    let s2 = seen.clone();
    let t2 = ticks.clone();
    BEFORE_MERGE.with(|b| *b.borrow_mut() = Some(Box::new(move || {
        let a = std::fs::metadata(&t2).map(|m| m.len()).unwrap_or(0);
        std::thread::sleep(Duration::from_millis(300));
        *crate::lock(&s2) = Some(std::fs::metadata(&t2).map(|m| m.len()).unwrap_or(0) == a);
    })));
    let r = f.gw.integrate(&it, &f.base, std::slice::from_ref(&c));
    BEFORE_MERGE.with(|b| *b.borrow_mut() = None);
    r.unwrap();
    assert_eq!(*crate::lock(&seen), Some(true), "stopped inside the content call");
    assert!(grows(), "resumed after");
    sup.kill("w-live").unwrap();
    no_marker(&f);
}

/// #146 gate L4: a trusted filter program rewritten after the baseline, or shadowed by a new binary earlier on git's
/// search path, is a repo config change; the rewritten program never runs.
#[test]
fn a_changed_or_shadowed_filter_program_is_tampered() {
    let mut f = fx_with("prog", "[user]\n\tname = U\n\temail = u@e\n", &[(".gitattributes", "*.rs filter=x\n")]);
    use std::os::unix::fs::PermissionsExt;
    let tool = f.d.join("tool.sh");
    std::fs::write(&tool, "#!/bin/sh\ncat\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (early, late) = (f.d.join("bin-early"), f.d.join("bin-late"));
    for b in [&early, &late] {
        std::fs::create_dir_all(b).unwrap();
    }
    std::fs::copy(&tool, late.join("vtool")).unwrap();
    f.gw.prog_dirs = vec![early.clone(), late.clone()];
    ugit(&f.home, &f.repo, &["config", "filter.x.clean", tool.to_str().unwrap()]);
    ugit(&f.home, &f.repo, &["config", "merge.y.driver", "vtool %O %A %B"]);
    let common = f.gw.common_of(&f.repo).unwrap();
    f.gw.repos.accept_new(&f.gw, &common, false).unwrap();
    f.gw.repos.accept_programs(&common, false).unwrap();
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    assert!(f.gw.snapshot(&t, &globs(), &[], "w m").unwrap().commit.is_some());
    // The trusted script is rewritten to leave a marker.
    std::fs::write(&tool, format!("#!/bin/sh\necho ran >> '{}'\ncat\n", f.marker.display())).unwrap();
    let t2 = f.gw.worktree_add(&f.repo, "p", "r1", "t2", 1, &f.base);
    match t2 {
        Err(GitwError::RepoConfigChanged { keys }) => assert!(keys.iter().any(|k| k.starts_with("program ") && k.contains("tool.sh")), "{keys:?}"),
        other => panic!("{other:?}"),
    }
    no_marker(&f);
    // The human accepts it; then a new `vtool` shadows the trusted one.
    f.gw.repos.accept_new(&f.gw, &common, false).unwrap();
    f.gw.repos.accept_programs(&common, false).unwrap();
    std::fs::copy(late.join("vtool"), early.join("vtool")).unwrap();
    match f.gw.worktree_add(&f.repo, "p", "r1", "t3", 1, &f.base) {
        Err(GitwError::RepoConfigChanged { keys }) => assert_eq!(keys, vec!["program vtool".to_string()]),
        other => panic!("{other:?}"),
    }
    assert!(first_word("  \"/a b/c\" --x").as_deref() == Some("/a b/c") && first_word("git-lfs filter-process").as_deref() == Some("git-lfs"));
    assert!(program_key("filter.lfs.process") && program_key("merge.ours.driver") && program_key("diff.x.textconv") && !program_key("filter.process") && !program_key("user.email"));
}

/// #146 re-check N1: APFS case-folds beyond ASCII. Long s (U+017F) and the Kelvin sign (U+212A) spell protected
/// names; every probe form counts as that name, so none is committed and no filter runs.
#[test]
fn unicode_case_variants_of_protected_names_are_caught() {
    for p in [".mcp.j\u{17F}on", "AGENT\u{17F}.md", "sub/AGENT\u{17F}.md", ".hu\u{17F}ky/pre-commit", ".hus\u{212A}y/pre-commit", ".github/workflow\u{17F}/ci.yml", ".Codex/c.toml"] {
        assert!(agent_config(p), "{p}");
    }
    assert_eq!(git_special(".gitattribute\u{17F}"), Some(".gitattributes"));
    assert_eq!(git_special(".gitmodule\u{17F}"), Some(".gitmodules"));
    assert_eq!(git_special(".LFSCONFIG"), Some(".lfsconfig"));
    assert_eq!(git_special(".GIT"), Some(".git"));
    assert_eq!(git_special(".gitignore"), None);
    let f = fx("unicase");
    let mark = marker_script(&f.d, &f.marker);
    std::fs::write(f.home.join(".gitconfig"), format!("[user]\n\tname = U\n\temail = u@e\n[filter \"x\"]\n\tclean = {}\n", mark.display())).unwrap();
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t0", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    edit(&t, ".hu\u{17F}ky/pre-commit", "#!/bin/sh\necho hook\n");
    edit(&t, ".mcp.j\u{17F}on", "{}\n");
    edit(&t, "src/AGENT\u{17F}.md", "injected\n");
    let s = f.gw.snapshot(&t, &["**".into()], &[], "w m").unwrap();
    assert_eq!(s.files, vec!["src/lib.rs".to_string()], "leftovers: {:?}", s.leftovers);
    assert_eq!(s.leftovers.len(), 3, "{:?}", s.leftovers);
    for (task, path) in [("t1", "src/.gitattribute\u{17F}"), ("t2", ".gitmodule\u{17F}"), ("t3", "src/.lfsconfig"), ("t4", "src/.LFSCONFIG")] {
        let t = f.gw.worktree_add(&f.repo, "p", "r1", task, 1, &f.base).unwrap();
        edit(&t, "src/lib.rs", "x\n");
        edit(&t, path, "* filter=x\n");
        assert_eq!(f.gw.snapshot(&t, &["**".into()], &[], "w m").unwrap_err().code(), "tampered", "{path}");
    }
    no_marker(&f);
}

/// #146 re-check N1: where the volume folds a name, its own answer (same dev and ino) decides, whatever `fold` knows.
#[test]
fn the_volume_decides_what_a_name_is() {
    let d = tmpdir("vol");
    std::fs::write(d.join(".gitattributes"), "x").unwrap();
    std::fs::create_dir_all(d.join(".husky")).unwrap();
    assert_eq!(on_volume(&d, "other", &SPECIAL_GIT), None);
    assert_eq!(on_volume(&d, ".gitattributes", &SPECIAL_GIT), None, "the name itself is not a second spelling");
    if !d.join(".GITATTRIBUTES").exists() {
        return; // a case-sensitive volume: no other spelling exists
    }
    assert_eq!(on_volume(&d, ".GITATTRIBUTES", &SPECIAL_GIT), Some(".gitattributes"));
    assert_eq!(canonical(Some(&d), ".HUSKY/x"), ".husky/x");
    if d.join(".gitattribute\u{17F}").exists() {
        assert_eq!(on_volume(&d, ".gitattribute\u{17F}", &SPECIAL_GIT), Some(".gitattributes"));
    }
}

/// #146 re-check N2: a trusted `python3 -m` filter would import a module a worker planted in the worktree when the
/// core's git refreshes or adds. A program that isn't git-lfs makes the repo ineligible for content calls until the
/// human accepts it, and the planted module never runs.
#[test]
fn an_interpreter_filter_is_refused_until_the_human_accepts_it_and_planted_code_never_runs() {
    let f = fx_with("pyfilter", "[user]\n\tname = U\n\temail = u@e\n", &[("nb.ipynb", "{}\n")]);
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    // The shape `nbstripout --install` writes.
    std::fs::write(f.repo.join(".git/info/attributes"), "*.ipynb filter=nb\n").unwrap();
    ugit(&f.home, &f.repo, &["config", "filter.nb.clean", "/usr/bin/python3 -m nbx"]);
    let common = f.gw.common_of(&f.repo).unwrap();
    f.gw.repos.accept_new(&f.gw, &common, false).unwrap();
    edit(&t, "nbx.py", &format!("open('{}', 'a').write('ran')\n", f.marker.display()));
    edit(&t, "nb.ipynb", "{\"x\": 1}\n");
    match f.gw.snapshot(&t, &["**".into()], &[], "w m") {
        Err(GitwError::Tampered { what }) => assert!(what.contains("program filter.nb.clean"), "{what}"),
        other => panic!("{other:?}"),
    }
    match f.gw.worktree_add(&f.repo, "p", "r1", "t2", 1, &f.base) {
        Err(GitwError::RepoConfigChanged { keys }) => assert_eq!(keys, vec!["program filter.nb.clean".to_string()]),
        other => panic!("{other:?}"),
    }
    assert_eq!(f.gw.repos.unvetted_programs(&common), vec!["filter.nb.clean".to_string()]);
    no_marker(&f);
    // Only the human accepts it, with no agent session on the repo; then the card no longer lists it.
    assert!(f.gw.repos.accept_programs(&common, true).is_err());
    f.gw.repos.accept_programs(&common, false).unwrap();
    assert!(f.gw.repos.unvetted_programs(&common).is_empty());
    for c in ["git-lfs filter-process", "git-lfs clean -- %f", "git-lfs smudge -- %f", "/opt/homebrew/bin/git-lfs filter-process"] {
        assert!(vetted_program(c), "{c}");
    }
    for c in ["/usr/bin/python3 -m nbx", "python3 x.py", "node x.js", "sh ./x", "bundle exec x", "./git-lfs clean", "bin/git-lfs clean", "~/bin/git-lfs clean", "git-lfs track", "git-lfs", "git-lfs filter-process; sh x", "git-lfs clean $(x)", "git-lfs clean `x`", "cat"] {
        assert!(!vetted_program(c), "{c}");
    }
}

/// #146 re-check N4 and N5: the baseline copy lives under the guard-protected control dir, and a `~/` program word is
/// fingerprinted (a relative one resolves to nothing, and is never vetted).
#[test]
fn the_baseline_copy_is_under_control_and_home_programs_are_fingerprinted() {
    let f = fx("homeprog");
    assert!(f.gw.global.starts_with(f.gw.data.join("control/git-baseline")), "{}", f.gw.global.display());
    use std::os::unix::fs::PermissionsExt;
    let tool = f.home.join("bin/tool");
    std::fs::create_dir_all(tool.parent().unwrap()).unwrap();
    std::fs::write(&tool, "#!/bin/sh\ncat\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let a = f.gw.program("~/bin/tool");
    assert!(a.at.is_some());
    std::fs::write(&tool, "#!/bin/sh\necho changed\ncat\n").unwrap();
    assert_ne!(f.gw.program("~/bin/tool"), a, "a rewrite is seen");
    assert!(f.gw.program("./bin/tool").at.is_none());
    assert!(!vetted_program("~/bin/tool"));
}

/// #146 review: two core calls on one repo overlap. The session stays stopped until the last pause drops, a session
/// started during the first pause is stopped by the second, and a pause on another repo is independent.
#[test]
fn overlapping_pauses_on_one_repo_resume_only_when_the_last_ends() {
    use crate::bus::{Bus, Config};
    let d = tmpdir("overlap");
    let bus = Arc::new(Bus::open(&d.join("spill"), Config::default()).unwrap());
    let sup = Arc::new(crate::sessions::Supervisor::new(bus, None));
    let ticker = |id: &str| {
        let f = d.join(id);
        let script = format!("while :; do echo x >> '{}'; sleep 0.02; done", f.display());
        let spec = crate::sessions::SpawnSpec { argv: vec!["sh".into(), "-c".into(), script], cwd: d.clone(), rows: 24, cols: 80, redact: Default::default(), extra_env: Vec::new() };
        sup.spawn(id, spec).unwrap();
        f
    };
    let growth = |f: &Path| {
        let size = || std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        let a = size();
        std::thread::sleep(Duration::from_millis(300));
        size() - a
    };
    let grows = |f: &Path| growth(f) > 0;
    // Held: at most two ticks in 300 ms (a running ticker writes about fifteen). The kernel can continue a stopped
    // session's group the moment a member exits, and under heavy test load the next 3 ms re-stop can come late, so a
    // tick can slip through: the window ADR 16.6 records as an accepted residual.
    let held = |f: &Path| growth(f) <= 4;
    let a = ticker("w-a");
    let live = Arc::new(Mutex::new(vec!["w-a".to_string()]));
    let l2 = live.clone();
    let q = SessionQuiesce::new(sup.clone(), move |c| if c == Path::new("/repo-one") { crate::lock(&l2).clone() } else { vec!["w-other".to_string()] });
    let other = ticker("w-other");
    assert!(grows(&a) && grows(&other), "both run before");
    let p1 = q.pause(Path::new("/repo-one"));
    assert!(held(&a), "stopped by the first pause");
    assert!(grows(&other), "a session on another repo keeps running");
    // A session that starts during the first pause is stopped by the second.
    let b = ticker("w-b");
    crate::lock(&live).push("w-b".into());
    let p2 = q.pause(Path::new("/repo-one"));
    assert!(held(&b), "stopped by the second pause");
    drop(p1);
    assert!(held(&a) && held(&b), "the first drop resumes nothing while the second call runs");
    drop(p2);
    assert!(grows(&a) && grows(&b), "the last drop resumes both");
    // A session that starts during a single held pause is stopped too, with no second pause (#146 bhishma delta).
    let p = q.pause(Path::new("/repo-one"));
    let c = ticker("w-c");
    assert!(grows(&c), "it runs until the repo's session list names it");
    crate::lock(&live).push("w-c".into());
    std::thread::sleep(Duration::from_millis(100));
    assert!(held(&c), "stopped while the single pause is held");
    drop(p);
    assert!(grows(&c), "resumed by its drop");
    // A fresh pause after both ended stops them again, and its drop resumes them.
    let p3 = q.pause(Path::new("/repo-one"));
    assert!(held(&a));
    drop(p3);
    assert!(grows(&a));
    for id in ["w-a", "w-b", "w-c", "w-other"] {
        sup.kill(id).unwrap();
    }
}


/// #146 bhishma delta: a pause whose `on_repo` panics leaves no count behind, so the repo's sessions are never left
/// stopped and a later pause still resumes them.
#[test]
fn a_pause_that_panics_part_way_leaves_no_count() {
    use crate::bus::{Bus, Config};
    let d = tmpdir("panicpause");
    let bus = Arc::new(Bus::open(&d.join("spill"), Config::default()).unwrap());
    let sup = Arc::new(crate::sessions::Supervisor::new(bus, None));
    let boom = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let b2 = boom.clone();
    let q = SessionQuiesce::new(sup, move |_| if b2.load(Ordering::SeqCst) { panic!("on_repo failed") } else { Vec::new() });
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| q.pause(Path::new("/repo-p"))));
    assert!(r.is_err());
    assert!(crate::lock(&q.held).is_empty(), "the unwind dropped the guard and its count");
    boom.store(false, Ordering::SeqCst);
    drop(q.pause(Path::new("/repo-p")));
    assert!(crate::lock(&q.held).is_empty());
}

/// #146 codex: an integration ref planted as a symbolic ref to another branch is refused, and that branch never moves:
/// with no commits, with commits, and when it is planted between merges.
#[test]
fn a_symbolic_integration_ref_is_refused_and_its_target_never_moves() {
    let f = fx("intsym");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    let c = f.gw.snapshot(&t, &globs(), &[], "w m").unwrap().commit.unwrap();
    // `other` is not a Vajra branch, and sits at a different commit than the integration base.
    let other = out(ugit(&f.home, &f.repo, &["-c", "user.name=W", "-c", "user.email=w@e", "commit-tree", &format!("{}^{{tree}}", f.base), "-p", &f.base, "-m", "other"]));
    ugit(&f.home, &f.repo, &["-c", "core.hooksPath=/dev/null", "update-ref", "refs/heads/other", &other]);
    let it = f.gw.integration_tree(&f.repo, "p", "r1", 1, &f.base).unwrap();
    let plant = |home: &Path, repo: &Path| {
        ugit(home, repo, &["-c", "core.hooksPath=/dev/null", "symbolic-ref", "refs/heads/vajra/r1/integration", "refs/heads/other"]);
    };
    plant(&f.home, &f.repo);
    for commits in [Vec::new(), vec![c.clone()]] {
        assert_eq!(f.gw.integrate(&it, &f.base, &commits).unwrap_err(), GitwError::Tampered { what: "the integration branch is a symbolic ref".into() });
        assert_eq!(rev(&f, "refs/heads/other"), other, "the target never moves");
    }
    ugit(&f.home, &f.repo, &["-c", "core.hooksPath=/dev/null", "symbolic-ref", "--delete", "refs/heads/vajra/r1/integration"]);
    let (home, repo) = (f.home.clone(), f.repo.clone());
    BEFORE_MERGE.with(|b| *b.borrow_mut() = Some(Box::new(move || plant(&home, &repo))));
    let r = f.gw.integrate(&it, &f.base, std::slice::from_ref(&c));
    BEFORE_MERGE.with(|b| *b.borrow_mut() = None);
    assert_eq!(r.unwrap_err().code(), "tampered");
    assert_eq!(rev(&f, "refs/heads/other"), other, "planted between merges: the target never moves");
    // A plain ref again: the run integrates, and only its own branch moved.
    ugit(&f.home, &f.repo, &["-c", "core.hooksPath=/dev/null", "symbolic-ref", "--delete", "refs/heads/vajra/r1/integration"]);
    let head = f.gw.integrate(&it, &f.base, std::slice::from_ref(&c)).unwrap();
    assert_eq!(rev(&f, "refs/heads/vajra/r1/integration"), head);
    assert_eq!(rev(&f, "refs/heads/other"), other);
    assert_eq!(rev(&f, "main"), f.base);
    no_marker(&f);
}

/// #146 codex: a program defined in an include whose condition holds only in a linked worktree is listed for the human
/// and refuses the repo's content calls until accepted, so its planted script never runs.
#[test]
fn a_program_in_an_include_active_only_in_a_linked_worktree_is_gated() {
    let f = fx_with("condinc", "[user]\n\tname = U\n\temail = u@e\n", &[("x.dat", "x\n")]);
    let common = f.gw.common_of(&f.repo).unwrap();
    let inc = f.d.join("wt-only.gitconfig");
    std::fs::write(&inc, "[filter \"x\"]\n\tclean = sh ./filter.sh\n").unwrap();
    ugit(&f.home, &f.repo, &["config", "includeIf.gitdir:**/.git/worktrees/**.path", inc.to_str().unwrap()]);
    // The condition is false in the common dir and true in a linked worktree.
    let at_common = Command::new(GIT).args(["-C", common.to_str().unwrap(), "config", "--get", "filter.x.clean"]).env_clear().env("HOME", &f.home).env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
    assert!(!at_common.status.success(), "inactive in the common dir");
    let manual = f.d.join("manual");
    ugit(&f.home, &f.repo, &["-c", "core.hooksPath=/dev/null", "worktree", "add", "-q", "--detach", manual.to_str().unwrap(), &f.base]);
    assert_eq!(out(ugit(&f.home, &manual, &["config", "--get", "filter.x.clean"])), "sh ./filter.sh", "active in a linked worktree");
    f.gw.repos.accept_new(&f.gw, &common, false).unwrap();
    assert_eq!(f.gw.repos.unvetted_programs(&common), vec!["filter.x.clean".to_string()]);
    match f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base) {
        Err(GitwError::RepoConfigChanged { keys }) => assert_eq!(keys, vec!["program filter.x.clean".to_string()]),
        other => panic!("{other:?}"),
    }
    no_marker(&f);
}

/// #146 codex: a repo include graph over the limit is refused, never trusted in part, so a target past the recorded
/// set that a worker rewrites to define a filter never runs.
#[test]
fn a_repo_include_graph_over_the_limit_is_refused() {
    let f = fx("manyinc");
    let mark = marker_script(&f.d, &f.marker);
    let common = f.gw.common_of(&f.repo).unwrap();
    let mut cfg = std::fs::read_to_string(common.join("config")).unwrap();
    for i in 0..70 {
        let p = f.d.join(format!("inc{i}.gitconfig"));
        std::fs::write(&p, "").unwrap();
        cfg.push_str(&format!("[include]\n\tpath = {}\n", p.display()));
    }
    std::fs::write(common.join("config"), cfg).unwrap();
    f.gw.repos.accept_new(&f.gw, &common, false).unwrap();
    f.gw.repos.accept_programs(&common, false).unwrap();
    let refused = |f: &Fx| match f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base) {
        Err(GitwError::RepoConfigChanged { keys }) => assert_eq!(keys, vec![format!("include.path (more than {INCLUDE_MAX} config files)")]),
        other => panic!("{other:?}"),
    };
    refused(&f);
    for i in 0..70 {
        std::fs::write(f.d.join(format!("inc{i}.gitconfig")), format!("[filter \"x\"]\n\tclean = {}\n", mark.display())).unwrap();
    }
    refused(&f);
    no_marker(&f);
}

/// Counts pauses the way `SessionQuiesce` does. When the last one ends it records whether `watch` still exists, and
/// writes into it if so, the way a resumed session would.
#[derive(Debug, Default)]
struct Counting {
    n: Arc<Mutex<usize>>,
    watch: Arc<Mutex<Option<PathBuf>>>,
    seen: Arc<Mutex<Vec<bool>>>,
}

impl Quiesce for Counting {
    fn pause(&self, _: &Path) -> Pause {
        *crate::lock(&self.n) += 1;
        let (n, watch, seen) = (self.n.clone(), self.watch.clone(), self.seen.clone());
        Pause::new(move || {
            let mut n = crate::lock(&n);
            *n -= 1;
            if *n == 0 {
                if let Some(w) = crate::lock(&watch).as_ref() {
                    let there = w.exists();
                    if there {
                        let _ = std::fs::write(w.join("late.txt"), "x\n");
                    }
                    crate::lock(&seen).push(there);
                }
            }
        })
    }
}

/// #146 codex: the repo stays paused from the capture check until the removal ends; no session resumes in between to
/// write a file that `--force` would delete unseen.
#[test]
fn the_pause_holds_through_worktree_removal() {
    let q = Arc::new(Counting::default());
    let f = fx_q("rmpause", "[user]\n\tname = U\n\temail = u@e\n", &[], q.clone());
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    let c = f.gw.snapshot(&t, &globs(), &[], "w m").unwrap().commit.unwrap();
    let it = f.gw.integration_tree(&f.repo, "p", "r1", 1, &f.base).unwrap();
    f.gw.integrate(&it, &f.base, std::slice::from_ref(&c)).unwrap();
    *crate::lock(&q.watch) = Some(t.wt.clone());
    f.gw.worktree_remove(&t, &c, &Gitw::integration_ref(&it)).unwrap();
    assert_eq!(*crate::lock(&q.seen), vec![false], "the only resume came after the tree was gone");
    no_marker(&f);
}

/// #146 codex: a task ref planted as a dangling symbolic ref to a branch outside `vajra/` is refused, and the target is
/// never created, either when planted before the snapshot or just before the ref is written.
#[test]
fn a_symbolic_task_ref_is_refused_and_its_target_is_never_created() {
    let f = fx("tasksym");
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    let plant = |home: &Path, repo: &Path| {
        ugit(home, repo, &["-c", "core.hooksPath=/dev/null", "symbolic-ref", "refs/heads/vajra/r1/task-t1-r1", "refs/heads/release"]);
    };
    let absent = |f: &Fx| {
        let o = Command::new(GIT).args(["-C", f.repo.to_str().unwrap(), "show-ref", "--verify", "--quiet", "refs/heads/release"]).env_clear().env("HOME", &f.home).env("GIT_CONFIG_NOSYSTEM", "1").status().unwrap();
        assert!(!o.success(), "release was created");
    };
    plant(&f.home, &f.repo);
    assert_eq!(f.gw.snapshot(&t, &globs(), &[], "w m").unwrap_err(), GitwError::Tampered { what: "the task ref is a symbolic ref".into() });
    absent(&f);
    // Planted after the last check: the `--no-deref` write replaces the planted ref itself and never follows it, so
    // only the task ref moves.
    ugit(&f.home, &f.repo, &["-c", "core.hooksPath=/dev/null", "symbolic-ref", "--delete", "refs/heads/vajra/r1/task-t1-r1"]);
    let (home, repo) = (f.home.clone(), f.repo.clone());
    BEFORE_TASK_REF.with(|b| *b.borrow_mut() = Some(Box::new(move || plant(&home, &repo))));
    let r = f.gw.snapshot(&t, &globs(), &[], "w m");
    BEFORE_TASK_REF.with(|b| *b.borrow_mut() = None);
    let c = r.unwrap().commit.unwrap();
    absent(&f);
    assert_eq!(rev(&f, "refs/heads/vajra/r1/task-t1-r1"), c);
    let sym = Command::new(GIT).args(["-C", f.repo.to_str().unwrap(), "symbolic-ref", "-q", "refs/heads/vajra/r1/task-t1-r1"]).env_clear().env("HOME", &f.home).env("GIT_CONFIG_NOSYSTEM", "1").status().unwrap();
    assert_eq!(sym.code(), Some(1), "a plain ref now");
    no_marker(&f);
}

/// #146 codex: include paths resolve the way git's pathname rules do, and the forms that can't are refused.
#[test]
fn include_paths_expand_like_git() {
    let (home, from) = (Path::new("/h"), Path::new("/repo/.git/config"));
    assert_eq!(include_target("~/x.cfg", from, home), Some(PathBuf::from("/h/x.cfg")));
    assert_eq!(include_target("~", from, home), Some(PathBuf::from("/h")));
    assert_eq!(include_target("/abs/x.cfg", from, home), Some(PathBuf::from("/abs/x.cfg")));
    assert_eq!(include_target("rel/x.cfg", from, home), Some(PathBuf::from("/repo/.git/rel/x.cfg")));
    assert_eq!(include_target("~vajra-no-such-user-0/x.cfg", from, home), None, "no passwd entry: refused");
    assert_eq!(include_target("%(prefix)/etc/x.cfg", from, home), None);
    // A real account resolves through the passwd database (a path only: nothing is read there).
    let me = unsafe { std::ffi::CStr::from_ptr((*libc::getpwuid(libc::getuid())).pw_name) }.to_str().unwrap().to_string();
    let got = include_target(&format!("~{me}/x.cfg"), from, home).unwrap();
    assert!(got.is_absolute() && got.ends_with("x.cfg") && got != Path::new("/repo/.git").join(format!("~{me}/x.cfg")), "{got:?}");
    USER_HOMES.with(|u| u.borrow_mut().push(("vajra-alice".into(), PathBuf::from("/homes/alice"))));
    assert_eq!(include_target("~vajra-alice/shared.cfg", from, home), Some(PathBuf::from("/homes/alice/shared.cfg")));
    assert_eq!(include_target("~vajra-alice", from, home), Some(PathBuf::from("/homes/alice")));
    USER_HOMES.with(|u| u.borrow_mut().clear());
}

/// #146 codex: a `~user/` include is hashed at its expanded target, so a filter a worker adds there later is a repo
/// config change and never runs; a `~user/` that doesn't resolve refuses the repo's content calls. The include sits
/// behind a condition git finds false, so git never looks the test user up; the core hashes every include whatever its
/// condition, so this is the path a live one takes.
#[test]
fn a_tilde_user_include_is_hashed_where_git_reads_it() {
    let f = fx("tildeuser");
    let mark = marker_script(&f.d, &f.marker);
    let alice = f.d.join("alice");
    std::fs::create_dir_all(&alice).unwrap();
    std::fs::write(alice.join("shared.gitconfig"), "").unwrap();
    USER_HOMES.with(|u| u.borrow_mut().push(("vajra-alice".into(), alice.clone())));
    let common = f.gw.common_of(&f.repo).unwrap();
    ugit(&f.home, &f.repo, &["config", "includeIf.gitdir:/vajra-never/.path", "~vajra-alice/shared.gitconfig"]);
    f.gw.repos.accept_new(&f.gw, &common, false).unwrap();
    f.gw.repos.accept_programs(&common, false).unwrap();
    let t = f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base).unwrap();
    edit(&t, "src/lib.rs", "x\n");
    std::fs::write(alice.join("shared.gitconfig"), format!("[filter \"x\"]\n\tclean = {}\n", mark.display())).unwrap();
    match f.gw.snapshot(&t, &globs(), &[], "w m") {
        Err(GitwError::Tampered { what }) => assert!(what.contains("include.path"), "{what}"),
        other => panic!("{other:?}"),
    }
    // A `~user` with no passwd entry: refused, never hashed at a path git wouldn't read.
    std::fs::write(alice.join("shared.gitconfig"), "").unwrap();
    ugit(&f.home, &f.repo, &["config", "includeIf.gitdir:/vajra-never-2/.path", "~vajra-no-such-user-0/x.gitconfig"]);
    f.gw.repos.accept_new(&f.gw, &common, false).unwrap();
    match f.gw.worktree_add(&f.repo, "p", "r1", "t2", 1, &f.base) {
        Err(GitwError::RepoConfigChanged { keys }) => assert_eq!(keys, vec!["include.path (a ~user or %(prefix) path Vajra can't resolve)".to_string()]),
        other => panic!("{other:?}"),
    }
    USER_HOMES.with(|u| u.borrow_mut().clear());
    no_marker(&f);
}

/// #146 bhishma delta: an include git can't be followed through exactly (a non-UTF-8 target, or a `~user` home that
/// isn't UTF-8) makes the graph partial, never silently skipped.
#[test]
fn a_non_utf8_include_makes_the_graph_partial() {
    use std::os::unix::ffi::OsStrExt;
    let d = tmpdir("nonutf8");
    let home = d.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let cfg = d.join("config");
    let mut bytes = b"[include]\n\tpath = ".to_vec();
    bytes.extend(d.join("x").as_os_str().as_bytes());
    bytes.extend(b"\xff.gitconfig\n");
    std::fs::write(&cfg, bytes).unwrap();
    let (_, partial) = config_sources(Path::new(GIT), &home, None, std::slice::from_ref(&cfg), false);
    assert_eq!(partial.as_deref(), Some("an include path that is not UTF-8"));
    std::fs::write(&cfg, "[include]\n\tpath = ~vajra-bytes/x.gitconfig\n").unwrap();
    USER_HOMES.with(|u| u.borrow_mut().push(("vajra-bytes".into(), PathBuf::from(std::ffi::OsStr::from_bytes(b"/homes/\xff")))));
    assert_eq!(include_target("~vajra-bytes/x.gitconfig", &cfg, &home), None);
    let (_, partial) = config_sources(Path::new(GIT), &home, None, std::slice::from_ref(&cfg), false);
    USER_HOMES.with(|u| u.borrow_mut().clear());
    assert_eq!(partial.as_deref(), Some("a ~user or %(prefix) path Vajra can't resolve"));
}

/// #146 bhishma delta: a run's ref directory planted as a symlink to another branch directory is refused before any
/// ref write, so that branch's `integration` file never moves.
#[test]
fn a_symlinked_ref_directory_is_refused_and_the_other_branch_never_moves() {
    let f = fx("refdirlink");
    let common = f.gw.common_of(&f.repo).unwrap();
    let other = out(ugit(&f.home, &f.repo, &["-c", "user.name=W", "-c", "user.email=w@e", "commit-tree", &format!("{}^{{tree}}", f.base), "-p", &f.base, "-m", "other"]));
    ugit(&f.home, &f.repo, &["-c", "core.hooksPath=/dev/null", "update-ref", "refs/heads/feat/integration", &other]);
    let it = f.gw.integration_tree(&f.repo, "p", "r1", 1, &f.base).unwrap();
    std::fs::create_dir_all(common.join("refs/heads/vajra")).unwrap();
    std::os::unix::fs::symlink("../feat", common.join("refs/heads/vajra/r1")).unwrap();
    assert_eq!(f.gw.integrate(&it, &f.base, &[]).unwrap_err(), GitwError::Tampered { what: "the integration branch's directory is not a plain directory".into() });
    assert_eq!(rev(&f, "refs/heads/feat/integration"), other, "the other branch never moves");
    no_marker(&f);
}

/// #146 bhishma delta M-b: a run's ref directory swapped for a symlink after integrate's last check (a process the
/// pause can't reach) makes the first write fail on its old value; the other branch never moves.
#[test]
fn a_ref_directory_swapped_before_the_first_integrate_write_moves_nothing() {
    let f = fx("refdirswap");
    let common = f.gw.common_of(&f.repo).unwrap();
    let other = out(ugit(&f.home, &f.repo, &["-c", "user.name=W", "-c", "user.email=w@e", "commit-tree", &format!("{}^{{tree}}", f.base), "-p", &f.base, "-m", "other"]));
    ugit(&f.home, &f.repo, &["-c", "core.hooksPath=/dev/null", "update-ref", "refs/heads/feat/integration", &other]);
    let it = f.gw.integration_tree(&f.repo, "p", "r1", 1, &f.base).unwrap();
    let c2 = common.clone();
    BEFORE_REF_WRITE.with(|b| *b.borrow_mut() = Some(Box::new(move || {
        std::fs::create_dir_all(c2.join("refs/heads/vajra")).unwrap();
        std::os::unix::fs::symlink("../feat", c2.join("refs/heads/vajra/r1")).unwrap();
    })));
    let r = f.gw.integrate(&it, &f.base, &[]);
    BEFORE_REF_WRITE.with(|b| *b.borrow_mut() = None);
    assert_eq!(r.unwrap_err(), GitwError::Tampered { what: "the integration branch".into() });
    assert_eq!(rev(&f, "refs/heads/feat/integration"), other, "the other branch never moves");
    // A plain directory again: the run integrates (the ref absent, so its old value is the zero id).
    std::fs::remove_file(common.join("refs/heads/vajra/r1")).unwrap();
    assert_eq!(f.gw.integrate(&it, &f.base, &[]).unwrap(), f.base);
    // And again from the value it now has.
    assert_eq!(f.gw.integrate(&it, &f.base, &[]).unwrap(), f.base);
    no_marker(&f);
}

/// #146 bhishma delta M-a: `branch_delete` holds the repo's pause through the delete and never deletes through a
/// planted directory symlink, so another run's branch of the same name survives.
#[test]
fn branch_delete_is_paused_and_never_deletes_through_a_symlinked_directory() {
    let q = Arc::new(Counting::default());
    let f = fx_q("brdel", "[user]\n\tname = U\n\temail = u@e\n", &[], q.clone());
    let common = f.gw.common_of(&f.repo).unwrap();
    ugit(&f.home, &f.repo, &["-c", "core.hooksPath=/dev/null", "update-ref", "refs/heads/other/task-t1-r1", &f.base]);
    std::fs::create_dir_all(common.join("refs/heads/vajra")).unwrap();
    std::os::unix::fs::symlink("../other", common.join("refs/heads/vajra/r1")).unwrap();
    assert_eq!(f.gw.branch_delete(&f.repo, "refs/heads/vajra/r1/task-t1-r1").unwrap_err(), GitwError::Tampered { what: "the task branch's directory is not a plain directory".into() });
    assert_eq!(rev(&f, "refs/heads/other/task-t1-r1"), f.base, "the other branch survives");
    // A real task branch: deleted while the pause is held (the only resume comes after its ref file is gone).
    std::fs::remove_file(common.join("refs/heads/vajra/r1")).unwrap();
    ugit(&f.home, &f.repo, &["-c", "core.hooksPath=/dev/null", "update-ref", "refs/heads/vajra/r1/task-t1-r1", &f.base]);
    *crate::lock(&q.watch) = Some(common.join("refs/heads/vajra/r1/task-t1-r1"));
    f.gw.branch_delete(&f.repo, "refs/heads/vajra/r1/task-t1-r1").unwrap();
    assert_eq!(*crate::lock(&q.seen), vec![false]);
    no_marker(&f);
}

/// #146 codex: a shared include reached first at the depth cap (through an inactive chain) and also directly is
/// expanded at its direct depth, so the file it includes is hashed and a filter a worker adds there refuses content
/// calls before any filter runs.
#[test]
fn an_include_reached_at_the_depth_cap_and_directly_is_expanded() {
    let f = fx("incdepth");
    let mark = marker_script(&f.d, &f.marker);
    let common = f.gw.common_of(&f.repo).unwrap();
    let (a, b) = (f.d.join("a.gitconfig"), f.d.join("b.gitconfig"));
    std::fs::write(&b, "").unwrap();
    std::fs::write(&a, format!("[include]\n\tpath = {}\n", b.display())).unwrap();
    // c1 -> c2 -> ... -> c9 -> a: from the repo config, `a` is at depth 10 that way.
    for i in 1..=9 {
        let next = if i == 9 { a.clone() } else { f.d.join(format!("c{}.gitconfig", i + 1)) };
        std::fs::write(f.d.join(format!("c{i}.gitconfig")), format!("[include]\n\tpath = {}\n", next.display())).unwrap();
    }
    // `a` directly first, then the chain behind a condition git finds false.
    ugit(&f.home, &f.repo, &["config", "include.path", a.to_str().unwrap()]);
    ugit(&f.home, &f.repo, &["config", "includeIf.gitdir:/vajra-never/.path", f.d.join("c1.gitconfig").to_str().unwrap()]);
    f.gw.repos.accept_new(&f.gw, &common, false).unwrap();
    f.gw.repos.accept_programs(&common, false).unwrap();
    let (files, partial) = repo_sources(&f.gw, &common);
    assert_eq!(partial, None);
    assert!(files.iter().any(|(p, _)| *p == b), "b is hashed");
    std::fs::write(&b, format!("[filter \"x\"]\n\tclean = {}\n", mark.display())).unwrap();
    match f.gw.worktree_add(&f.repo, "p", "r1", "t1", 1, &f.base) {
        Err(GitwError::RepoConfigChanged { keys }) => assert!(keys.contains(&"include.path".to_string()), "{keys:?}"),
        other => panic!("{other:?}"),
    }
    no_marker(&f);
}
