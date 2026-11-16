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

    /// The per-project agent settings Settings > Providers edits (`vajra.project_agents/1`): the project's default, a
    /// choice per crew role and the allowed set, from the same preferences store the Director-General reads. A project
    /// with no override reads as the global agents. The project must exist; an id that is not one is refused.
    pub fn provider_agents(&self, project_id: &str) -> Res {
        self.agents_scope(project_id)?;
        let view = lock(&self.preferences).read(Some(project_id)).map_err(error)?;
        Ok(agents_json(&view.agents))
    }

    /// Saves them for that project only. The UI's three fields replace the stored ones; the lead seat and the per-account
    /// caps (not editable here) stay as they were. The same validation as the preferences write: known role keys, valid
    /// account and model ids, and nothing chosen outside the allowed set.
    pub fn provider_agents_save(&self, project_id: &str, agents: serde_json::Value) -> Res {
        self.agents_scope(project_id)?;
        let sent: AgentsIn = serde_json::from_value(agents).map_err(|_| super::invalid("agents", "not a valid agents object"))?;
        let store = lock(&self.preferences);
        let view = store.read(Some(project_id)).map_err(error)?;
        let next = crate::preferences::Agents { default: sent.default, roles: sent.roles, allowed: sent.allowed, lead: view.agents.lead, account_caps: view.agents.account_caps };
        let patch = Patch { agents: Some(next), ..Patch::default() };
        let saved = store.update(Some(project_id), view.rev, patch, false).map_err(error)?;
        Ok(agents_json(&saved.agents))
    }

    fn agents_scope(&self, project_id: &str) -> Result<(), serde_json::Value> {
        if !crate::projects::contract::valid_project_id(project_id) {
            return Err(super::invalid("project_id", "not a project id"));
        }
        self.projects_usable()?;
        self.projects.get(project_id).map(|_| ()).map_err(|_| super::fail("not_found", "project not found"))
    }
}

/// What the UI may send: no `lead` and no `account_caps`, so a save from this screen cannot change them.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentsIn {
    default: crate::preferences::Choice,
    #[serde(default)]
    roles: std::collections::BTreeMap<String, crate::preferences::Choice>,
    allowed: Vec<crate::preferences::Allowed>,
}

fn agents_json(a: &crate::preferences::Agents) -> serde_json::Value {
    serde_json::json!({"schema": "vajra.project_agents/1", "default": a.default, "roles": a.roles, "allowed": a.allowed})
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

    #[test]
    fn provider_agents_read_and_save_one_projects_agents_with_the_preferences_validation() {
        let temp = std::fs::canonicalize(std::env::temp_dir()).unwrap().join(format!("vajra-pref-agents-{}", crate::ids::uuid_v4().unwrap()));
        let repo = temp.join("home/src/alpha");
        std::fs::create_dir_all(repo.join(".git/objects")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir_all(temp.join("home/.claude/vajra/campaigns")).unwrap();
        let app = App::open(&temp.join("data"), &temp.join("home")).unwrap();
        let id = app.project_add(repo.to_str().unwrap(), false).unwrap()["project"]["id"].as_str().unwrap().to_string();
        let other = app.project_add(&{
            let p = temp.join("home/src/beta");
            std::fs::create_dir_all(p.join(".git/objects")).unwrap();
            std::fs::write(p.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            p.to_string_lossy().into_owned()
        }, false).unwrap()["project"]["id"].as_str().unwrap().to_string();

        // Nothing saved: the global agents, in the contract shape.
        let read = app.provider_agents(&id).unwrap();
        assert_eq!(read["schema"], "vajra.project_agents/1");
        assert_eq!(read["default"]["account"], "claude");

        let want = json!({
            "default": {"account": "claude", "model": null, "effort": null},
            "roles": {"docs": {"account": "codex", "model": null, "effort": "medium"}},
            "allowed": [{"account": "claude", "model": null}, {"account": "codex", "model": null}],
        });
        let saved = app.provider_agents_save(&id, want.clone()).unwrap();
        assert_eq!(saved["roles"]["docs"]["account"], "codex");
        assert_eq!(app.provider_agents(&id).unwrap()["roles"], want["roles"]);
        // Scoped to that project: the other one and the global settings are untouched.
        assert_eq!(app.provider_agents(&other).unwrap()["roles"], json!({}));
        assert_eq!(app.preferences_read(None).unwrap()["agents"]["roles"], json!({}));

        // The ceiling and the role keys are enforced as on any preferences write.
        let outside = json!({"default": {"account": "claude"}, "roles": {"docs": {"account": "codex"}}, "allowed": [{"account": "claude"}]});
        assert_eq!(app.provider_agents_save(&id, outside).unwrap_err()["error"], "outside_allowed");
        let role = json!({"default": {"account": "claude"}, "roles": {"janitor": {"account": "claude"}}, "allowed": [{"account": "claude"}]});
        assert_eq!(app.provider_agents_save(&id, role).unwrap_err()["error"], "unknown_role");
        let junk = json!({"default": {"account": "claude"}, "allowed": [{"account": "claude"}], "extra": 1});
        assert_eq!(app.provider_agents_save(&id, junk).unwrap_err()["error"], "invalid_args");
        // A refused save changes nothing.
        assert_eq!(app.provider_agents(&id).unwrap()["roles"], want["roles"]);

        // An id that is not a project, and one that is well formed but unknown, are refused on both commands.
        assert_eq!(app.provider_agents("../x").unwrap_err()["error"], "invalid_args");
        assert_eq!(app.provider_agents("global").unwrap_err()["error"], "invalid_args");
        let unknown = format!("p_{}", "0".repeat(26));
        assert_eq!(app.provider_agents(&unknown).unwrap_err()["error"], "not_found");
        assert_eq!(app.provider_agents_save(&unknown, want).unwrap_err()["error"], "not_found");
        std::fs::remove_dir_all(temp).unwrap();
    }
}
