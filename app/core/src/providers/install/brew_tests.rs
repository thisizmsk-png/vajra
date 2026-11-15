use super::super::env::setup_env;
use super::super::testkit::FakeBrew;
use super::*;

const TAG: &str = "arm64_sequoia";

fn me() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

fn cask_json(extra: &str) -> String {
    format!(
        r#"{{"formulae":[],"casks":[{{"token":"codex","full_token":"codex","tap":"homebrew/cask","version":"0.167.1","sha256":"{}","url":"https://github.com/openai/codex/releases/download/v0.167.1/codex.tar.gz","deprecated":false,"disabled":false,"depends_on":{{}},"artifacts":[{{"binary":["codex-aarch64-apple-darwin",{{"target":"codex"}}]}}]{extra}}}]}}"#,
        "a".repeat(64)
    )
}

fn formula_json(bottle_tag: &str) -> String {
    format!(
        r#"{{"formulae":[{{"name":"gemini-cli","full_name":"gemini-cli","tap":"homebrew/core","versions":{{"stable":"0.67.0"}},"deprecated":false,"disabled":false,"dependencies":["node"],"bottle":{{"stable":{{"files":{{"{bottle_tag}":{{"url":"https://ghcr.io/v2/homebrew/core/gemini-cli/blobs/sha256:{}","sha256":"{}"}}}}}}}}}}],"casks":[]}}"#,
        "b".repeat(64),
        "b".repeat(64)
    )
}

fn cask_method() -> catalog::Brew {
    catalog::Brew { package: "codex".into(), tap: Some("homebrew/cask".into()), cask_url_hosts: vec!["github.com".into()], command: "brew install --cask codex".into() }
}

fn formula_method() -> catalog::Brew {
    catalog::Brew { package: "gemini-cli".into(), tap: Some("homebrew/core".into()), cask_url_hosts: vec![], command: "brew install gemini-cli".into() }
}

fn env() -> SetupEnv {
    setup_env(&[], |_| None)
}

fn cask(info: &str) -> Result<(Plan, BrewRun), &'static str> {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let f = FakeBrew::new(&format!("c{}", N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)), info);
    plan("codex", &["codex".into()], Kind::Cask, &cask_method(), &f.brew(), &env(), TAG, me())
}

#[test]
fn a_clean_official_cask_gives_a_plan_with_the_fully_qualified_command() {
    let f = FakeBrew::new("cask", &cask_json(""));
    let (p, run) = plan("codex", &["codex".into()], Kind::Cask, &cask_method(), &f.brew(), &env(), TAG, me()).unwrap();
    let brew = f.dir.join("brew/bin/brew").display().to_string();
    assert_eq!(p.command, [brew.as_str(), "install", "--cask", "homebrew/cask/codex"]);
    assert_eq!((p.method, p.version.as_str(), p.source.as_str()), ("brew_cask", "0.167.1", "homebrew/cask"));
    assert_eq!(p.pin, Pin::Brew { token: "homebrew/cask/codex".into(), sha256: "a".repeat(64), url: "https://github.com/openai/codex/releases/download/v0.167.1/codex.tar.gz".into() });
    assert!(p.notes.contains(&"brew_not_pinned") && p.notes.contains(&"brew_env_applies"));
    assert_eq!(p.target, f.dir.join("prefix").display().to_string());
    assert_eq!(p.needs_admin, None);
    assert_eq!(run.token, "homebrew/cask/codex");
    assert!(f.log().contains("info --json=v2 --cask homebrew/cask/codex"), "{}", f.log());
    assert!(!f.log().contains("install"), "planning never installs");
}

