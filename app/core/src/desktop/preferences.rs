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
    pub fn act_preferences_update(
        &self,
        envelope: &super::Envelope,
        patch: Patch,
        reset: bool,
    ) -> serde_json::Value {
        use crate::control::{receipt::Caller, sha256};
        // Hash the bounded typed payload into the receipt target: reusing an id for a different intent is refused.
        let intent =
            serde_json::to_vec(&(envelope.expect_generation, &patch, reset)).expect("plain fields");
        let target = format!("preferences:{}", sha256::hex_digest(&intent));
        // Store::update checks the revision under its exclusive file lock. It supplies revision_conflict rather
        // than the generic target-generation error; the receipt engine still owns dedupe and human-only access.
        self.act(
            envelope,
            Caller::Ui,
            true,
            "preferences_update",
            &target,
            None,
            || {
                let revision = envelope
                    .expect_generation
                    .ok_or_else(|| serde_json::json!({"error":"revision_required"}))?;
                let project = if envelope.project_id == "global" {
                    None
                } else {
                    Some(envelope.project_id.as_str())
                };
                lock(&self.preferences)
                    .update(project, revision, patch, reset)
                    .map(|v| v.json())
                    .map_err(error)
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::Envelope;
    use serde_json::json;
    #[test]
    fn receipts_guard_preferences_replay_conflicts_and_scope() {
        let temp =
            std::env::temp_dir().join(format!("vajra-pref-act-{}", crate::ids::uuid_v4().unwrap()));
        std::fs::create_dir_all(temp.join("home/.claude/vajra/campaigns")).unwrap();
        let app = App::open(&temp.join("data"), &temp.join("home")).unwrap();
        let env = Envelope {
            action_id: "prefs-save-0001".into(),
            project_id: "global".into(),
            expect_generation: Some(0),
        };
        let patch = || {
            serde_json::from_value(json!({"notifications":true,"notifications_text":false}))
                .unwrap()
        };
        let first = app.act_preferences_update(&env, patch(), false);
        assert_eq!(first["receipt"]["state"], "done");
        let replay = app.act_preferences_update(&env, patch(), false);
        assert_eq!(replay["replayed"], true);
        let changed = app.act_preferences_update(&env, Patch::default(), true);
        assert_eq!(changed["receipt"]["reason"], "action_id_reuse");
        assert_eq!(app.preferences_read(None).unwrap()["rev"], 1);
        let stale = Envelope {
            action_id: "prefs-save-0002".into(),
            ..env.clone()
        };
        let rejected = app.act_preferences_update(&stale, patch(), false);
        assert_eq!(rejected["receipt"]["state"], "rejected");
        assert_eq!(rejected["receipt"]["reason"], "revision_conflict");
        let reset = Envelope {
            action_id: "prefs-reset-0001".into(),
            expect_generation: Some(1),
            ..env
        };
        assert_eq!(
            app.act_preferences_update(&reset, Patch::default(), true)["receipt"]["state"],
            "done"
        );
        let v = app.preferences_read(None).unwrap();
        assert_eq!(v["effective"]["notifications"], false);
        assert_eq!(v["effective"]["notifications_text"], false);
        std::fs::remove_dir_all(temp).unwrap();
    }
}
