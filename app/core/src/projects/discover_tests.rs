use std::os::unix::fs::symlink;

use super::*;

struct Tmp(PathBuf);
impl Tmp {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("vajra-discover-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("home/.claude")).unwrap();
        Self(fs::canonicalize(&d).unwrap())
    }
    fn guard(&self) -> Guard {
        Guard { home: self.0.join("home"), data: self.0.join("home/Library/Data") }
    }
    fn repo(&self, rel: &str) -> PathBuf {
        let p = self.0.join("home").join(rel);
        fs::create_dir_all(p.join(".git/objects")).unwrap();
        fs::write(p.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::canonicalize(&p).unwrap()
    }
    /// A real linked worktree of `main` at `rel`, with git's back-link intact.
    fn worktree(&self, main: &Path, name: &str, rel: &str) -> PathBuf {
        let wt = self.0.join("home").join(rel);
        fs::create_dir_all(&wt).unwrap();
        let wt = fs::canonicalize(&wt).unwrap();
        let gd = main.join(".git/worktrees").join(name);
        fs::create_dir_all(&gd).unwrap();
        fs::write(gd.join("commondir"), "../..\n").unwrap();
        fs::write(gd.join("gitdir"), format!("{}\n", wt.join(".git").display())).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", gd.display())).unwrap();
        wt
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn none(_: &Path) -> bool {
    false
}

#[test]
fn finds_repos_in_common_folders_and_counts_worktrees_under_the_main_repo() {
    let t = Tmp::new("basic");
    let a = t.repo("Development/alpha");
    t.repo("Projects/beta");
    t.worktree(&a, "feat", "Development/alpha-feat");
    let d = discover(&t.guard(), &[], &none);
    let names: Vec<_> = d.found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["alpha", "beta"]);
    assert_eq!(d.found[0].worktrees, 1);
    assert_eq!(d.found[1].worktrees, 0);
    assert!(!d.truncated);
}

#[test]
fn skips_owned_hidden_build_and_symlinked_folders_and_does_not_enter_repos() {
    let t = Tmp::new("skip");
    let a = t.repo("Code/owned");
    t.repo("Code/keep");
    t.repo("Code/.hidden/x");
    t.repo("Code/node_modules/y");
    t.repo("Code/keep/inner"); // inside a repo: never looked at
    let elsewhere = t.repo("elsewhere/z");
    symlink(&elsewhere, t.0.join("home/Code/link")).unwrap();
    let a2 = a.clone();
    let d = discover(&t.guard(), &[], &move |p| p == a2);
    let names: Vec<_> = d.found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["keep"]);
}

#[test]
fn an_extra_folder_is_scanned_and_missing_ones_are_ignored() {
    let t = Tmp::new("extra");
    t.repo("odd/place/r");
    let d = discover(&t.guard(), &[t.0.join("home/odd"), t.0.join("home/nope")], &none);
    assert_eq!(d.found.len(), 1);
    assert_eq!(d.found[0].name, "r");
}

#[test]
fn a_fake_dot_git_file_pointing_elsewhere_is_not_folded_into_another_repo() {
    let t = Tmp::new("fake");
    let real = t.repo("Code/real");
    let fake = t.0.join("home/Code/fake");
    fs::create_dir_all(&fake).unwrap();
    fs::write(fake.join(".git"), format!("gitdir: {}\n", real.join(".git").display())).unwrap();
    let d = discover(&t.guard(), &[], &none);
    assert_eq!(d.found.len(), 2); // fake is its own entry (no git claim honoured), real is unaffected
    assert!(d.found.iter().all(|f| f.worktrees == 0));
}

#[test]
fn stops_at_the_repository_cap() {
    let t = Tmp::new("cap");
    for i in 0..(MAX_FOUND + 5) {
        t.repo(&format!("Code/r{i:04}"));
    }
    let d = discover(&t.guard(), &[], &none);
    assert_eq!(d.found.len(), MAX_FOUND);
    assert!(d.truncated);
}

#[test]
fn the_automatic_set_never_touches_macos_protected_folders() {
    for f in COMMON_FOLDERS {
        let first = f.split('/').next().unwrap();
        assert!(!["Documents", "Desktop", "Downloads"].contains(&first), "{f}");
    }
    let t = Tmp::new("tcc");
    t.repo("Documents/GitHub/hidden");
    assert!(discover(&t.guard(), &[], &none).found.is_empty());
}

#[test]
fn a_common_folder_that_is_a_symlink_into_a_protected_folder_is_not_scanned() {
    let t = Tmp::new("rootlink");
    t.repo("Documents/secret");
    symlink(t.0.join("home/Documents"), t.0.join("home/Code")).unwrap();
    assert!(discover(&t.guard(), &[], &none).found.is_empty());
    // the person naming that same folder is an explicit choice, so it is scanned
    assert_eq!(discover(&t.guard(), &[t.0.join("home/Code")], &none).found.len(), 1);
}

#[test]
fn a_very_wide_folder_is_cut_off_while_listing_and_marked_truncated() {
    let t = Tmp::new("wide");
    // The repositories live INSIDE the wide root, with more files than the listing cap around them, and nothing found
    // in an earlier root: the entries collected before the cut-off must still be inspected.
    let wide = t.0.join("home/Development");
    fs::create_dir_all(&wide).unwrap();
    for i in 0..(MAX_LISTED + 500) {
        fs::write(wide.join(format!("f{i}")), "").unwrap();
    }
    for i in 0..20 {
        t.repo(&format!("Development/repo{i:02}"));
    }
    let d = discover(&t.guard(), &[], &none);
    assert!(d.truncated, "the cut-off is reported");
    // read_dir order is unspecified, so which repositories fall before the cut is too; some always do, because
    // 20 directories among 5,520 entries cannot all sit after the first 5,000.
    assert!(!d.found.is_empty(), "repositories reached before the cut-off are still offered");
}

#[test]
fn a_wide_folder_with_repos_and_a_later_root_both_keep_their_finds() {
    let t = Tmp::new("wide2");
    let wide = t.0.join("home/Development");
    fs::create_dir_all(&wide).unwrap();
    for i in 0..(MAX_LISTED + 10) {
        fs::write(wide.join(format!("f{i}")), "").unwrap();
    }
    t.repo("Projects/later");
    let d = discover(&t.guard(), &[], &none);
    assert!(d.truncated);
    assert!(d.found.iter().any(|f| f.name == "later"), "a later root is still scanned after a wide one");
}
