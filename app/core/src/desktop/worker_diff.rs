//! The change count shown on a worker (`+N −M`), kept off the snapshot path.
//!
//! `crew()` only READS this cache: building a snapshot never runs git or opens a file. When a worker is due, `crew()`
//! hands the work to one short-lived background thread (at most one at a time), which counts with
//! `providers::effective::diff_from_git` (bounded by its own caps and time budget) and stores the result.
//!
//! A worker is due when it has never been counted, or when at least `DIFF_TTL` has passed since its last count AND its
//! output stream moved (its `stamp`, the stream's newest seq). A worker that said nothing since the last count has not
//! changed its worktree, so it is not counted again. A failed count is remembered as "no diff" until the next due time, so
//! a broken worktree is not asked about on every snapshot either.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::providers::effective::diff_from_git;
use crate::vajra::bridge::{Bridge, Request};

/// The least time between two counts of one worker.
pub(super) const DIFF_TTL: Duration = Duration::from_secs(5);
/// One refresh round stops starting new workers after this long (each count has its own caps and budget besides).
const ROUND_BUDGET: Duration = Duration::from_secs(8);

/// What the cache knows about one worker.
struct Slot {
    checked: Instant,
    stamp: u64,
    /// `None`: no base to count against, or git could not answer.
    diff: Option<Value>,
}

/// A worker to count: the cwd its transcript names (not yet checked), the run repo it belongs to, and where its output
/// stream is. Nothing here touches the disk; the background round resolves and checks the path.
#[derive(Clone, Debug)]
pub(super) struct Target {
    pub id: String,
    pub cwd: String,
    pub repo: PathBuf,
    pub stamp: u64,
}

#[derive(Default)]
pub(super) struct WorkerDiffs {
    slots: Mutex<HashMap<String, Slot>>,
    busy: AtomicBool,
    /// How many counts ran (tests assert a snapshot does not add to it).
    counts: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl WorkerDiffs {
    /// The cached diff for a worker, as the UI reads it. Never runs git.
    pub fn get(&self, id: &str) -> Option<Value> {
        lock(&self.slots).get(id)?.diff.clone()
    }

    #[cfg(test)]
    pub fn counts(&self) -> u64 {
        self.counts.load(Ordering::SeqCst)
    }

    pub(super) fn due(&self, t: &Target, now: Instant) -> bool {
        match lock(&self.slots).get(&t.id) {
            None => true,
            Some(s) => now.duration_since(s.checked) >= DIFF_TTL && s.stamp != t.stamp,
        }
    }

    /// Count the due workers now, on the calling thread. `refresh_in_background` is the production entry.
    pub fn refresh(&self, bridge: &Bridge, targets: &[Target]) {
        let start = Instant::now();
        for t in targets {
            if start.elapsed() >= ROUND_BUDGET {
                break;
            }
            if !self.due(t, Instant::now()) {
                continue;
            }
            let diff = count(bridge, &t.cwd, &t.repo);
            self.counts.fetch_add(1, Ordering::SeqCst);
            lock(&self.slots).insert(t.id.clone(), Slot { checked: Instant::now(), stamp: t.stamp, diff });
        }
    }

    /// Hand the due workers to one background thread and return at once. A round already running takes the place of
    /// this one (the next snapshot asks again), so the threads never pile up.
    pub fn refresh_in_background(self: &Arc<Self>, bridge: &Bridge, targets: Vec<Target>) {
        let now = Instant::now();
        let due: Vec<Target> = targets.into_iter().filter(|t| self.due(t, now)).collect();
        if due.is_empty() || self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (me, bridge) = (Arc::clone(self), bridge.clone());
        std::thread::spawn(move || {
            me.refresh(&bridge, &due);
            me.busy.store(false, Ordering::SeqCst);
        });
    }

    /// Tests: wait until no round is running.
    #[cfg(test)]
    pub fn wait_idle(&self) {
        while self.busy.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// `cwd` as a crew worktree of `repo`: resolved through links, and strictly under `<repo>/.claude/worktrees/`. Anything
/// else (the repo itself, a link out, a missing directory) is not counted, the rule `worktree_info` applies.
pub(super) fn crew_worktree_of(cwd: &str, repo: &Path) -> Option<PathBuf> {
    let wt = Path::new(cwd).canonicalize().ok()?;
    let root = repo.canonicalize().ok()?.join(".claude/worktrees");
    (wt.is_dir() && wt.strip_prefix(&root).is_ok_and(|rest| rest.components().next().is_some())).then_some(wt)
}

/// The worker's change set against the base the Diff tab uses (the merge-base of its HEAD and the run repo's HEAD),
/// as the JSON the UI parses. `None` when it is not a crew worktree, there is no base, or git cannot answer.
fn count(bridge: &Bridge, cwd: &str, repo: &Path) -> Option<Value> {
    let worktree = &crew_worktree_of(cwd, repo)?;
    let git = |dir: &Path, args: &[&str]| -> Option<String> {
        let o = bridge.run(&Request::Git { cwd: dir.to_path_buf(), args: args.iter().map(|a| a.to_string()).collect() }).ok()?;
        (o.exit() == Some(0)).then(|| o.stdout.trim().to_string())
    };
    let head = git(repo, &["rev-parse", "HEAD"])?;
    let base = git(worktree, &["merge-base", "HEAD", &head]).filter(|b| !b.is_empty() && !b.starts_with('-'))?;
    serde_json::to_value(diff_from_git(bridge, worktree, &base)?).ok()
}
