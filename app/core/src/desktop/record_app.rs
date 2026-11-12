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
pub(super) fn record_dir(data: &Path) -> std::io::Result<PathBuf> {
    let dir = data.join("record");
    let git = dir.join(".git");
    for d in [&dir, &git, &git.join("objects"), &git.join("refs")] {
        fsx::private_dir(d)?;
    }
    if std::fs::symlink_metadata(git.join("HEAD")).is_err() {
        fsx::write_atomic(&git, "HEAD", b"ref: refs/heads/main\n")?;
    }
    Ok(dir)
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
        Ok(Arc::new(CoreRecord::new(cfg, bridge)?))
    }

    /// The runs going on now: a workflow run whose process runs (`active_runs`'s rule), or a core run with a ticket
    /// that hasn't ended. Never an orphan.
    pub(super) fn live_run_ids(&self) -> Vec<String> {
        self.reap_runs();
        let runs = super::lock(&self.runs).clone();
        let mut live: Vec<String> = runs.iter().filter(|(s, _)| self.sup.is_running(s).unwrap_or(false)).map(|(_, r)| r.run_id.clone()).collect();
        live.extend(self.recorded_runs().into_iter().filter(|r| self.workers.run_has_live_tickets(r)));
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
        let bridge = record_bridge(&self.bridge, &self.data).map_err(|e| json!({"error": "io", "detail": e.to_string()}))?;
        record::close_abandoned(&bridge, &slug).map_err(|why| json!({"error": "close_failed", "detail": why}))?;
        Ok(json!({"run_id": run_id, "campaign": slug, "status": "abandoned"}))
    }

    /// "Close as abandoned" through the receipt engine: the human's click only, deduped by its action id. The orphan
    /// is re-listed at the click, so a run that is going on now is refused (`not_found`).
    pub fn act_close_orphan(&self, env: &super::Envelope, run_id: &str) -> Value {
        let target = format!("record:orphan:{run_id}");
        self.act(env, crate::control::receipt::Caller::Ui, true, "close_orphan", &target, None::<u64>, || self.record_close_orphan(run_id, &[]))
    }
}
