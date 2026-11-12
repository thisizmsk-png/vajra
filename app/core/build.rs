// Every command `main.rs` registers in `generate_handler!`. With an app manifest Tauri checks the ACL for every app
// command, not only the listed ones, so each must be here and granted to the main window
// (`permissions/main-window-commands.toml`, named by `capabilities/default.json`). `tests/tauri_config.rs` keeps this
// list, the handler list and the grant equal, so a new command can't be refused at runtime.
const COMMANDS: &[&str] = &[
    "preferences_read",
    "act_preferences_update",
    "preflight",
    "list_campaigns",
    "campaign_integrity",
    "start_run",
    "resume_run",
    "provider_list",
    "provider_accounts",
    "setup_list",
    "setup_sign_in",
    "setup_plan",
    "setup_consent",
    "setup_run",
    "provider_key_saved",
    "provider_key_save",
    "provider_key_replace",
    "provider_key_remove",
    "provider_usage",
    "verify_campaign",
    "read_report",
    "resume_targets",
    "crew",
    "tail",
    "read_spill",
    "read_spill_before",
    "stream_window",
    "task",
    "diff",
    "evidence_list",
    "evidence_detail",
    "run_overview",
    "worktree_info",
    "control_message",
    "control_redirect",
    "control_stop",
    "confirm_stop",
    "cancel_stop",
    "resume_worker",
    "ledger",
    "kb_search",
    "take_over",
    "active_runs",
    "run_limits",
    "set_run_limits",
    "ask_lead",
    "cancel_ask",
    "lead_status",
    "new_conversation",
    "start_proposal",
    "dismiss_proposal",
    "inbox",
    "approve_plan",
    "reject_plan",
    "app_info",
    "project_list",
    "project_for_folder",
    "project_add",
    "project_rename",
    "project_archive",
    "project_sessions",
    "project_migration_suggest",
    "project_migration_confirm",
    "act_approve_plan",
    "act_reject_plan",
    "act_answer_question",
    "act_stop_worker",
    "act_worker_clean_up",
    "install_dev_tools",
    "worker_disk",
    "act_close_worker",
    "act_close_finished",
    "closeout_status",
    "act_start_run",
    "act_ask_lead",
    "act_new_conversation",
    "act_cancel_ask",
    "act_start_proposal",
    "act_dismiss_proposal",
    "project_state",
    "term_open",
    "term_attach",
    "term_list",
    "lifecycle",
    "recover_orphans",
    "diagnostics_preview",
    "term_write",
    "term_resize",
    "term_visible",
    "term_ack",
    "term_close",
];

#[path = "engine_pack.rs"]
mod engine_pack;

fn main() {
    pack_engine();
    // The one-click install commands (ADR-17, bhishma condition 7) and every other command are in the app manifest, so
    // Tauri refuses each to any window no capability grants it to; only the main window's capability does.
    let manifest = tauri_build::AppManifest::new().commands(COMMANDS);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest)).expect("tauri build");
}

/// D7 (ADR-17 item 10 step 2, bhishma C1): pack the engine this build was made from into `gen/engine/engine.tgz`, the
/// bundle resource, and compile its hash, version and state into the binary. The core checks the resource against
/// these before it installs anything. A release (`VAJRA_RELEASE=1`) refuses an engine whose files don't match its
/// manifest, that git shows changed, or whose version is outside the tested range.
fn pack_engine() {
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let repo = dir.join("../..");
    println!("cargo:rerun-if-env-changed=VAJRA_RELEASE");
    println!("cargo:rerun-if-changed={}", repo.join(engine_pack::MANIFEST).display());
    let p = engine_pack::pack(&repo).unwrap_or_else(|e| panic!("engine pack: {e}"));
    let listed = engine_pack::manifest_files(&std::fs::read(repo.join(engine_pack::MANIFEST)).expect("read above")).expect("checked above");
    for name in listed.keys() {
        println!("cargo:rerun-if-changed={}", repo.join(name).display());
    }
    let release = std::env::var("VAJRA_RELEASE").is_ok_and(|v| v == "1");
    if release {
        assert!(p.tree_ok, "engine pack: a release needs every engine file to match manifest.json; these differ: {:?}", p.mismatched);
        let package = std::fs::read(repo.join("package.json")).expect("package.json");
        assert!(engine_pack::engine_version(&package).is_some_and(|(_, ok)| ok), "engine pack: a release needs the engine version {} inside the tested range", p.version);
        let clean = std::process::Command::new("git").arg("-C").arg(&repo).args(["diff", "--quiet", "HEAD", "--"]).args(listed.keys()).arg(engine_pack::MANIFEST).status();
        assert!(clean.is_ok_and(|s| s.success()), "engine pack: a release needs the packed engine files committed (git diff shows changes)");
    } else if !p.tree_ok {
        println!("cargo:warning=engine pack: {} file(s) differ from manifest.json, so one-click engine install is off in this build (engine_not_pinned)", p.mismatched.len());
    }
    let out = dir.join("gen/engine");
    std::fs::create_dir_all(&out).expect("gen/engine");
    // Rewrite only when the bytes change, so an unchanged pack doesn't re-trigger the bundler.
    let file = out.join("engine.tgz");
    if std::fs::read(&file).ok().as_deref() != Some(p.tgz.as_slice()) {
        std::fs::write(&file, &p.tgz).expect("write gen/engine/engine.tgz");
    }
    println!("cargo:rustc-env=VAJRA_ENGINE_TGZ_SHA256={}", p.sha256);
    println!("cargo:rustc-env=VAJRA_ENGINE_VERSION={}", p.version);
    println!("cargo:rustc-env=VAJRA_ENGINE_TREE_OK={}", p.tree_ok);
    println!("cargo:rustc-env=VAJRA_ENGINE_UNPACKED={}", p.unpacked);
    println!("cargo:rustc-env=VAJRA_ENGINE_FILES={}", p.files);
    println!("cargo:rustc-env=VAJRA_ENGINE_BUILD={}", if release { "release" } else { "development" });
}
