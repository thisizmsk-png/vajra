use std::os::unix::fs::{PermissionsExt, symlink};
use std::time::{Duration, UNIX_EPOCH};

use super::*;
use crate::projects::contract::{SessionKind, SessionStatus};

struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("vajra-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        Self(fs::canonicalize(&d).unwrap())
    }
    fn home(&self) -> PathBuf {
        let p = self.0.join("home");
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn data(&self) -> PathBuf {
        self.0.join("home/Library/Data")
    }
    fn repo(&self, rel: &str) -> PathBuf {
        let p = self.home().join(rel);
        fs::create_dir_all(p.join(".git/objects")).unwrap();
        fs::write(p.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::canonicalize(&p).unwrap()
    }
    fn store(&self) -> Store {
        let clock = || UNIX_EPOCH + Duration::from_secs(1_793_872_800);
        Store::with_clock(&self.data(), &self.home(), Box::new(clock)).unwrap()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn mode(p: &Path) -> u32 {
    fs::metadata(p).unwrap().permissions().mode() & 0o777
}

fn session(project: &str, id: &str, status: SessionStatus) -> Session {
    Session {
        schema: contract::SESSION_SCHEMA.into(),
        id: id.into(),
        project_id: project.into(),
        kind: SessionKind::Run,
        parent: None,
        provider: "claude".into(),
        native_session_id: None,
        run_id: Some("20261108-0930".into()),
        campaign: None,
        task_id: None,
        attempt: None,
        generation: 1,
        status,
        started: "2026-11-08T09:30:00Z".into(),
        ended: None,
    }
}

// ----------------------------------------------------------------- FG-01: two folders, restart, stable ids

#[test]
fn two_folders_added_then_a_restart_keep_their_ids_roots_and_records() {
    let t = Tmp::new("restart");
    let (a, b) = (t.repo("src/alpha"), t.repo("src/beta"));
    let (pa, pb) = {
        let s = t.store();
        (s.add(&a, false).unwrap().project().clone(), s.add(&b, false).unwrap().project().clone())
    };
    assert_ne!(pa.id, pb.id);
    assert!(contract::valid_project_id(&pa.id) && contract::valid_project_id(&pb.id));
    // A new Store over the same data dir is a restart.
    let s2 = t.store();
    let listed = s2.list(false).unwrap();
    assert_eq!(listed.skipped, 0);
    let by_name: Vec<(&str, &str, &str)> = listed.projects.iter().map(|p| (p.name.as_str(), p.id.as_str(), p.root.as_str())).collect();
    assert!(by_name.contains(&("alpha", pa.id.as_str(), a.to_str().unwrap())), "{by_name:?}");
    assert!(by_name.contains(&("beta", pb.id.as_str(), b.to_str().unwrap())), "{by_name:?}");
    assert_eq!(s2.get(&pa.id).unwrap(), pa);
}

#[test]
fn each_project_keeps_its_own_sessions() {
    let t = Tmp::new("sessions");
    let s = t.store();
    let (pa, pb) = (s.add(&t.repo("src/a"), false).unwrap().project().clone(), s.add(&t.repo("src/b"), false).unwrap().project().clone());
    let sid_a = contract::fresh_id("s").unwrap();
    let sid_b = contract::fresh_id("s").unwrap();
    s.append_session(&session(&pa.id, &sid_a, SessionStatus::Running)).unwrap();
    s.append_session(&session(&pa.id, &sid_a, SessionStatus::Done)).unwrap();
    s.append_session(&session(&pb.id, &sid_b, SessionStatus::Running)).unwrap();
    let a = s.sessions(&pa.id).unwrap();
    let b = s.sessions(&pb.id).unwrap();
    assert_eq!((a.len(), a[0].status), (1, SessionStatus::Done), "the newest line per id wins");
    assert_eq!((b.len(), b[0].id.as_str()), (1, sid_b.as_str()));
    // A session for a project that does not exist, or one that fails validation, is refused.
    assert!(matches!(s.append_session(&session("p_01JC5X8Z3A9K2M4N6P7Q8R9S0T", &sid_a, SessionStatus::Running)), Err(StoreError::NotFound)));
    let mut bad = session(&pa.id, &sid_a, SessionStatus::Running);
    bad.provider = "NOT OK".into();
    assert!(matches!(s.append_session(&bad), Err(StoreError::Invalid(_))));
    assert!(s.sessions(&pb.id).unwrap().iter().all(|x| x.project_id == pb.id));
}

#[test]
fn records_and_session_files_are_private_and_written_atomically() {
    let t = Tmp::new("modes");
    let s = t.store();
    let p = s.add(&t.repo("src/a"), false).unwrap().project().clone();
    s.append_session(&session(&p.id, &contract::fresh_id("s").unwrap(), SessionStatus::Running)).unwrap();
    let dir = t.data().join("projects").join(&p.id);
    assert_eq!(mode(&t.data().join("projects")), 0o700);
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&dir.join("project.json")), 0o600);
    assert_eq!(mode(&dir.join("sessions.jsonl")), 0o600);
    let leftovers: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".tmp")).collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(fs::read_to_string(dir.join("project.json")).unwrap().contains("\"schema\": \"vajra.project/1\""));
}

