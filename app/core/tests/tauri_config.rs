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

/// The one-click install commands (ADR-17, bhishma condition 7).
const SETUP_COMMANDS: [&str; 3] = ["setup_plan", "setup_consent", "setup_run"];

#[test]
fn the_install_commands_are_acl_guarded_and_only_the_main_window_may_call_them() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // Listed in the app manifest, Tauri refuses them to any window no capability grants them to.
    let build = std::fs::read_to_string(root.join("build.rs")).unwrap();
    assert!(build.contains("AppManifest::new().commands("), "{build}");
    let mut granted = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(root.join("capabilities")).unwrap() {
        let p = entry.unwrap().path();
        let cap: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        let perms: Vec<String> = cap["permissions"].as_array().unwrap().iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
        for c in SETUP_COMMANDS {
            assert!(build.contains(&format!("\"{c}\"")), "{c} is not in the app manifest");
            if perms.contains(&format!("allow-{}", c.replace('_', "-"))) {
                assert_eq!(cap["windows"], serde_json::json!(["main"]), "{} grants {c} beyond the main window", p.display());
                assert!(cap.get("webviews").is_none() && cap.get("remote").is_none(), "{} grants {c} to a webview or a remote URL", p.display());
                granted.insert(c);
            }
        }
    }
    assert_eq!(granted.len(), SETUP_COMMANDS.len(), "the main window can call each: {granted:?}");
    let csp = conf()["app"]["security"]["csp"].as_str().map(str::to_string).unwrap_or_default();
    assert!(csp.starts_with("default-src 'self'") && !csp.contains("unsafe-eval"), "the CSP stays: {csp}");
}