#[test]
fn a_package_brew_already_records_is_reinstalled_and_the_verb_is_in_the_hash() {
    let fresh = cask(&cask_json("")).unwrap().0;
    let (repair, _) = cask(&cask_json(r#","installed":"0.167.1""#)).unwrap();
    assert_eq!(repair.command[1..], ["reinstall", "--cask", "homebrew/cask/codex"], "install would skip it and repair nothing");
    let mut swapped = fresh.clone();
    swapped.command[1] = "reinstall".into();
    assert_ne!(swapped.rehash().hash, fresh.hash, "a consent to install is not a consent to reinstall");
    let none = cask(&cask_json(r#","installed":null"#)).unwrap().0;
    assert_eq!(none.command[1], "install");
}

#[test]
fn a_no_check_cask_is_refused() {
    let info = cask_json("").replace(&"a".repeat(64), "no_check");
    assert_eq!(cask(&info).unwrap_err(), "cask_no_check");
}

#[test]
fn a_cask_from_another_tap_or_token_or_a_deprecated_one_is_refused() {
    assert_eq!(cask(&cask_json("").replace("\"tap\":\"homebrew/cask\"", "\"tap\":\"someone/tap\"")).unwrap_err(), "brew_tap");
    assert_eq!(cask(&cask_json("").replace("\"full_token\":\"codex\"", "\"full_token\":\"someone/tap/codex\"")).unwrap_err(), "brew_token");
    assert_eq!(cask(&cask_json("").replace("\"deprecated\":false", "\"deprecated\":true")).unwrap_err(), "brew_deprecated");
    assert_eq!(cask(&cask_json("").replace("\"disabled\":false", "\"disabled\":true")).unwrap_err(), "brew_deprecated");
}

#[test]
fn a_cask_url_off_the_vendor_hosts_or_not_https_is_refused() {
    assert_eq!(cask(&cask_json("").replace("https://github.com/", "https://evil.example/")).unwrap_err(), "cask_url_host");
    assert_eq!(cask(&cask_json("").replace("https://github.com/", "http://github.com/")).unwrap_err(), "cask_url_host");
}

#[test]
fn a_cask_that_runs_an_installer_or_links_another_binary_is_refused() {
    for art in [r#"{"pkg":["x.pkg"]}"#, r#"{"installer":[{"script":"x"}]}"#, r#"{"postflight":null}"#, r#"{"preflight":null}"#] {
        let info = cask_json("").replace(r#"{"binary":["codex-aarch64-apple-darwin",{"target":"codex"}]}"#, art);
        assert_eq!(cask(&info).unwrap_err(), "cask_runs_installer", "{art}");
    }
    let other = cask_json("").replace(r#"{"target":"codex"}"#, r#"{"target":"git"}"#);
    assert_eq!(cask(&other).unwrap_err(), "cask_binary");
}

#[test]
fn a_cask_whose_uninstall_runs_scripts_or_touches_the_system_is_refused() {
    for un in [r#"{"uninstall":[{"script":{"executable":"x"}}]}"#, r#"{"uninstall":[{"pkgutil":"com.x"}]}"#, r#"{"uninstall":[{"launchctl":"com.x"}]}"#, r#"{"uninstall":[{"delete":"/Library/x"}]}"#, r#"{"uninstall_postflight":null}"#, r#"{"uninstall_preflight":null}"#] {
        let info = cask_json("").replace(r#"{"binary":["codex-aarch64-apple-darwin",{"target":"codex"}]}"#, &format!(r#"{{"binary":["codex-aarch64-apple-darwin",{{"target":"codex"}}]}},{un}"#));
        assert_eq!(cask(&info).unwrap_err(), "cask_uninstall", "{un}");
    }
    let quit = cask_json("").replace(r#"{"binary":["codex-aarch64-apple-darwin",{"target":"codex"}]}"#, r#"{"binary":["codex-aarch64-apple-darwin",{"target":"codex"}]},{"uninstall":[{"quit":"com.x"}]}"#);
    assert!(cask(&quit).is_ok(), "a harmless uninstall stanza is fine");
}

#[test]
fn a_formula_needs_a_bottle_for_this_platform_and_shows_its_dependencies() {
    let f = FakeBrew::new("formula", &formula_json(TAG));
    let (p, _) = plan("gemini", &["gemini".into()], Kind::Formula, &formula_method(), &f.brew(), &env(), TAG, me()).unwrap();
    assert_eq!(p.command.last().map(String::as_str), Some("homebrew/core/gemini-cli"));
    assert!(!p.command.contains(&"--cask".to_string()));
    assert_eq!(p.deps, ["node"]);
    assert_eq!(p.version, "0.67.0");
    assert!(matches!(p.pin, Pin::Brew { ref sha256, .. } if *sha256 == "b".repeat(64)));
    let all = FakeBrew::new("formula-all", &formula_json("all"));
    assert!(plan("gemini", &["gemini".into()], Kind::Formula, &formula_method(), &all.brew(), &env(), TAG, me()).is_ok(), "an `all` bottle pours anywhere");
    let none = FakeBrew::new("formula-none", &formula_json("x86_64_linux"));
    assert_eq!(plan("gemini", &["gemini".into()], Kind::Formula, &formula_method(), &none.brew(), &env(), TAG, me()).unwrap_err(), "no_bottle");
}

#[test]
fn the_platform_tag_names_the_arch_and_the_macos_release() {
    assert_eq!(platform_tag("aarch64", 15), Some("arm64_sequoia".to_string()));
    assert_eq!(platform_tag("x86_64", 14), Some("sonoma".to_string()));
    assert_eq!(platform_tag("aarch64", 26), Some("arm64_tahoe".to_string()));
    assert_eq!(platform_tag("aarch64", 9), None);
}

#[test]
fn a_root_owned_homebrew_prefix_needs_admin() {
    let f = FakeBrew::new("admin", &cask_json(""));
    std::fs::remove_dir_all(f.dir.join("prefix")).unwrap();
    std::os::unix::fs::symlink("/var/empty", f.dir.join("prefix")).unwrap();
    let (p, _) = plan("codex", &["codex".into()], Kind::Cask, &cask_method(), &f.brew(), &env(), TAG, me()).unwrap();
    assert_eq!(p.needs_admin.map(|a| a.reason), Some("root_owned"));
}
