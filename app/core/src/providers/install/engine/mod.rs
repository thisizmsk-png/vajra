//! One-click install of the Vajra engine scripts (ADR-17 item 10 step 2, ADR-16 N13, D7). The engine is a copy packed
//! into the app at build time (`build.rs`, `engine_pack.rs`); its hash is compiled in, so only that exact copy is ever
//! installed. Design: bhishma's conditions C1-C10 on the D7 design note.

#[path = "../../../../engine_pack.rs"]
pub mod pack;

pub mod bundle;

/// What this binary was built with: the bundled engine's sha256, version and whether every file matched its manifest
/// (`tree_ok`), how big it is unpacked, and whether the build was a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub sha256: String,
    pub version: String,
    pub tree_ok: bool,
    pub unpacked: u64,
    pub files: usize,
    pub release: bool,
}

impl Pin {
    /// The pin compiled into this binary.
    pub fn built() -> Self {
        Pin {
            sha256: env!("VAJRA_ENGINE_TGZ_SHA256").to_string(),
            version: env!("VAJRA_ENGINE_VERSION").to_string(),
            tree_ok: env!("VAJRA_ENGINE_TREE_OK") == "true",
            unpacked: env!("VAJRA_ENGINE_UNPACKED").parse().unwrap_or(0),
            files: env!("VAJRA_ENGINE_FILES").parse().unwrap_or(0),
            release: env!("VAJRA_ENGINE_BUILD") == "release",
        }
    }
}

#[cfg(test)]
#[path = "pack_tests.rs"]
mod pack_tests;
