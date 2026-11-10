use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn tmp() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("vajra-codex-home-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn parsed(text: &str) -> toml::Table {
    text.parse().expect("the generated config is valid TOML")
}

#[test]
fn config_parses_and_names_the_keychain_command() {
    let t = parsed(&render("openrouter", DEFAULT_BASE_URL).unwrap());
    assert_eq!(t["model_provider"].as_str(), Some("vajra-openrouter"));
    let p = &t["model_providers"][PROVIDER_ID];
    assert_eq!(p["base_url"].as_str(), Some(DEFAULT_BASE_URL));
    assert_eq!(p["wire_api"].as_str(), Some("responses"));
    let auth = &p["auth"];
    assert_eq!(auth["command"].as_str(), Some("/usr/bin/security"));
    let args: Vec<&str> = auth["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert_eq!(args, ["find-generic-password", "-s", "vajra.provider.openrouter", "-a", "openrouter", "-w"]);
    // Only the user's own entry is called `openrouter`; this one never is.
    assert_eq!(t["model_providers"].as_table().unwrap().len(), 1);
}

#[test]
fn config_has_no_env_key_and_nothing_secret() {
    let text = render("openrouter", DEFAULT_BASE_URL).unwrap();
    for bad in ["env_key", "api_key", "bearer", "Bearer", "sk-", "token", "experimental_bearer_token"] {
        assert!(!text.contains(bad), "{bad}");
    }
}

#[test]
fn base_url_is_https_and_cannot_break_out_of_the_string() {
    for bad in [
        "http://openrouter.ai/api/v1",
        "https://",
        "https:///x",
        "https://a.example/\"\nenv_key = \"X",
        "https://a.example/x\\",
        "https://user:pw@a.example/",
        "https://a.example/ x",
        "https://a.example/\u{e9}",
        "",
    ] {
        assert_eq!(render("openrouter", bad), Err(Error::BadBaseUrl), "{bad:?}");
    }
    assert!(render("openrouter", "https://glm.example:8443/v1").is_ok());
}

#[test]
fn only_a_key_account_gets_a_home() {
    let d = tmp();
    assert_eq!(render("codex", DEFAULT_BASE_URL), Err(Error::UnknownAccount));
    assert_eq!(write(&d, "../x", DEFAULT_BASE_URL), Err(Error::UnknownAccount));
    assert_eq!(home_for(&d, "claude"), Err(Error::UnknownAccount));
    assert!(!d.join("codex-home").exists());
}

#[test]
fn write_makes_a_private_home_and_never_touches_the_users_codex_folder() {
    use std::os::unix::fs::PermissionsExt;
    let home = tmp();
    let theirs = home.join(".codex");
    std::fs::create_dir_all(&theirs).unwrap();
    std::fs::write(theirs.join("config.toml"), "[model_providers.openrouter]\nenv_key = \"THEIRS\"\n").unwrap();
    std::fs::write(theirs.join("auth.json"), "{\"fixture\":true}").unwrap();
    let before = (std::fs::read(theirs.join("config.toml")).unwrap(), std::fs::read(theirs.join("auth.json")).unwrap());
    let data = home.join("Library/Application Support/VajraDesktop");

    let h = write(&data, "openrouter", DEFAULT_BASE_URL).unwrap();
    write(&data, "openrouter", "https://other.example/v1").unwrap();

    assert_eq!(h, data.join("codex-home/openrouter"));
    assert_eq!(std::fs::metadata(&h).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::metadata(h.join("config.toml")).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(std::fs::read_to_string(h.join("config.toml")).unwrap().contains("other.example"), "rewritten at each launch");
    assert!(!h.starts_with(&theirs));
    let after = (std::fs::read(theirs.join("config.toml")).unwrap(), std::fs::read(theirs.join("auth.json")).unwrap());
    assert_eq!(before, after);
    let mut names: Vec<_> = std::fs::read_dir(&theirs).unwrap().map(|e| e.unwrap().file_name()).collect();
    names.sort();
    assert_eq!(names, ["auth.json", "config.toml"]);
}

/// A temp HOME with a user `.codex` folder and a data dir, plus a snapshot of the user's file (bytes and mode).
fn fixture() -> (PathBuf, PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let home = tmp();
    let theirs = home.join(".codex");
    std::fs::create_dir_all(&theirs).unwrap();
    let cfg = theirs.join("config.toml");
    std::fs::write(&cfg, "[model_providers.openrouter]\nenv_key = \"THEIRS\"\n").unwrap();
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o640)).unwrap();
    (home.clone(), theirs, home.join("data"))
}

fn snapshot(theirs: &Path) -> (Vec<u8>, u32, u32) {
    use std::os::unix::fs::PermissionsExt;
    let m = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    (std::fs::read(theirs.join("config.toml")).unwrap(), m(&theirs.join("config.toml")), m(theirs))
}

#[test]
fn a_linked_config_file_is_replaced_not_followed() {
    let (_home, theirs, data) = fixture();
    let before = snapshot(&theirs);
    let h = write(&data, "openrouter", DEFAULT_BASE_URL).unwrap();
    std::fs::remove_file(h.join("config.toml")).unwrap();
    std::os::unix::fs::symlink(theirs.join("config.toml"), h.join("config.toml")).unwrap();

    write(&data, "openrouter", "https://other.example/v1").unwrap();

    assert_eq!(snapshot(&theirs), before, "the user's file was not touched");
    let mine = h.join("config.toml");
    assert!(!std::fs::symlink_metadata(&mine).unwrap().file_type().is_symlink(), "the link was replaced by a plain file");
    assert!(std::fs::read_to_string(&mine).unwrap().contains("other.example"));
    let leftovers: Vec<_> = std::fs::read_dir(&h).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(leftovers, ["config.toml"], "no temp file is left behind");
}

#[test]
fn a_linked_account_dir_is_refused() {
    let (_home, theirs, data) = fixture();
    let before = snapshot(&theirs);
    std::fs::create_dir_all(data.join("codex-home")).unwrap();
    std::os::unix::fs::symlink(&theirs, data.join("codex-home/openrouter")).unwrap();

    assert!(matches!(write(&data, "openrouter", DEFAULT_BASE_URL), Err(Error::Io(_))));

    assert_eq!(snapshot(&theirs), before);
    assert_eq!(std::fs::read_dir(&theirs).unwrap().count(), 1, "nothing was created in their folder");
}

#[test]
fn a_linked_codex_home_dir_is_refused() {
    let (_home, theirs, data) = fixture();
    let before = snapshot(&theirs);
    std::fs::create_dir_all(&data).unwrap();
    std::os::unix::fs::symlink(&theirs, data.join("codex-home")).unwrap();

    assert!(matches!(write(&data, "openrouter", DEFAULT_BASE_URL), Err(Error::Io(_))));

    assert_eq!(snapshot(&theirs), before);
    assert_eq!(std::fs::read_dir(&theirs).unwrap().count(), 1, "nothing was created in their folder");
}

#[test]
fn a_linked_data_dir_is_refused() {
    let (home, theirs, _data) = fixture();
    let before = snapshot(&theirs);
    let data = home.join("linked-data");
    std::os::unix::fs::symlink(&theirs, &data).unwrap();

    assert!(matches!(write(&data, "openrouter", DEFAULT_BASE_URL), Err(Error::Io(_))));
    assert_eq!(snapshot(&theirs), before);
    assert_eq!(std::fs::read_dir(&theirs).unwrap().count(), 1);
}
