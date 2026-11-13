//! C4 state reader and allowlist, against fixtures under a temp HOME.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use vajra_desktop::vajra::allowlist::{Allowlist, Denied};
use vajra_desktop::vajra::state::{EvidenceState, Reader, evidence_state, last_integration};

static N: AtomicUsize = AtomicUsize::new(0);

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

struct Home {
    home: PathBuf,
}

impl Home {
    fn new() -> Self {
        let home = std::env::temp_dir()
            .join(format!("vajra-state-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&home);
        let vajra = home.join(".claude/vajra");
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/campaigns"), &vajra.join("campaigns"));
        fs::write(vajra.join(".hmac-key"), "fixture-key-not-a-secret").unwrap();
        fs::create_dir_all(home.join("appdata")).unwrap();
        Self { home }
    }
    fn vajra(&self) -> PathBuf {
        self.home.join(".claude/vajra")
    }
    fn allow(&self) -> Allowlist {
        Allowlist::new(&self.home.join(".claude"), &self.home.join("appdata")).unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

const ACTIVE: &str = "deliver-20261101-0900";

#[test]
fn hmac_key_is_refused_directly_and_via_symlink() {
    let h = Home::new();
    let key = h.vajra().join(".hmac-key");
    assert!(matches!(h.allow().open_allowlisted(&key), Err(Denied::SecretKey)));
    // A symlink with an innocent name that points at the key.
    let link = h.vajra().join("campaigns").join(ACTIVE).join("progress-link.md");
    std::os::unix::fs::symlink(&key, &link).unwrap();
    assert!(matches!(h.allow().open_allowlisted(&link), Err(Denied::SecretKey)));
    // Even a path that does not exist is refused by name before any lookup.
    let ghost = h.home.join("nowhere/.hmac-key");
    assert!(matches!(h.allow().resolve(&ghost), Err(Denied::SecretKey)));
}

#[test]
fn hmac_key_is_never_opened_by_any_reader() {
    let h = Home::new();
    fs::write(h.vajra().join("audit.log"), "2026-11-05T00:00:00Z ALLOW Bash\n").unwrap();
    let allow = h.allow();
    let opened: RefCell<Vec<PathBuf>> = RefCell::new(Vec::new());
    // Walk every file under ~/.claude/vajra through the injected opener,
    // the way a hostile caller would.
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() { walk(&p, out) } else { out.push(p) }
        }
    }
    let mut all = Vec::new();
    walk(&h.vajra(), &mut all);
    for p in &all {
        let _ = allow.open_with(p, |real| {
            opened.borrow_mut().push(real.to_path_buf());
            Ok(())
        });
    }
    assert!(!opened.borrow().is_empty());
    assert!(opened.borrow().iter().all(|p| p.file_name().unwrap() != ".hmac-key"));

    // And the real readers, end to end.
    let mut r = Reader::new(allow);
    for slug in r.campaigns().unwrap() {
        let _ = r.campaign(&slug);
        let _ = r.tasks(&slug);
    }
    let _ = r.audit_tail();
}

#[test]
fn no_source_under_src_vajra_names_the_key_except_the_allowlist() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vajra");
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.file_name().unwrap() == "allowlist.rs" {
            continue;
        }
        let text = fs::read_to_string(&p).unwrap();
        assert!(!text.contains("hmac-key"), "{} mentions the key file", p.display());
    }
}

#[test]
fn dotdot_and_symlink_escape_are_rejected() {
    let h = Home::new();
    let allow = h.allow();
    let sneaky = h.vajra().join("campaigns").join(ACTIVE).join("../../audit.log");
    fs::write(h.vajra().join("audit.log"), "x\n").unwrap();
    assert!(matches!(allow.resolve(&sneaky), Err(Denied::Traversal)));

    let outside = h.home.join("outside.json");
    fs::write(&outside, "{}").unwrap();
    let link = h.vajra().join("campaigns").join(ACTIVE).join("tasks.json");
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert!(matches!(allow.resolve(&link), Err(Denied::NotAllowed)));
    // A campaign dir symlinked elsewhere does not show up in the listing.
    let elsewhere = h.home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, h.vajra().join("campaigns/deliver-link")).unwrap();
    assert!(!Reader::new(allow).campaigns().unwrap().contains(&"deliver-link".to_string()));
}

