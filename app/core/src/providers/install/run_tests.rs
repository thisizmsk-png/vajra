use std::sync::Arc;

use super::super::env::setup_env;
use super::super::npm;
use super::super::plan::integrity;
use super::super::testkit::{tgz, FakeNpm};
use super::*;
use crate::bus::{Bus, Config};
use crate::providers::catalog;

const NOT_INSTALLED: &str = r#"{"dependencies":{}}"#;
const GOOD: &str = r#"{"dependencies":{"x":{"version":"1.0.0"}}}"#;
const EXTRA: &str = r#"{"dependencies":{"x":{"version":"1.0.0","dependencies":{"evil":{"version":"9.9.9"}}}}}"#;

fn me() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

struct World {
    f: FakeNpm,
    sup: Supervisor,
    bus: Arc<Bus>,
    data: PathBuf,
    env: SetupEnv,
}

fn world(tag: &str, before: &str) -> World {
    let bytes = tgz(&[("package/package.json", r#"{"name":"x","version":"1.0.0","bin":{"x":"cli.js"}}"#), ("package/cli.js", "1")]);
    let f = FakeNpm::new(&format!("run-{tag}"), &bytes, before, None);
    let data = f.dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(f.dir.join("prefix/bin/x"), b"").unwrap();
    let bus = Arc::new(Bus::open(&data.join("spill"), Config::default()).unwrap());
    let env = setup_env(&[f.dir.join("tool/bin")], |k| std::env::var_os(k).filter(|_| ["HOME", "USER", "LANG"].contains(&k)));
    World { sup: Supervisor::new(bus.clone(), None), bus, data, env, f }
}

impl World {
    fn plan(&self) -> (Plan, npm::NpmRun) {
        let bytes = std::fs::read(self.f.dir.join("fixture.tgz")).unwrap();
        let m = catalog::Npm { package: "x".into(), version: "1.0.0".into(), integrity: integrity(&bytes), command: "npm install -g x".into() };
        npm::plan("x", "x", &m, &self.f.npm(), &self.env, me()).unwrap()
    }
    fn run(&self) -> Report {
        let (p, r) = self.plan();
        let bin = self.f.dir.join("prefix/bin/x");
        let current = move || Some((bin.clone(), Some("1.0.0".to_string())));
        let ctx = Ctx { sup: &self.sup, env: &self.env, marks: &Marks::at(&self.data), audit: &self.data.join("audit.log"), timeout: Duration::from_secs(20), current: &current };
        run_npm(&ctx, &p, r)
    }
    fn marked(&self) -> Option<String> {
        Marks::at(&self.data).active("x", &self.f.dir.join("prefix/bin/x"), Some("1.0.0"))
    }
    fn audit(&self) -> String {
        std::fs::read_to_string(self.data.join("audit.log")).unwrap_or_default()
    }
}

#[test]
fn a_clean_install_writes_the_verified_tarball_privately_and_checks_the_result() {
    let w = world("ok", NOT_INSTALLED);
    std::fs::write(w.f.dir.join("install.ls"), GOOD).unwrap();
    let r = w.run();
    assert_eq!((r.outcome, r.reason, r.exit), (Outcome::Installed, None, Some(0)), "{}", w.f.log());
    assert_eq!(std::fs::read(w.f.dir.join("installed.tgz")).unwrap(), std::fs::read(w.f.dir.join("fixture.tgz")).unwrap(), "npm got the bytes that were hashed");
    assert!(std::fs::read_to_string(w.f.dir.join("installed.dir")).unwrap().starts_with("drwx------"), "the tarball's folder is private");
    assert!(w.f.log().contains("install -g --ignore-scripts --registry=https://registry.npmjs.org/ /"), "{}", w.f.log());
    assert!(!w.f.log().contains("uninstall"));
    let a = w.audit();
    assert!(a.contains("SETUP desktop-control reason=\"setup id=x method=npm version=1.0.0 exit=0 result=installed\""), "{a}");
    assert!(!a.contains(&w.f.dir.display().to_string()), "no paths in the audit line: {a}");
}

#[test]
fn a_result_outside_the_approved_closure_is_rolled_back_and_must_be_gone() {
    let w = world("extra", NOT_INSTALLED);
    std::fs::write(w.f.dir.join("install.ls"), EXTRA).unwrap();
    let r = w.run();
    assert_eq!((r.outcome, r.reason), (Outcome::RolledBack, Some("closure_mismatch")));
    let node = w.f.dir.join("tool/bin/node");
    assert!(w.f.log().contains("uninstall -g --ignore-scripts --registry=https://registry.npmjs.org/ x"), "{}", w.f.log());
    assert!(std::fs::read_to_string(w.f.dir.join("env.uninstall")).unwrap().contains("PATH="), "the uninstall runs on the Setup env");
    assert!(node.exists());
    assert_eq!(w.marked(), None);
}

#[test]
fn a_failed_exit_is_rolled_back_too() {
    let w = world("fail", NOT_INSTALLED);
    std::fs::write(w.f.dir.join("install.ls"), GOOD).unwrap();
    std::fs::write(w.f.dir.join("install.fail"), b"").unwrap();
    let r = w.run();
    assert_eq!((r.outcome, r.reason, r.exit), (Outcome::RolledBack, Some("install_failed"), Some(1)));
    assert!(w.f.log().contains("uninstall -g"), "rollback on every exit, not only 0");
    assert!(w.audit().contains("exit=1 result=rolled_back"), "{}", w.audit());
}

#[test]
fn a_rollback_that_leaves_it_installed_marks_it_broken() {
    let w = world("stuck", NOT_INSTALLED);
    std::fs::write(w.f.dir.join("install.ls"), EXTRA).unwrap();
    std::fs::write(w.f.dir.join("uninstall.keep"), b"").unwrap();
    let r = w.run();
    assert_eq!((r.outcome, r.reason), (Outcome::Broken, Some("rollback_failed")));
    assert_eq!(w.marked().as_deref(), Some("rollback_failed"));
}

#[test]
fn a_rollback_whose_check_cannot_be_read_is_not_called_done() {
    for (tag, after) in [("rb-bad", "npm ERR! something broke"), ("rb-err", r#"{"error":{"code":"EJSONPARSE","summary":"bad"}}"#)] {
        let w = world(tag, NOT_INSTALLED);
        std::fs::write(w.f.dir.join("install.ls"), EXTRA).unwrap();
        std::fs::write(w.f.dir.join("uninstall.ls"), after).unwrap();
        let r = w.run();
        assert_eq!((r.outcome, r.reason), (Outcome::Broken, Some("rollback_failed")), "{after}");
        assert_eq!(w.marked().as_deref(), Some("rollback_failed"), "{after}");
    }
}

#[test]
fn a_failed_upgrade_that_changed_the_install_marks_it_broken_and_a_clean_one_clears_it() {
    let w = world("upgrade", r#"{"dependencies":{"x":{"version":"0.9.0"}}}"#);
    std::fs::write(w.f.dir.join("install.ls"), EXTRA).unwrap();
    let r = w.run();
    assert_eq!((r.outcome, r.reason), (Outcome::Broken, Some("closure_mismatch")));
    assert!(!w.f.log().contains("uninstall"), "an upgrade is never uninstalled");
    assert_eq!(w.marked().as_deref(), Some("closure_mismatch"));
    std::fs::write(w.f.dir.join("install.ls"), GOOD).unwrap();
    assert_eq!(w.run().outcome, Outcome::Installed);
    assert_eq!(w.marked(), None, "a passing closure check clears the mark");
}

#[test]
fn a_failed_upgrade_that_changed_nothing_is_just_failed() {
    let w = world("same", r#"{"dependencies":{"x":{"version":"0.9.0"}}}"#);
    std::fs::write(w.f.dir.join("install.fail"), b"").unwrap();
    let r = w.run();
    assert_eq!((r.outcome, r.reason), (Outcome::Failed, Some("install_failed")));
    assert_eq!(w.marked(), None);
}

#[test]
fn the_run_streams_on_the_setup_stream() {
    let w = world("stream", NOT_INSTALLED);
    std::fs::write(w.f.dir.join("install.ls"), GOOD).unwrap();
    w.run();
    let _ = &w.bus;
    assert!(w.sup.wait(&stream("x"), Duration::from_secs(1)).unwrap().is_some(), "the run is a setup-x session");
}

mod brew_run {
    use super::super::super::brew::{self, Kind};
    use super::super::super::testkit::FakeBrew;
    use super::*;

    fn cask(installed: &str) -> String {
        format!(
            r#"{{"formulae":[],"casks":[{{"token":"codex","full_token":"codex","tap":"homebrew/cask","version":"0.167.1","installed":{installed},"sha256":"{}","url":"https://github.com/openai/codex/releases/download/v0.167.1/codex.tar.gz","deprecated":false,"disabled":false,"depends_on":{{}},"artifacts":[{{"binary":["codex-aarch64-apple-darwin",{{"target":"codex"}}]}}]}}]}}"#,
            "a".repeat(64)
        )
    }

    /// What Set up's detection sees after the run: the program and its version (None: it doesn't run), or nothing.
    type Seen = Option<(PathBuf, Option<String>)>;

    fn working() -> Seen {
        Some((PathBuf::from("/opt/homebrew/bin/codex"), Some("0.168.0".into())))
    }

    fn go(tag: &str, after: Option<&str>, fail: bool) -> (Report, String, String) {
        let (r, audit, log, _) = go_with(tag, "null", after, fail, working());
        (r, audit, log)
    }

    /// Plan with brew recording `before` as installed, run, and return the report, audit, brew log and Broken mark.
    fn go_with(tag: &str, before: &str, after: Option<&str>, fail: bool, seen: Seen) -> (Report, String, String, Option<String>) {
        let f = FakeBrew::new(&format!("run-{tag}"), &cask(before));
        if let Some(a) = after {
            std::fs::write(f.dir.join("after.json"), a).unwrap();
        }
        if fail {
            std::fs::write(f.dir.join("install.fail"), b"").unwrap();
        }
        let data = f.dir.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let bus = Arc::new(Bus::open(&data.join("spill"), Config::default()).unwrap());
        let sup = Supervisor::new(bus, None);
        let env = setup_env(&[], |_| None);
        let m = catalog::Brew { package: "codex".into(), tap: Some("homebrew/cask".into()), cask_url_hosts: vec!["github.com".into()], command: "brew install --cask codex".into() };
        let (p, r) = brew::plan("codex", &["codex".into()], Kind::Cask, &m, &f.brew(), &env, "arm64_sequoia", me()).unwrap();
        let path = seen.as_ref().map(|(p, _)| p.clone());
        let current = move || seen.clone();
        let marks = Marks::at(&data);
        let ctx = Ctx { sup: &sup, env: &env, marks: &marks, audit: &data.join("audit.log"), timeout: Duration::from_secs(20), current: &current };
        let rep = run_brew(&ctx, &p, &r);
        let mark = path.and_then(|p| marks.active("codex", &p, None));
        (rep, std::fs::read_to_string(data.join("audit.log")).unwrap_or_default(), f.log(), mark)
    }

    #[test]
    fn a_brew_install_records_the_version_read_after_it() {
        let (r, audit, log) = go("ok", Some(&cask("\"0.168.0\"")), false);
        assert_eq!((r.outcome, r.version.as_deref()), (Outcome::Installed, Some("0.168.0")), "{log}");
        assert!(log.contains("install --cask homebrew/cask/codex"), "{log}");
        assert!(audit.contains("method=brew_cask version=0.168.0 exit=0 result=installed"), "{audit}");
    }

    #[test]
    fn a_brew_install_that_fails_or_leaves_nothing_installed_is_failed() {
        assert_eq!(go("fail", None, true).0.outcome, Outcome::Failed);
        let (r, _, _) = go("none", None, false);
        assert_eq!((r.outcome, r.reason), (Outcome::Failed, Some("not_installed")));
    }

    #[test]
    fn a_repair_of_a_registered_package_reinstalls_it() {
        let (r, _, log, _) = go_with("repair", "\"0.167.1\"", Some(&cask("\"0.168.0\"")), false, working());
        assert_eq!(r.outcome, Outcome::Installed, "{log}");
        assert!(log.contains("reinstall --cask homebrew/cask/codex"), "{log}");
    }

    #[test]
    fn brew_saying_installed_is_not_enough_the_program_must_run() {
        let broken = Some((PathBuf::from("/opt/homebrew/bin/codex"), None));
        let (r, audit, _, mark) = go_with("still-broken", "\"0.167.1\"", None, false, broken);
        assert_eq!((r.outcome, r.reason), (Outcome::Broken, Some("not_working")), "registered, exit 0, but detection fails");
        assert_eq!(mark.as_deref(), Some("not_working"), "the Broken mark stays");
        assert!(audit.contains("result=broken"), "{audit}");
        let (r, _, _, _) = go_with("gone", "\"0.167.1\"", None, false, None);
        assert_eq!((r.outcome, r.reason), (Outcome::Failed, Some("not_detected")));
    }
}

mod script_run {
    use super::*;

    #[test]
    fn a_vendor_script_runs_as_a_checked_file_and_its_folder_is_removed() {
        let d = std::env::temp_dir().join(format!("vajra-run-script-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let bus = Arc::new(Bus::open(&d.join("spill"), Config::default()).unwrap());
        let sup = Supervisor::new(bus.clone(), None);
        let env = setup_env(&[], |_| None);
        let body = b"echo vendor-ran\n";
        use sha2::Digest;
        let m = catalog::VendorScript { url: "https://vendor.example/install.sh".into(), sha256: super::super::super::plan::hex(&sha2::Sha256::digest(body)), self_verifying: true, command: String::new() };
        let p = super::super::super::script::plan("v", &m).unwrap();
        let seen = std::cell::RefCell::new(None);
        let get = |_: &str, dest: &Path| {
            *seen.borrow_mut() = Some(dest.to_path_buf());
            std::fs::write(dest, body).map_err(|_| "fetch")
        };
        let current = || None;
        let ctx = Ctx { sup: &sup, env: &env, marks: &Marks::at(&d), audit: &d.join("audit.log"), timeout: Duration::from_secs(20), current: &current };
        let r = run_script(&ctx, &p, &m, &get);
        assert_eq!((r.outcome, r.exit), (Outcome::Installed, Some(0)));
        assert!(!seen.into_inner().unwrap().exists(), "the checked script is removed after the run");
        let lines: Vec<String> = bus.tail(&stream("v"), 0, usize::MAX).lines.iter().map(|l| l.text.to_string()).collect();
        assert!(lines.iter().any(|l| l.contains("vendor-ran")), "{lines:?}");
        assert!(std::fs::read_to_string(d.join("audit.log")).unwrap().contains("method=vendor_script"));
    }
}
