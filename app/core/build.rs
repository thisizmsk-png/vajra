use std::path::Path;

// Modules whose files are owned by later tasks. Until a task adds
// src/<name>/mod.rs, lib.rs falls back to an empty inline module.
const MODULES: [&str; 5] = ["bus", "sessions", "vajra", "crew", "control"];

fn main() {
    println!("cargo::rerun-if-changed=src");
    for name in MODULES {
        let file = format!("src/{name}/mod.rs");
        println!("cargo::rerun-if-changed={file}");
        println!("cargo::rustc-check-cfg=cfg(has_mod_{name})");
        if Path::new(&file).exists() {
            println!("cargo::rustc-cfg=has_mod_{name}");
        }
    }
    tauri_build::build();
}
