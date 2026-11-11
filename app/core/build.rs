fn main() {
    // The one-click install commands (ADR-17, bhishma condition 7) are in the app manifest, so Tauri refuses them to any
    // window that no capability grants them to; only `capabilities/default.json` (the main window) does.
    let manifest = tauri_build::AppManifest::new().commands(&["setup_plan", "setup_consent", "setup_run"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest)).expect("tauri build");
}
