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

type Names = std::collections::BTreeSet<String>;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The double-quoted strings in `text` after `start`, up to the first `end`.
fn quoted(text: &str, start: &str, end: &str) -> Names {
    let from = text.find(start).unwrap_or_else(|| panic!("{start} not found")) + start.len();
    let body = &text[from..from + text[from..].find(end).unwrap()];
    body.split('"').skip(1).step_by(2).map(str::to_string).collect()
}

/// Every command `main.rs` registers in `generate_handler!`.
fn handler_commands() -> Names {
    let main = std::fs::read_to_string(root().join("src/main.rs")).unwrap();
    let from = main.find("generate_handler![").unwrap() + "generate_handler![".len();
    let body = &main[from..from + main[from..].find(']').unwrap()];
    body.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// What each capability file grants: its permissions, with an app permission set expanded to its members.
fn capabilities() -> Vec<(std::path::PathBuf, serde_json::Value, Names)> {
    let mut sets = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(root().join("permissions")).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "toml") {
            let t = std::fs::read_to_string(&p).unwrap();
            sets.insert(quoted(&t, "identifier", "\n").into_iter().next().unwrap(), quoted(&t, "permissions", "]"));
        }
    }
    std::fs::read_dir(root().join("capabilities"))
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            let cap: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            let mut granted = Names::new();
            for v in cap["permissions"].as_array().unwrap().iter().filter_map(|v| v.as_str()) {
                granted.extend(sets.get(v).cloned().unwrap_or_else(|| [v.to_string()].into()));
            }
            (p, cap, granted)
        })
        .collect()
}

fn allow(command: &str) -> String {
    format!("allow-{}", command.replace('_', "-"))
}

/// With an app manifest Tauri checks the ACL for every app command, so a command missing from the manifest or from the
/// main window's grant is refused at runtime (#154 review). Every registered command must be in both.
#[test]
fn every_registered_command_is_in_the_app_manifest_and_granted_to_the_main_window_only() {
    let handlers = handler_commands();
    assert!(handlers.len() > 80, "parsed {} commands", handlers.len());
    let build = std::fs::read_to_string(root().join("build.rs")).unwrap();
    assert_eq!(quoted(&build, "const COMMANDS: &[&str] = &[", "];"), handlers, "build.rs COMMANDS must equal generate_handler!");
    let mut granted = Names::new();
    for (p, cap, perms) in capabilities() {
        let app: Names = perms.iter().filter(|x| x.starts_with("allow-")).cloned().collect();
        if !app.is_empty() {
            assert_eq!(cap["windows"], serde_json::json!(["main"]), "{} grants app commands beyond the main window", p.display());
            assert!(cap.get("webviews").is_none() && cap.get("remote").is_none(), "{} grants app commands to a webview or a remote URL", p.display());
        }
        granted.extend(app);
    }
    let want: Names = handlers.iter().map(|c| allow(c)).collect();
    assert_eq!(granted, want, "the main window is granted exactly the registered commands");
}

#[test]
fn the_install_commands_are_acl_guarded_and_only_the_main_window_may_call_them() {
    // Listed in the app manifest, Tauri refuses them to any window no capability grants them to.
    let build = std::fs::read_to_string(root().join("build.rs")).unwrap();
    assert!(build.contains("AppManifest::new().commands("), "{build}");
    let mut granted = Names::new();
    for (p, cap, perms) in capabilities() {
        for c in SETUP_COMMANDS {
            assert!(build.contains(&format!("\"{c}\"")), "{c} is not in the app manifest");
            if perms.contains(&allow(c)) {
                assert_eq!(cap["windows"], serde_json::json!(["main"]), "{} grants {c} beyond the main window", p.display());
                assert!(cap.get("webviews").is_none() && cap.get("remote").is_none(), "{} grants {c} to a webview or a remote URL", p.display());
                granted.insert(c.to_string());
            }
        }
    }
    assert_eq!(granted.len(), SETUP_COMMANDS.len(), "the main window can call each: {granted:?}");
    let csp = conf()["app"]["security"]["csp"].as_str().map(str::to_string).unwrap_or_default();
    assert!(csp.starts_with("default-src 'self'") && !csp.contains("unsafe-eval"), "the CSP stays: {csp}");
}

