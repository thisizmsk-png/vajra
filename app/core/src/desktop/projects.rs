//! Project commands (ADR-15 section 1) for the app window. These are Tauri commands only: nothing here is an MCP tool, so
//! an agent cannot add, rename or archive a project; only the person at the window can. Each answer is plain JSON; a
//! refusal is `{error, detail}` with extra fields where the UI needs them (`nested` names the project in the way,
//! `conflict` the current revision).

use serde_json::{Value, json};

use super::{App, Res, fail, invalid, runindex};
use crate::projects::migrate::{self, Launch, MigrateError};
use crate::projects::contract::{self, ContractError, Project};
use crate::projects::identity::IdentityError;
use crate::projects::store::{Added, StoreError};

/// The largest launch index the migration will read (it streams, so this bounds time, not memory).
const MAX_INDEX_BYTES: u64 = 256 * 1024 * 1024;

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
        StoreError::Full => fail("store_full", "there are too many projects (or stray entries) to add another"),
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
        StoreError::Unavailable(u) => json!({"error": "projects_unavailable", "detail": u.what(), "fix": u.fix(), "path": u.path}),
    }
}

impl App {
    /// The preflight row for a refused projects folder (#77 follow-up), or `None` when the store is usable.
    pub fn projects_row(&self) -> Option<Value> {
        let u = self.projects.unavailable()?;
        Some(json!({"name": "Projects folder", "ok": false, "detail": format!("{} ({})", u.what(), u.path.display()), "fix": u.fix()}))
    }

    /// A refused projects folder refuses the command (`projects_unavailable`). Needed where the store's own answer would
    /// mislead: with no folder, the migration marker reads as written though it never was (#106 review).
    pub(super) fn projects_usable(&self) -> Result<(), Value> {
        match self.projects.unavailable() {
            Some(u) => Err(refusal(StoreError::Unavailable(u.clone()))),
            None => Ok(()),
        }
    }

    pub fn app_info(&self) -> Res {
        Ok(info())
    }

    pub fn project_list(&self, include_archived: bool) -> Res {
        let l = self.projects.list(include_archived).map_err(refusal)?;
        Ok(json!({"projects": l.projects.iter().map(project_json).collect::<Vec<_>>(), "skipped": l.skipped, "truncated": l.truncated}))
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

    /// What the owner is asked to confirm once: suggested projects for campaigns started before projects existed.
    /// Nothing is written. `done` is true once the owner has confirmed.
    pub fn project_migration_suggest(&self) -> Res {
        self.projects_usable()?;
        if migrate::is_done(&self.projects) {
            return Ok(json!({"done": true, "groups": [], "unsorted": [], "terminal": []}));
        }
        let (launches, campaigns) = self.migration_inputs()?;
        let plan = migrate::plan(&launches, &campaigns, self.projects.guard());
        let mut v = serde_json::to_value(plan).unwrap_or_else(|_| json!({}));
        v["done"] = json!(false);
        Ok(v)
    }

    /// The owner accepted these suggested roots (each must be in a fresh suggestion). Runs once.
    pub fn project_migration_confirm(&self, roots: &[String]) -> Res {
        self.projects_usable()?;
        let (launches, campaigns) = self.migration_inputs()?;
        match migrate::confirm(&self.projects, &self.data, self.projects.guard(), &launches, &campaigns, roots) {
            Ok(o) => Ok(serde_json::to_value(o).unwrap_or_else(|_| json!({}))),
            Err(MigrateError::AlreadyDone) => Err(fail("already_migrated", "campaigns were already sorted into projects")),
            Err(MigrateError::Store(e)) => Err(refusal(e)),
            Err(MigrateError::Io(e)) => Err(fail("io", e)),
        }
    }

    fn migration_inputs(&self) -> Result<(Vec<Launch>, Vec<String>), Value> {
        // The WHOLE index, not its recent tail: the migration is once-only, so it must not see a partial history. An
        // index that cannot be read completely is refused, and nothing is marked done.
        let launches = runindex::entries_all(&self.data.join("runs.jsonl"), MAX_INDEX_BYTES)
            .map_err(|e| fail("launch_index_unreadable", e))?
            .into_iter()
            .map(|e| Launch { campaign: e.campaign, run_id: e.run_id, session_id: e.session_id, cwd: e.cwd })
            .collect();
        let campaigns = self.with_reader(|r| r.campaigns().map_err(|e| fail("io_error", e)))?;
        Ok((launches, campaigns))
    }

    pub fn project_sessions(&self, id: &str) -> Res {
        let s = self.projects.sessions(id).map_err(refusal)?;
        Ok(json!({
            "sessions": serde_json::to_value(s.sessions).unwrap_or_else(|_| json!([])),
            "skipped": s.skipped,
            "unsupported": s.unsupported,
            "truncated": s.truncated,
        }))
    }
}

#[cfg(test)]
#[path = "projects_tests.rs"]
mod tests;
