//! D7: the REAL bundled engine installer, end to end. CI only (vajra-1 decision 2026-11-12): the `engine-real-installer`
//! job runs this on a fresh macos-latest runner with a temp HOME and no secrets. It is `#[ignore]`, and even under
//! `--ignored` it does nothing unless `VAJRA_REAL_INSTALLER_CI=1` and `GITHUB_ACTIONS=true` are both set, so no agent
//! session or local `cargo test` ever runs the real installer.
//!
//! What it proves on the build CI made:
//! - pinned build (the manifest matches the tree): a fresh install lands, the real `session-start.sh --check` passes,
//!   the marks stay clean, `settings.json` is never created; then a Repair whose check is forced to fail is rolled
//!   back to the previous copy byte for byte, with no Broken mark;
//! - unpinned build (the manifest is stale, before the owner's regen): the plan refuses with `engine_not_pinned` and
//!   nothing is written under HOME.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::Pin as BuildPin;
use super::*;
use crate::bus::{Bus, Config};
use crate::providers::install::env::engine_env;
use crate::providers::install::marks::Marks;
use crate::providers::install::run::Outcome;
use crate::sessions::Supervisor;

fn allowed() -> Option<PathBuf> {
    let on = std::env::var("VAJRA_REAL_INSTALLER_CI").as_deref() == Ok("1") && std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true");
    let tmp = std::env::var_os("RUNNER_TEMP").map(PathBuf::from).filter(|p| p.is_absolute());
    if !on || tmp.is_none() {
        eprintln!("real installer test skipped: CI only (VAJRA_REAL_INSTALLER_CI=1, GITHUB_ACTIONS=true, RUNNER_TEMP)");
        return None;
    }
    tmp
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let t = e.file_type().unwrap();
            if t.is_dir() {
                stack.push(p);
            } else if t.is_file() {
                out.insert(p.strip_prefix(root).unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

#[test]
#[ignore = "runs the real engine installer: CI job engine-real-installer only"]
fn the_real_bundled_installer_installs_checks_and_rolls_back() {
    let Some(tmp) = allowed() else { return };
    let root = tmp.join(format!("vajra-real-installer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (home, data) = (root.join("home"), root.join("data"));
    for d in [&root, &home, &data] {
        std::fs::create_dir_all(d).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let home = home.canonicalize().unwrap();
    let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("gen");
    let pin = BuildPin::built();
    let tools = Tools::system();
    let busy = || false;
    let m = Machine { home: &home, data: &data, resources: &resources, pin: &pin, tools: &tools, uid: place::uid(), busy: &busy };

    let prep = match plan(&m) {
        Offer::Plan(p) => *p,
        Offer::Manual(code) => {
            assert!(!pin.tree_ok, "a pinned build must offer a plan, got {code}");
            assert_eq!(code, "engine_not_pinned");
            assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0, "an unpinned plan writes nothing under HOME");
            eprintln!("unpinned build (manifest stale until the owner's regen): refusal checked; install phases skipped");
            return;
        }
    };

    let bus = Arc::new(Bus::open(&data.join("spill"), Config::default()).unwrap());
    let sup = Supervisor::new(bus, None);
    let env = engine_env(&home, |k| std::env::var_os(k).filter(|_| ["USER", "LANG"].contains(&k)));
    let marks = Marks::at(&data);
    let audit = data.join("audit.log");
    let current = || None;
    let ctx = crate::providers::install::run::Ctx { sup: &sup, env: &env, marks: &marks, audit: &audit, timeout: TIMEOUT, current: &current };

    // 1. A fresh install with the real check.
    let check = session_check(&env);
    let done = run(&m, &ctx, &prep, &check);
    assert_eq!(done.report.outcome, Outcome::Installed, "{:?}", done.report);
    assert_eq!(done.case, "fresh");
    assert_eq!(marks.any(ID), None, "a clean install leaves no Broken mark");
    let skill = home.join(".claude/skills/vajra");
    assert!(skill.join(place::MARKER).is_file(), "the app's marker is written");
    assert!(skill.join("scripts/vajra_tasks.py").is_file());
    assert!(home.join(".claude/vajra/manifest.sig").is_file(), "the installer signed the manifest");
    assert!(!home.join(".claude/settings.json").exists(), "--no-hooks: settings.json is never written");
    let installed = tree(&skill);

    // 2. A Repair whose check fails: the real installer runs again, then everything is put back as it was.
    let repair = match plan(&m) {
        Offer::Plan(p) => *p,
        Offer::Manual(code) => panic!("repair plan: {code}"),
    };
    assert!(matches!(repair.plan.pin, crate::providers::install::plan::Pin::Engine { case: "repair", differs: Some(0), .. }), "{:?}", repair.plan.pin);
    let failing = |_: &Path| false;
    let rolled = run(&m, &ctx, &repair, &failing);
    assert_eq!(rolled.report.outcome, Outcome::RolledBack, "{:?}", rolled.report);
    assert_eq!(tree(&skill), installed, "the previous copy is back byte for byte");
    assert_eq!(marks.any(ID), None, "a confirmed rollback is not Broken");
    let log = std::fs::read_to_string(&audit).unwrap();
    assert!(log.contains("case=fresh") && log.contains("case=repair"), "{log}");

    let _ = std::fs::remove_dir_all(&root);
}
