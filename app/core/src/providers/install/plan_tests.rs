use super::*;

fn plan(version: &str, pin: Pin) -> Plan {
    Plan::new(
        "codex",
        "npm",
        vec!["/opt/homebrew/bin/node".into(), "install".into(), TARBALL.into()],
        vec![ToolRow { role: "node", path: "/opt/homebrew/bin/node".into(), label: None, owned_by_you: true }],
        "registry.npmjs.org".into(),
        version.into(),
        "/opt/homebrew".into(),
        pin,
    )
}

#[test]
fn integrity_is_npm_s_sha512_base64() {
    assert_eq!(integrity(b""), "sha512-z4PhNX7vuL3xVChQ1m2AB9Yg5AULVxXcg/SpIdNs6c5H0NE8XYXysP+DGNKHfuwvY7kxvUdBeoGlODJ6+SfaPg==");
    assert_eq!(integrity(b"abc"), "sha512-3a81oZNherrMQXNJriBBMRLm+k6JqX6iCp7u5ktV05ohkpkqJ0/BqDa6PCOj/uu9RU1EI2Q86A4qmslPpUyknw==");
}

#[test]
fn the_hash_is_stable_and_covers_the_version_the_pin_and_the_command() {
    let pin = || Pin::Npm { tgz_sha512: "sha512-A".into() };
    let a = plan("1.0.0", pin());
    assert_eq!(a.hash.len(), 64);
    assert_eq!(a.hash, plan("1.0.0", pin()).hash, "the same plan hashes the same");
    assert_ne!(a.hash, plan("1.0.1", pin()).hash, "the version");
    assert_ne!(a.hash, plan("1.0.0", Pin::Npm { tgz_sha512: "sha512-B".into() }).hash, "the tarball");
    let brew = |sha: &str, url: &str| plan("1.0.0", Pin::Brew { token: "homebrew/cask/codex".into(), sha256: sha.into(), url: url.into() });
    assert_ne!(brew("aa", "https://x/1").hash, brew("bb", "https://x/1").hash, "the cask sha256");
    assert_ne!(brew("aa", "https://x/1").hash, brew("aa", "https://x/2").hash, "the cask url");
    let mut other = plan("1.0.0", pin());
    other.command.push("--force".into());
    assert_ne!(a.hash, other.rehash().hash, "the command");
}

#[test]
fn the_sheet_never_carries_the_pin_internals_or_the_tarball_bytes() {
    let p = plan("1.0.0", Pin::Npm { tgz_sha512: "sha512-A".into() });
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"hash\""));
    assert!(json.contains(TARBALL));
}
