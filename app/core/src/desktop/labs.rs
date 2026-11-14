//! Labs (ADR-16 16.13): a per-project switch, only in developer builds, behind which worker picking runs before it
//! is ready for everyone. With Labs on for a project, its Director-General is told about the seat fields and a
//! proposal with seats can start. A release build has no Labs at all: `LABS_BUILD` is false, nothing reads the file,
//! and `set` refuses, so worker picking can't be user-visible before 6g, 6h, 6i and 6j land.
//!
//! The switch is `<data>/labs.json`, `{"v": 1, "projects": [<project id>, ...]}`, written 0600 through
//! `fsx::write_atomic` and read without following a link. A missing or unreadable file is "Labs off everywhere".

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::control::fsx;

/// A developer build: one built without optimisations' release profile. A compile-time fact, never a setting.
pub const LABS_BUILD: bool = cfg!(debug_assertions);

const FILE: &str = "labs.json";
const MAX_BYTES: u64 = 64 * 1024;
/// More projects than anyone runs Labs on; a longer list is refused rather than grown without bound.
const MAX_PROJECTS: usize = 256;

pub struct Labs {
    data: PathBuf,
    build: bool,
}

impl Labs {
    pub fn new(data: &Path) -> Self {
        Self { data: data.to_path_buf(), build: LABS_BUILD }
    }

    /// Tests only: a release build's Labs.
    #[cfg(test)]
    pub fn release(data: &Path) -> Self {
        Self { data: data.to_path_buf(), build: false }
    }

    fn read(&self) -> Vec<String> {
        let Ok(b) = fsx::read_nofollow_capped(&self.data.join(FILE), MAX_BYTES) else { return Vec::new() };
        let Ok(v) = serde_json::from_slice::<Value>(&b) else { return Vec::new() };
        if v["v"] != 1 {
            return Vec::new();
        }
        v["projects"].as_array().map(|a| a.iter().filter_map(Value::as_str).filter(|p| crate::projects::contract::valid_project_id(p)).map(String::from).collect()).unwrap_or_default()
    }

    /// Is Labs on for `project`? Always false in a release build.
    pub fn on(&self, project: &str) -> bool {
        self.build && crate::projects::contract::valid_project_id(project) && self.read().iter().any(|p| p == project)
    }

    /// Turn Labs on or off for `project` (the human's switch). Refused in a release build.
    pub fn set(&self, project: &str, on: bool) -> Result<(), &'static str> {
        if !self.build {
            return Err("labs_unavailable");
        }
        if !crate::projects::contract::valid_project_id(project) {
            return Err("invalid_project");
        }
        let mut list = self.read();
        list.retain(|p| p != project);
        if on {
            if list.len() >= MAX_PROJECTS {
                return Err("too_many");
            }
            list.push(project.to_string());
        }
        list.sort();
        crate::paths::ensure_private_dir(&self.data).map_err(|_| "write_failed")?;
        let mut bytes = json!({"v": 1, "projects": list}).to_string();
        bytes.push('\n');
        fsx::write_atomic(&self.data, FILE, bytes.as_bytes()).map(|_| ()).map_err(|_| "write_failed")
    }
}

#[cfg(test)]
#[path = "labs_tests.rs"]
mod tests;