// ------------------------------------------------------------------------------------------------- identity

#[test]
fn the_same_folder_through_a_symlink_finds_the_existing_project() {
    let t = Tmp::new("alias");
    let s = t.store();
    let repo = t.repo("src/vajra");
    let link = t.home().join("alias-to-vajra");
    symlink(&repo, &link).unwrap();
    let first = s.add(&repo, false).unwrap();
    let again = s.add(&link, false).unwrap();
    assert_eq!(first.outcome(), "created");
    assert_eq!(again.outcome(), "existing");
    assert_eq!(again.project().id, first.project().id);
    assert_eq!(s.list(true).unwrap().projects.len(), 1);
}

#[test]
fn a_worktree_of_a_project_finds_that_project_and_never_makes_its_own() {
    let t = Tmp::new("worktree");
    let s = t.store();
    let main = t.repo("src/vajra");
    let wt_git = main.join(".git/worktrees/w1");
    fs::create_dir_all(&wt_git).unwrap();
    fs::write(wt_git.join("commondir"), "../..\n").unwrap();
    let wt = t.home().join("work/w1");
    fs::create_dir_all(&wt).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
    let first = s.add(&main, false).unwrap();
    let from_wt = s.add(&wt, false).unwrap();
    assert_eq!(from_wt.outcome(), "existing");
    assert_eq!(from_wt.project().id, first.project().id);
    assert_eq!(s.list(true).unwrap().projects.len(), 1);
}

#[test]
fn a_folder_inside_or_holding_another_project_is_refused_until_confirmed() {
    let t = Tmp::new("nested");
    let s = t.store();
    let outer = t.repo("src/outer");
    let inner = t.repo("src/outer/vendor/inner");
    let o = s.add(&outer, false).unwrap().project().clone();
    match s.add(&inner, false) {
        Err(StoreError::Nested { with_id, with_name }) => assert_eq!((with_id, with_name), (o.id.clone(), "outer".to_string())),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.list(true).unwrap().projects.len(), 1, "a refused add writes nothing");
    let confirmed = s.add(&inner, true).unwrap();
    assert_eq!(confirmed.outcome(), "created");
    // The other direction: adding the parent of an existing project.
    let t2 = Tmp::new("nested2");
    let s2 = t2.store();
    s2.add(&t2.repo("src/outer/inner"), false).unwrap();
    assert!(matches!(s2.add(&t2.repo("src/outer"), false), Err(StoreError::Nested { .. })));
}

#[test]
fn a_folder_that_cannot_be_a_project_is_refused_before_anything_is_written() {
    let t = Tmp::new("refuse");
    let s = t.store();
    assert!(matches!(s.add(&t.home(), false), Err(StoreError::Identity(IdentityError::Refused(_)))));
    assert!(matches!(s.add(&t.data(), false), Err(StoreError::Identity(IdentityError::Refused(_)))));
    assert!(matches!(s.add(&t.0.join("home/missing"), false), Err(StoreError::Identity(IdentityError::NotFound))));
    assert_eq!(s.list(true).unwrap().projects.len(), 0);
}

