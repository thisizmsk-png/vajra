use super::*;
use serde_json::json;
use std::os::unix::fs::{symlink, PermissionsExt};
const P: &str = "p_01JC5X8Z3A9K2M4N6P7Q8R9S0T";
const Q: &str = "p_01JC5X8Z3B0K2M4N6P7Q8R9S0T";
struct Home(std::path::PathBuf);
impl Home {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "vajra-preferences-{}",
            crate::ids::uuid_v4().unwrap()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn store(&self) -> Store {
        Store::new(self.0.clone())
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn patch(v: serde_json::Value) -> Patch {
    serde_json::from_value(v).unwrap()
}
#[test]
fn fg08_global_defaults_and_project_overrides_survive_reopen_without_touching_other_scopes() {
    let h = Home::new();
    let s = h.store();
    assert_eq!(s.read(None).unwrap().effective.theme, Theme::System);
    let v = s
        .update(
            None,
            0,
            patch(json!({"theme":"light","shell":"zsh","notifications":false})),
            false,
        )
        .unwrap();
    assert_eq!(v.rev, 1);
    let v = s
        .update(
            Some(P),
            1,
            patch(json!({"theme":"dark","agent":"codex"})),
            false,
        )
        .unwrap();
    assert_eq!(v.effective.theme, Theme::Dark);
    assert_eq!(v.sources["theme"], "project");
    assert_eq!(v.sources["shell"], "global");
    let s = h.store();
    assert_eq!(s.read(Some(P)).unwrap().effective.agent, Agent::Codex);
    assert_eq!(s.read(Some(Q)).unwrap().effective.theme, Theme::Light);
    let v = s.update(Some(P), 2, Patch::default(), true).unwrap();
    assert_eq!(v.effective.theme, Theme::Light);
    assert_eq!(v.effective.shell, Shell::Zsh);
    let v = s.update(None, 3, Patch::default(), true).unwrap();
    assert_eq!(v.effective.theme, Theme::System);
}
#[test]
fn fg08_launch_snapshot_is_immutable_and_stale_updates_do_not_win() {
    let h = Home::new();
    let s = h.store();
    let launch = s.read(Some(P)).unwrap();
    s.update(None, 0, patch(json!({"theme":"dark","workers":2})), false)
        .unwrap();
    assert_eq!(launch.effective.workers, 4);
    assert_eq!(launch.rev, 0);
    assert_eq!(
        s.update(None, 0, Patch::default(), false)
            .unwrap_err()
            .code(),
        "revision_conflict"
    );
}
#[test]
fn fg08_reset_is_scoped_and_inherit_removes_one_override() {
    let h = Home::new();
    let s = h.store();
    s.update(
        Some(P),
        0,
        patch(json!({"theme":"dark","workers":2})),
        false,
    )
    .unwrap();
    s.update(Some(Q), 1, patch(json!({"theme":"light"})), false)
        .unwrap();
    let p: Patch = serde_json::from_value(json!({"inherit":["theme"]})).unwrap();
    let v = s.update(Some(P), 2, p, false).unwrap();
    assert_eq!(v.effective.theme, Theme::System);
    assert_eq!(v.effective.workers, 2);
    s.update(None, 3, Patch::default(), true).unwrap();
    assert_eq!(s.read(Some(Q)).unwrap().effective.theme, Theme::Light);
}
#[test]
fn fg08_rejects_unknown_secret_fields_invalid_ids_ranges_and_shortcut_conflicts() {
    let h = Home::new();
    let s = h.store();
    assert!(serde_json::from_value::<Patch>(json!({"token":"secret"})).is_err());
    assert!(serde_json::from_value::<Patch>(json!({"shell":"sh -c evil"})).is_err());
    assert_eq!(
        s.read(Some("../../other")).unwrap_err().code(),
        "invalid_preferences"
    );
    for v in [
        json!({"workers":0}),
        json!({"rounds":4}),
        json!({"model":"https://user:token@host"}),
        json!({"shortcuts":{"palette":"Primary+Q"}}),
        json!({"shortcuts":{"palette":"Primary+K","settings":"Primary+K"}}),
    ] {
        assert!(s.update(None, 0, patch(v), false).is_err());
    }
    assert_eq!(s.read(None).unwrap().rev, 0);
}
#[test]
fn fg08_symlink_and_fifo_and_oversized_files_fail_without_overwriting() {
    let h = Home::new();
    let s = h.store();
    s.update(None, 0, Patch::default(), false).unwrap();
    let path = h.0.join("preferences/settings.json");
    std::fs::remove_file(&path).unwrap();
    let target = h.0.join("secret");
    std::fs::write(&target, "unchanged").unwrap();
    symlink(&target, &path).unwrap();
    assert!(s.read(None).is_err());
    assert!(s.update(None, 1, Patch::default(), true).is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "unchanged");
    std::fs::remove_file(&path).unwrap();
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(s.read(None).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, vec![b'x'; MAX_BYTES + 1]).unwrap();
    assert!(s.read(None).is_err());
}
#[test]
fn fg08_future_schema_and_malformed_data_are_preserved_and_files_are_private() {
    let h = Home::new();
    let s = h.store();
    s.update(None, 0, Patch::default(), false).unwrap();
    let dir = h.0.join("preferences");
    let path = dir.join("settings.json");
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v["schema"] = json!("vajra.preferences/2");
    let bytes = serde_json::to_vec(&v).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(s.read(None).unwrap_err().code(), "unsupported_version");
    assert!(s.update(None, 1, Patch::default(), true).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}
#[test]
fn fg08_refuses_symlinked_settings_directory() {
    let h = Home::new();
    let target = h.0.join("outside");
    std::fs::create_dir(&target).unwrap();
    symlink(&target, h.0.join("preferences")).unwrap();
    assert!(h.store().read(None).is_err());
    assert!(!target.join("settings.json").exists());
}

#[test]
fn fg08_response_contract_and_notifications_are_private_by_default() {
    let h = Home::new();
    let actual = h.store().read(None).unwrap().json();
    let expected: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/preferences-v1.json")).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["caps"]["editable"], false);
}

#[test]
fn an_old_agent_preference_reads_as_a_default_and_an_allowed_set() {
    let h = Home::new();
    let s = h.store();
    assert_eq!(
        s.read(None).unwrap().agents,
        Agents::from_legacy(Agent::Claude)
    );
    s.update(Some(P), 0, patch(json!({"agent":"codex"})), false)
        .unwrap();
    let a = s.read(Some(P)).unwrap().agents;
    assert_eq!(
        (
            a.default.account.as_str(),
            a.allowed.len(),
            a.allowed[0].account.as_str(),
            a.roles.len()
        ),
        ("codex", 1, "codex", 0)
    );
    assert_eq!(s.read(Some(P)).unwrap().sources["agents"], "project");
    assert_eq!(s.read(Some(Q)).unwrap().agents.default.account, "claude");
}

fn agents_patch() -> serde_json::Value {
    json!({"agents": {
        "default": {"account": "claude"},
        "roles": {"docs": {"account": "openrouter", "model": "deepseek/deepseek-v3.2"}, "tests": {"account": "codex", "effort": "medium"}},
        "allowed": [{"account": "claude"}, {"account": "codex"}, {"account": "openrouter", "model": "deepseek/deepseek-v3.2"}]
    }})
}

#[test]
fn a_role_with_no_entry_falls_back_to_the_default() {
    let h = Home::new();
    let s = h.store();
    let a = s
        .update(None, 0, patch(agents_patch()), false)
        .unwrap()
        .agents;
    assert_eq!(a.choice_for(Some("docs")).account, "openrouter");
    assert_eq!(
        a.choice_for(Some("tests")).effort.as_deref(),
        Some("medium")
    );
    assert_eq!(a.choice_for(Some("backend")), &a.default);
    assert_eq!(a.choice_for(None), &a.default);
    // Survives a reload, and the serialized view still has the old key.
    let v = h.store().read(None).unwrap();
    assert_eq!(v.agents, a);
    assert_eq!(v.effective.agent, Agent::Claude);
}

#[test]
fn the_allowed_ceiling_is_enforced_on_write() {
    let h = Home::new();
    let s = h.store();
    // A role outside the ceiling.
    let mut p = agents_patch();
    p["agents"]["roles"]["docs"] = json!({"account": "openrouter", "model": "x-ai/grok-4"});
    assert_eq!(
        s.update(None, 0, patch(p), false).unwrap_err().code(),
        "outside_allowed"
    );
    // A default outside the ceiling.
    let mut p = agents_patch();
    p["agents"]["default"] = json!({"account": "gemini"});
    assert_eq!(
        s.update(Some(P), 0, patch(p), false).unwrap_err().code(),
        "outside_allowed"
    );
    // An empty ceiling, a bad id and a secret-shaped model are refused as invalid.
    for bad in [json!([]), json!([{"account": "Bad Id"}])] {
        let mut p = agents_patch();
        p["agents"]["allowed"] = bad;
        assert!(s.update(None, 0, patch(p), false).is_err());
    }
    let mut p = agents_patch();
    p["agents"]["roles"]["docs"]["model"] = json!("sk proj abc");
    assert_eq!(
        s.update(None, 0, patch(p), false).unwrap_err().code(),
        "invalid_preferences"
    );
    // Nothing was written by the refusals.
    assert_eq!(s.read(None).unwrap().rev, 0);
    // A project cannot raise itself past the ceiling it inherits... it brings its own, validated the same way.
    s.update(None, 0, patch(agents_patch()), false).unwrap();
    let mut p = agents_patch();
    p["agents"]["allowed"] = json!([{"account": "claude"}]);
    assert_eq!(
        s.update(Some(P), 1, patch(p), false).unwrap_err().code(),
        "outside_allowed",
        "its roles are outside its own ceiling"
    );
}

#[test]
fn a_newer_old_style_write_replaces_agents_and_unknown_fields_are_refused() {
    let h = Home::new();
    let s = h.store();
    s.update(Some(P), 0, patch(agents_patch()), false).unwrap();
    let v = s
        .update(Some(P), 1, patch(json!({"agent": "codex"})), false)
        .unwrap();
    assert_eq!(
        (v.agents.default.account.as_str(), v.agents.roles.len()),
        ("codex", 0)
    );
    assert!(serde_json::from_value::<Patch>(json!({"agents": {"default": {"account": "claude", "token": "x"}, "allowed": [{"account": "claude"}]}})).is_err(), "no bucket for a secret");
    // Inherit drops the project's choice back to the global one.
    s.update(Some(P), 2, patch(agents_patch()), false).unwrap();
    let v = s
        .update(
            Some(P),
            3,
            serde_json::from_value(json!({"inherit": ["agent", "agents"]})).unwrap(),
            false,
        )
        .unwrap();
    assert_eq!(v.agents, Agents::from_legacy(Agent::Claude));
}
