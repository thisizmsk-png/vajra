//! The resource-profile harness (`examples/profile_run.rs`) must still be able to start a run on the real core. It is
//! not part of the app, so a change to what `launch` requires can break it without any other test noticing: it once
//! launched in a folder that was not a project, was refused, and every load scenario of the bench stopped working.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Build the harness in this test's own profile and return the executable cargo reports for it. A filtered run
/// (`cargo test --test profile_harness`) does not build examples, so the test builds it. The path comes from cargo's
/// own build messages, not from this test binary's location: cargo 1.100 moved test binaries into a per-unit build
/// folder, so "two folders up from this test" is no longer the profile folder.
fn harness() -> PathBuf {
    let mut build = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    build.args(["build", "--example", "profile_run", "--message-format=json"]).current_dir(env!("CARGO_MANIFEST_DIR"));
    if !cfg!(debug_assertions) {
        build.arg("--release");
    }
    let out = build.output().unwrap();
    assert!(out.status.success(), "the harness does not build: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact" && m["target"]["name"] == "profile_run")
        .find_map(|m| m["executable"].as_str().map(PathBuf::from))
        .expect("cargo reports the profile_run executable")
}

#[test]
fn the_profile_harness_starts_a_run() {
    let bin = harness();
    assert!(bin.is_file(), "{} is missing", bin.display());
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let agent = core.join("../bench/profile/fake_agent.py");
    let fixture = core.join("tests/fixtures/transcripts/session/subagents");
    let home = std::env::temp_dir().join(format!("vajra-profile-harness-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // The home is canonical, as the bench's own temp home is: a project root is stored by its real path.
    let home = home.canonicalize().unwrap();

    let out = Command::new(&bin)
        .args([home.as_os_str(), home.join("data").as_os_str(), "run1".as_ref(), agent.as_os_str(), fixture.as_os_str(), "2".as_ref()])
        .env("HOME", &home)
        .env("VAJRA_DESKTOP_DATA", home.join("data"))
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&home);

    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "the harness failed: {stderr}");
    // The sampler reads exactly these lines: `PID n`, then `RUNS [pid]`, and `STATS {json}` at the end.
    let mut lines = stdout.lines();
    assert!(lines.next().is_some_and(|l| l.strip_prefix("PID ").is_some_and(|n| n.parse::<u32>().is_ok())), "{stdout}");
    let runs: Vec<u32> = serde_json::from_str(lines.next().and_then(|l| l.strip_prefix("RUNS ")).expect("a RUNS line")).unwrap();
    assert_eq!(runs.len(), 1, "run1 starts exactly one run session: {stdout}");
    assert!(stdout.lines().any(|l| l.starts_with("STATS {")), "{stdout}");
}
