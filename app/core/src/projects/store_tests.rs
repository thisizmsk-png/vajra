use std::path::PathBuf;
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
    let a = s.sessions(&pa.id).unwrap().sessions;
    let b = s.sessions(&pb.id).unwrap().sessions;
    assert_eq!((a.len(), a[0].status), (1, SessionStatus::Done), "the newest line per id wins");
    assert_eq!((b.len(), b[0].id.as_str()), (1, sid_b.as_str()));
    // A session for a project that does not exist, or one that fails validation, is refused.
    assert!(matches!(s.append_session(&session("p_01JC5X8Z3A9K2M4N6P7Q8R9S0T", &sid_a, SessionStatus::Running)), Err(StoreError::NotFound)));
    let mut bad = session(&pa.id, &sid_a, SessionStatus::Running);
    bad.provider = "NOT OK".into();
    assert!(matches!(s.append_session(&bad), Err(StoreError::Invalid(_))));
    assert!(s.sessions(&pb.id).unwrap().sessions.iter().all(|x| x.project_id == pb.id));
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
    fs::write(wt_git.join("gitdir"), format!("{}\n", wt.join(".git").display())).unwrap();
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
    assert_eq!(s.sessions(&p.id).unwrap().sessions.len(), 1, "its history is still there");
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
    let log = s.sessions(&p.id).unwrap();
    assert_eq!((log.sessions.len(), log.skipped, log.unsupported), (1, 2, true), "the damaged and the newer-major line are counted");
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

#[test]
fn a_symlink_or_a_fifo_in_place_of_sessions_jsonl_is_refused_without_hanging() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let t = Tmp::new("sessionsplanted");
    let s = t.store();
    let p = s.add(&t.repo("src/a"), false).unwrap().project().clone();
    let file = t.data().join("projects").join(&p.id).join("sessions.jsonl");
    // A symlink to a file holding valid-looking session lines must not be read as this project's history.
    let other = t.0.join("elsewhere.jsonl");
    let sid = contract::fresh_id("s").unwrap();
    fs::write(&other, format!("{}\n", serde_json::to_string(&session(&p.id, &sid, SessionStatus::Running)).unwrap())).unwrap();
    symlink(&other, &file).unwrap();
    assert!(s.sessions(&p.id).is_err(), "a symlink is not followed");
    assert!(s.append_session(&session(&p.id, &sid, SessionStatus::Done)).is_err(), "nor appended through");
    assert_eq!(fs::read_to_string(&other).unwrap().lines().count(), 1, "the target is untouched");
    // A FIFO must not block the reader (and with it every other command).
    fs::remove_file(&file).unwrap();
    let c = CString::new(file.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    let s = std::sync::Arc::new(s);
    let s2 = s.clone();
    let id = p.id.clone();
    std::thread::spawn(move || tx.send(s2.sessions(&id).is_err()).ok());
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(true), "a FIFO is refused, not waited on");
    assert!(s.list(true).is_ok(), "the store is still usable");
}

#[test]
fn a_symlinked_project_folder_is_refused_and_the_outside_target_is_untouched() {
    let t = Tmp::new("symdir");
    let s = t.store();
    let real = s.add(&t.repo("src/real"), false).unwrap().project().clone();
    let projects = t.data().join("projects");
    // An outside folder holding a perfectly valid record, and a symlink under a valid-looking id pointing at it.
    let outside = t.0.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let id = "p_01JC5X8Z3B0K2M4N6P7Q8R9S0T";
    let mut forged = real.clone();
    forged.id = id.into();
    fs::write(outside.join("project.json"), serde_json::to_vec(&forged).unwrap()).unwrap();
    symlink(&outside, projects.join(id)).unwrap();
    let before: Vec<_> = fs::read_dir(&outside).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert!(s.get(id).is_err(), "the record behind a folder symlink is not served");
    let l = s.list(true).unwrap();
    assert_eq!((l.projects.len(), l.skipped), (1, 1), "and it is skipped when listing");
    assert!(s.rename(id, "x", None).is_err());
    assert!(s.archive(id, None).is_err());
    assert!(s.sessions(id).is_err());
    assert!(s.append_session(&session(id, &contract::fresh_id("s").unwrap(), SessionStatus::Running)).is_err());
    let after: Vec<_> = fs::read_dir(&outside).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(before, after, "nothing was created or changed outside the store");
    assert_eq!(fs::read(outside.join("project.json")).unwrap(), serde_json::to_vec(&forged).unwrap());
    // Adding a folder is unaffected, and the real project still works.
    assert_eq!(s.get(&real.id).unwrap().id, real.id);
}