#[test]
fn only_the_section_8_paths_are_allowed() {
    let h = Home::new();
    let allow = h.allow();
    let c = h.vajra().join("campaigns").join(ACTIVE);
    for ok in ["campaign.json", "features.json", "tasks.json", "progress.md", "checkpoints/0001.json"] {
        assert!(allow.resolve(&c.join(ok)).is_ok(), "{ok}");
    }
    assert!(allow.resolve(&h.vajra().join("campaigns/.active")).is_ok());
    // Not on the list.
    assert!(matches!(allow.resolve(&c.join("state.sig")), Err(Denied::NotAllowed)));
    fs::create_dir_all(h.vajra().join("kb")).unwrap();
    fs::write(h.vajra().join("kb/x.json"), "{}").unwrap();
    assert!(matches!(allow.resolve(&h.vajra().join("kb/x.json")), Err(Denied::NotAllowed)));
    fs::write(h.home.join(".claude/settings.json"), "{}").unwrap();
    assert!(matches!(allow.resolve(&h.home.join(".claude/settings.json")), Err(Denied::NotAllowed)));
    // Outside ~/.claude entirely, and the app data dir is allowed.
    assert!(matches!(allow.resolve(Path::new("/etc/hosts")), Err(Denied::NotAllowed)));
    fs::write(h.home.join("appdata/runs.jsonl"), "").unwrap();
    assert!(allow.resolve(&h.home.join("appdata/runs.jsonl")).is_ok());
    // Transcripts and the workflow file.
    fs::create_dir_all(h.home.join(".claude/projects/-repo")).unwrap();
    fs::write(h.home.join(".claude/projects/-repo/s.jsonl"), "").unwrap();
    assert!(allow.resolve(&h.home.join(".claude/projects/-repo/s.jsonl")).is_ok());
    fs::create_dir_all(h.home.join(".claude/workflows")).unwrap();
    fs::write(h.home.join(".claude/workflows/vajra-deliver.js"), "").unwrap();
    assert!(allow.resolve(&allow.workflow_path()).is_ok());
}

#[test]
fn campaigns_root_lists_directories_only() {
    let h = Home::new();
    assert!(h.vajra().join("campaigns/5B4A3760-0000-4000-8000-000000000000.json").exists());
    let r = Reader::new(h.allow());
    assert_eq!(
        r.campaigns().unwrap(),
        vec![
            "deliver-20261011-1710",
            "deliver-20261101-0900",
            "deliver-badsig",
            "deliver-empty-worktree",
            "deliver-evidence-gone",
        ]
    );
}

