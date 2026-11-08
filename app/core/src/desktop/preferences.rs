//! Preferences are separate from the Lead's generated security settings. No policy or running launch is mutated.
use super::{App, Res, lock};
use crate::preferences::{Error, Patch};
fn error(e: Error) -> serde_json::Value {
    serde_json::json!({"error":e.code()})
}
impl App {
    pub fn preferences_read(&self, project_id: Option<&str>) -> Res {
        lock(&self.preferences)
            .read(project_id)
            .map(|v| v.json())
            .map_err(error)
    }
    pub fn preferences_update(
        &self,
        project_id: Option<&str>,
        expect_rev: u64,
        patch: Patch,
        reset: bool,
    ) -> Res {
        lock(&self.preferences)
            .update(project_id, expect_rev, patch, reset)
            .map(|v| v.json())
            .map_err(error)
    }
}