// ----------------------------------------------------------------------------- rename, archive, revisions

#[test]
fn rename_changes_the_name_and_bumps_the_revision_once() {
    let t = Tmp::new("rename");
    let s = t.store();
    let p = s.add(&t.repo("src/a"), false).unwrap().project().clone();
    assert_eq!((p.name.as_str(), p.rev), ("a", 1));
    let r = s.rename(&p.id, "  My App  ", Some(1)).unwrap();
    assert_eq!((r.name.as_str(), r.rev), ("My App", 2));
    assert_eq!(s.get(&p.id).unwrap().name, "My App");
    let same = s.rename(&p.id, "My App", None).unwrap();
    assert_eq!(same.rev, 2, "renaming to the same name changes nothing");
    assert!(matches!(s.rename(&p.id, "   ", None), Err(StoreError::Invalid(_))));
    assert!(matches!(s.rename(&p.id, &"x".repeat(300), None), Err(StoreError::Invalid(_))));
    assert_eq!(s.get(&p.id).unwrap().rev, 2, "a refused rename writes nothing");
}

#[test]
fn a_stale_revision_is_a_conflict_not_an_overwrite() {
    let t = Tmp::new("conflict");
    let s = t.store();
    let p = s.add(&t.repo("src/a"), false).unwrap().project().clone();
    s.rename(&p.id, "first", Some(1)).unwrap();
    match s.rename(&p.id, "second", Some(1)) {
        Err(StoreError::Conflict { current_rev }) => assert_eq!(current_rev, 2),
        other => panic!("{other:?}"),
    }
    assert!(matches!(s.archive(&p.id, Some(1)), Err(StoreError::Conflict { .. })));
    assert_eq!(s.get(&p.id).unwrap().name, "first");
}

#[test]
fn archiving_hides_the_project_deletes_nothing_and_adding_it_again_restores_it() {
    let t = Tmp::new("archive");
    let s = t.store();
    let repo = t.repo("src/a");
    let p = s.add(&repo, false).unwrap().project().clone();
    let sid = contract::fresh_id("s").unwrap();
    s.append_session(&session(&p.id, &sid, SessionStatus::Done)).unwrap();
    let a = s.archive(&p.id, None).unwrap();
    assert!(a.archived.is_some());
    assert_eq!(a.rev, 2);
    assert_eq!(s.archive(&p.id, None).unwrap().rev, 2, "archiving twice changes nothing");
    assert!(s.list(false).unwrap().projects.is_empty());
    assert_eq!(s.list(true).unwrap().projects.len(), 1);
    assert_eq!(s.sessions(&p.id).unwrap().len(), 1, "its history is still there");
    assert!(repo.join(".git/HEAD").exists(), "the folder is untouched");
    let back = s.add(&repo, false).unwrap();
    assert_eq!(back.outcome(), "restored");
    assert_eq!((back.project().id.as_str(), back.project().archived.clone(), back.project().rev), (p.id.as_str(), None, 3));
}

#[test]
fn an_unknown_project_is_not_found() {
    let t = Tmp::new("unknown");
    let s = t.store();
    for id in ["p_01JC5X8Z3A9K2M4N6P7Q8R9S0T", "../../etc", "", "p_x"] {
        assert!(matches!(s.get(id), Err(StoreError::NotFound)), "{id}");
        assert!(matches!(s.rename(id, "x", None), Err(StoreError::NotFound)), "{id}");
        assert!(matches!(s.archive(id, None), Err(StoreError::NotFound)), "{id}");
    }
}

// ------------------------------------------------------------------------------ untrusted files on disk