#[test]
fn a_machine_with_no_campaigns_folder_lists_no_campaigns() {
    // First launch: the agent folder exists, but nothing has ever run, so `vajra/campaigns` is not there.
    let h = Home::new();
    fs::remove_dir_all(h.vajra().join("campaigns")).unwrap();
    assert_eq!(Reader::new(h.allow()).campaigns().unwrap(), Vec::<String>::new());
    // Nor is `vajra/` itself, before the engine is installed.
    let allow = h.allow();
    fs::remove_dir_all(h.vajra()).unwrap();
    assert_eq!(Reader::new(allow).campaigns().unwrap(), Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn a_campaigns_link_that_leads_nowhere_or_outside_is_still_refused() {
    let h = Home::new();
    let dir = h.vajra().join("campaigns");
    fs::remove_dir_all(&dir).unwrap();
    // Present but dangling: not "no campaigns yet".
    std::os::unix::fs::symlink(h.home.join("gone"), &dir).unwrap();
    assert!(Reader::new(h.allow()).campaigns().is_err());
    // Present and pointing outside the allowlist.
    fs::remove_file(&dir).unwrap();
    let outside = h.home.join("outside");
    fs::create_dir_all(outside.join("deliver-x")).unwrap();
    std::os::unix::fs::symlink(&outside, &dir).unwrap();
    assert!(Reader::new(h.allow()).campaigns().is_err());
}

#[test]
fn active_and_done_campaign_summaries() {
    let h = Home::new();
    let mut r = Reader::new(h.allow());
    let a = r.campaign(ACTIVE).unwrap();
    assert_eq!((a.status.as_str(), a.passing, a.total), ("active", 1, 2));
    assert_eq!(a.checkpoints, vec!["0001.json", "0002.json"]);
    let li = a.last_integration.unwrap();
    assert_eq!((li.branch.as_str(), li.sha.as_str(), li.from.as_str()), ("vajra/20261101-0900/t2", "def5678", "abc1234"));
    let d = r.campaign("deliver-20261011-1710").unwrap();
    assert_eq!((d.status.as_str(), d.passing, d.total), ("done", 2, 2));
    assert!(d.last_integration.is_none());
}

#[test]
fn invalid_signature_fixture_is_still_readable() {
    // The reader does not judge signatures; the bridge's `verify` does.
    let h = Home::new();
    assert_eq!(fs::read_to_string(h.vajra().join("campaigns/deliver-badsig/state.sig")).unwrap().trim(), "deadbeef");
    let c = Reader::new(h.allow()).campaign("deliver-badsig").unwrap();
    assert_eq!(c.total, 1);
}

#[test]
fn empty_worktree_is_preserved_as_empty() {
    let h = Home::new();
    let t = Reader::new(h.allow()).tasks("deliver-empty-worktree").value.unwrap();
    assert_eq!(t["tasks"][0]["worktree"], "");
}

#[test]
fn evidence_gone_is_file_gone_not_verified() {
    let h = Home::new();
    let t = Reader::new(h.allow()).tasks("deliver-evidence-gone").value.unwrap();
    let ev = &t["tasks"][0]["evidence"];
    assert_eq!(evidence_state(Some(ev), Some(true)), EvidenceState::FileGone);
    assert_eq!(evidence_state(None, None), EvidenceState::None);
    assert_eq!(evidence_state(Some(&json!(null)), None), EvidenceState::None);
    let here = json!({"path": env!("CARGO_MANIFEST_DIR")});
    assert_eq!(evidence_state(Some(&here), Some(true)), EvidenceState::Verified);
    assert_eq!(evidence_state(Some(&here), Some(false)), EvidenceState::Unverified);
}

#[test]
fn tasks_parse_error_keeps_last_good_snapshot() {
    let h = Home::new();
    let mut r = Reader::new(h.allow());
    let good = r.tasks(ACTIVE);
    assert!(!good.stale);
    let path = h.vajra().join("campaigns").join(ACTIVE).join("tasks.json");
    fs::write(&path, "{\"tasks\": [").unwrap(); // mid-replace
    let bad = r.tasks(ACTIVE);
    assert!(bad.stale);
    assert_eq!(bad.value, good.value);
    fs::write(&path, "{\"tasks\": []}").unwrap();
    let fresh = r.tasks(ACTIVE);
    assert!(!fresh.stale);
    assert_eq!(fresh.value.unwrap()["tasks"], json!([]));
    // Never seen a good one: no value, stale.
    fs::write(h.vajra().join("campaigns/deliver-badsig/tasks.json"), "nope").unwrap();
    let s = r.tasks("deliver-badsig");
    assert!(s.stale && s.value.is_none());
}

#[test]
fn progress_and_integration_parsing() {
    let h = Home::new();
    let r = Reader::new(h.allow());
    assert!(r.progress(ACTIVE).unwrap().contains("integration vajra/"));
    assert_eq!(last_integration("nothing here"), None);
    assert_eq!(last_integration("- ts integration b @ s from "), None);
}

#[test]
fn audit_tail_is_last_256_kib_in_whole_lines() {
    let h = Home::new();
    let line = format!("2026-11-05T00:00:00Z ALLOW Bash {}\n", "x".repeat(90));
    let mut big = String::new();
    while big.len() < 600 * 1024 {
        big.push_str(&line);
    }
    big.push_str("LAST-LINE\n");
    fs::write(h.vajra().join("audit.log"), &big).unwrap();
    let tail = Reader::new(h.allow()).audit_tail().unwrap();
    assert!(tail.len() <= 256 * 1024);
    assert!(tail.len() > 200 * 1024);
    assert!(tail.ends_with("LAST-LINE\n"));
    assert!(tail.starts_with("2026-11-05T00:00:00Z"), "first line must be whole");
    // Small file comes back whole.
    fs::write(h.vajra().join("audit.log"), "a\nb\n").unwrap();
    assert_eq!(Reader::new(h.allow()).audit_tail().unwrap(), "a\nb\n");
}

#[test]
fn missing_checkpoints_dir_is_an_empty_list() {
    let h = Home::new();
    assert!(Reader::new(h.allow()).checkpoints("deliver-badsig").unwrap().is_empty());
}
