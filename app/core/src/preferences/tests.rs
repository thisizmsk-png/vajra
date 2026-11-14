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
/// The app spawns processes all the time (git, workers, the Lead). A child forked while a store call holds the lock
/// gets a copy of the directory descriptor until it execs, and a flock lives on until every copy is closed. So the
/// store must release its lock itself, not leave it to closing its own copy, or the next call is `preferences_busy`
/// for as long as some unrelated child sits between fork and exec (seen as random failures in the parallel suite).
#[test]
fn a_child_forked_while_the_lock_is_held_does_not_keep_the_store_busy() {
    use std::os::unix::process::CommandExt;
    let h = Home::new();
    let s = h.store();
    s.read(None).unwrap();
    let dir = s.directory().unwrap();
    let held = lock_file(&dir, true).unwrap();
    // Two close-on-exec pipes (no other test's child keeps them): the child says it has forked, holding its copy of
    // the descriptor, and then waits in its pre-exec hook until the parent has finished both store calls.
    let pipe = || {
        let mut fds = [0; 2];
        // SAFETY: pipe writes two descriptors into the array; fcntl only sets their close-on-exec flag.
        unsafe {
            assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
            for fd in fds {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
        }
        (fds[0], fds[1])
    };
    let ((ready_r, ready_w), (go_r, go_w)) = (pipe(), pipe());
    let child = std::thread::spawn(move || {
        let mut c = std::process::Command::new("/usr/bin/true");
        // SAFETY: the hook calls only close, write and read, which are async-signal-safe, and reads errno.
        unsafe {
            c.pre_exec(move || {
                // Its own copies of the ends it never uses: a parent that dies before `go` leaves it at EOF, not stuck.
                libc::close(ready_r);
                libc::close(go_w);
                libc::write(ready_w, b"x".as_ptr().cast(), 1);
                let mut b = 0u8;
                while libc::read(go_r, (&mut b as *mut u8).cast(), 1) < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {}
                Ok(())
            })
        };
        c.status().unwrap()
    });
    let mut p = libc::pollfd { fd: ready_r, events: libc::POLLIN, revents: 0 };
    // SAFETY: poll and read on our own pipe's read end.
    assert_eq!(unsafe { libc::poll(&mut p, 1, 20_000) }, 1, "the child never forked");
    let mut b = 0u8;
    assert_eq!(unsafe { libc::read(ready_r, (&mut b as *mut u8).cast(), 1) }, 1);
    // The child now holds a copy of the descriptor. The call ends, as read() and update() end: the lock, then the
    // descriptor go.
    drop(held);
    drop(dir);
    let read = s.read(None);
    let wrote = s.update(None, 0, patch(json!({"theme": "light"})), false);
    // SAFETY: a one-byte write to our own pipe lets the child exec, then every pipe end is closed once.
    unsafe { libc::write(go_w, b"x".as_ptr().cast(), 1) };
    assert!(child.join().unwrap().success());
    for fd in [ready_r, ready_w, go_r, go_w] {
        unsafe { libc::close(fd) };
    }
    assert!(read.is_ok(), "{:?}", read.err());
    assert!(wrote.is_ok(), "{:?}", wrote.err());
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

#[test]
fn the_lead_defaults_to_the_default_choice_and_account_caps_default_empty() {
    let h = Home::new();
    let s = h.store();
    let a = s.update(None, 0, patch(agents_patch()), false).unwrap().agents;
    assert_eq!(a.lead(), &a.default);
    assert!(a.account_caps.is_empty());
    let mut p = agents_patch();
    p["agents"]["lead"] = json!({"account": "codex", "effort": "high"});
    p["agents"]["account_caps"] = json!({"codex": 3});
    let a = s.update(None, 1, patch(p), false).unwrap().agents;
    assert_eq!((a.lead().account.as_str(), a.account_caps["codex"]), ("codex", 3));
    assert_eq!(h.store().read(None).unwrap().agents, a, "survives a reload");
}

#[test]
fn a_lead_outside_the_ceiling_or_a_bad_cap_is_refused() {
    let h = Home::new();
    let s = h.store();
    let mut p = agents_patch();
    p["agents"]["lead"] = json!({"account": "gemini"});
    assert_eq!(s.update(None, 0, patch(p), false).unwrap_err().code(), "outside_allowed");
    let mut p = agents_patch();
    p["agents"]["account_caps"] = json!({"Bad Id": 1});
    assert_eq!(s.update(None, 0, patch(p), false).unwrap_err().code(), "invalid_preferences");
    let mut p = agents_patch();
    p["agents"]["account_caps"] = json!({"codex": 0});
    assert_eq!(s.update(None, 0, patch(p), false).unwrap_err().code(), "invalid_preferences");
}

#[test]
fn a_role_key_must_be_a_known_crew_role() {
    let h = Home::new();
    let s = h.store();
    let mut p = agents_patch();
    p["agents"]["roles"]["arjuna"] = json!({"account": "claude"});
    assert_eq!(s.update(None, 0, patch(p), false).unwrap_err().code(), "unknown_role");
    // `architect` is the planner's key (the UI offers it) and is accepted.
    let mut p = agents_patch();
    p["agents"]["roles"]["architect"] = json!({"account": "claude"});
    s.update(None, 0, patch(p), false).unwrap();
    for r in ["verifier", "reviewer", "security_reviewer", "platform", "numerics", "analysis"] {
        let mut p = agents_patch();
        p["agents"]["roles"][r] = json!({"account": "claude"});
        s.update(None, s.read(None).unwrap().rev, patch(p), false).unwrap_or_else(|e| panic!("{r}: {}", e.code()));
    }
}

#[test]
fn a_settings_file_already_on_disk_with_any_role_key_still_loads() {
    let h = Home::new();
    let s = h.store();
    s.update(None, 0, patch(agents_patch()), false).unwrap();
    let path = h.0.join("preferences/settings.json");
    let mut doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    doc["global"]["agents"]["roles"]["architect"] = json!({"account": "claude"});
    doc["global"]["agents"]["roles"]["some_old_role"] = json!({"account": "claude"});
    std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    let v = s.read(None).unwrap();
    assert!(v.agents.roles.contains_key("architect") && v.agents.roles.contains_key("some_old_role"));
    // An unrelated write still works, and the old key is left alone.
    s.update(None, v.rev, patch(json!({"theme": "dark"})), false).unwrap();
    assert!(s.read(None).unwrap().agents.roles.contains_key("some_old_role"));
}

// Given the ping rule, When it is saved globally, Then it reads back and survives a reopen; out-of-range budgets are refused.
#[test]
fn the_ping_rule_is_a_bounded_global_preference() {
    let h = Home::new();
    let s = h.store();
    assert_eq!(s.read(None).unwrap().effective.ping, Ping::default());
    let rule = json!({"ping": {"per_hour": 3, "batch_minutes": 0, "decisions": true, "questions": false, "blocked": true}});
    s.update(None, 0, patch(rule), false).unwrap();
    let v = h.store().read(Some(P)).unwrap();
    assert_eq!((v.effective.ping.per_hour, v.effective.ping.batch_minutes, v.effective.ping.questions), (3, 0, false));
    assert_eq!(v.sources["ping"], "global");
    for bad in [
        json!({"ping": {"per_hour": 0}}),
        json!({"ping": {"per_hour": 31}}),
        json!({"ping": {"batch_minutes": 11}}),
    ] {
        assert_eq!(s.update(None, 1, patch(bad), false).unwrap_err().code(), "invalid_preferences");
    }
    assert!(serde_json::from_value::<Patch>(json!({"ping": {"start_work": true}})).is_err(), "the rule has no field that acts");
    assert_eq!(s.read(None).unwrap().rev, 1);
}

// Given a project, When it tries to set or inherit its own ping rule, Then it is refused: one budget for the person.
#[test]
fn a_project_cannot_override_the_ping_rule() {
    let h = Home::new();
    let s = h.store();
    let rule = json!({"ping": {"per_hour": 1}});
    assert_eq!(s.update(Some(P), 0, patch(rule), false).unwrap_err().code(), "global_only");
    assert_eq!(s.update(Some(P), 0, patch(json!({"inherit": ["ping"]})), false).unwrap_err().code(), "global_only");
    // A file on disk that carries a project ping override is refused on read, not half-applied.
    s.update(Some(P), 0, patch(json!({"theme": "dark"})), false).unwrap();
    let path = h.0.join("preferences/settings.json");
    let mut doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    doc["overrides"][P]["ping"] = json!({"per_hour": 1});
    std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    assert_eq!(s.read(None).unwrap_err().code(), "invalid_preferences");
}

// Given a settings file written before the ping rule existed, Then it loads with the default rule.
#[test]
fn a_settings_file_without_a_ping_rule_reads_the_default() {
    let h = Home::new();
    let s = h.store();
    s.update(None, 0, patch(json!({"theme": "dark"})), false).unwrap();
    let path = h.0.join("preferences/settings.json");
    let mut doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    doc["global"].as_object_mut().unwrap().remove("ping");
    std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    assert_eq!(s.read(None).unwrap().effective.ping, Ping::default());
}
