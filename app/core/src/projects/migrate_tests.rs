use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;

use super::*;

const S1: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
const S2: &str = "1a8fad5b-d9cb-469f-a165-70867728950f";
const LEAD: &str = "2b8fad5b-d9cb-469f-a165-70867728950a";

struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("vajra-migrate-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("home/.claude")).unwrap();
        Self(fs::canonicalize(&d).unwrap())
    }
    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
    fn data(&self) -> PathBuf {
        self.0.join("home/Library/Data")
    }
    fn guard(&self) -> Guard {
        Guard { home: self.home(), data: self.data() }
    }
    fn repo(&self, rel: &str) -> PathBuf {
        let p = self.home().join(rel);
        fs::create_dir_all(p.join(".git/objects")).unwrap();
        fs::write(p.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::canonicalize(&p).unwrap()
    }
    fn store(&self) -> Store {
        Store::open(&self.data(), &self.home()).unwrap()
    }
    fn lead(&self, run: &str, sid: &str) {
        let d = self.data().join("control/lead").join(run);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("lead_session"), sid).unwrap();
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn launch(c: &str, run: &str, sid: &str, cwd: &Path) -> Launch {
    Launch { campaign: c.into(), run_id: run.into(), session_id: sid.into(), cwd: cwd.to_path_buf() }
}

fn names(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

#[test]
fn campaigns_are_grouped_by_their_git_root_and_a_subfolder_or_alias_joins_it() {
    let t = Tmp::new("group");
    let (a, b) = (t.repo("src/alpha"), t.repo("src/beta"));
    fs::create_dir_all(a.join("pkg")).unwrap();
    let alias = t.home().join("alpha-link");
    symlink(&a, &alias).unwrap();
    let ls = [
        launch("c1", "20261101-0900", S1, &a),
        launch("c2", "20261102-0900", S1, &alias),
        launch("c3", "20261103-0900", S2, &b),
    ];
    let cs: Vec<String> = ["c1", "c2", "c3"].map(String::from).to_vec();
    let p = plan(&ls, &cs, &t.guard());
    assert_eq!(p.groups.len(), 2);
    assert_eq!((p.groups[0].name.as_str(), names(&p.groups[0].campaigns)), ("alpha", vec!["c1", "c2"]));
    assert_eq!((p.groups[1].name.as_str(), names(&p.groups[1].campaigns)), ("beta", vec!["c3"]));
    assert!(p.unsorted.is_empty() && p.terminal.is_empty());
}

#[test]
fn a_campaign_is_placed_by_its_newest_launch() {
    let t = Tmp::new("newest");
    let (a, b) = (t.repo("src/alpha"), t.repo("src/beta"));
    let ls = [launch("c", "20261101-0900", S1, &a), launch("c", "20261105-0900", S1, &b)];
    let p = plan(&ls, &["c".to_string()], &t.guard());
    assert_eq!((p.groups.len(), p.groups[0].name.as_str()), (1, "beta"));
}

#[test]
fn untrusted_launch_lines_never_place_a_campaign_silently() {
    let t = Tmp::new("untrusted");
    let ok = t.repo("src/ok");
    let ls = [
        launch("gone", "20261101-0900", S1, &t.home().join("deleted")),
        launch("rel", "20261101-0901", S1, Path::new("relative/dir")),
        launch("home", "20261101-0902", S1, &t.home()),
        launch("data", "20261101-0903", S1, &t.data()),
        launch("claude", "20261101-0904", S1, &t.home().join(".claude")),
        launch("root", "20261101-0905", S1, Path::new("/")),
        launch("fine", "20261101-0906", S1, &ok),
    ];
    fs::create_dir_all(t.data()).unwrap();
    let cs: Vec<String> = ["gone", "rel", "home", "data", "claude", "root", "fine"].map(String::from).to_vec();
    let p = plan(&ls, &cs, &t.guard());
    assert_eq!(names(&p.unsorted), vec!["gone", "rel", "home", "data", "claude", "root"]);
    assert_eq!(p.groups.len(), 1);
    assert_eq!(names(&p.groups[0].campaigns), vec!["fine"]);
}

#[test]
fn a_campaign_with_no_launch_line_is_terminal_owned_and_a_launch_for_no_campaign_is_ignored() {
    let t = Tmp::new("terminal");
    let a = t.repo("src/a");
    let ls = [launch("mine", "20261101-0900", S1, &a), launch("ghost", "20261101-0901", S1, &a)];
    let cs: Vec<String> = ["mine", "from-terminal", "mine"].map(String::from).to_vec();
    let p = plan(&ls, &cs, &t.guard());
    assert_eq!(names(&p.terminal), vec!["from-terminal"]);
    assert_eq!(names(&p.groups[0].campaigns), vec!["mine"], "listed once, and the ghost launch is not a campaign");
}

#[test]
fn confirming_creates_the_project_with_run_and_orchestrator_sessions_and_touches_nothing_else() {
    let t = Tmp::new("confirm");
    let a = t.repo("src/alpha");
    t.lead("20261101-0900", LEAD);
    let ls = [launch("c1", "20261101-0900", S1, &a), launch("c1", "20261102-1030", S2, &a)];
    let cs = vec!["c1".to_string()];
    let runs = t.data().join("runs.jsonl");
    fs::write(&runs, "unchanged\n").unwrap();
    let s = t.store();
    let out = confirm(&s, &t.data(), &t.guard(), &ls, &cs, &[a.to_string_lossy().into_owned()]).unwrap();
    assert_eq!(out.projects.len(), 1);
    assert_eq!((out.projects[0].name.as_str(), out.projects[0].outcome, out.projects[0].sessions), ("alpha", "created", 3));
    let sessions = s.sessions(&out.projects[0].id).unwrap().sessions;
    let orch: Vec<_> = sessions.iter().filter(|x| x.kind == SessionKind::Orchestrator).collect();
    let runs_s: Vec<_> = sessions.iter().filter(|x| x.kind == SessionKind::Run).collect();
    assert_eq!((orch.len(), runs_s.len()), (1, 2));
    assert_eq!(orch[0].native_session_id.as_deref(), Some(LEAD));
    let first = runs_s.iter().find(|x| x.run_id.as_deref() == Some("20261101-0900")).unwrap();
    assert_eq!((first.parent.as_deref(), first.native_session_id.as_deref()), (Some(orch[0].id.as_str()), Some(S1)));
    assert_eq!(first.started, "2026-11-01T09:00:00Z");
    assert!(sessions.iter().all(|x| x.status == SessionStatus::Stopped && x.campaign.as_deref() == Some("c1")));
    assert_eq!(fs::read_to_string(&runs).unwrap(), "unchanged\n");
    assert_eq!(fs::metadata(marker(&t.data())).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn it_runs_once_and_a_second_confirm_changes_nothing() {
    let t = Tmp::new("once");
    let a = t.repo("src/alpha");
    let ls = [launch("c1", "20261101-0900", S1, &a)];
    let cs = vec!["c1".to_string()];
    let s = t.store();
    assert!(!is_done(&t.data()));
    confirm(&s, &t.data(), &t.guard(), &ls, &cs, &[a.to_string_lossy().into_owned()]).unwrap();
    assert!(is_done(&t.data()));
    let again = confirm(&s, &t.data(), &t.guard(), &ls, &cs, &[a.to_string_lossy().into_owned()]);
    assert!(matches!(again, Err(MigrateError::AlreadyDone)));
    let id = s.list(true).unwrap().projects[0].id.clone();
    assert_eq!(s.sessions(&id).unwrap().sessions.len(), 1);
}

#[test]
fn only_roots_from_a_fresh_suggestion_can_be_accepted_and_declined_groups_stay_unsorted() {
    let t = Tmp::new("accept");
    let (a, b) = (t.repo("src/alpha"), t.repo("src/beta"));
    let outside = t.repo("src/never-launched");
    let ls = [launch("c1", "20261101-0900", S1, &a), launch("c2", "20261101-0901", S1, &b)];
    let cs: Vec<String> = ["c1", "c2"].map(String::from).to_vec();
    let s = t.store();
    let out = confirm(&s, &t.data(), &t.guard(), &ls, &cs, &[a.to_string_lossy().into_owned(), outside.to_string_lossy().into_owned()]).unwrap();
    assert_eq!(out.projects.len(), 1);
    assert_eq!(out.unknown, vec![outside.to_string_lossy().into_owned()]);
    let listed = s.list(true).unwrap().projects;
    assert_eq!(listed.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["alpha"], "beta was declined, never added");
}

#[test]
fn a_forged_lead_session_file_or_run_id_is_not_trusted() {
    let t = Tmp::new("forged");
    let a = t.repo("src/alpha");
    t.lead("20261101-0900", "--last");
    let ls = [launch("c1", "20261101-0900", S1, &a), launch("c1", "not-a-run-id", S1, &a)];
    let cs = vec!["c1".to_string()];
    let s = t.store();
    let out = confirm(&s, &t.data(), &t.guard(), &ls, &cs, &[a.to_string_lossy().into_owned()]).unwrap();
    let sessions = s.sessions(&out.projects[0].id).unwrap().sessions;
    assert_eq!(sessions.len(), 1, "no orchestrator for a bad lead id, no session for a bad run id");
    assert_eq!(sessions[0].kind, SessionKind::Run);
}

#[test]
fn an_accepted_root_inside_an_existing_project_is_reported_not_forced() {
    let t = Tmp::new("nested");
    let outer = t.repo("src/outer");
    let inner = t.repo("src/outer/inner");
    let s = t.store();
    s.add(&outer, false).unwrap();
    let ls = [launch("c1", "20261101-0900", S1, &inner)];
    let out = confirm(&s, &t.data(), &t.guard(), &ls, &["c1".to_string()], &[inner.to_string_lossy().into_owned()]).unwrap();
    assert_eq!((out.projects.len(), out.nested.len()), (0, 1));
    assert_eq!(s.list(true).unwrap().projects.len(), 1);
}