#[test]
fn a_project_folder_swapped_for_a_symlink_after_the_store_opened_is_not_followed_by_writes() {
    let t = Tmp::new("swapdir");
    let s = t.store();
    let p = s.add(&t.repo("src/a"), false).unwrap().project().clone();
    let dir = t.data().join("projects").join(&p.id);
    let outside = t.0.join("outside2");
    fs::create_dir_all(&outside).unwrap();
    fs::rename(&dir, t.0.join("moved")).unwrap();
    symlink(&outside, &dir).unwrap();
    assert!(s.rename(&p.id, "other", None).is_err());
    assert!(s.append_session(&session(&p.id, &contract::fresh_id("s").unwrap(), SessionStatus::Running)).is_err());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0, "nothing was written through the swapped folder");
}

#[test]
fn a_session_line_from_another_project_is_skipped_and_cannot_replace_a_local_one() {
    use std::io::Write;
    let t = Tmp::new("foreign");
    let s = t.store();
    let a = s.add(&t.repo("src/a"), false).unwrap().project().clone();
    let b = s.add(&t.repo("src/b"), false).unwrap().project().clone();
    let local = contract::fresh_id("s").unwrap();
    s.append_session(&session(&a.id, &local, SessionStatus::Running)).unwrap();
    let foreign_new = contract::fresh_id("s").unwrap();
    // Lines copied (as a same-user process could) from B into A's untrusted log: a session of B's, and a B line that
    // reuses A's local session id with a different status.
    let mut f = fs::OpenOptions::new().append(true).open(t.data().join("projects").join(&a.id).join("sessions.jsonl")).unwrap();
    for line in [session(&b.id, &foreign_new, SessionStatus::Running), session(&b.id, &local, SessionStatus::Failed)] {
        f.write_all(format!("{}\n", serde_json::to_string(&line).unwrap()).as_bytes()).unwrap();
    }
    let log = s.sessions(&a.id).unwrap();
    assert_eq!(log.skipped, 2);
    assert!(!log.unsupported);
    assert_eq!(log.sessions.len(), 1);
    assert_eq!((log.sessions[0].id.as_str(), log.sessions[0].project_id.as_str(), log.sessions[0].status), (local.as_str(), a.id.as_str(), SessionStatus::Running));
}

