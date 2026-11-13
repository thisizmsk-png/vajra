//! The run record on the app's side (PR 6h): the record's cwd (an empty git dir under the app's data, so the active
//! pointer `create` sets is one no user session reads), a run's `CoreRecord` from its `seats.json`, and orphans.
//! Files here are the app's own (`<data>/record`); nothing under `~/.claude/vajra` is opened.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use super::App;
use super::record::{self, CoreRecord, RECORD_TIMEOUT, RecordConfig, RunRecord};
use crate::control::fsx;
use crate::vajra::bridge::Bridge;

/// Make `<data>/record` an empty git dir: `HEAD`, `objects/`, `refs/` and nothing else (no config, no hooks), so the
/// campaign script's `git rev-parse --git-common-dir` names it and nothing in it runs.
/// An existing one is used only when it is exactly that: owned by this user, mode 0700, no links, `.git` holding only
/// `HEAD` (with the fixed text), `objects/` and `refs/`. A planted `config`, `commondir` or `hooks` could change which
/// active pointer `create` moves, so anything else is refused, never cleaned.
pub(super) fn record_dir(data: &Path) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    const HEAD: &[u8] = b"ref: refs/heads/main\n";
    let dir = data.join("record");
    let git = dir.join(".git");
    let fresh = std::fs::symlink_metadata(&dir).is_err();
    for d in [&dir, &git, &git.join("objects"), &git.join("refs")] {
        fsx::private_dir(d)?;
    }
    if fresh {
        fsx::write_atomic(&git, "HEAD", HEAD)?;
    }
    let refuse = |why: &str| std::io::Error::other(format!("the record's folder {} {why}", dir.display()));
    let uid = unsafe { libc::getuid() };
    for d in [&dir, &git, &git.join("objects"), &git.join("refs")] {
        let m = std::fs::symlink_metadata(d)?;
        if !m.is_dir() || m.uid() != uid || m.permissions().mode() & 0o777 != 0o700 {
            return Err(refuse("is not a private directory of this user"));
        }
    }
    let names = |d: &Path| -> std::io::Result<Vec<String>> {
        let mut v: Vec<String> = std::fs::read_dir(d)?.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
        v.sort();
        Ok(v)
    };
    if names(&dir)? != [".git"] || names(&git)? != ["HEAD", "objects", "refs"] || !names(&git.join("objects"))?.is_empty() || !names(&git.join("refs"))?.is_empty() {
        return Err(refuse("holds more than an empty git dir"));
    }
    let head = std::fs::symlink_metadata(git.join("HEAD"))?;
    if !head.is_file() || head.uid() != uid || std::fs::read(git.join("HEAD"))? != HEAD {
        return Err(refuse("has a changed HEAD"));
    }
    Ok(dir)
}

/// The core's ledger of one campaign (`<data>/control/record-ledgers/<slug>.json`, the app's own, private) and the
/// campaign read back through the held reader: the bytes are read once, their signature and features definition
/// checked through the scripts, and the task graph read with `list --json`.
pub(super) struct AppLedgerStore {
    dir: PathBuf,
    slug: String,
    campaigns: PathBuf,
    key: PathBuf,
    bridge: Bridge,
}

impl AppLedgerStore {
    pub(super) fn new(data: &Path, slug: &str, campaigns: PathBuf, key: PathBuf, bridge: Bridge) -> Result<Self, String> {
        crate::vajra::bridge_record::slug(slug).map_err(|e| e.to_string())?;
        Ok(AppLedgerStore { dir: data.join("control/record-ledgers"), slug: slug.into(), campaigns, key, bridge })
    }

    fn file(&self) -> String {
        format!("{}.json", self.slug)
    }
}

