//! The packaged frontend must be buildable from a clean checkout: `ui/dist` is not tracked, so tauri.conf.json has to
//! build it itself (a review finding: a local build passed only because dist was prebuilt).

use std::path::Path;

fn conf() -> serde_json::Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

#[test]
fn a_clean_checkout_builds_its_own_frontend() {
    let c = conf();
    let build = &c["build"];
    assert_eq!(build["frontendDist"], "../ui/dist");
    let cmd = build["beforeBuildCommand"].as_str().expect("beforeBuildCommand builds the UI");
    // Independent of Tauri's working directory, installs from the lockfile, then builds.
    assert!(cmd.contains("git rev-parse --show-toplevel") && cmd.contains("app/ui"), "{cmd}");
    assert!(cmd.contains("npm ci") && cmd.contains("npm run build"), "{cmd}");
    let ignore = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/.gitignore")).unwrap();
    assert!(ignore.lines().any(|l| l.trim_end_matches('/') == "dist"), "dist is untracked, which is why the hook is needed");
}