#[test]
fn an_append_after_a_torn_tail_is_not_lost() {
    use std::io::Write;
    let t = Tmp::new("torn");
    let s = t.store();
    let p = s.add(&t.repo("src/a"), false).unwrap().project().clone();
    let first = contract::fresh_id("s").unwrap();
    s.append_session(&session(&p.id, &first, SessionStatus::Running)).unwrap();
    // A crash mid-append: a JSON fragment with no newline.
    let file = t.data().join("projects").join(&p.id).join("sessions.jsonl");
    fs::OpenOptions::new().append(true).open(&file).unwrap().write_all(br#"{"schema":"vajra.session/1","id":"s_0"#).unwrap();
    s.append_session(&session(&p.id, &first, SessionStatus::Done)).unwrap();
    let second = contract::fresh_id("s").unwrap();
    s.append_session(&session(&p.id, &second, SessionStatus::Running)).unwrap();
    let log = s.sessions(&p.id).unwrap();
    assert_eq!(log.skipped, 1, "only the fragment is lost");
    let got: Vec<_> = log.sessions.iter().map(|x| (x.id.as_str(), x.status)).collect();
    assert_eq!(got, vec![(first.as_str(), SessionStatus::Done), (second.as_str(), SessionStatus::Running)]);
    assert!(fs::read_to_string(&file).unwrap().ends_with('\n'));
}

#[test]
fn a_scan_that_hit_its_limit_is_reported_and_add_refuses_instead_of_duplicating() {
    let t = Tmp::new("limit");
    let s = t.store().with_limits(3, 100);
    let repos: Vec<_> = (0..4).map(|i| t.repo(&format!("src/r{i}"))).collect();
    for r in &repos[..3] {
        s.add(r, false).unwrap();
    }
    // At capacity: a fourth is refused and writes nothing; the three existing ones still resolve as existing.
    assert!(matches!(s.add(&repos[3], false), Err(StoreError::Full)));
    assert_eq!(s.add(&repos[0], false).unwrap().outcome(), "existing");
    let l = s.list(true).unwrap();
    assert_eq!((l.projects.len(), l.truncated), (3, false));
    // More valid records than a (smaller) cap: the listing says it is partial, and add refuses everything that needs
    // the whole inventory, including a re-add of a project the scan did not reach.
    let small = t.store().with_limits(2, 100);
    let l = small.list(true).unwrap();
    assert_eq!((l.projects.len(), l.truncated), (2, true));
    assert!(matches!(small.add(&repos[0], false), Err(StoreError::Full)));
    assert!(matches!(small.add(&repos[2], false), Err(StoreError::Full)));
    assert_eq!(t.store().list(true).unwrap().projects.len(), 3, "nothing was written");
    // Stray entries use up the scan budget too, and are reported the same way.
    let tight = t.store().with_limits(500, 2);
    assert!(tight.list(true).unwrap().truncated);
    assert!(matches!(tight.add(&repos[3], false), Err(StoreError::Full)));
}

#[test]
fn the_projects_folder_swapped_for_a_symlink_after_open_redirects_nothing() {
    let t = Tmp::new("swapparent");
    let s = t.store();
    let p = s.add(&t.repo("src/a"), false).unwrap().project().clone();
    // An outside folder with a valid record under the same id, and the projects folder replaced by a symlink to it.
    let outside = t.0.join("outside-projects");
    fs::create_dir_all(outside.join(&p.id)).unwrap();
    let planted = serde_json::to_vec(&p).unwrap();
    fs::write(outside.join(&p.id).join("project.json"), &planted).unwrap();
    let projects = t.data().join("projects");
    fs::rename(&projects, t.0.join("moved-projects")).unwrap();
    symlink(&outside, &projects).unwrap();
    // The open store keeps working on the folder it opened, and never touches the outside one.
    s.rename(&p.id, "renamed", None).unwrap();
    s.append_session(&session(&p.id, &contract::fresh_id("s").unwrap(), SessionStatus::Running)).unwrap();
    assert_eq!(fs::read(outside.join(&p.id).join("project.json")).unwrap(), planted);
    assert!(!outside.join(&p.id).join("sessions.jsonl").exists());
    assert!(t.0.join("moved-projects").join(&p.id).join("sessions.jsonl").exists());
    // A store opened afterwards refuses the symlinked folder.
    assert!(Store::open(&t.data(), &t.home()).is_err());
}

#[test]
fn a_log_longer_than_the_read_window_says_it_was_cut() {
    use std::io::Write;
    let t = Tmp::new("longlog");
    let s = t.store();
    let p = s.add(&t.repo("src/a"), false).unwrap().project().clone();
    let old = contract::fresh_id("s").unwrap();
    s.append_session(&session(&p.id, &old, SessionStatus::Running)).unwrap();
    assert!(!s.sessions(&p.id).unwrap().truncated, "a short log is complete");
    // Enough newer lines (updates of another session) to push the first one out of the 1 MiB window.
    let busy = contract::fresh_id("s").unwrap();
    let line = format!("{}\n", serde_json::to_string(&session(&p.id, &busy, SessionStatus::Running)).unwrap());
    let mut f = fs::OpenOptions::new().append(true).open(t.data().join("projects").join(&p.id).join("sessions.jsonl")).unwrap();
    for _ in 0..(SESSIONS_READ_CAP as usize / line.len() + 10) {
        f.write_all(line.as_bytes()).unwrap();
    }
    drop(f);
    let log = s.sessions(&p.id).unwrap();
    assert!(log.truncated, "the caller can tell the history is incomplete");
    assert_eq!(log.skipped, 0, "the cut first line is not counted as damage");
    assert!(log.sessions.iter().all(|x| x.id == busy), "the old session is outside the window");
}
