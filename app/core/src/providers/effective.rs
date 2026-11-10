//! What a worker actually ran on (H3, R2/R18): the harness's own report of its provider, model and effort, read after
//! it starts, and the worker's diff read from git. Nothing here is copied from what was asked for: a value the harness
//! does not report is `"unknown"` ("Not reported"), never the requested one.
//!
//! - Codex: its `--json` stream names a `thread_id` (`thread.started`) but not the model. The model is in the session
//!   file `<CODEX_HOME>/sessions/**/rollout-*-<thread_id>.jsonl` (`session_meta.model_provider`, `turn_context.model`,
//!   `.effort`, `.sandbox_policy`). It records what Codex sent; the receipt says so (`read_from`).
//! - Claude: the `system`/`init` event of its stream names the model.
//! - Diff: `git diff --numstat` in the worker's worktree, plus untracked files. A worker that edits through a shell
//!   leaves no edit events, so events never decide what changed.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::vajra::bridge::{Bridge, Request};

/// A value the harness did not report.
pub const UNKNOWN: &str = "unknown";
/// `read_from` of a Codex receipt.
pub const FROM_SESSION_FILE: &str = "harness_session_file";
/// `read_from` of a receipt built from the harness's own event stream.
pub const FROM_EVENT: &str = "harness_event";
/// Largest session file or event text read.
const MAX_READ: u64 = 1024 * 1024;
/// Records scanned for the first `turn_context`.
const MAX_RECORDS: usize = 400;

/// What the slot's choice asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Requested {
    pub account: String,
    pub harness: String,
    pub model: String,
    pub effort: Option<String>,
}

/// What the harness reported. Every text field is `"unknown"` when it was not reported; `effort` is `None` only for a
/// harness with no effort setting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Effective {
    pub account: String,
    pub harness: String,
    pub model: String,
    pub effort: Option<String>,
    pub sandbox: String,
    pub billing: String,
    /// Where the values were read from, or `None` when nothing was read.
    pub read_from: Option<String>,
}

impl Effective {
    fn unknown(harness: &str, effort_unknown: bool) -> Self {
        Self {
            account: UNKNOWN.into(),
            harness: harness.into(),
            model: UNKNOWN.into(),
            effort: effort_unknown.then(|| UNKNOWN.into()),
            sandbox: UNKNOWN.into(),
            billing: UNKNOWN.into(),
            read_from: None,
        }
    }

    /// `{account, family, model, billing}` for a crew node or a `project_state` worker, or `None` while the model is
    /// not reported (a surface shows "Not reported", never a guess).
    pub fn tag(&self) -> Option<ProviderTag> {
        (self.model != UNKNOWN).then(|| ProviderTag {
            account: self.account.clone(),
            family: self.harness.clone(),
            model: self.model.clone(),
            billing: self.billing.clone(),
        })
    }
}

/// The provider chip data every surface draws from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderTag {
    pub account: String,
    pub family: String,
    pub model: String,
    pub billing: String,
}

/// The account and billing a Codex `model_provider` id stands for. An id Vajra did not write is unknown, never guessed.
fn account_of(model_provider: &str) -> (&'static str, &'static str) {
    match model_provider {
        "vajra-openrouter" => ("openrouter", "gateway"),
        "openai" => ("codex", "subscription"),
        _ => (UNKNOWN, UNKNOWN),
    }
}

/// A model, effort or provider id as plain text: short and printable, so a hostile session file cannot put a long or
/// control-laden string into a receipt, an event or the UI.
fn clean(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| (1..=100).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic()))
        .map(String::from)
}

/// The `thread_id` of the `thread.started` event in a Codex `--json` stream. Only a plain id is returned, since it is
/// matched against file names.
pub fn codex_thread_id(events: &str) -> Option<String> {
    events.lines().filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok()).find_map(|e| {
        (e["type"] == "thread.started")
            .then(|| e["thread_id"].as_str().map(String::from))
            .flatten()
            .filter(|t| (8..=64).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
    })
}

