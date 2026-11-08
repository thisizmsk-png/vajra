use std::os::unix::fs::symlink;

use super::*;

struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("vajra-ident-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        Self(fs::canonicalize(&d).unwrap())
    }
    fn dir(&self, rel: &str) -> PathBuf {
        let p = self.0.join(rel);
        fs::create_dir_all(&p).unwrap();
        p
    }
    /// A folder with a real-looking `.git` directory.
    fn repo(&self, rel: &str) -> PathBuf {
        let p = self.dir(rel);
        fs::create_dir_all(p.join(".git/objects")).unwrap();
        fs::write(p.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        p
    }
    fn guard(&self) -> Guard {
        Guard { home: self.dir("home"), data: self.dir("home/Library/Data") }
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_repo_folder_resolves_to_its_realpath_device_inode_and_git_dir() {
    let t = Tmp::new("basic");
    let repo = t.repo("home/src/vajra");
    let r = resolve(&repo, &t.guard()).unwrap();
    let meta = fs::metadata(&repo).unwrap();
    assert_eq!(r.root, repo);
    assert_eq!((r.key.dev, r.key.ino), (meta.dev(), meta.ino()));
    assert_eq!(r.key.git_common_dir.as_deref(), Some(repo.join(".git").to_str().unwrap()));
}

#[test]
fn a_symlink_to_the_folder_resolves_to_the_same_root_and_key() {
    let t = Tmp::new("alias");
    let repo = t.repo("home/src/vajra");
    let link = t.0.join("home/alias");
    symlink(&repo, &link).unwrap();
    assert_eq!(resolve(&link, &t.guard()).unwrap(), resolve(&repo, &t.guard()).unwrap());
}

#[test]
fn a_folder_without_git_is_a_project_with_no_common_dir() {
    let t = Tmp::new("nogit");
    let d = t.dir("home/notes");
    let r = resolve(&d, &t.guard()).unwrap();
    assert_eq!((r.root, r.key.git_common_dir), (d, None));
}

#[test]
fn a_worktree_resolves_to_the_main_repo_folder_and_shares_its_common_dir() {
    let t = Tmp::new("worktree");
    let main = t.repo("home/src/vajra");
    let wt_git = main.join(".git/worktrees/w1");
    fs::create_dir_all(&wt_git).unwrap();
    fs::write(wt_git.join("commondir"), "../..\n").unwrap();
    fs::write(wt_git.join("HEAD"), "ref: refs/heads/w1\n").unwrap();
    let wt = t.dir("home/work/w1");
    fs::write(wt_git.join("gitdir"), format!("{}\n", wt.join(".git").display())).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
    let from_main = resolve(&main, &t.guard()).unwrap();
    let from_wt = resolve(&wt, &t.guard()).unwrap();
    assert_eq!(from_wt, from_main, "a worktree never becomes a project of its own");
    assert_eq!(from_wt.root, main);
}

#[test]
fn a_relative_gitdir_in_a_worktree_file_is_read_relative_to_the_worktree() {
    let t = Tmp::new("relgit");
    let main = t.repo("home/src/vajra");
    let wt_git = main.join(".git/worktrees/w2");
    fs::create_dir_all(&wt_git).unwrap();
    fs::write(wt_git.join("commondir"), "../..").unwrap();
    let wt = t.dir("home/src/vajra-w2");
    fs::write(wt_git.join("gitdir"), format!("{}\n", wt.join(".git").display())).unwrap();
    fs::write(wt.join(".git"), "gitdir: ../vajra/.git/worktrees/w2").unwrap();
    assert_eq!(resolve(&wt, &t.guard()).unwrap().root, main);
}

#[test]
fn a_git_entry_that_does_not_lead_to_a_git_dir_is_ignored() {
    let t = Tmp::new("fakegit");
    // A .git file pointing at a folder with no HEAD or objects, and one pointing outside the tree.
    let elsewhere = t.dir("home/elsewhere");
    let d = t.dir("home/proj");
    fs::write(d.join(".git"), format!("gitdir: {}", elsewhere.display())).unwrap();
    assert_eq!(resolve(&d, &t.guard()).unwrap().key.git_common_dir, None);
    let big = t.dir("home/big");
    fs::write(big.join(".git"), format!("gitdir: {}", "x".repeat(GIT_FILE_CAP + 10))).unwrap();
    assert_eq!(resolve(&big, &t.guard()).unwrap().key.git_common_dir, None);
}

#[test]
fn a_missing_path_and_a_plain_file_are_reported() {
    let t = Tmp::new("missing");
    assert!(matches!(resolve(&t.0.join("home/nope"), &t.guard()), Err(IdentityError::NotFound)));
    let f = t.0.join("home/file");
    fs::write(&f, "x").unwrap();
    assert!(matches!(resolve(&f, &t.guard()), Err(IdentityError::NotADirectory)));
}

#[test]
fn folders_that_must_never_be_projects_are_refused() {
    let t = Tmp::new("refuse");
    let g = t.guard();
    let claude = t.dir("home/.claude");
    let under_claude = t.dir("home/.claude/vajra/campaigns");
    let under_data = t.dir("home/Library/Data/projects");
    let above_data = t.dir("home/Library");
    for (p, what) in [
        (g.home.clone(), "home"),
        (claude, "the config folder"),
        (under_claude, "inside the config folder"),
        (g.data.clone(), "the data folder"),
        (under_data, "inside the data folder"),
        (above_data, "a folder holding the data folder"),
        (t.0.clone(), "a folder holding home"),
        (PathBuf::from("/"), "the filesystem root"),
    ] {
        assert!(matches!(resolve(&p, &g), Err(IdentityError::Refused(_))), "{what}: {}", p.display());
    }
    assert!(resolve(&t.dir("home/src"), &g).is_ok());
}

// A `.git` entry is the folder's own claim. It moves the project root to another folder only when git's own back-link
// agrees: the git dir is `<common>/worktrees/<name>` and its `gitdir` file names this folder's `.git` file.

#[test]
fn a_git_file_pointing_straight_at_another_repo_does_not_move_the_root() {
    let t = Tmp::new("hijack");
    let victim = t.repo("home/src/victim");
    let dl = t.dir("home/downloads/thing");
    fs::write(dl.join(".git"), format!("gitdir: {}\n", victim.join(".git").display())).unwrap();
    let r = resolve(&dl, &t.guard()).unwrap();
    assert_eq!(r.root, dl, "the project is the folder that was picked");
    assert_eq!(r.key.git_common_dir, None, "an unverified common dir is not used to match other projects");
    assert_ne!(r, resolve(&victim, &t.guard()).unwrap());
}

#[test]
fn a_git_file_naming_a_worktree_entry_of_another_repo_needs_the_back_link() {
    let t = Tmp::new("nolink");
    let victim = t.repo("home/src/victim");
    let wt_git = victim.join(".git/worktrees/w1");
    fs::create_dir_all(&wt_git).unwrap();
    fs::write(wt_git.join("commondir"), "../..\n").unwrap();
    // The real worktree, which git linked back to /home/real/w1/.git:
    let real = t.dir("home/real/w1");
    fs::write(wt_git.join("gitdir"), format!("{}\n", real.join(".git").display())).unwrap();
    fs::write(real.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
    assert_eq!(resolve(&real, &t.guard()).unwrap().root, victim, "the genuine worktree still resolves");
    // A different folder claiming the same entry is not it.
    let fake = t.dir("home/downloads/fake");
    fs::write(fake.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
    let r = resolve(&fake, &t.guard()).unwrap();
    assert_eq!((r.root, r.key.git_common_dir), (fake, None));
    // And so does an entry with no back-link file at all.
    let bare = victim.join(".git/worktrees/w2");
    fs::create_dir_all(&bare).unwrap();
    fs::write(bare.join("commondir"), "../..\n").unwrap();
    let nolink = t.dir("home/downloads/nolink");
    fs::write(nolink.join(".git"), format!("gitdir: {}\n", bare.display())).unwrap();
    assert_eq!(resolve(&nolink, &t.guard()).unwrap().root, nolink);
}

#[test]
fn a_git_symlink_to_another_repo_does_not_move_the_root() {
    let t = Tmp::new("symgit");
    let victim = t.repo("home/src/victim");
    let d = t.dir("home/downloads/linked");
    symlink(victim.join(".git"), d.join(".git")).unwrap();
    let r = resolve(&d, &t.guard()).unwrap();
    assert_eq!((r.root, r.key.git_common_dir), (d, None));
}
