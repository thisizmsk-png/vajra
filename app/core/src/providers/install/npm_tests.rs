use super::super::env::setup_env;
use super::super::plan::{integrity, TARBALL};
use super::super::testkit::{tgz, FakeNpm};
use super::*;

const NOT_INSTALLED: &str = r#"{"dependencies":{}}"#;

fn me() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

fn method(pkg: &str, bytes: &[u8]) -> catalog::Npm {
    catalog::Npm { package: pkg.into(), version: "1.0.0".into(), integrity: integrity(bytes), command: format!("npm install -g {pkg}") }
}

fn env(f: &FakeNpm) -> SetupEnv {
    setup_env(&[f.dir.join("tool/bin")], |k| std::env::var_os(k).filter(|_| ["HOME", "USER", "LANG"].contains(&k)))
}

fn clean() -> Vec<u8> {
    tgz(&[("package/package.json", r#"{"name":"x","version":"1.0.0","bin":{"x":"cli.js"}}"#), ("package/cli.js", "1")])
}

#[test]
fn a_pinned_tarball_with_no_outside_dependencies_gives_a_plan_with_the_exact_command() {
    let bytes = clean();
    let f = FakeNpm::new("plan", &bytes, NOT_INSTALLED, None);
    let (p, run) = plan("x", "x", &method("x", &bytes), &f.npm(), &env(&f), me()).unwrap();
    let node = f.dir.join("tool/bin/node").display().to_string();
    let cli = f.dir.join("tool/lib/node_modules/npm/bin/npm-cli.js").display().to_string();
    assert_eq!(p.command, [node.as_str(), cli.as_str(), "install", "-g", "--ignore-scripts", "--registry=https://registry.npmjs.org/", TARBALL]);
    assert_eq!((p.method, p.version.as_str(), p.source.as_str()), ("npm", "1.0.0", "registry.npmjs.org"));
    assert_eq!(p.target, f.dir.join("prefix").display().to_string());
    assert_eq!(p.needs_admin, None);
    assert!(p.notes.contains(&"npmrc_applies"));
    assert_eq!(p.tools.iter().map(|t| t.role).collect::<Vec<_>>(), ["node", "npm"]);
    assert_eq!(run.tgz, bytes);
    assert_eq!(run.snapshot, None, "nothing installed before");
    let log = f.log();
    assert!(log.contains("pack x@1.0.0 --ignore-scripts --registry=https://registry.npmjs.org/ --pack-destination"), "{log}");
    assert!(!log.contains("install -g"), "planning never installs: {log}");
}

#[test]
fn a_tarball_whose_sha512_differs_from_the_pin_is_refused() {
    let f = FakeNpm::new("mismatch", &clean(), NOT_INSTALLED, None);
    let m = method("x", b"other bytes");
    assert_eq!(plan("x", "x", &m, &f.npm(), &env(&f), me()).unwrap_err(), "integrity_mismatch");
}

#[test]
fn an_unpinned_dependency_is_install_it_yourself_and_npm_install_never_runs() {
    let bytes = tgz(&[("package/package.json", r#"{"name":"x","version":"1.0.0","dependencies":{"dep":"^1.0.0"}}"#)]);
    let f = FakeNpm::new("unpinned", &bytes, NOT_INSTALLED, None);
    assert_eq!(plan("x", "x", &method("x", &bytes), &f.npm(), &env(&f), me()).unwrap_err(), "unpinned_dependency");
    assert!(!f.log().contains("install"), "{}", f.log());
}

#[test]
fn pack_must_leave_exactly_one_regular_file_whatever_its_json_says() {
    let bytes = clean();
    let f = FakeNpm::new("two", &bytes, NOT_INSTALLED, None);
    std::fs::write(f.dir.join("extra"), "x").unwrap();
    assert_eq!(plan("x", "x", &method("x", &bytes), &f.npm(), &env(&f), me()).unwrap_err(), "pack_output");
}

#[test]
fn a_root_owned_prefix_shows_the_command_with_the_reason() {
    let bytes = clean();
    let f = FakeNpm::new("admin", &bytes, NOT_INSTALLED, Some("/var/empty"));
    let (p, _) = plan("x", "x", &method("x", &bytes), &f.npm(), &env(&f), me()).unwrap();
    let a = p.needs_admin.expect("root-owned prefix");
    assert_eq!((a.path.as_str(), a.reason), ("/var/empty", "root_owned"));
}

#[test]
fn a_scoped_package_shows_its_effective_scope_registry() {
    let bytes = tgz(&[("package/package.json", r#"{"name":"@s/x","version":"1.0.0"}"#)]);
    let f = FakeNpm::new("scope", &bytes, NOT_INSTALLED, None);
    let (p, _) = plan("x", "x", &method("@s/x", &bytes), &f.npm(), &env(&f), me()).unwrap();
    assert_eq!(p.scope_registry.as_deref(), Some("https://mirror.example/"));
    assert!(f.log().contains("config get @s:registry"), "{}", f.log());
}

#[test]
fn the_snapshot_records_what_is_installed_now() {
    let bytes = clean();
    let f = FakeNpm::new("snap", &bytes, r#"{"dependencies":{"x":{"version":"0.9.0"}}}"#, None);
    let (_, run) = plan("x", "x", &method("x", &bytes), &f.npm(), &env(&f), me()).unwrap();
    assert_eq!(run.snapshot, Some([("x".to_string(), "0.9.0".to_string())].into()));
}

#[test]
fn npm_runs_with_the_setup_env_only() {
    let bytes = clean();
    let f = FakeNpm::new("env", &bytes, NOT_INSTALLED, None);
    plan("x", "x", &method("x", &bytes), &f.npm(), &env(&f), me()).unwrap();
    let seen = std::fs::read_to_string(f.dir.join("env.pack")).unwrap();
    for line in seen.lines() {
        let k = line.split_once('=').map_or(line, |(k, _)| k);
        assert!(super::super::env::ALLOWED.contains(&k) || k == "PWD" || k == "SHLVL" || k == "_" || k == "OLDPWD", "{k} reached npm");
    }
}