/// Find `rollout-*-<thread_id>.jsonl` under `<codex_home>/sessions/<y>/<m>/<d>/`. Links are skipped, so a planted link
/// cannot point the reader outside the Vajra-owned home.
fn rollout_file(codex_home: &Path, thread_id: &str) -> Option<PathBuf> {
    let suffix = format!("-{thread_id}.jsonl");
    let dirs = |p: &Path| -> Vec<PathBuf> {
        fs::read_dir(p)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect()
    };
    let days = dirs(&codex_home.join("sessions")).iter().flat_map(|y| dirs(y)).flat_map(|m| dirs(&m)).collect::<Vec<_>>();
    days.iter().flat_map(|d| fs::read_dir(d).into_iter().flatten().flatten()).find_map(|e| {
        let name = e.file_name();
        let name = name.to_str()?;
        (name.starts_with("rollout-") && name.ends_with(&suffix) && e.file_type().ok()?.is_file()).then(|| e.path())
    })
}

/// What a Codex worker ran on, from its event stream (for the `thread_id`) and its session file. With no thread or no
/// readable file everything stays unknown and `read_from` is `None`.
pub fn read_codex(events: &str, codex_home: &Path) -> Effective {
    let mut eff = Effective::unknown("codex", true);
    let Some(file) = codex_thread_id(events).and_then(|t| rollout_file(codex_home, &t)) else { return eff };
    let mut text = String::new();
    if fs::File::open(&file).and_then(|f| f.take(MAX_READ).read_to_string(&mut text)).is_err() {
        return eff;
    }
    for rec in text.lines().take(MAX_RECORDS).filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        let p = &rec["payload"];
        match rec["type"].as_str() {
            Some("session_meta") => {
                if let Some(mp) = clean(p.get("model_provider")) {
                    let (account, billing) = account_of(&mp);
                    eff.account = account.into();
                    eff.billing = billing.into();
                }
            }
            Some("turn_context") => {
                eff.model = clean(p.get("model")).unwrap_or_else(|| UNKNOWN.into());
                eff.effort = Some(clean(p.get("effort")).unwrap_or_else(|| UNKNOWN.into()));
                eff.sandbox = clean(p.pointer("/sandbox_policy/type")).unwrap_or_else(|| UNKNOWN.into());
                eff.read_from = Some(FROM_SESSION_FILE.into());
                break;
            }
            _ => {}
        }
    }
    eff
}

/// What a Claude worker ran on, from the `system`/`init` event of its stream. Claude has no effort setting here, so the
/// effective effort is `None`; its account and billing are those of the CLI login.
pub fn read_claude(events: &str) -> Effective {
    let mut eff = Effective::unknown("claude", false);
    let init = events.lines().take(MAX_RECORDS).filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok()).find(|e| e["type"] == "system" && e["subtype"] == "init");
    if let Some(model) = init.as_ref().and_then(|e| clean(e.get("model"))) {
        eff.model = model;
        eff.account = "claude".into();
        eff.billing = "subscription".into();
        eff.read_from = Some(FROM_EVENT.into());
    }
    eff
}

/// A worker's change set as git reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Diff {
    pub files: Vec<String>,
    pub added: u64,
    pub removed: u64,
}

/// The diff of `cwd` (the worker's worktree) against `base`, read through the read-only git bridge: tracked changes
/// from `git diff --numstat <base>` and new files from `git ls-files --others --exclude-standard`. `None` when git could
/// not answer; an empty `Diff` means git saw no change.
pub fn diff_from_git(bridge: &Bridge, cwd: &Path, base: &str) -> Option<Diff> {
    let git = |args: &[&str]| {
        let out = bridge.run(&Request::Git { cwd: cwd.to_path_buf(), args: args.iter().map(|a| a.to_string()).collect() }).ok()?;
        (out.code == Some(0) && out.whole()).then_some(out.stdout)
    };
    let tracked = git(&["diff", "--numstat", base, "--"])?;
    let untracked = git(&["ls-files", "--others", "--exclude-standard"])?;
    let mut diff = Diff::default();
    for line in tracked.lines() {
        let mut f = line.splitn(3, '\t');
        let (a, r, name) = (f.next()?, f.next()?, f.next()?);
        // A binary file reports `-` for both counts: it changed, with no line counts.
        diff.added += a.parse::<u64>().unwrap_or(0);
        diff.removed += r.parse::<u64>().unwrap_or(0);
        diff.files.push(name.into());
    }
    diff.files.extend(untracked.lines().filter(|l| !l.is_empty()).map(String::from));
    Some(diff)
}

#[cfg(test)]
#[path = "effective_tests.rs"]
mod tests;
