//! D7 run, with a FAKE engine installer only (vajra-1 decision 2026-11-12: no agent session ever runs the real one).
//! Each test packs a tiny engine whose installer is a script written here, so every path of the flow is exercised:
//! bhishma C3 (re-verify, core-written workflows), C4 (key and state), C5 (rollback restores the signature state,
//! never runs old code), C6 (journal recovery), C7 (env, hash), C8 (busy, lock), C9 (marks).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::super::pack::{raw_tgz, sha256_hex};
use super::super::Pin as BuildPin;
use super::*;
use crate::bus::{Bus, Config};
use crate::providers::install::env::engine_env;
use crate::providers::install::marks::Marks;
use crate::sessions::Supervisor;

/// A fake installer: writes the signature and (unless told otherwise) the key, and records that it ran.
const GOOD: &str = "#!/bin/bash\nset -e\nV=\"$HOME/.claude/vajra\"\nmkdir -p \"$V\"\nchmod 700 \"$V\"\n[ -f \"$V/.hmac-key\" ] || { printf '%064d\\n' 0 > \"$V/.hmac-key\"; chmod 600 \"$V/.hmac-key\"; }\necho signed > \"$V/manifest.sig\"\necho ran >> \"$HOME/installer-ran\"\necho \"PATH=$PATH\" >> \"$HOME/installer-ran\"\n";

struct World {
    root: PathBuf,
    home: PathBuf,
    data: PathBuf,
    res: PathBuf,
    sup: Supervisor,
    pin: BuildPin,
}