impl record::LedgerStore for AppLedgerStore {
    fn load(&self) -> Result<Option<record::Ledger>, String> {
        match fsx::read_nofollow_capped(&self.dir.join(self.file()), 4 * 1024 * 1024) {
            Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|_| "the run record's ledger can't be read".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("the run record's ledger can't be read: {e}")),
        }
    }

    fn save(&self, l: &record::Ledger) -> Result<(), String> {
        let bytes = serde_json::to_vec(l).map_err(|e| e.to_string())?;
        fsx::private_dir(self.dir.parent().unwrap_or(&self.dir)).map_err(|e| e.to_string())?;
        fsx::private_dir(&self.dir).map_err(|e| e.to_string())?;
        fsx::write_atomic(&self.dir, &self.file(), &bytes).map(|_| ()).map_err(|e| e.to_string())
    }

    fn evidence_sha256(&self, path: &Path) -> Result<String, String> {
        // The script reads at most 5 MB of evidence; a bigger file is refused, never hashed short.
        let b = fsx::read_nofollow_capped(path, 5_000_000).map_err(|e| e.to_string())?;
        Ok(crate::control::sha256::hex_digest(&b))
    }

    fn read_back(&self, slug: &str) -> Result<record::ReadBack, String> {
        use crate::control::sha256::hex_digest;
        use crate::vajra::bridge::Request;
        use crate::vajra::held;
        let unread = |what: &str| format!("the campaign can't be read back ({what})");
        let h = held::read_campaign(&self.campaigns, slug, &self.key, &|_| {}).map_err(|e| unread(&format!("{e:?}")))?;
        if !held::json_without_duplicate_keys(&h.campaign) || !held::json_without_duplicate_keys(&h.features) {
            return Err("the campaign's files aren't the scripts' JSON".into());
        }
        let campaign: Value = serde_json::from_slice(&h.campaign).map_err(|_| unread("campaign.json"))?;
        let features: Value = serde_json::from_slice(&h.features).map_err(|_| unread("features.json"))?;
        let progress = String::from_utf8(h.progress.clone()).map_err(|_| unread("progress.md"))?;
        let sig: String = String::from_utf8_lossy(&h.sig).chars().filter(|c| !c.is_whitespace()).collect();
        if sig.len() != 64 || !sig.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err("the campaign's signature is broken".into());
        }
        let input = format!("campaign {}\nfeatures {}\nprogress {}\nsig {}\n", hex_digest(&h.campaign), hex_digest(&h.features), hex_digest(&h.progress), sig);
        match self.bridge.run(&Request::VerifyDigest { input: input.into_bytes() }) {
            Ok(o) if o.exit() == Some(0) => {}
            Ok(o) if o.exit() == Some(3) => return Err("the campaign's signature is broken".into()),
            Ok(_) | Err(_) => return Err(unread("the signature check didn't run")),
        }
        let def = match self.bridge.run(&Request::FeaturesDef { features: h.features.clone() }) {
            Ok(o) if o.exit() == Some(0) => o.stdout.replace('\n', ""),
            Ok(_) | Err(_) => return Err(unread("the criteria check didn't run")),
        };
        if campaign.get("featuresDefSha256").and_then(Value::as_str) != Some(hex_digest(def.as_bytes()).as_str()) {
            return Err("the campaign's criteria aren't the signed ones".into());
        }
        let tasks = match self.bridge.run(&Request::TasksList { slug: slug.into() }) {
            Ok(o) if o.exit() == Some(0) && !o.truncated => serde_json::from_str(&o.stdout).map_err(|_| unread("the task graph"))?,
            Ok(_) | Err(_) => return Err(unread("the task graph")),
        };
        Ok(record::ReadBack { campaign, features, progress, tasks })
    }
}

/// The bridge the record writes with: the record's cwd and the writes' longer timeout.
pub(super) fn record_bridge(bridge: &Bridge, data: &Path) -> std::io::Result<Bridge> {
    let mut b = bridge.clone();
    b.record_cwd = Some(record_dir(data)?);
    b.timeout = RECORD_TIMEOUT;
    Ok(b)
}

impl App {
    /// A run's record, bound to the campaign its `seats.json` names (copied from the original run on a resume, never
    /// recomputed). `resumed`: this run continues one the core started.
    #[allow(dead_code)] // 6d2c's Start builds it for a core run
    pub(super) fn run_record(&self, run_id: &str, resumed: bool) -> Result<Arc<dyn RunRecord>, String> {
        let seats = self.read_run_seats(run_id).map_err(|e| format!("the run's seats can't be read: {e}"))?.ok_or("the run has no seats")?;
        let (slug, trust) = (seats["campaign_slug"].as_str(), seats["evidence_trust"].as_str());
        let (Some(slug), Some(trust)) = (slug, trust) else { return Err("the run's seats name no campaign".into()) };
        let cfg = RecordConfig { run: run_id.into(), slug: slug.into(), trust: trust.into(), home: self.home.clone(), resumed };
        let bridge = record_bridge(&self.bridge, &self.data).map_err(|e| format!("the record's folder can't be made: {e}"))?;
        let store = self.ledger_store(slug, &bridge)?;
        let rec = Arc::new(CoreRecord::new(cfg, bridge, Arc::new(store))?);
        super::lock(&self.live_records).insert(run_id.to_string(), Arc::downgrade(&rec));
        Ok(rec)
    }

