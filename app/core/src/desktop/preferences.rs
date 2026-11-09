//! Preferences are separate from the Lead's generated security settings. No policy or running launch is mutated.
use super::{App, Res, lock};
use crate::preferences::{Error, Patch};
fn error(e: Error) -> serde_json::Value {
    match e.code() {
        // settings.json was replaced but the write could not be confirmed: the receipt says unknown, not refused.
        "outcome_unknown" => {
            serde_json::json!({"error":"outcome_unknown","detail":"The new preferences were written but could not be confirmed on disk. Reload and review them before saving again."})
        }
        code => serde_json::json!({"error":code}),
    }
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
        // A conflict tells the caller to reload and review, on the first answer and on a replay.
        assert_eq!(rejected["receipt"]["retry"], "after_refresh");
        let again = app.act_preferences_update(&stale, patch(), false);
        assert_eq!(again["replayed"], true);
        assert_eq!(
            (
                again["receipt"]["state"].as_str(),
                again["receipt"]["retry"].as_str()
            ),
            (Some("rejected"), Some("after_refresh"))
        );
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
        assert_eq!(v["effective"]["notifications"], true);
        assert_eq!(v["effective"]["notifications_text"], false);
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn a_failure_after_the_rename_is_an_unknown_outcome_not_a_refusal() {
        let temp = std::env::temp_dir().join(format!(
            "vajra-pref-sync-{}",
            crate::ids::uuid_v4().unwrap()
        ));
        std::fs::create_dir_all(temp.join("home/.claude/vajra/campaigns")).unwrap();
        let app = App::open(&temp.join("data"), &temp.join("home")).unwrap();
        let env = Envelope {
            action_id: "prefs-sync-0001".into(),
            project_id: "global".into(),
            expect_generation: Some(0),
        };
        let patch = || serde_json::from_value::<Patch>(json!({"theme":"dark"})).unwrap();
        crate::preferences::FAIL_SYNC_AFTER_RENAME.with(|f| f.set(true));
        let out = app.act_preferences_update(&env, patch(), false);
        crate::preferences::FAIL_SYNC_AFTER_RENAME.with(|f| f.set(false));
        // The document changed: the new value and revision are on disk.
        let v = app.preferences_read(None).unwrap();
        assert_eq!(v["rev"], 1);
        assert_eq!(v["effective"]["theme"], "dark");
        // And the receipt says so honestly: unknown, with review guidance, never rejected.
        assert_eq!(out["receipt"]["state"], "unknown", "{out}");
        assert_eq!(out["receipt"]["reason"], "outcome_unknown");
        assert!(
            out["receipt"]["detail"]
                .as_str()
                .unwrap_or("")
                .contains("Reload and review"),
            "{out}"
        );
        // A replay of the same action reports the same unknown outcome, not a refusal.
        let replay = app.act_preferences_update(&env, patch(), false);
        assert_eq!(replay["replayed"], true);
        assert_eq!(replay["receipt"]["state"], "unknown");
        std::fs::remove_dir_all(temp).unwrap();
    }
}