/// The folder picker (D5) is the only plugin permission: the system open dialog, for the main window only. Nothing
/// broader such as `dialog:default` (which adds save, message, ask and confirm) and no fs or shell plugin grant.
#[test]
fn the_only_plugin_permission_is_the_open_dialog_for_the_main_window() {
    let mut plugin = Names::new();
    for (p, cap, perms) in capabilities() {
        for x in perms.iter().filter(|x| x.contains(':') && !x.starts_with("core:")) {
            assert_eq!(cap["windows"], serde_json::json!(["main"]), "{} grants {x} beyond the main window", p.display());
            assert!(cap.get("webviews").is_none() && cap.get("remote").is_none(), "{} grants {x} to a webview or a remote URL", p.display());
            plugin.insert(x.clone());
        }
    }
    assert_eq!(plugin, Names::from(["dialog:allow-open".to_string()]), "plugin permissions");
    let main = std::fs::read_to_string(root().join("src/main.rs")).unwrap();
    assert_eq!(main.matches(".plugin(").count(), 1, "one plugin is registered");
    assert!(main.contains(".plugin(tauri_plugin_dialog::init())"), "the dialog plugin is the one registered");
}

/// Plan #184 L3: no capability grants a file or shell permission, and the dialog stays open-only, so a later edit cannot
/// widen the grant unseen. The fs plugin is compiled in (the dialog plugin depends on it) but never registered or
/// granted, and the shell plugin is not a dependency at all. Every grant is checked in every form Tauri reads: string
/// entries and object entries (which can carry a scope) in each capability file, and capabilities inline in the config.
#[test]
fn no_capability_grants_a_file_or_shell_permission_and_the_dialog_is_open_only() {
    let security = &conf()["app"]["security"];
    assert!(security.get("capabilities").is_none(), "no capability is defined inline in tauri.conf.json: {security}");
    let file_or_shell = |id: &str| {
        let plugin = id.split(':').next().unwrap_or(id);
        plugin == "fs" || plugin == "shell" || plugin.starts_with("fs-") || plugin.starts_with("shell-")
    };
    let mut dialog = Names::new();
    for (p, cap, perms) in capabilities() {
        for v in cap["permissions"].as_array().unwrap() {
            // An object entry (`{"identifier": .., "allow": [..]}`) carries a scope; the folder picker needs none.
            assert!(v.is_string(), "{} has a non-string permission entry: {v}", p.display());
        }
        for x in &perms {
            assert!(!file_or_shell(x), "{} grants a file or shell permission: {x}", p.display());
            if x.starts_with("dialog:") {
                dialog.insert(x.clone());
            }
        }
    }
    assert_eq!(dialog, Names::from(["dialog:allow-open".to_string()]), "the dialog is open-only (no save, message, ask or confirm)");
    let main = std::fs::read_to_string(root().join("src/main.rs")).unwrap();
    assert!(!main.contains("tauri_plugin_fs") && !main.contains("tauri_plugin_shell"), "neither the fs nor the shell plugin is registered");
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).unwrap();
    assert!(!manifest.contains("tauri-plugin-fs") && !manifest.contains("tauri-plugin-shell"), "neither is a direct dependency");
    // The one call the UI makes: a single directory, opened (never saved over).
    let ipc = std::fs::read_to_string(root().join("../ui/src/ipc.ts")).unwrap();
    let calls: Vec<&str> = ipc.match_indices("\"plugin:").map(|(i, _)| &ipc[i + 1..i + 1 + ipc[i + 1..].find('"').unwrap()]).collect();
    assert_eq!(calls, ["plugin:dialog|open"], "the UI calls only the open dialog");
    assert!(ipc.contains("directory: true, multiple: false"), "it picks one folder");
}