    fn ledger_store(&self, slug: &str, bridge: &Bridge) -> Result<AppLedgerStore, String> {
        let allow = crate::vajra::allowlist::Allowlist::new(&self.claude, &self.data).map_err(|e| e.to_string())?;
        AppLedgerStore::new(&self.data, slug, allow.campaigns_dir().to_path_buf(), allow.key_path(), bridge.clone())
    }

    /// The runs going on now: a workflow run whose process runs (`active_runs`'s rule), or a core run with a ticket
    /// that hasn't ended. Never an orphan.
    pub(super) fn live_run_ids(&self) -> Vec<String> {
        self.reap_runs();
        let runs = super::lock(&self.runs).clone();
        let mut live: Vec<String> = runs.iter().filter(|(s, _)| self.sup.is_running(s).unwrap_or(false)).map(|(_, r)| r.run_id.clone()).collect();
        live.extend(self.recorded_runs().into_iter().filter(|r| self.workers.run_has_live_tickets(r)));
        // A core run whose protocol hasn't finalized is going on, ticket or not (before dispatch, while the gate
        // runs, while the lead plans the next round). A dropped record (the app restarted) isn't.
        let mut recs = super::lock(&self.live_records);
        // An ended record the app still holds stays registered, so an orphan close goes through its writer.
        recs.retain(|_, w| w.strong_count() > 0);
        live.extend(recs.iter().filter(|(_, w)| w.upgrade().is_some_and(|r| r.live())).map(|(k, _)| k.clone()));
        live
    }

    /// Orphans: a campaign a core run's `seats.json` names that is still active though no run is going on now (the
    /// live runs, and any `extra_live`, are left out). Only slugs the core recorded count; a CLI campaign named
    /// `deliver-*` never does.
    pub fn record_orphans(&self, extra_live: &[String]) -> Value {
        let Ok(bridge) = record_bridge(&self.bridge, &self.data) else { return json!({"orphans": []}) };
        let live = self.live_run_ids();
        let orphans: Vec<Value> = self
            .recorded_runs()
            .into_iter()
            .filter(|r| !live.contains(r) && !extra_live.contains(r))
            .filter_map(|r| {
                let slug = self.read_run_seats(&r).ok().flatten()?["campaign_slug"].as_str()?.to_string();
                (record::campaign_status(&bridge, &slug)? == "active").then(|| json!({"run_id": r, "campaign": slug}))
            })
            .collect();
        json!({"orphans": orphans})
    }

    /// "Close as abandoned", for one orphan `record_orphans` listed: re-checked here, so only a core run's campaign.
    pub fn record_close_orphan(&self, run_id: &str, live: &[String]) -> Result<Value, Value> {
        let listed = self.record_orphans(live);
        let Some(o) = listed["orphans"].as_array().and_then(|a| a.iter().find(|o| o["run_id"] == run_id)) else {
            return Err(json!({"error": "not_found", "detail": "that run has no abandoned campaign"}));
        };
        let slug = o["campaign"].as_str().unwrap_or_default().to_string();
        // The app still holds this run's record: close through its own writer and ledger, never a second copy.
        let held = super::lock(&self.live_records).get(run_id).and_then(std::sync::Weak::upgrade);
        if let Some(rec) = held {
            rec.close_abandoned().map_err(|why| json!({"error": "close_failed", "detail": why}))?;
            return Ok(json!({"run_id": run_id, "campaign": slug, "status": "abandoned"}));
        }
        let bridge = record_bridge(&self.bridge, &self.data).map_err(|e| json!({"error": "io", "detail": e.to_string()}))?;
        let store = self.ledger_store(&slug, &bridge).map_err(|e| json!({"error": "io", "detail": e}))?;
        record::close_abandoned(&bridge, &slug, &store).map_err(|why| json!({"error": "close_failed", "detail": why}))?;
        Ok(json!({"run_id": run_id, "campaign": slug, "status": "abandoned"}))
    }

    /// "Close as abandoned" through the receipt engine: the human's click only, deduped by its action id. The orphan
    /// is re-listed at the click, so a run that is going on now is refused (`not_found`).
    pub fn act_close_orphan(&self, env: &super::Envelope, run_id: &str) -> Value {
        let target = format!("record:orphan:{run_id}");
        self.act(env, crate::control::receipt::Caller::Ui, true, "close_orphan", &target, None::<u64>, || self.record_close_orphan(run_id, &[]))
    }
}
