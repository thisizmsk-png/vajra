//! Project commands (ADR-15 section 1) for the app window. These are Tauri commands only: nothing here is an MCP tool, so
//! an agent cannot add, rename or archive a project; only the person at the window can. Each answer is plain JSON; a
//! refusal is `{error, detail}` with extra fields where the UI needs them (`nested` names the project in the way,
//! `conflict` the current revision).

use serde_json::{Value, json};

use super::{App, Res, fail, invalid};
use crate::projects::contract::{self, ContractError, Project};
use crate::projects::identity::IdentityError;
use crate::projects::store::{Added, StoreError};

/// `app_info` (`vajra.app_info/1`): which record versions this build reads and writes.
fn info() -> Value {
    serde_json::to_value(contract::app_info()).unwrap_or_else(|_| json!({}))
}

fn project_json(p: &Project) -> Value {
    serde_json::to_value(p).unwrap_or_else(|_| json!({}))
}

fn refusal(e: StoreError) -> Value {
    match e {
        StoreError::NotFound => fail("not_found", "project not found"),
        StoreError::Conflict { current_rev } => {
            json!({"error": "conflict", "detail": "the project changed; reload and try again", "current_rev": current_rev})
        }
        StoreError::Nested { with_id, with_name } => json!({
            "error": "nested",
            "detail": format!("this folder overlaps the project {with_name}"),
            "with": {"id": with_id, "name": with_name},
        }),
        StoreError::Identity(IdentityError::Refused(why)) => fail("refused_folder", why),
        StoreError::Identity(IdentityError::NotFound) => invalid("path", "folder not found"),
        StoreError::Identity(IdentityError::NotADirectory) => invalid("path", "not a folder"),
        StoreError::Identity(IdentityError::Io(e)) => fail("io", e),
        StoreError::Invalid(ContractError::Invalid(why)) => invalid("name", why),
        StoreError::Invalid(e) => fail("bad_record", e),
        StoreError::Io(e) => fail("io", e),
    }
}

impl App {
    pub fn app_info(&self) -> Res {
        Ok(info())
    }

    pub fn project_list(&self, include_archived: bool) -> Res {
        let l = self.projects.list(include_archived).map_err(refusal)?;
        Ok(json!({"projects": l.projects.iter().map(project_json).collect::<Vec<_>>(), "skipped": l.skipped}))
    }

    pub fn project_add(&self, path: &str, confirm_nested: bool) -> Res {
        let p = std::path::Path::new(path);
        if !p.is_absolute() {
            return Err(invalid("path", "must be an absolute folder path"));
        }
        let added: Added = self.projects.add(p, confirm_nested).map_err(refusal)?;
        Ok(json!({"outcome": added.outcome(), "project": project_json(added.project())}))
    }

    pub fn project_rename(&self, id: &str, name: &str, expect_rev: Option<u64>) -> Res {
        self.projects.rename(id, name, expect_rev).map(|p| json!({"project": project_json(&p)})).map_err(refusal)
    }

    pub fn project_archive(&self, id: &str, expect_rev: Option<u64>) -> Res {
        self.projects.archive(id, expect_rev).map(|p| json!({"project": project_json(&p)})).map_err(refusal)
    }

    pub fn project_sessions(&self, id: &str) -> Res {
        let s = self.projects.sessions(id).map_err(refusal)?;
        Ok(json!({
            "sessions": serde_json::to_value(s.sessions).unwrap_or_else(|_| json!([])),
            "skipped": s.skipped,
            "unsupported": s.unsupported,
        }))
    }
}

#[cfg(test)]
#[path = "projects_tests.rs"]
mod tests;
