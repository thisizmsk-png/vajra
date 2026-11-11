use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use super::*;

fn getter(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let m: HashMap<String, OsString> = vars.iter().map(|(k, v)| (k.to_string(), OsString::from(v))).collect();
    move |k| m.get(k).cloned()
}

fn names(e: &SetupEnv) -> Vec<&'static str> {
    e.vars().iter().map(|(k, _)| *k).collect()
}

#[test]
fn the_setup_env_is_the_allowed_names_the_tool_path_and_only_the_proxy_and_ca_variables() {
    let get = getter(&[
        ("HOME", "/Users/sam"),
        ("USER", "sam"),
        ("LANG", "en_US.UTF-8"),
        ("PATH", "/Users/sam/.local/bin:/usr/bin"),
        ("HTTPS_PROXY", "http://proxy.example:3128"),
        ("NO_PROXY", "localhost"),
        ("NODE_EXTRA_CA_CERTS", "/etc/ca.pem"),
        ("HTTP_PROXY", "http://other:1"),
        ("ANTHROPIC_API_KEY", "sentinel"),
        ("OPENAI_API_KEY", "sentinel"),
        ("GITHUB_TOKEN", "sentinel"),
        ("CODEX_HOME", "/x"),
        ("NPM_CONFIG_REGISTRY", "https://evil.example"),
    ]);
    let e = setup_env(&[PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/opt/homebrew/bin")], get);
    let mut n = names(&e);
    n.sort_unstable();
    assert_eq!(n, ["HOME", "HTTPS_PROXY", "LANG", "NODE_EXTRA_CA_CERTS", "NO_PROXY", "PATH", "TERM", "USER"]);
    let path = e.vars().iter().find(|(k, _)| *k == "PATH").unwrap().1.clone();
    assert_eq!(path, OsString::from("/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin"), "tool dirs first, then the base PATH; never the user's PATH");
}

#[test]
fn empty_or_nul_values_and_relative_tool_dirs_are_dropped() {
    let e = setup_env(&[PathBuf::from("rel/bin")], getter(&[("HTTPS_PROXY", ""), ("HOME", "/Users/sam")]));
    assert!(!names(&e).contains(&"HTTPS_PROXY"));
    let path = e.vars().iter().find(|(k, _)| *k == "PATH").unwrap().1.clone();
    assert!(!path.to_string_lossy().contains("rel/bin"));
}

#[test]
fn a_helper_run_sees_exactly_the_setup_env_and_nothing_inherited() {
    // The test process has CARGO_* variables (and whatever else); none may reach a setup helper.
    let e = setup_env(&[], |k| std::env::var_os(k));
    let out = crate::vajra::bridge::run_process_exact(std::path::Path::new("/usr/bin/env"), &[], std::path::Path::new("/"), std::time::Duration::from_secs(5), e.vars()).unwrap();
    let seen: Vec<&str> = out.stdout.lines().filter_map(|l| l.split_once('=').map(|(k, _)| k)).collect();
    assert!(!seen.is_empty());
    for k in &seen {
        assert!(ALLOWED.contains(k), "{k} leaked into the setup env: {seen:?}");
    }
}

#[test]
fn proxy_credentials_are_redacted_from_the_setup_stream() {
    let e = setup_env(&[], getter(&[("HTTPS_PROXY", "http://alice:s3cret@proxy.example:3128"), ("NO_PROXY", "localhost")]));
    let r = redactor(&e);
    let line = r.clean("npm ERR! connect via http://alice:s3cret@proxy.example:3128 failed (alice:s3cret@proxy.example)");
    assert!(!line.contains("s3cret"), "{line}");
    assert!(line.contains("[redacted]"), "{line}");
    assert_eq!(r.clean("npm notice localhost"), "npm notice localhost", "a proxy without credentials redacts nothing");
}

#[test]
fn a_short_proxy_password_is_still_redacted() {
    let e = setup_env(&[], getter(&[("HTTPS_PROXY", "http://a:b@p.ex:1")]));
    let line = redactor(&e).clean("via http://a:b@p.ex:1 and a:b@p.ex");
    assert!(!line.contains("a:b@"), "{line}");
}