impl World {
    fn new(tag: &str, installer: &str, version: &str) -> World {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!("vajra-eflow-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, data, res) = (root.join("home"), root.join("data"), root.join("res"));
        for d in [&root, &home, &data, &res] {
            std::fs::create_dir_all(d).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let bus = Arc::new(Bus::open(&data.join("spill"), Config::default()).unwrap());
        let mut w = World { root, home, data, res, sup: Supervisor::new(bus, None), pin: BuildPin { sha256: String::new(), version: String::new(), tree_ok: true, unpacked: 0, files: 0, release: false } };
        w.bundle(installer, version);
        w
    }

    /// Pack a fake engine with this installer and version into the resource dir, and pin it.
    fn bundle(&mut self, installer: &str, version: &str) {
        let pkg = format!(r#"{{"version":"{version}"}}"#);
        let check = "#!/bin/bash\n[ \"$(cat \"$HOME/.claude/vajra/manifest.sig\" 2>/dev/null)\" = signed ]\n";
        let files: Vec<(&str, Vec<u8>, u32)> = vec![
            ("package.json", pkg.into_bytes(), 0o644),
            ("scripts/install.sh", installer.as_bytes().to_vec(), 0o755),
            ("scripts/vajra-campaign.sh", format!("echo campaign {version}").into_bytes(), 0o755),
            ("hooks/session-start.sh", check.as_bytes().to_vec(), 0o755),
            ("workflows/vajra-deliver.js", format!("// deliver {version}").into_bytes(), 0o644),
        ];
        let manifest: serde_json::Map<String, serde_json::Value> = files.iter().map(|(n, b, _)| (n.to_string(), sha256_hex(b).into())).collect();
        let mut all: Vec<(String, Vec<u8>, u32)> = files.into_iter().map(|(n, b, m)| (n.to_string(), b, m)).collect();
        all.push(("manifest.json".into(), serde_json::to_vec(&serde_json::json!({ "files": manifest })).unwrap(), 0o644));
        all.sort_by(|a, b| a.0.cmp(&b.0));
        let unpacked = all.iter().map(|e| e.1.len() as u64).sum();
        let tgz = raw_tgz(&all, b'0');
        std::fs::create_dir_all(self.res.join("engine")).unwrap();
        std::fs::write(self.res.join(bundle::RESOURCE), &tgz).unwrap();
        self.pin = BuildPin { sha256: sha256_hex(&tgz), version: version.into(), tree_ok: true, unpacked, files: all.len(), release: false };
    }

    fn tools(&self) -> Tools {
        Tools { bash: "/bin/bash".into(), jq: "/bin/bash".into(), openssl: "/bin/bash".into(), clt: true }
    }

    fn plan_with(&self, busy: bool) -> Offer {
        let tools = self.tools();
        let b = move || busy;
        plan(&Machine { home: &self.home, data: &self.data, resources: &self.res, pin: &self.pin, tools: &tools, uid: place::uid(), busy: &b })
    }

    fn plan(&self) -> Prepared {
        match self.plan_with(false) {
            Offer::Plan(p) => *p,
            Offer::Manual(r) => panic!("manual: {r}"),
        }
    }

    fn manual(&self) -> &'static str {
        match self.plan_with(false) {
            Offer::Plan(_) => panic!("expected manual"),
            Offer::Manual(r) => r,
        }
    }

    fn run(&self, p: &Prepared) -> Done {
        self.run_busy(p, false)
    }

    fn run_busy(&self, p: &Prepared, busy: bool) -> Done {
        let tools = self.tools();
        let b = move || busy;
        let m = Machine { home: &self.home, data: &self.data, resources: &self.res, pin: &self.pin, tools: &tools, uid: place::uid(), busy: &b };
        let env = engine_env(&self.home, |k| std::env::var_os(k).filter(|_| ["USER", "LANG"].contains(&k)));
        let current = || None;
        let ctx = Ctx { sup: &self.sup, env: &env, marks: &self.marks(), audit: &self.data.join("audit.log"), timeout: Duration::from_secs(20), current: &current };
        let check = session_check(&env);
        run(&m, &ctx, p, &check)
    }

    fn marks(&self) -> Marks {
        Marks::at(&self.data)
    }
    fn skill(&self) -> PathBuf {
        self.home.join(".claude/skills/vajra")
    }
    fn vajra(&self) -> PathBuf {
        self.home.join(".claude/vajra")
    }
    fn ran(&self) -> String {
        std::fs::read_to_string(self.home.join("installer-ran")).unwrap_or_default()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn hash_of(p: &Prepared) -> String {
    p.plan.hash.clone()
}

#[test]
fn a_fresh_install_lands_the_engine_runs_the_installer_with_no_hooks_and_is_recorded() {
    let w = World::new("fresh", GOOD, "0.6.2");
    let p = w.plan();
    assert_eq!(p.plan.command[1..], [w.skill().join("scripts/install.sh").display().to_string(), "--no-hooks".into()]);
    assert!(p.plan.notes.contains(&"no_hooks") && p.plan.notes.contains(&"creates_key"));
    let d = w.run(&p);
    assert_eq!((d.report.outcome, d.report.reason, d.case), (Outcome::Installed, None, "fresh"));
    assert_eq!(std::fs::read_to_string(w.skill().join("scripts/vajra-campaign.sh")).unwrap(), "echo campaign 0.6.2");
    assert_eq!(std::fs::read_to_string(w.home.join(".claude/workflows/vajra-deliver.js")).unwrap(), "// deliver 0.6.2", "the core wrote the workflow");
    assert!(w.ran().contains("PATH=/usr/bin:/bin:/usr/sbin:/sbin"), "the installer ran with the root-only PATH: {}", w.ran());
    assert!(!w.home.join(".claude/settings.json").exists(), "--no-hooks: settings.json never created");
    let audit = std::fs::read_to_string(w.data.join("audit.log")).unwrap();
    assert!(audit.contains("method=engine_bundle case=fresh version=0.6.2 old_version=none") && audit.contains("result=installed"), "{audit}");
    assert!(!audit.contains("0000000000000000000000000000000000000000000000000000000000000000"), "no key bytes in the audit");
    assert!(matches!(w.plan_with(false), Offer::Plan(ref p) if p.plan.notes.contains(&"no_hooks") && !p.plan.notes.contains(&"creates_key")), "a second plan is a Repair with no new key");
}

#[test]
fn settings_json_is_byte_identical_after_an_install() {
    let w = World::new("settings", GOOD, "0.6.2");
    std::fs::create_dir_all(w.home.join(".claude")).unwrap();
    std::fs::write(w.home.join(".claude/settings.json"), b"{\"hooks\":{}}\n").unwrap();
    let p = w.plan();
    assert_eq!(w.run(&p).report.outcome, Outcome::Installed);
    assert_eq!(std::fs::read(w.home.join(".claude/settings.json")).unwrap(), b"{\"hooks\":{}}\n");
}

#[test]
fn an_installer_that_fails_or_edits_the_tree_is_rolled_back_and_leaves_no_engine() {
    for (tag, installer, why) in [
        ("exit1", "#!/bin/bash\nexit 1\n", "install_failed"),
        ("edits", "#!/bin/bash\nmkdir -p \"$HOME/.claude/vajra\"; echo signed > \"$HOME/.claude/vajra/manifest.sig\"; echo evil > \"$(dirname \"$0\")/vajra-campaign.sh\"\n", "tree_changed"),
        ("adds", "#!/bin/bash\nmkdir -p \"$HOME/.claude/vajra\"; echo signed > \"$HOME/.claude/vajra/manifest.sig\"; echo x > \"$(dirname \"$0\")/extra.sh\"\n", "tree_changed"),
        ("forged-sig", "#!/bin/bash\nmkdir -p \"$HOME/.claude/vajra\"; echo forged > \"$HOME/.claude/vajra/manifest.sig\"\n", "not_signed"),
    ] {
        let w = World::new(tag, installer, "0.6.2");
        let p = w.plan();
        let d = w.run(&p);
        assert_eq!((d.report.outcome, d.report.reason), (Outcome::RolledBack, Some(why)), "{tag}");
        assert!(!w.skill().exists(), "{tag}: a fresh install rolled back leaves nothing");
        assert!(!w.home.join(".claude/workflows/vajra-deliver.js").exists(), "{tag}: the workflow it added is removed again");
        assert!(!w.vajra().join("manifest.sig").exists(), "{tag}: the signature state is restored (there was none)");
    }
}

#[test]
fn an_update_that_fails_restores_the_old_copy_its_workflow_and_its_signature_exactly() {
    let mut w = World::new("update-fail", GOOD, "0.6.2");
    let p = w.plan();
    assert_eq!(w.run(&p).report.outcome, Outcome::Installed);
    let sig = std::fs::read(w.vajra().join("manifest.sig")).unwrap();
    w.bundle("#!/bin/bash\necho old-code-ran >> \"$HOME/installer-ran\"\nexit 3\n", "0.6.3");
    let p = w.plan();
    assert!(matches!(p.plan.pin, Pin::Engine { case: "update", ref old_version, .. } if old_version.as_deref() == Some("0.6.2")));
    let d = w.run(&p);
    assert_eq!((d.report.outcome, d.report.reason), (Outcome::RolledBack, Some("install_failed")));
    assert_eq!(std::fs::read_to_string(w.skill().join("scripts/vajra-campaign.sh")).unwrap(), "echo campaign 0.6.2");
    assert_eq!(std::fs::read_to_string(w.home.join(".claude/workflows/vajra-deliver.js")).unwrap(), "// deliver 0.6.2");
    assert_eq!(std::fs::read(w.vajra().join("manifest.sig")).unwrap(), sig, "the signature is byte for byte what it was");
    assert_eq!(w.ran().matches("old-code-ran").count(), 1, "only the new installer ran; the old copy's never runs on rollback");
}

#[test]
fn an_update_installs_the_new_version_and_a_changed_old_copy_is_backed_up() {
    let mut w = World::new("update", GOOD, "0.6.2");
    let p = w.plan();
    assert_eq!(w.run(&p).report.outcome, Outcome::Installed);
    std::fs::write(w.skill().join("scripts/vajra-campaign.sh"), "my edit").unwrap();
    // A changed app copy is still ours (marker and record agree), so it can be updated, and the edit is kept.
    w.bundle(GOOD, "0.6.3");
    let p = w.plan();
    let d = w.run(&p);
    assert_eq!(d.report.outcome, Outcome::Installed);
    assert_eq!(std::fs::read_to_string(w.skill().join("scripts/vajra-campaign.sh")).unwrap(), "echo campaign 0.6.3");
    let backup = d.backup.expect("the changed copy is kept");
    assert_eq!(std::fs::read_to_string(w.home.join(".claude/.vajra-install").join(backup).join("scripts/vajra-campaign.sh")).unwrap(), "my edit");
}

#[test]
fn a_users_own_copy_is_never_touched() {
    let w = World::new("user", GOOD, "0.6.2");
    std::fs::create_dir_all(w.skill().join(".git")).unwrap();
    std::fs::write(w.skill().join("x"), "mine").unwrap();
    assert_eq!(w.manual(), "engine_user_copy");
    assert_eq!(std::fs::read_to_string(w.skill().join("x")).unwrap(), "mine");
}

#[test]
fn nothing_is_installed_while_the_engine_is_in_use() {
    let w = World::new("busy", GOOD, "0.6.2");
    assert!(matches!(w.plan_with(true), Offer::Manual("engine_in_use")));
    let p = w.plan();
    let d = w.run_busy(&p, true);
    assert_eq!((d.report.outcome, d.report.reason), (Outcome::Failed, Some("engine_in_use")));
    assert!(!w.skill().exists() && w.ran().is_empty(), "no write, no spawn");
}

#[test]
fn a_second_install_while_one_holds_the_lock_is_refused() {
    let w = World::new("lock", GOOD, "0.6.2");
    let p = w.plan();
    let place = Place::open(&w.home, &w.data, place::uid()).unwrap();
    let held = lock(&place).unwrap();
    let d = w.run(&p);
    assert_eq!(d.report.reason, Some("setup_running"));
    drop(held);
    assert_eq!(w.run(&p).report.outcome, Outcome::Installed);
}

#[test]
fn an_unsafe_key_or_state_folder_is_refused_before_the_installer_runs() {
    // A planted temp key (a link the installer would write the key through).
    let w = World::new("keytmp", GOOD, "0.6.2");
    std::fs::create_dir_all(w.vajra()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(w.vajra(), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(w.root.join("stolen"), w.vajra().join(".hmac-key.tmp")).unwrap();
    assert_eq!(w.manual(), "engine_state_unsafe");
    // A malformed key the installer would replace: refused, and its bytes and inode stay.
    let w = World::new("keybad", GOOD, "0.6.2");
    std::fs::create_dir_all(w.vajra()).unwrap();
    std::fs::set_permissions(w.vajra(), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(w.vajra().join(".hmac-key"), "abc").unwrap();
    use std::os::unix::fs::MetadataExt;
    let ino = std::fs::metadata(w.vajra().join(".hmac-key")).unwrap().ino();
    assert_eq!(w.manual(), "engine_state_unsafe");
    assert_eq!((std::fs::read(w.vajra().join(".hmac-key")).unwrap(), std::fs::metadata(w.vajra().join(".hmac-key")).unwrap().ino()), (b"abc".to_vec(), ino));
    // A state folder others can read.
    let w = World::new("statemode", GOOD, "0.6.2");
    std::fs::create_dir_all(w.vajra()).unwrap();
    std::fs::set_permissions(w.vajra(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(w.manual(), "engine_state_unsafe");
}

#[test]
fn an_existing_key_is_never_rewritten() {
    let w = World::new("keykeep", GOOD, "0.6.2");
    std::fs::create_dir_all(w.vajra()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(w.vajra(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let key = format!("{}\n", "a".repeat(64));
    std::fs::write(w.vajra().join(".hmac-key"), &key).unwrap();
    std::fs::set_permissions(w.vajra().join(".hmac-key"), std::fs::Permissions::from_mode(0o600)).unwrap();
    let p = w.plan();
    assert!(!p.plan.notes.contains(&"creates_key"));
    assert_eq!(w.run(&p).report.outcome, Outcome::Installed);
    assert_eq!(std::fs::read_to_string(w.vajra().join(".hmac-key")).unwrap(), key);
}

#[test]
fn a_key_left_unsafe_by_the_installer_marks_the_engine_broken() {
    let w = World::new("keyafter", "#!/bin/bash\nV=\"$HOME/.claude/vajra\"; mkdir -p \"$V\"; chmod 700 \"$V\"; printf '%064d\\n' 0 > \"$V/.hmac-key\"; chmod 644 \"$V/.hmac-key\"; echo signed > \"$V/manifest.sig\"\n", "0.6.2");
    let p = w.plan();
    let d = w.run(&p);
    assert_eq!((d.report.outcome, d.report.reason), (Outcome::Broken, Some("key_unsafe")));
    assert_eq!(w.marks().any(ID).as_deref(), Some("key_unsafe"));
}

#[test]
fn a_hard_linked_workflow_is_refused_and_its_target_keeps_its_bytes() {
    let w = World::new("wflink", GOOD, "0.6.2");
    std::fs::create_dir_all(w.home.join(".claude/workflows")).unwrap();
    std::fs::write(w.home.join(".claude/settings.json"), "{}").unwrap();
    std::fs::hard_link(w.home.join(".claude/settings.json"), w.home.join(".claude/workflows/vajra-deliver.js")).unwrap();
    assert_eq!(w.manual(), "workflows_link");
    assert_eq!(std::fs::read_to_string(w.home.join(".claude/settings.json")).unwrap(), "{}");
}

#[test]
fn the_plan_hash_binds_the_case_the_build_kind_the_workflows_and_the_home() {
    let mut w = World::new("hash", GOOD, "0.6.2");
    let fresh = hash_of(&w.plan());
    w.pin.release = true;
    let release = hash_of(&w.plan());
    assert_ne!(fresh, release, "development and release builds plan differently");
    w.pin.release = false;
    std::fs::create_dir_all(w.home.join(".claude/workflows")).unwrap();
    std::fs::write(w.home.join(".claude/workflows/vajra-deliver.js"), "// deliver 0.6.2").unwrap();
    assert_ne!(hash_of(&w.plan()), fresh, "a workflow that no longer changes changes the hash");
    std::fs::remove_file(w.home.join(".claude/workflows/vajra-deliver.js")).unwrap();
    assert_eq!(hash_of(&w.plan()), fresh, "the same facts give the same hash");
    std::fs::create_dir_all(w.vajra()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(w.vajra(), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(w.vajra().join(".integrity-violation"), "").unwrap();
    let p = w.plan();
    assert!(p.plan.notes.contains(&"clears_integrity_lock"));
    assert_ne!(hash_of(&p), fresh, "the tamper flag is in the hash");
}

#[test]
fn an_unpinned_or_tampered_bundle_is_never_offered() {
    let mut w = World::new("unpinned", GOOD, "0.6.2");
    w.pin.tree_ok = false;
    assert_eq!(w.manual(), "engine_not_pinned");
    w.pin.tree_ok = true;
    let mut tgz = std::fs::read(w.res.join(bundle::RESOURCE)).unwrap();
    let n = tgz.len() - 1;
    tgz[n] ^= 1;
    std::fs::write(w.res.join(bundle::RESOURCE), tgz).unwrap();
    assert_eq!(w.manual(), "engine_bundle_mismatch");
}

#[test]
fn a_missing_tool_is_a_manual_reason_and_nothing_runs() {
    let w = World::new("tools", GOOD, "0.6.2");
    let tools = Tools { bash: "/bin/bash".into(), jq: w.root.join("no-jq"), openssl: "/bin/bash".into(), clt: false };
    let b = || false;
    let o = plan(&Machine { home: &w.home, data: &w.data, resources: &w.res, pin: &w.pin, tools: &tools, uid: place::uid(), busy: &b });
    assert!(matches!(o, Offer::Manual("engine_needs_jq")));
}

#[test]
fn a_crash_after_the_swap_is_recovered_by_the_next_consented_run() {
    let mut w = World::new("crash", GOOD, "0.6.2");
    let p = w.plan();
    assert_eq!(w.run(&p).report.outcome, Outcome::Installed);
    // Simulate a crash mid-update: staged and swapped, journal written, no commit.
    w.bundle(GOOD, "0.6.3");
    let p = w.plan();
    let place = Place::open(&w.home, &w.data, place::uid()).unwrap();
    let staged = place.stage(&p.engine, &w.pin.sha256).unwrap();
    place.swap_in(&staged, &p.found).unwrap();
    assert_eq!(std::fs::read_to_string(w.skill().join("scripts/vajra-campaign.sh")).unwrap(), "echo campaign 0.6.3");
    let before = std::fs::read_dir(w.home.join(".claude/.vajra-install")).unwrap().count();
    // A plan reports it and changes nothing on disk.
    let o = w.plan_with(false);
    assert!(matches!(o, Offer::Manual("engine_user_copy") | Offer::Plan(_)));
    assert_eq!(std::fs::read_dir(w.home.join(".claude/.vajra-install")).unwrap().count(), before, "planning is read-only");
    // The consented run recovers first (the old copy comes back), then installs.
    w.bundle(GOOD, "0.6.2");
    let p = w.plan();
    assert!(p.plan.notes.contains(&"recovers_previous"));
    assert_eq!(w.run(&p).report.outcome, Outcome::Installed);
    assert!(Place::open(&w.home, &w.data, place::uid()).unwrap().journal().is_none());
}

#[test]
fn a_hanging_installer_times_out_and_is_rolled_back() {
    let w = World::new("hang", "#!/bin/bash\nsleep 60\n", "0.6.2");
    let p = w.plan();
    let tools = w.tools();
    let b = || false;
    let m = Machine { home: &w.home, data: &w.data, resources: &w.res, pin: &w.pin, tools: &tools, uid: place::uid(), busy: &b };
    let env = engine_env(&w.home, |_| None);
    let current = || None;
    let ctx = Ctx { sup: &w.sup, env: &env, marks: &w.marks(), audit: &w.data.join("audit.log"), timeout: Duration::from_secs(2), current: &current };
    let d = run(&m, &ctx, &p, &session_check(&env));
    assert_eq!((d.report.outcome, d.report.reason), (Outcome::RolledBack, Some("install_timeout")));
    assert!(!w.skill().exists());
}