#[test]
fn a_damaged_foreign_or_impersonating_record_is_skipped_not_fatal() {
    let t = Tmp::new("damaged");
    let s = t.store();
    let good = s.add(&t.repo("src/good"), false).unwrap().project().clone();
    let projects = t.data().join("projects");
    // Garbage JSON in a valid-looking folder.
    let bad_id = "p_01JC5X8Z3A9K2M4N6P7Q8R9S0T";
    fs::create_dir_all(projects.join(bad_id)).unwrap();
    fs::write(projects.join(bad_id).join("project.json"), "{not json").unwrap();
    // A record that is valid but filed under another project's folder name.
    let other = "p_01JC5X8Z3B0K2M4N6P7Q8R9S0T";
    fs::create_dir_all(projects.join(other)).unwrap();
    fs::copy(projects.join(&good.id).join("project.json"), projects.join(other).join("project.json")).unwrap();
    // A record from a newer major version.
    let v2 = "p_01JC5X8Z3C1K2M4N6P7Q8R9S0T";
    fs::create_dir_all(projects.join(v2)).unwrap();
    fs::write(projects.join(v2).join("project.json"), r#"{"schema":"vajra.project/2","id":"x"}"#).unwrap();
    // Stray files and folders that are not project ids.
    fs::write(projects.join("notes.txt"), "x").unwrap();
    fs::create_dir_all(projects.join("..evil")).unwrap();
    let l = s.list(true).unwrap();
    assert_eq!(l.projects.len(), 1);
    assert_eq!(l.projects[0].id, good.id);
    assert_eq!(l.skipped, 4, "garbage, impersonator, newer major, stray file; dot-names are ignored");
    assert!(matches!(s.get(other), Err(StoreError::Invalid(_))), "the impersonating record is not served");
}

#[test]
fn a_symlinked_record_is_not_followed() {
    let t = Tmp::new("symlinked");
    let s = t.store();
    let good = s.add(&t.repo("src/good"), false).unwrap().project().clone();
    let projects = t.data().join("projects");
    let id = "p_01JC5X8Z3B0K2M4N6P7Q8R9S0T";
    fs::create_dir_all(projects.join(id)).unwrap();
    symlink(projects.join(&good.id).join("project.json"), projects.join(id).join("project.json")).unwrap();
    assert!(s.get(id).is_err());
    assert_eq!(s.list(true).unwrap().projects.len(), 1);
}

#[test]
fn an_oversized_record_is_refused_unread() {
    let t = Tmp::new("oversize");
    let s = t.store();
    let p = s.add(&t.repo("src/a"), false).unwrap().project().clone();
    fs::write(t.data().join("projects").join(&p.id).join("project.json"), vec![b' '; contract::MAX_RECORD_BYTES + 10]).unwrap();
    assert!(s.get(&p.id).is_err());
    assert_eq!(s.list(true).unwrap().skipped, 1);
}

#[test]
fn a_damaged_sessions_line_does_not_hide_the_rest() {
    let t = Tmp::new("sessionsdamaged");
    let s = t.store();
    let p = s.add(&t.repo("src/a"), false).unwrap().project().clone();
    let sid = contract::fresh_id("s").unwrap();
    s.append_session(&session(&p.id, &sid, SessionStatus::Running)).unwrap();
    use std::io::Write;
    let mut f = fs::OpenOptions::new().append(true).open(t.data().join("projects").join(&p.id).join("sessions.jsonl")).unwrap();
    f.write_all(b"garbage line\n{\"schema\":\"vajra.session/9\"}\n").unwrap();
    assert_eq!(s.sessions(&p.id).unwrap().len(), 1);
}

#[test]
fn two_adds_of_the_same_folder_at_once_make_one_project() {
    let t = Tmp::new("race");
    let s = std::sync::Arc::new(t.store());
    let repo = t.repo("src/a");
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let (s, r) = (s.clone(), repo.clone());
            std::thread::spawn(move || s.add(&r, false).unwrap().outcome())
        })
        .collect();
    let outcomes: Vec<&str> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|o| **o == "created").count(), 1, "{outcomes:?}");
    assert_eq!(s.list(true).unwrap().projects.len(), 1);
}
