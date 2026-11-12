//! 6h acceptance tests: fake engine scripts in a temp dir record their argv (one call per line, arguments separated by
//! the unit separator), in a temp HOME with a fake key the app can't read. Nothing touches the owner's home.

use super::*;
use crate::vajra::bridge::{KILL_GRACE, fnv32};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        let d = std::env::temp_dir().join(format!("vajra-record-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["scripts", "home/.claude/vajra/campaigns", "data", "cwd/.git/objects", "cwd/.git/refs"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        let d = d.canonicalize().unwrap();
        // A fake key the app process could not open even if it tried.
        let key = d.join("home/.claude/vajra/.hmac-key");
        std::fs::write(&key, "fake").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o000)).unwrap();
        let t = Tmp(d);
        t.campaign_script("");
        t.python_script("");
        t
    }
    fn log(&self) -> PathBuf {
        self.0.join("calls.log")
    }
    /// A fake `vajra-campaign.sh`: logs `cwd` and its argv, then runs `body` (which may print and `exit`).
    fn campaign_script(&self, body: &str) {
        self.script("scripts/vajra-campaign.sh", &format!("printf 'campaign\\037%s' \"$PWD\" >> '{log}'\nfor a in \"$@\"; do printf '\\037%s' \"$a\" >> '{log}'; done\nprintf '\\n' >> '{log}'\n{body}\n", log = self.log().display()));
    }
    /// A fake python: logs its argv (the first is `-I`, then the script path), then runs `body`.
    fn python_script(&self, body: &str) {
        self.script("python", &format!("printf 'python' >> '{log}'\nfor a in \"$@\"; do printf '\\037%s' \"$a\" >> '{log}'; done\nprintf '\\n' >> '{log}'\n{body}\n", log = self.log().display()));
    }
    fn script(&self, rel: &str, body: &str) {
        let p = self.0.join(rel);
        std::fs::write(&p, format!("#!/bin/bash\n{body}\nexit 0\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn bridge(&self) -> Bridge {
        let mut b = Bridge::new(self.0.join("scripts")).with_home(self.0.join("home"));
        b.python = self.0.join("python");
        b.record_cwd = Some(self.0.join("cwd"));
        b.timeout = Duration::from_secs(10);
        b
    }
    fn cfg(&self, resumed: bool) -> RecordConfig {
        RecordConfig { run: "20261112-1200".into(), slug: "deliver-20261112-1200".into(), trust: "guarded".into(), home: self.0.join("home"), resumed }
    }
    fn record(&self, resumed: bool) -> CoreRecord {
        CoreRecord::new(self.cfg(resumed), self.bridge(), mem()).unwrap()
    }
    /// Every call so far: the program kind, then its argv, with the script path shown as its file name.
    fn calls(&self) -> Vec<Vec<String>> {
        let s = std::fs::read_to_string(self.log()).unwrap_or_default();
        let scripts = self.0.join("scripts");
        s.lines()
            .map(|l| {
                l.split('\u{1f}')
                    .map(|a| a.strip_prefix(&format!("{}/", scripts.display())).unwrap_or(a).to_string())
                    .map(|a| if a == self.0.join("cwd").display().to_string() { "<record cwd>".into() } else { a })
                    .collect()
            })
            .collect()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(self.0.join("home/.claude/vajra/.hmac-key"), std::fs::Permissions::from_mode(0o600));
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The ledger in memory; `back` is what a read-back returns (none: the read-back fails).
#[derive(Default)]
struct MemStore {
    saved: Mutex<Option<Ledger>>,
    back: Mutex<Option<ReadBack>>,
    no_save: std::sync::atomic::AtomicBool,
}

impl LedgerStore for MemStore {
    fn load(&self) -> Result<Option<Ledger>, String> {
        Ok(crate::lock(&self.saved).clone())
    }
    fn save(&self, l: &Ledger) -> Result<(), String> {
        if self.no_save.load(Ordering::SeqCst) {
            return Err("disk full".into());
        }
        *crate::lock(&self.saved) = Some(l.clone());
        Ok(())
    }
    fn read_back(&self, _: &str) -> Result<ReadBack, String> {
        crate::lock(&self.back).clone().ok_or_else(|| "no read-back".into())
    }
    fn evidence_sha256(&self, p: &Path) -> Result<String, String> {
        Ok(crate::control::sha256::hex_digest(p.to_string_lossy().as_bytes()))
    }
}

fn mem() -> Arc<dyn LedgerStore> {
    Arc::new(MemStore::default())
}

fn v(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

const RUN: &str = "20261112-1200";
/// Long enough for bash to start and log under the OS sandbox; short next to the scripts' `sleep 30`.
const SHORT: Duration = Duration::from_secs(2);
const SLUG: &str = "deliver-20261112-1200";

fn plan() -> PlanRecord {
    PlanRecord {
        goal: "add a --json flag\nto export".into(),
        features: vec![Feature { id: "ac-1".into(), desc: "export has --json".into() }, Feature { id: "ac-2".into(), desc: "docs say so".into() }],
        tasks: vec![
            TaskRecord { id: format!("{RUN}.t1"), title: "do t1".into(), agent: "bhima".into(), deps: vec![], files: v(&["src/a/", "src/*.rs"]), criteria: v(&["ac-1"]), branch: format!("vajra/{RUN}/task-t1-r1") },
            TaskRecord { id: format!("{RUN}.t2"), title: "do t2".into(), agent: "abhimanyu".into(), deps: vec![format!("{RUN}.t1")], files: v(&["docs/"]), criteria: v(&["ac-2"]), branch: format!("vajra/{RUN}/task-t2-r1") },
        ],
    }
}

// ---------------------------------------------------------------- argv, in the workflow's order

#[test]
fn the_plan_creates_adds_and_validates_with_exactly_the_scripts_argv() {
    let t = Tmp::new();
    t.record(false).plan(&plan()).unwrap();
    let c = t.calls();
    assert_eq!(c[0], v(&["campaign", "<record cwd>", "create", SLUG, "--goal", "add a --json flag to export", "--feature", "ac-1|export has --json", "--feature", "ac-2|docs say so"]));
    let t1 = format!("{RUN}.t1");
    let t2 = format!("{RUN}.t2");
    let b1 = format!("vajra/{RUN}/task-t1-r1");
    let b2 = format!("vajra/{RUN}/task-t2-r1");
    assert_eq!(c[1], v(&["python", "-I", "vajra_tasks.py", "add", &t1, "--title", "do t1", "--owner", "bhima", "--files", "src/a/,src/*.rs", "--acceptance", "ac-1", "--branch", &b1, "--campaign", SLUG]));
    assert_eq!(c[2], v(&["python", "-I", "vajra_tasks.py", "add", &t2, "--title", "do t2", "--owner", "abhimanyu", "--deps", &t1, "--files", "docs/", "--acceptance", "ac-2", "--branch", &b2, "--campaign", SLUG]));
    assert_eq!(c[3], v(&["python", "-I", "vajra_tasks.py", "validate", "--campaign", SLUG]));
    assert_eq!(c.len(), 4);
}

#[test]
fn events_run_in_order_with_exactly_the_scripts_argv() {
    let t = Tmp::new();
    let r = t.record(false);
    let id = format!("{RUN}.t1");
    r.send(Event::Claim { task: id.clone(), agent: "bhima".into(), worktree: Some(PathBuf::from("/wt/one")) });
    r.send(Event::Report { task: id.clone(), status: TaskStatus::Question, summary: "which\tformat?".into() });
    r.send(Event::Progress(format!("run {RUN} round 1: waiting for the user on t1; no integration yet")));
    r.send(Event::Checkpoint("stuck after 3 rounds".into()));
    r.flush();
    let c = t.calls();
    assert_eq!(c[0], v(&["python", "-I", "vajra_tasks.py", "claim", &id, "bhima", "--stacked", "--campaign", SLUG, "--worktree", "/wt/one"]));
    assert_eq!(c[1], v(&["python", "-I", "vajra_tasks.py", "report", &id, "in_progress", "--campaign", SLUG]));
    assert_eq!(c[2], v(&["python", "-I", "vajra_tasks.py", "report", &id, "question", "--campaign", SLUG, "--summary", "which format?"]));
    let line = format!("run {RUN} round 1: waiting for the user on t1; no integration yet");
    assert_eq!(c[3], v(&["campaign", "<record cwd>", "progress", &line, "--campaign", SLUG]));
    assert_eq!(c[4], v(&["campaign", "<record cwd>", "checkpoint", "stuck after 3 rounds", "--campaign", SLUG]));
    assert!(r.card().notes.is_empty(), "{:?}", r.card().notes);
}

#[test]
fn the_six_i_variants_build_the_workflows_argv() {
    let t = Tmp::new();
    let b = t.bridge();
    let plan_of = |r: Request| {
        let (p, a, cwd) = b.plan(&r).unwrap();
        (p.file_name().unwrap().to_string_lossy().into_owned(), a.into_iter().map(|x| x.replace(&format!("{}/", t.0.join("scripts").display()), "")).collect::<Vec<_>>(), cwd)
    };
    let (p, a, cwd) = plan_of(Request::CampaignPass { slug: SLUG.into(), feature: "ac-1".into(), evidence: "cargo test (exit 0)".into() });
    assert_eq!((p.as_str(), a, cwd), ("vajra-campaign.sh", v(&["pass", "ac-1", "--evidence", "cargo test (exit 0)", "--campaign", SLUG]), Some(t.0.join("cwd"))));
    assert_eq!(plan_of(Request::CampaignClose { slug: SLUG.into(), status: CloseStatus::Done }).1, v(&["close", SLUG, "--status", "done"]));
    let b64 = "IyByZXBvcnQK";
    let (_, a, _) = plan_of(Request::CampaignReport { slug: SLUG.into(), bytes: 9, check: fnv32(b64), base64: b64.into() });
    assert_eq!(a, v(&["report", "--campaign", SLUG, "--bytes", "9", "--check", &fnv32(b64), "--base64", b64]));
    assert_eq!(plan_of(Request::CampaignUnlockStale { slug: SLUG.into() }).1, v(&["unlock", SLUG, "--if-stale"]));
    let h = "a".repeat(40);
    let (_, a, cwd) = plan_of(Request::TasksClose {
        slug: SLUG.into(),
        id: format!("{RUN}.t1"),
        evidence: PathBuf::from("/data/walkthroughs/x/evidence.json"),
        head: h.clone(),
        commit: "b".repeat(40),
        branch: format!("vajra/{RUN}/task-t1-r1"),
        base_commit: "c".repeat(40),
        changed: v(&["src/a.rs", "src/b c.rs"]),
        repo: PathBuf::from("/repo"),
    });
    let id = format!("{RUN}.t1");
    let br = format!("vajra/{RUN}/task-t1-r1");
    let (bb, cc) = ("b".repeat(40), "c".repeat(40));
    assert_eq!(a, v(&["-I", "vajra_tasks.py", "close", &id, "--evidence", "/data/walkthroughs/x/evidence.json", "--head", &h, "--commit", &bb, "--branch", &br, "--base-commit", &cc, "--changed", "src/a.rs,src/b c.rs", "--json", "--campaign", SLUG]));
    assert_eq!(cwd, Some(PathBuf::from("/repo")), "close runs in the repository, as write-back does");
    assert_eq!(plan_of(Request::TasksDep { slug: SLUG.into(), id: id.clone(), dep: format!("{RUN}.t0"), remove: true }).1, v(&["-I", "vajra_tasks.py", "dep", &id, &format!("{RUN}.t0"), "--remove", "--campaign", SLUG]));
}

#[test]
fn fnv32_matches_the_workflow_and_the_script() {
    // fnv("") and fnv("a") from the FNV-1a 32 reference.
    assert_eq!(fnv32(""), "811c9dc5");
    assert_eq!(fnv32("a"), "e40c292c");
}

// ---------------------------------------------------------------- refusals: nothing is spawned

#[test]
fn every_variant_refuses_a_value_outside_its_regex_or_cap_and_spawns_nothing() {
    let t = Tmp::new();
    let b = t.bridge();
    let id = format!("{RUN}.t1");
    let add = |f: &dyn Fn(&mut Request)| {
        let mut r = Request::TasksAdd { slug: SLUG.into(), id: id.clone(), title: "t".into(), owner: "bhima".into(), deps: vec![], files: v(&["src/"]), acceptance: vec![], branch: "vajra/x".into() };
        f(&mut r);
        r
    };
    let mut bad: Vec<Request> = vec![
        Request::CampaignCreate { slug: "Deliver-1".into(), goal: "g".into(), features: vec![Feature { id: "a".into(), desc: "d".into() }] },
        Request::CampaignCreate { slug: SLUG.into(), goal: "--goal".into(), features: vec![Feature { id: "a".into(), desc: "d".into() }] },
        Request::CampaignCreate { slug: SLUG.into(), goal: "g".repeat(1001), features: vec![Feature { id: "a".into(), desc: "d".into() }] },
        Request::CampaignCreate { slug: SLUG.into(), goal: "g".into(), features: vec![] },
        Request::CampaignCreate { slug: SLUG.into(), goal: "g".into(), features: vec![Feature { id: "-a".into(), desc: "d".into() }] },
        Request::CampaignCreate { slug: SLUG.into(), goal: "g".into(), features: vec![Feature { id: "a".into(), desc: "line\nbreak".into() }] },
        Request::CampaignCreate { slug: SLUG.into(), goal: "g\u{202e}".into(), features: vec![Feature { id: "a".into(), desc: "d".into() }] },
        Request::CampaignCreate { slug: SLUG.into(), goal: "g\u{85}".into(), features: vec![Feature { id: "a".into(), desc: "d".into() }] },
        Request::CampaignShow { slug: "../x".into() },
        Request::CampaignPass { slug: SLUG.into(), feature: "a b".into(), evidence: "e".into() },
        Request::CampaignPass { slug: SLUG.into(), feature: "a".into(), evidence: "e".repeat(301) },
        Request::CampaignCheckpoint { slug: SLUG.into(), label: "".into() },
        Request::CampaignCheckpoint { slug: SLUG.into(), label: "--campaign".into() },
        Request::CampaignClose { slug: "".into(), status: CloseStatus::Done },
        Request::CampaignReport { slug: SLUG.into(), bytes: 9, check: "00000000".into(), base64: "IyByZXBvcnQK".into() },
        Request::CampaignReport { slug: SLUG.into(), bytes: 0, check: fnv32("IyByZXBvcnQK"), base64: "IyByZXBvcnQK".into() },
        Request::CampaignReport { slug: SLUG.into(), bytes: 9, check: fnv32("not base64!"), base64: "not base64!".into() },
        Request::CampaignUnlockStale { slug: "a/b".into() },
        add(&|r| if let Request::TasksAdd { owner, .. } = r { *owner = "Bhima".into() }),
        add(&|r| if let Request::TasksAdd { id, .. } = r { *id = "x".repeat(65) }),
        add(&|r| if let Request::TasksAdd { files, .. } = r { *files = v(&["../etc"]) }),
        add(&|r| if let Request::TasksAdd { files, .. } = r { *files = v(&["/abs"]) }),
        add(&|r| if let Request::TasksAdd { files, .. } = r { *files = v(&["a,b"]) }),
        add(&|r| if let Request::TasksAdd { files, .. } = r { *files = vec![] }),
        add(&|r| if let Request::TasksAdd { deps, .. } = r { *deps = v(&["a,b"]) }),
        add(&|r| if let Request::TasksAdd { branch, .. } = r { *branch = "-b".into() }),
        add(&|r| if let Request::TasksAdd { title, .. } = r { *title = "two  spaces".into() }),
        add(&|r| if let Request::TasksAdd { acceptance, .. } = r { *acceptance = v(&["c 1"]) }),
        Request::TasksDep { slug: SLUG.into(), id: id.clone(), dep: "--remove".into(), remove: false },
        Request::TasksValidate { slug: "UPPER".into() },
        Request::TasksClaim { slug: SLUG.into(), id: id.clone(), worker: "w1 seat".into(), worktree: None },
        Request::TasksClaim { slug: SLUG.into(), id: id.clone(), worker: "bhima".into(), worktree: Some(PathBuf::from("relative/wt")) },
        Request::TasksReport { slug: SLUG.into(), id: id.clone(), status: TaskStatus::Done, summary: Some("s".repeat(501)) },
        Request::TasksReport { slug: SLUG.into(), id: "".into(), status: TaskStatus::Done, summary: None },
    ];
    let close = |f: &dyn Fn(&mut Request)| {
        let mut r = Request::TasksClose {
            slug: SLUG.into(), id: id.clone(), evidence: "/e.json".into(), head: "a".repeat(40), commit: "b".repeat(40), branch: "vajra/x".into(), base_commit: "c".repeat(40), changed: vec![], repo: "/repo".into(),
        };
        f(&mut r);
        r
    };
    bad.push(close(&|r| if let Request::TasksClose { head, .. } = r { *head = "a".repeat(39) }));
    bad.push(close(&|r| if let Request::TasksClose { commit, .. } = r { *commit = "XYZ1234".into() }));
    bad.push(close(&|r| if let Request::TasksClose { evidence, .. } = r { *evidence = "e.json".into() }));
    bad.push(close(&|r| if let Request::TasksClose { changed, .. } = r { *changed = v(&["a,b"]) }));
    bad.push(close(&|r| if let Request::TasksClose { repo, .. } = r { *repo = "repo".into() }));
    for r in &bad {
        assert!(matches!(b.run(r), Err(BridgeError::Invalid(_))), "accepted {r:?}");
    }
    assert!(t.calls().is_empty(), "a refused request spawns nothing: {:?}", t.calls());
}

#[test]
fn a_campaign_call_with_no_record_cwd_is_refused() {
    let t = Tmp::new();
    let mut b = t.bridge();
    b.record_cwd = None;
    assert!(matches!(b.run(&Request::CampaignShow { slug: SLUG.into() }), Err(BridgeError::Invalid(_))));
    assert!(CoreRecord::new(t.cfg(false), b, mem()).is_err());
    assert!(t.calls().is_empty());
}

#[test]
fn the_slug_must_be_the_one_the_evidence_trust_names() {
    let t = Tmp::new();
    let mut cfg = t.cfg(false);
    cfg.trust = "outside_guard".into();
    assert!(CoreRecord::new(cfg.clone(), t.bridge(), mem()).is_err(), "an outside_guard run never writes an unmarked campaign");
    cfg.slug = format!("{SLUG}-og");
    let og = CoreRecord::new(cfg, t.bridge(), mem()).unwrap();
    assert!(!og.card().tamper_evident, "an outside_guard record is never called tamper-evident");
    assert!(t.record(false).card().tamper_evident);
}

#[test]
fn the_cleaner_gives_what_the_checker_accepts() {
    for s in ["  --lead\ttext\r\nwith\u{202e} odd\u{85}  bits  ", "-", "", &"é".repeat(400)] {
        let c = bridge_text::one_line(s, 200);
        assert!(bridge_text::text(&c, 200, false).is_ok(), "{s:?} → {c:?}");
    }
    assert_eq!(bridge_text::one_line("  --lead\ttext ", 200), "lead text");
}

// ---------------------------------------------------------------- the key and the store files

#[test]
fn the_record_works_with_a_key_file_the_app_cannot_read() {
    let t = Tmp::new();
    let key = t.0.join("home/.claude/vajra/.hmac-key");
    assert!(std::fs::read(&key).is_err() || unsafe { libc::geteuid() } == 0, "the fake key is unreadable to this process");
    let r = t.record(false);
    r.plan(&plan()).unwrap();
    r.send(Event::Progress("p".into()));
    r.flush();
    assert!(r.card().notes.is_empty());
    assert_eq!(std::fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0, "nothing changed the key's mode");
}

#[test]
fn the_record_modules_open_no_file_themselves() {
    for (name, src) in [("desktop/record.rs", include_str!("record.rs")), ("vajra/bridge_record.rs", include_str!("../vajra/bridge_record.rs"))] {
        let code: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for banned in ["std::fs", "File::", "OpenOptions", "read_to_string", "hmac-key", ".claude/vajra/.", "campaign.json", "tasks.json\""] {
            assert!(!code.contains(banned), "{name} uses {banned}: the record reaches the store only through the scripts");
        }
    }
}

// ---------------------------------------------------------------- failure table rows

#[test]
fn a_failed_create_add_or_validate_fails_the_plan_with_the_first_error_line() {
    for (campaign, python, want) in [
        ("[ \"$1\" = create ] && { echo 'vajra-campaign: cannot create dir' >&2; exit 1; }", "", "vajra-campaign: cannot create dir"),
        ("", "[ \"$3\" = add ] && { echo 'vajra_tasks: invalid owner agent' >&2; exit 2; }", "vajra_tasks: invalid owner agent"),
        ("", "[ \"$3\" = validate ] && { echo 'vajra_tasks: task x: unknown agent bhima (not in ~/.claude/agents)' >&2; exit 2; }", "unknown agent bhima"),
    ] {
        let t = Tmp::new();
        t.campaign_script(campaign);
        t.python_script(python);
        let err = t.record(false).plan(&plan()).unwrap_err();
        assert!(err.contains(want), "{err}");
    }
}

#[test]
fn a_best_effort_write_is_retried_once_then_noted() {
    let t = Tmp::new();
    t.python_script("[ \"$3\" = claim ] && { echo 'vajra_tasks: task x is pending, not ready' >&2; exit 3; }");
    let r = t.record(false);
    r.send(Event::Claim { task: format!("{RUN}.t1"), agent: "bhima".into(), worktree: None });
    r.flush();
    let claims = t.calls().iter().filter(|c| c.get(3).is_some_and(|a| a == "claim")).count();
    assert_eq!(claims, 2, "one retry");
    assert_eq!(r.card().notes, vec!["the run record's claim failed: vajra_tasks: task x is pending, not ready".to_string()]);
}

#[test]
fn a_timed_out_write_is_never_retried_and_a_broken_signature_is_on_the_card() {
    let t = Tmp::new();
    t.campaign_script("case \"$1\" in progress) sleep 30 ;; verify) echo 'deliver: signature INVALID' >&2; exit 3 ;; esac");
    let mut b = t.bridge();
    b.timeout = SHORT;
    let store = Arc::new(MemStore::default());
    let r = CoreRecord::new(t.cfg(false), b, store.clone()).unwrap();
    r.send(Event::Progress("p".into()));
    r.flush();
    let saved = crate::lock(&store.saved).clone().unwrap();
    assert!(saved.uncertain.is_some() && saved.pending.is_none(), "a timed-out write's outcome is unknown for good: {saved:?}");
    let kinds: Vec<String> = t.calls().iter().map(|c| c[2].clone()).collect();
    // Under load bash may be killed before it logs its first line, so the one progress call may not show.
    assert!(kinds.iter().filter(|k| *k == "progress").count() <= 1, "no retry after a timeout: {kinds:?}");
    assert_eq!(kinds.last().map(String::as_str), Some("verify"), "verify instead: {kinds:?}");
    assert!(r.card().signature_broken);
}

// ---------------------------------------------------------------- idempotent create

fn show_json(goal: &str, features: &[(&str, &str, bool)], status: &str, sig: bool) -> String {
    let f: Vec<Value> = features.iter().map(|(i, d, p)| serde_json::json!({"id": i, "description": d, "passes": p})).collect();
    serde_json::json!({"campaign": {"slug": SLUG, "goal": goal, "status": status}, "features": f, "signature_valid": sig}).to_string()
}

fn with_existing(t: &Tmp, show: &str) {
    let file = t.0.join("show.json");
    std::fs::write(&file, show).unwrap();
    t.campaign_script(&format!(
        "case \"$1\" in create) echo \"vajra-campaign: campaign already exists: $2\" >&2; exit 1 ;; show) cat '{}' ;; esac",
        file.display()
    ));
}

/// The ledger the first run left after its `create`, and the campaign read back as it is: what a resume reuses.
fn resumable() -> Arc<dyn LedgerStore> {
    let mut l = Ledger::new(SLUG);
    l.apply(&Request::CampaignCreate { slug: SLUG.into(), goal: "add a --json flag to export".into(), features: plan().features });
    let s = MemStore::default();
    *crate::lock(&s.saved) = Some(l);
    *crate::lock(&s.back) = Some(ReadBack {
        campaign: serde_json::json!({"slug": SLUG, "goal": "add a --json flag to export", "status": "active"}),
        features: serde_json::json!({"features": [{"id": "ac-1", "description": "export has --json", "passes": false}, {"id": "ac-2", "description": "docs say so", "passes": false}]}),
        progress: "## Progress\n- 2026-11-12T12:00:00Z created with 2 feature(s)\n".into(),
        tasks: serde_json::json!({"tasks": []}),
    });
    Arc::new(s)
}

#[test]
fn a_second_create_reuses_the_campaign_only_for_a_resume_that_matches_exactly() {
    let feats = [("ac-1", "export has --json", false), ("ac-2", "docs say so", false)];
    let t = Tmp::new();
    with_existing(&t, &show_json("add a --json flag to export", &feats, "active", true));
    CoreRecord::new(t.cfg(true), t.bridge(), resumable()).unwrap().plan(&plan()).unwrap();
    let kinds: Vec<String> = t.calls().iter().map(|c| if c[0] == "campaign" { c[2].clone() } else { c[3].clone() }).collect();
    assert_eq!(kinds, v(&["create", "show", "add", "add", "validate"]), "reused, then the adds (unchanged) and validate");

    let t = Tmp::new();
    with_existing(&t, &show_json("add a --json flag to export", &feats, "active", true));
    let err = t.record(false).plan(&plan()).unwrap_err();
    assert!(err.contains("didn't create it"), "a fresh run never adopts an existing campaign: {err}");

    for (goal, feats, status, sig) in [
        ("another goal", feats.to_vec(), "active", true),
        ("add a --json flag to export", vec![feats[1], feats[0]], "active", true),
        ("add a --json flag to export", vec![feats[0]], "active", true),
        ("add a --json flag to export", vec![feats[0], ("ac-2", "docs say so", true)], "active", true),
        ("add a --json flag to export", feats.to_vec(), "done", true),
        ("add a --json flag to export", feats.to_vec(), "active", false),
    ] {
        let t = Tmp::new();
        with_existing(&t, &show_json(goal, &feats, status, sig));
        assert!(CoreRecord::new(t.cfg(true), t.bridge(), resumable()).unwrap().plan(&plan()).is_err(), "{goal} {feats:?} {status} {sig}");
        assert!(!t.calls().iter().any(|c| c.get(3).is_some_and(|a| a == "add")), "nothing is added to a campaign that isn't this run's");
    }
}

// ---------------------------------------------------------------- locks

#[test]
fn a_held_lock_shows_its_path_and_repair_and_the_core_never_removes_it() {
    let t = Tmp::new();
    let lock = t.0.join("home/.claude/vajra/campaigns").join(SLUG).join(".lock");
    std::fs::create_dir_all(&lock).unwrap();
    t.campaign_script("case \"$1\" in progress) echo \"vajra-campaign: campaign is locked (/elsewhere/.lock); remove it if no other run is active\" >&2; exit 1 ;; unlock) exit 2 ;; esac");
    let r = t.record(false);
    r.send(Event::Progress("p".into()));
    r.flush();
    let card = r.card().lock.expect("a lock card");
    assert_eq!(card.path, lock.display().to_string(), "the path comes from HOME and the stored slug, never from script output");
    assert_eq!(card.text, LOCK_TEXT);
    assert!(card.repair);
    assert!(lock.is_dir(), "the core never removes a lock");
    let after = r.repair();
    assert!(!after.lock.unwrap().repair, "an engine without unlock --if-stale (usage error) keeps the manual text");
    assert!(t.calls().iter().any(|c| c[2..] == v(&["unlock", SLUG, "--if-stale"])));
    assert!(lock.is_dir());
}

#[test]
fn repair_clears_the_card_when_the_script_removed_a_stale_lock() {
    let t = Tmp::new();
    t.campaign_script("case \"$1\" in progress) echo 'vajra-campaign: campaign is locked (x)' >&2; exit 1 ;; esac");
    let r = t.record(false);
    r.send(Event::Progress("p".into()));
    r.flush();
    assert!(r.card().lock.is_some());
    assert!(r.repair().lock.is_none());
}

#[test]
fn a_sigtermed_campaign_write_frees_its_lock_in_the_grace_period() {
    let t = Tmp::new();
    let lock = t.0.join("lockdir");
    t.campaign_script(&format!("mkdir '{l}'; trap 'rmdir \"{l}\"' EXIT; sleep 30 & wait", l = lock.display()));
    let mut b = t.bridge();
    b.timeout = SHORT;
    let started = std::time::Instant::now();
    let o = b.run(&Request::CampaignProgress { slug: SLUG.into(), text: "p".into() }).unwrap();
    assert!(o.timed_out);
    assert!(!lock.exists(), "the EXIT trap ran: SIGTERM before SIGKILL");
    assert!(started.elapsed() < SHORT + KILL_GRACE, "a script that exits on SIGTERM doesn't wait out the grace");
}

#[test]
fn a_read_still_gets_sigkill_at_once() {
    let t = Tmp::new();
    let lock = t.0.join("lockdir");
    t.campaign_script(&format!("mkdir '{l}'; trap 'rmdir \"{l}\"' EXIT; sleep 30 & wait", l = lock.display()));
    let mut b = t.bridge();
    b.timeout = SHORT;
    let o = b.run(&Request::CampaignShow { slug: SLUG.into() }).unwrap();
    assert!(o.timed_out);
    assert!(lock.exists(), "show is a read: no grace (and no lock in the real script)");
}

// ---------------------------------------------------------------- bhishma L2 to L4

#[test]
fn l2_a_record_dir_holding_anything_but_head_objects_and_refs_is_refused() {
    let t = Tmp::new();
    let data = t.0.join("data");
    assert!(super::super::record_app::record_dir(&data).is_ok(), "a fresh one is made");
    assert!(super::super::record_app::record_dir(&data).is_ok(), "and reused as it is");
    for planted in ["config", "commondir", "hooks"] {
        let t = Tmp::new();
        let data = t.0.join("data");
        let git = super::super::record_app::record_dir(&data).unwrap().join(".git");
        if planted == "hooks" { std::fs::create_dir(git.join(planted)).unwrap() } else { std::fs::write(git.join(planted), "x").unwrap() }
        assert!(super::super::record_app::record_dir(&data).is_err(), "a planted {planted} is refused");
    }
    let t = Tmp::new();
    let data = t.0.join("data");
    let dir = super::super::record_app::record_dir(&data).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(super::super::record_app::record_dir(&data).is_ok());
    assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700, "an open mode is closed again");
    let t = Tmp::new();
    let data = t.0.join("data");
    let git = super::super::record_app::record_dir(&data).unwrap().join(".git");
    std::fs::write(git.join("HEAD"), "ref: refs/heads/other\n").unwrap();
    assert!(super::super::record_app::record_dir(&data).is_err(), "a changed HEAD is refused");
}

#[test]
fn l3_a_write_the_script_refuses_for_a_broken_signature_marks_the_card_at_once() {
    let t = Tmp::new();
    t.campaign_script("[ \"$1\" = progress ] && { echo \"vajra-campaign: state signature INVALID for 'x' (edited outside vajra-campaign.sh?).\" >&2; exit 3; }");
    let r = t.record(false);
    r.send(Event::Progress("p".into()));
    r.flush();
    assert!(r.card().signature_broken, "exit 3 / INVALID is a broken record, not just a note");
    assert_eq!(t.calls().iter().filter(|c| c[2] == "progress").count(), 1, "a broken record isn't retried");
}

#[test]
fn l4_the_bridge_checks_progress_text_itself() {
    let t = Tmp::new();
    let b = t.bridge();
    for bad in ["--campaign", "two  spaces", "line\nbreak", "bidi\u{202e}", ""] {
        assert!(matches!(b.run(&Request::CampaignProgress { slug: SLUG.into(), text: bad.into() }), Err(BridgeError::Invalid(_))), "{bad:?}");
    }
    assert!(matches!(b.run(&Request::CampaignProgress { slug: SLUG.into(), text: "x".repeat(301) }), Err(BridgeError::Invalid(_))));
    assert!(t.calls().is_empty());
}

// ---------------------------------------------------------------- orphans

#[test]
fn an_orphan_is_a_core_runs_campaign_still_active_and_only_that_closes() {
    let t = Tmp::new();
    let home = t.0.join("home");
    let scripts = home.join(".claude/skills/vajra/scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let log = t.log();
    // `show` answers active for the -og run's campaign and done for the other; every call is logged.
    std::fs::write(
        scripts.join("vajra-campaign.sh"),
        format!(
            "#!/bin/bash\nprintf '%s\\037' \"$@\" >> '{}'; printf '\\n' >> '{}'\ncase \"$1 $2\" in\n  'show deliver-20261101-0900-og') echo '{{\"campaign\": {{\"status\": \"active\"}}}}' ;;\n  'show deliver-20261101-1000') echo '{{\"campaign\": {{\"status\": \"done\"}}}}' ;;\nesac\nexit 0\n",
            log.display(),
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(scripts.join("vajra-campaign.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let data = t.0.join("data");
    crate::desktop::test_project(&data, &home, &t.0.join("repo"));
    let app = crate::desktop::App::open(&data, &home).unwrap();
    for (run, slug, trust) in [("20261101-0900", "deliver-20261101-0900-og", "outside_guard"), ("20261101-1000", "deliver-20261101-1000", "guarded")] {
        app.record_run_seats(run, &serde_json::json!({"campaign_slug": slug, "evidence_trust": trust})).unwrap();
    }
    let o = app.record_orphans(&[]);
    assert_eq!(o["orphans"], serde_json::json!([{"run_id": "20261101-0900", "campaign": "deliver-20261101-0900-og"}]));
    assert_eq!(app.record_orphans(&["20261101-0900".into()])["orphans"], serde_json::json!([]), "a live run is no orphan");
    let shown: Vec<String> = std::fs::read_to_string(&log).unwrap().lines().filter(|l| l.starts_with("show")).map(str::to_string).collect();
    assert!(shown.iter().all(|l| l.contains("deliver-20261101-0900-og") || l.contains("deliver-20261101-1000")), "only slugs a core run recorded: {shown:?}");
    assert!(app.record_close_orphan("20261101-1000", &[]).is_err(), "a done campaign isn't an orphan");
    assert!(app.record_close_orphan("20261101-0900", &[]).is_ok());
    let closes: Vec<String> = std::fs::read_to_string(&log).unwrap().lines().filter(|l| l.starts_with("close")).map(str::to_string).collect();
    assert_eq!(closes, vec!["close\u{1f}deliver-20261101-0900-og\u{1f}--status\u{1f}abandoned\u{1f}".to_string()]);
    let rec = app.run_record("20261101-0900", true).unwrap();
    assert_eq!(rec.card().campaign, "deliver-20261101-0900-og", "a resume binds to the stored -og slug, never recomputed");
}

#[test]
fn a_run_going_on_now_is_never_an_orphan_through_the_ui_commands() {
    let t = Tmp::new();
    let home = t.0.join("home");
    let scripts = home.join(".claude/skills/vajra/scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let log = t.log();
    std::fs::write(scripts.join("vajra-campaign.sh"), format!("#!/bin/bash\nprintf '%s ' \"$@\" >> '{}'; printf '\\n' >> '{}'\n[ \"$1\" = show ] && echo '{{\"campaign\": {{\"status\": \"active\"}}}}'\nexit 0\n", log.display(), log.display())).unwrap();
    std::fs::set_permissions(scripts.join("vajra-campaign.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let data = t.0.join("data");
    let project = crate::desktop::test_project(&data, &home, &t.0.join("repo"));
    let app = crate::desktop::App::open(&data, &home).unwrap();
    let run = "20261112-1300";
    app.record_run_seats(run, &serde_json::json!({"campaign_slug": format!("deliver-{run}"), "evidence_trust": "guarded"})).unwrap();
    // The run's process is going on now, as `start_run` registers it.
    let stream = "run-0f8fad5b-d9cb-469f-a165-70867728950e";
    let spec = crate::sessions::SpawnSpec { argv: vec!["sleep".into(), "30".into()], cwd: t.0.clone(), rows: 24, cols: 80, redact: crate::crew::redact::Redactor::default(), extra_env: Vec::new() };
    app.sup.spawn(stream, spec).unwrap();
    crate::desktop::lock(&app.runs).insert(
        stream.into(),
        crate::desktop::Run { session_id: "0f8fad5b-d9cb-469f-a165-70867728950e".into(), run_id: run.into(), campaign: format!("deliver-{run}"), cwd: t.0.clone(), nonces: crate::crew::redact::Redactor::default() },
    );
    // What the Tauri commands call (`record_orphans(&[])`, `record_close_orphan` under `act_close_orphan`).
    assert_eq!(app.record_orphans(&[])["orphans"], serde_json::json!([]), "a live run is no orphan");
    assert!(app.record_close_orphan(run, &[]).is_err(), "closing a live run's campaign is refused");
    let env = crate::desktop::Envelope { action_id: "4b5d7e1a-6c2f-4d3e-9f80-1a2b3c4d5e6f".into(), project_id: project, expect_generation: None };
    let r = app.act_close_orphan(&env, run);
    assert_ne!(r["receipt"]["state"], "done", "{r}");
    assert!(!std::fs::read_to_string(&log).unwrap_or_default().contains("close "), "no close ran on a live campaign");
    app.sup.kill(stream).unwrap();
    // Once it has ended, it is one.
    for _ in 0..100 {
        if !app.sup.is_running(stream).unwrap_or(false) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(app.record_orphans(&[])["orphans"], serde_json::json!([{"run_id": run, "campaign": format!("deliver-{run}")}]));
}

/// claude on #190: a core run that has recorded its plan and has no ticket (before dispatch, during the gate, while
/// the lead plans) is going on, not an orphan, until its protocol finalizes.
#[test]
fn a_core_run_between_tickets_is_never_an_orphan_until_its_protocol_ends() {
    let t = Tmp::new();
    let home = t.0.join("home");
    let scripts = home.join(".claude/skills/vajra/scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let log = t.log();
    std::fs::write(scripts.join("vajra-campaign.sh"), format!("#!/bin/bash\nprintf '%s ' \"$@\" >> '{}'; printf '\\n' >> '{}'\n[ \"$1\" = show ] && echo '{{\"campaign\": {{\"status\": \"active\"}}}}'\nexit 0\n", log.display(), log.display())).unwrap();
    std::fs::set_permissions(scripts.join("vajra-campaign.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(scripts.join("vajra_tasks.py"), "import sys\nsys.exit(0)\n").unwrap();
    let data = t.0.join("data");
    crate::desktop::test_project(&data, &home, &t.0.join("repo"));
    let app = crate::desktop::App::open(&data, &home).unwrap();
    let run = "20261112-1400";
    app.record_run_seats(run, &serde_json::json!({"campaign_slug": format!("deliver-{run}"), "evidence_trust": "guarded"})).unwrap();
    let rec = app.run_record(run, false).unwrap();
    rec.plan(&plan()).unwrap();
    assert!(!app.workers.run_has_live_tickets(run), "the run has no ticket");
    assert_eq!(app.record_orphans(&[])["orphans"], serde_json::json!([]), "a run between tickets is no orphan");
    assert!(app.record_close_orphan(run, &[]).is_err());
    assert!(!std::fs::read_to_string(&log).unwrap_or_default().contains("close "), "no close ran on a live run's campaign");
    // Its protocol finalized: now it is one.
    rec.end();
    assert_eq!(app.record_orphans(&[])["orphans"], serde_json::json!([{"run_id": run, "campaign": format!("deliver-{run}")}]));
}

// ---------------------------------------------------------------- the real scripts (S8), when the machine has them

/// The engine's own scripts in a temp HOME, with the scrubbed env and no Claude session. Skipped where the script's
/// tools are missing (`/usr/bin/jq` and `/usr/bin/openssl`).
#[test]
fn the_real_scripts_create_reuse_and_record_from_the_cores_own_cwd() {
    if !Path::new("/usr/bin/jq").is_file() || !Path::new("/usr/bin/openssl").is_file() || !Path::new("/usr/bin/python3").is_file() {
        eprintln!("skipped: the engine needs /usr/bin/jq, /usr/bin/openssl and /usr/bin/python3");
        return;
    }
    let t = Tmp::new();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
    let mut b = Bridge::new(repo.canonicalize().unwrap()).with_home(t.0.join("home"));
    // The real scripts read and make the key: a readable fake one in the temp HOME.
    let key = t.0.join("home/.claude/vajra/.hmac-key");
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&key, "00".repeat(32)).unwrap();
    std::fs::create_dir_all(t.0.join("home/.claude/agents")).unwrap();
    for a in ["bhima", "abhimanyu"] {
        std::fs::write(t.0.join("home/.claude/agents").join(format!("{a}.md")), "---\nname: x\n---\n").unwrap();
    }
    b.record_cwd = Some(super::super::record_app::record_dir(&t.0.join("data")).unwrap());
    b.timeout = RECORD_TIMEOUT;
    let store = || -> Arc<dyn LedgerStore> {
        Arc::new(super::super::record_app::AppLedgerStore::new(&t.0.join("data"), SLUG, t.0.join("home/.claude/vajra/campaigns"), key.clone(), b.clone()).unwrap())
    };
    let r = CoreRecord::new(t.cfg(false), b.clone(), store()).unwrap();
    r.plan(&plan()).unwrap();
    r.send(Event::Claim { task: format!("{RUN}.t1"), agent: "bhima".into(), worktree: Some(PathBuf::from("/wt/one")) });
    r.send(Event::Report { task: format!("{RUN}.t1"), status: TaskStatus::Done, summary: "done".into() });
    r.send(Event::Progress(format!("run {RUN} round 1: 1/2 criteria passing; no integration yet")));
    r.send(Event::Checkpoint("stuck after 1 rounds".into()));
    r.flush();
    assert!(r.card().notes.is_empty(), "{:?}", r.card().notes);
    // A later round re-dispatches the task: the real script takes the claim again (reported done, then failed).
    r.send(Event::Claim { task: format!("{RUN}.t1"), agent: "bhima".into(), worktree: Some(PathBuf::from("/wt/two")) });
    r.send(Event::Report { task: format!("{RUN}.t1"), status: TaskStatus::Failed, summary: "round 2 failed".into() });
    r.send(Event::Claim { task: format!("{RUN}.t1"), agent: "bhima".into(), worktree: Some(PathBuf::from("/wt/three")) });
    r.flush();
    assert!(r.card().notes.is_empty(), "a re-dispatch's claim is accepted: {:?}", r.card().notes);
    assert!(matches!(b.run(&Request::CampaignVerify { slug: SLUG.into() }), Ok(o) if o.exit() == Some(0)));
    // The record read back is exactly the core's writes.
    r.verify_record().unwrap();
    // A resumed run reuses it; a fresh one refuses it, with its ledger or without.
    let resumed = CoreRecord::new(t.cfg(true), b.clone(), store()).unwrap();
    resumed.plan(&plan()).unwrap();
    resumed.verify_record().unwrap();
    assert!(CoreRecord::new(t.cfg(false), b.clone(), store()).is_err());
    assert!(CoreRecord::new(t.cfg(false), b.clone(), mem()).unwrap().plan(&plan()).is_err());
    // An agent runs the same signed script against the core's campaign: the signature is still valid, but the record
    // is no longer the core's. It isn't verified, and a resume refuses it.
    assert!(matches!(b.run(&Request::CampaignProgress { slug: SLUG.into(), text: "all criteria pass".into() }), Ok(o) if o.exit() == Some(0)));
    assert!(matches!(b.run(&Request::CampaignVerify { slug: SLUG.into() }), Ok(o) if o.exit() == Some(0)));
    assert!(r.verify_record().unwrap_err().contains("progress line"), "{:?}", r.verify_record());
    assert!(CoreRecord::new(t.cfg(true), b.clone(), store()).unwrap().plan(&plan()).is_err());
    // A task report written outside the core is caught the same way, with the progress line gone from the ledger's view.
    let other = super::super::record_app::AppLedgerStore::new(&t.0.join("data"), SLUG, t.0.join("home/.claude/vajra/campaigns"), key.clone(), b.clone()).unwrap();
    let mut back = other.read_back(SLUG).unwrap();
    back.progress = back.progress.lines().filter(|l| !l.ends_with("all criteria pass")).map(|l| format!("{l}\n")).collect();
    let saved = other.load().unwrap().unwrap();
    saved.check(&back).unwrap();
    assert!(matches!(b.run(&Request::TasksReport { slug: SLUG.into(), id: format!("{RUN}.t1"), status: TaskStatus::Done, summary: Some("done".into()) }), Ok(o) if o.exit() == Some(0)));
    let mut back = other.read_back(SLUG).unwrap();
    back.progress = back.progress.lines().filter(|l| !l.ends_with("all criteria pass")).map(|l| format!("{l}\n")).collect();
    assert!(saved.check(&back).unwrap_err().contains("task"), "{:?}", saved.check(&back));
    // The pointer `create` set is the record dir's, never the global one or a repo's.
    let campaigns = t.0.join("home/.claude/vajra/campaigns");
    let pointers: Vec<String> = std::fs::read_dir(&campaigns).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.starts_with(".active-")).collect();
    assert_eq!(pointers.len(), 1, "{pointers:?}");
    assert_ne!(pointers[0], ".active-global");
    assert_eq!(campaign_status(&b, SLUG).as_deref(), Some("active"));
    close_abandoned(&b, SLUG, &other).unwrap();
    assert_eq!(campaign_status(&b, SLUG).as_deref(), Some("abandoned"));
    assert_eq!(other.load().unwrap().unwrap().status, "abandoned", "the close went through the ledger");
}

/// HMAC-SHA256 (RFC 2104) over the crate's sha256: what the engine signs evidence with.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> String {
    let mut k = [0u8; 64];
    k[..key.len()].copy_from_slice(key);
    let pad = |b: u8| k.iter().map(|x| x ^ b).collect::<Vec<u8>>();
    let inner = crate::control::sha256::digest(&[pad(0x36), msg.to_vec()].concat());
    crate::control::sha256::hex(&crate::control::sha256::digest(&[pad(0x5c), inner.to_vec()].concat()))
}

/// bhishma on f689ce2: the core's close must reach the ledger as `closed` from the real script's answer, not only
/// from JSON a test hands in. Green signed evidence in a temp repo, the temp HOME's test key, no injected output.
#[test]
fn the_real_scripts_close_is_read_as_the_cores_own_and_the_record_verifies() {
    if ["/usr/bin/jq", "/usr/bin/openssl", "/usr/bin/python3", "/usr/bin/git"].iter().any(|p| !Path::new(p).is_file()) {
        eprintln!("skipped: the engine needs /usr/bin/jq, /usr/bin/openssl, /usr/bin/python3 and /usr/bin/git");
        return;
    }
    let t = Tmp::new();
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
    let mut b = Bridge::new(scripts.canonicalize().unwrap()).with_home(t.0.join("home"));
    let key = t.0.join("home/.claude/vajra/.hmac-key");
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&key, "00".repeat(32)).unwrap();
    std::fs::create_dir_all(t.0.join("home/.claude/agents")).unwrap();
    for a in ["bhima", "abhimanyu"] {
        std::fs::write(t.0.join("home/.claude/agents").join(format!("{a}.md")), "---\nname: x\n---\n").unwrap();
    }
    b.record_cwd = Some(super::super::record_app::record_dir(&t.0.join("data")).unwrap());
    b.timeout = RECORD_TIMEOUT;
    let store = super::super::record_app::AppLedgerStore::new(&t.0.join("data"), SLUG, t.0.join("home/.claude/vajra/campaigns"), key.clone(), b.clone()).unwrap();
    let r = CoreRecord::new(t.cfg(false), b.clone(), Arc::new(store)).unwrap();
    r.plan(&plan()).unwrap();
    r.send(Event::Claim { task: format!("{RUN}.t1"), agent: "bhima".into(), worktree: Some(PathBuf::from("/wt/one")) });
    r.flush();
    // The task's repository, with one commit; git sees only the temp HOME.
    let repo = t.0.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let repo = repo.canonicalize().unwrap();
    let git = |args: &[&str]| {
        let o = std::process::Command::new("/usr/bin/git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(&repo)
            .env("HOME", t.0.join("home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["commit", "-q", "--allow-empty", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    git(&["commit", "-q", "--allow-empty", "-m", "t1"]);
    let head = git(&["rev-parse", "HEAD"]);
    // Green verify-work evidence, signed as the engine signs it: HMAC over the sorted-key compact JSON.
    let body = format!(r#"{{"allPassed":true,"commands":[{{"cmd":"cargo test","exit":0,"passed":true}}],"commandsFailed":0,"commandsTotal":1,"git":{{"head":"{head}","treeStable":true}}}}"#);
    let sig = hmac_sha256(&[0u8; 32], body.as_bytes());
    let evidence = repo.join("data/walkthroughs/t1/evidence.json");
    std::fs::create_dir_all(evidence.parent().unwrap()).unwrap();
    std::fs::write(&evidence, format!("{},\"hmac\":\"{sig}\"}}", &body[..body.len() - 1])).unwrap();
    let close = Request::TasksClose {
        slug: SLUG.into(),
        id: format!("{RUN}.t1"),
        evidence: evidence.clone(),
        head: head.clone(),
        commit: head.clone(),
        branch: format!("vajra/{RUN}/task-t1-r1"),
        base_commit: base,
        changed: v(&["src/a.rs"]),
        repo: repo.clone(),
    };
    r.write(&close).unwrap();
    r.verify_record().unwrap();
    // A second close finds it done: exit 0, "already done", and the record no longer reads as only the core's.
    r.write(&close).unwrap();
    assert!(r.verify_record().unwrap_err().contains("isn't the core's"), "{:?}", r.verify_record());
}

// ---------------------------------------------------------------- the core's ledger (bhishma H1)

/// A ledger after the plan and one round, and the read-back the scripts would show for it.
fn ledger_and_back() -> (Ledger, ReadBack) {
    let mut l = Ledger::new(SLUG);
    let p = plan();
    l.apply(&Request::CampaignCreate { slug: SLUG.into(), goal: "g".into(), features: p.features.clone() });
    let add = |id: &str| Request::TasksAdd { slug: SLUG.into(), id: id.into(), title: "do".into(), owner: "bhima".into(), deps: vec![], files: v(&["src/"]), acceptance: v(&["ac-1"]), branch: "b".into() };
    l.apply(&add("t1"));
    l.apply(&Request::TasksClaim { slug: SLUG.into(), id: "t1".into(), worker: "bhima".into(), worktree: None });
    l.apply(&Request::TasksReport { slug: SLUG.into(), id: "t1".into(), status: TaskStatus::InProgress, summary: None });
    l.apply(&Request::CampaignProgress { slug: SLUG.into(), text: "round 1".into() });
    let task = serde_json::json!({"id": "t1", "status": "in_progress", "attempts": 1, "claimed_by": "bhima", "report": {"status": "in_progress", "summary": ""},
        "title": "do", "owner_agent": "bhima", "deps": [], "files": ["src/"], "acceptance": ["ac-1"]});
    let back = ReadBack {
        campaign: serde_json::json!({"slug": SLUG, "goal": "g", "status": "active"}),
        features: serde_json::json!({"features": [{"id": "ac-1", "description": "export has --json", "passes": false}, {"id": "ac-2", "description": "docs say so", "passes": false}]}),
        progress: "# Campaign\nGoal: g\n\n## Progress\n- 2026-11-12T12:00:00Z created with 2 feature(s)\n- 2026-11-12T12:01:00Z round 1\n".into(),
        tasks: serde_json::json!({"tasks": [task]}),
    };
    (l, back)
}

#[test]
fn the_ledger_matches_only_the_cores_own_writes() {
    let (l, back) = ledger_and_back();
    l.check(&back).unwrap();
    // An extra pass, progress line or task report, by anyone but the core, is a mismatch.
    let mut b = back.clone();
    b.features["features"][0]["passes"] = true.into();
    b.progress.push_str("- 2026-11-12T12:02:00Z pass: ac-1 (evidence: ok)\n");
    assert!(l.check(&b).is_err());
    let mut b = back.clone();
    b.progress.push_str("- 2026-11-12T12:02:00Z everything passes\n");
    assert!(l.check(&b).is_err());
    let mut b = back.clone();
    b.tasks["tasks"][0]["report"] = serde_json::json!({"status": "done", "summary": "all good"});
    assert!(l.check(&b).is_err());
    let mut b = back.clone();
    b.tasks["tasks"][0]["title"] = "something else".into();
    assert!(l.check(&b).is_err());
    let mut b = back.clone();
    b.campaign["reportSha256"] = "ab".into();
    assert!(l.check(&b).is_err());
    // A write whose outcome isn't known never matches.
    let mut p = l.clone();
    p.pending = Some("CampaignPass".into());
    assert!(p.check(&back).is_err());
    let mut u = l.clone();
    u.uncertain = Some("CampaignProgress".into());
    assert!(u.check(&back).is_err());
}

#[test]
fn n1_a_report_swapped_for_another_signed_one_is_not_the_cores() {
    let (mut l, back) = ledger_and_back();
    let b64 = "IyByZXBvcnQK"; // "# report\n"
    l.apply(&Request::CampaignReport { slug: SLUG.into(), bytes: 9, check: fnv32(b64), base64: b64.into() });
    assert_eq!(l.report.as_deref(), Some(crate::control::sha256::hex_digest(b"# report\n").as_str()));
    let mut b = back.clone();
    b.campaign["reportSha256"] = crate::control::sha256::hex_digest(b"# report\n").into();
    l.check(&b).unwrap();
    // An agent's own report, saved and signed by the same script: a report exists, but it isn't the core's.
    b.campaign["reportSha256"] = crate::control::sha256::hex_digest(b"# all good, ship it\n").into();
    assert!(l.check(&b).unwrap_err().contains("report"));
}

#[test]
fn n2_only_the_cores_own_close_with_its_evidence_commit_and_branch_counts() {
    let (base, back) = ledger_and_back();
    let close = Request::TasksClose {
        slug: SLUG.into(),
        id: "t1".into(),
        evidence: PathBuf::from("/wt/data/walkthroughs/t1/evidence.json"),
        head: "a".repeat(40),
        commit: "b".repeat(40),
        branch: "ao/t1".into(),
        base_commit: String::new(),
        changed: vec![],
        repo: PathBuf::from("/wt"),
    };
    let sha = crate::control::sha256::hex_digest(b"core evidence");
    let mut done = back.clone();
    let t = &mut done.tasks["tasks"][0];
    t["status"] = "done".into();
    t["evidence"] = serde_json::json!({"sha256": sha, "path": "/wt/data/walkthroughs/t1/evidence.json"});
    t["commit"] = "b".repeat(40).into();
    t["branch"] = "ao/t1".into();
    let mut l = base.clone();
    l.applied(&close, r#"{"ok": true, "id": "t1", "result": "closed", "status": "done"}"#, Some(&sha));
    l.check(&done).unwrap();
    // Evidence, commit or branch other than the core's close stored: not the core's.
    for (k, v) in [("evidence", serde_json::json!({"sha256": "0".repeat(64)})), ("commit", "c".repeat(40).into()), ("branch", "agent/own".into()), ("base_commit", "d".repeat(40).into())] {
        let mut b = done.clone();
        b.tasks["tasks"][0][k] = v;
        assert!(l.check(&b).is_err(), "{k}");
    }
    // The script found it already done (someone closed it first, with their evidence): exit 0, but not the core's.
    let mut l = base.clone();
    l.applied(&close, r#"{"ok": true, "id": "t1", "result": "already done", "status": "done"}"#, Some(&sha));
    assert!(l.check(&done).unwrap_err().contains("isn't the core's"));
}

#[test]
fn a_ledger_that_cant_be_saved_stops_the_write_and_a_resume_needs_a_matching_ledger() {
    let t = Tmp::new();
    let store = Arc::new(MemStore::default());
    store.no_save.store(true, Ordering::SeqCst);
    let r = CoreRecord::new(t.cfg(false), t.bridge(), store.clone()).unwrap();
    assert!(r.plan(&plan()).is_err());
    assert!(t.calls().is_empty(), "nothing is written without the ledger: {:?}", t.calls());
    // A write whose final save failed stays unknown in memory: the next save in the session keeps it.
    let t2 = Tmp::new();
    let s2 = Arc::new(MemStore::default());
    let r2 = CoreRecord::new(t2.cfg(false), t2.bridge(), s2.clone()).unwrap();
    let saves = Arc::new(AtomicUsize::new(0));
    struct FailSecond(Arc<MemStore>, Arc<AtomicUsize>);
    impl LedgerStore for FailSecond {
        fn load(&self) -> Result<Option<Ledger>, String> {
            self.0.load()
        }
        fn save(&self, l: &Ledger) -> Result<(), String> {
            if self.1.fetch_add(1, Ordering::SeqCst) == 1 { Err("disk full".into()) } else { self.0.save(l) }
        }
        fn read_back(&self, s: &str) -> Result<ReadBack, String> {
            self.0.read_back(s)
        }
        fn evidence_sha256(&self, p: &Path) -> Result<String, String> {
            self.0.evidence_sha256(p)
        }
    }
    drop(r2);
    let r2 = CoreRecord::new(t2.cfg(false), t2.bridge(), Arc::new(FailSecond(s2.clone(), saves))).unwrap();
    r2.send(Event::Progress("one".into()));
    r2.send(Event::Progress("two".into()));
    r2.flush();
    let saved = crate::lock(&s2.saved).clone().unwrap();
    assert!(saved.uncertain.is_some(), "the lost outcome of the first write survives the next save: {saved:?}");
    // A resume finds a campaign that exists, with no read-back that matches: refused.
    t.campaign_script("case \"$1\" in create) echo 'campaign already exists: x' >&2; exit 1 ;; show) printf '%s' '{\"signature_valid\":true,\"campaign\":{\"slug\":\"deliver-20261112-1200\",\"status\":\"active\",\"goal\":\"add a --json flag to export\"},\"features\":[{\"id\":\"ac-1\",\"description\":\"export has --json\",\"passes\":false},{\"id\":\"ac-2\",\"description\":\"docs say so\",\"passes\":false}]}' ;; esac");
    let store = Arc::new(MemStore::default());
    *crate::lock(&store.saved) = Some(Ledger::new(SLUG));
    let err = CoreRecord::new(t.cfg(true), t.bridge(), store).unwrap().plan(&plan()).unwrap_err();
    assert!(err.contains("isn't only the core's"), "{err}");
}
