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

use std::ffi::{CStr, CString};
use std::fs::{self, File};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::preferences::open_at;
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
/// New files whose lines are counted into a `Diff`; every one is still listed.
const MAX_UNTRACKED: usize = 5000;
/// A new file larger than this is listed but not read (its lines are not counted).
const PER_FILE_CAP: u64 = 8 * 1024 * 1024;
/// The most bytes read across all new files of one diff.
const TOTAL_READ_CAP: u64 = 64 * 1024 * 1024;
/// How many paths a `Diff` lists (`files_total` says how many there were).
pub const MAX_LISTED: usize = 200;
/// A NUL in the first bytes of a file marks it binary, as git decides.
const BINARY_PROBE: usize = 8000;
/// The most time spent counting new files' lines; past it the totals are marked partial.
const UNTRACKED_BUDGET: Duration = Duration::from_secs(2);

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

    /// The provider chip data for what ran, or `None` when nothing was read at all. A model the harness did not report is
    /// `"unknown"` ("Not reported"), not a guess. `requested` is set by the caller that knows what was asked for.
    pub fn tag(&self, requested: Option<TagRequested>) -> Option<ProviderTag> {
        self.read_from.is_some().then(|| ProviderTag::new(&self.account, &self.model, &self.billing, requested))
    }
}

/// What a surface draws beside a worker: the `provider` object of the UI contract (`app/ui/src/workerProvider.ts`,
/// `parseWorkerProvider`). Every field the UI requires is here, under the UI's own names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderTag {
    pub account: String,
    /// The provider's name as people know it ("Claude Code", "OpenRouter"): shown beside every model.
    pub label: String,
    /// The model family that picks the icon (`claude`, `deepseek`); the account when the model names none.
    pub family: String,
    /// The model id that ran; `"unknown"` when the harness did not report it.
    pub model: String,
    /// A display name for the model. None is known yet, so `null`.
    pub model_name: Option<String>,
    pub billing: String,
    /// What was asked for; the same as the effective side when nothing differs. `None` where it is not known.
    pub requested: Option<TagRequested>,
    /// The worker's current provider error (PR 5); `null` until there is one.
    pub error: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TagRequested {
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub label: String,
}

impl ProviderTag {
    pub fn new(account: &str, model: &str, billing: &str, requested: Option<TagRequested>) -> Self {
        Self {
            account: account.into(),
            label: label_of(account),
            family: family_of(model, account),
            model: model.into(),
            model_name: None,
            billing: billing.into(),
            requested,
            error: None,
        }
    }
}

/// The name people know an account by. An account Vajra has no name for shows as its own id.
pub fn label_of(account: &str) -> String {
    match account {
        "claude" => "Claude Code".into(),
        "codex" => "Codex".into(),
        "openrouter" => "OpenRouter".into(),
        other => other.into(),
    }
}

/// The model family that picks the icon: the vendor prefix of a gateway id (`deepseek/deepseek-v4-flash` is `deepseek`), or
/// `claude` for a Claude model; otherwise the account's own provider.
fn family_of(model: &str, account: &str) -> String {
    let vendor = model.split_once('/').map(|(v, _)| v).filter(|v| !v.is_empty() && v.len() <= 32 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    match vendor {
        Some(v) => v.to_ascii_lowercase(),
        None if model.starts_with("claude") => "claude".into(),
        None => account.into(),
    }
}

/// The account and billing a Codex `model_provider` id stands for. An id Vajra did not write is unknown, never guessed.
fn account_of(model_provider: &str) -> (&'static str, &'static str) {
    match model_provider {
        "vajra-openrouter" => ("openrouter", "gateway"),
        // Signed in with a plan or with an API key looks the same here, and the session file does not say which.
        "openai" => ("codex", UNKNOWN),
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

/// Entries listed from one directory at most.
const MAX_ENTRIES: usize = 4096;

/// Open `name` under the held directory `parent` as a directory. A link there is refused (`O_NOFOLLOW`), so nothing
/// planted in the tree can lead the walk outside it.
fn sub_dir(parent: &File, name: &str) -> Option<File> {
    let name = CString::new(name).ok()?;
    open_at(parent, &name, libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW, 0).ok()
}

/// The entry names of a held directory, read through its descriptor (not its path), at most `MAX_ENTRIES`.
fn entry_names(dir: &File) -> Vec<String> {
    // SAFETY: dup of a valid descriptor; on success `fdopendir` owns the copy and `closedir` closes it.
    let fd = unsafe { libc::dup(dir.as_raw_fd()) };
    if fd < 0 {
        return Vec::new();
    }
    // SAFETY: `fd` is a directory descriptor we own.
    let handle = unsafe { libc::fdopendir(fd) };
    if handle.is_null() {
        // SAFETY: `fdopendir` failed, so `fd` is still ours to close.
        unsafe { libc::close(fd) };
        return Vec::new();
    }
    let mut names = Vec::new();
    while names.len() < MAX_ENTRIES {
        // SAFETY: `handle` is an open DIR; the entry is read before the next call.
        let ent = unsafe { libc::readdir(handle) };
        if ent.is_null() {
            break;
        }
        // SAFETY: `d_name` is a NUL-terminated name inside the entry.
        let name = unsafe { CStr::from_ptr((*ent).d_name.as_ptr()) };
        if let Ok(n) = name.to_str() {
            if n != "." && n != ".." {
                names.push(n.to_owned());
            }
        }
    }
    // SAFETY: closes the DIR and the duplicated descriptor with it.
    unsafe { libc::closedir(handle) };
    names
}

/// Open `rollout-*-<thread_id>.jsonl` under `<codex_home>/sessions/<y>/<m>/<d>/`. The walk starts from the held home and
/// every step is a no-follow open relative to the step before, so a link at the home, at `sessions`, at a date folder or
/// at the file itself is refused and cannot lead the read outside the Vajra-owned home. The file is returned open and
/// checked to be a regular file, so a name swapped for a link after the lookup is not followed either.
fn rollout_file(codex_home: &Path, thread_id: &str) -> Option<File> {
    let suffix = format!("-{thread_id}.jsonl");
    let home = fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW).open(codex_home).ok()?;
    let mut level = vec![sub_dir(&home, "sessions")?];
    for _ in 0..3 {
        let mut next = Vec::new();
        for dir in &level {
            for name in entry_names(dir) {
                next.extend(sub_dir(dir, &name));
            }
        }
        level = next;
    }
    level.iter().find_map(|day| {
        entry_names(day).into_iter().filter(|n| n.starts_with("rollout-") && n.ends_with(&suffix)).find_map(|n| {
            let name = CString::new(n).ok()?;
            let file = open_at(day, &name, libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK, 0).ok()?;
            file.metadata().ok()?.is_file().then_some(file)
        })
    })
}

/// What a Codex worker ran on, from its event stream (for the `thread_id`) and its session file. With no thread or no
/// readable file everything stays unknown and `read_from` is `None`.
pub fn read_codex(events: &str, codex_home: &Path) -> Effective {
    let mut eff = Effective::unknown("codex", true);
    let Some(file) = codex_thread_id(events).and_then(|t| rollout_file(codex_home, &t)) else { return eff };
    let mut text = String::new();
    if file.take(MAX_READ).read_to_string(&mut text).is_err() {
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
/// effective effort is `None`. The stream does not say how the CLI was signed in (plan or API key), so billing stays
/// unknown.
pub fn read_claude(events: &str) -> Effective {
    let mut eff = Effective::unknown("claude", false);
    let init = events.lines().take(MAX_RECORDS).filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok()).find(|e| e["type"] == "system" && e["subtype"] == "init");
    if let Some(model) = init.as_ref().and_then(|e| clean(e.get("model"))) {
        eff.model = model;
        eff.account = "claude".into();
        eff.read_from = Some(FROM_EVENT.into());
    }
    eff
}

/// A worker's change set as git reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Diff {
    /// The first `MAX_LISTED` changed paths (lossy text, for display).
    pub files: Vec<String>,
    /// How many files changed in all.
    pub files_total: u64,
    pub added: u64,
    pub removed: u64,
    /// Some new files' lines were not counted (a bound ran out, a file was too large, or it is not a plain file or path):
    /// `added` is then a lower bound. Every file is still counted in `files_total`.
    pub partial: bool,
    /// How many new files were not counted.
    pub uncounted: u64,
}

/// The diff of `cwd` (the worker's worktree) against `base`, read through the read-only git bridge: tracked changes
/// from `git diff --numstat <base>` and new files from `git ls-files --others --exclude-standard`. A new file's lines are
/// counted by reading it directly (no git process per file), and count as added. `None` when git could not answer; an
/// empty `Diff` means git saw no change.
///
/// It reads files (bounded: `PER_FILE_CAP`, `TOTAL_READ_CAP`, `MAX_UNTRACKED`, `UNTRACKED_BUDGET`), so it is for a receipt
/// built on a background thread. It must never run on the `project_state` poll path.
pub fn diff_from_git(bridge: &Bridge, cwd: &Path, base: &str) -> Option<Diff> {
    diff_from_git_within(bridge, cwd, base, UNTRACKED_BUDGET)
}

fn diff_from_git_within(bridge: &Bridge, cwd: &Path, base: &str, budget: Duration) -> Option<Diff> {
    let git = |args: &[&str]| {
        let out = bridge.run(&Request::Git { cwd: cwd.to_path_buf(), args: args.iter().map(|a| a.to_string()).collect() }).ok()?;
        (out.code == Some(0) && out.whole()).then_some(out.stdout_raw)
    };
    // `-z` for both: paths come back NUL-terminated and unquoted, so a name with a space, quote, newline, tab or non-ASCII
    // character is its real path, and a rename is two plain records. They are split as bytes: a name that is not UTF-8
    // keeps its exact bytes for the read below (only the listed copy is lossy). A submodule's contents are not diffed.
    let tracked = git(&["diff", "--numstat", "-z", "--ignore-submodules=all", base, "--"])?;
    // `git ls-files` has no `--ignore-submodules`; a submodule or nested repository is listed as `dir/` and is not counted.
    let untracked = git(&["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut diff = Diff::default();
    let list = |diff: &mut Diff, path: &[u8]| {
        diff.files_total += 1;
        if diff.files.len() < MAX_LISTED {
            diff.files.push(String::from_utf8_lossy(path).into_owned());
        }
    };
    let mut records = tracked.split(|b| *b == 0).filter(|r| !r.is_empty());
    while let Some(rec) = records.next() {
        // `added<TAB>removed<TAB>path`; a rename or copy has an empty path there, followed by the old and new paths.
        let mut f = rec.splitn(3, |b| *b == b'\t');
        let (a, r, path) = (f.next()?, f.next()?, f.next()?);
        match (numstat_count(a), numstat_count(r)) {
            (Some(a), Some(r)) => (diff.added, diff.removed) = (diff.added + a, diff.removed + r),
            // Counts git did not give in a form we know: the file is listed, the totals are a lower bound.
            _ => diff.partial = true,
        }
        if path.is_empty() {
            let (_old, new) = (records.next()?, records.next()?);
            list(&mut diff, new);
        } else {
            list(&mut diff, path);
        }
    }
    // New files: a direct, link-safe read of each. Past `MAX_UNTRACKED` files, `UNTRACKED_BUDGET` or `TOTAL_READ_CAP`
    // counting stops and the rest are listed only (`partial`, `uncounted`).
    let root = open_dir_nofollow(cwd);
    let (start, mut counted, mut read_left) = (Instant::now(), 0usize, TOTAL_READ_CAP);
    let deadline = start + budget;
    for path in untracked.split(|b| *b == 0).filter(|l| !l.is_empty()) {
        list(&mut diff, path);
        let out_of_bounds = counted >= MAX_UNTRACKED || start.elapsed() >= budget || read_left == 0;
        let n = match (&root, out_of_bounds) {
            (Some(root), false) => {
                counted += 1;
                count_new_file_at(root, path, &mut read_left, deadline)
            }
            _ => None,
        };
        match n {
            Some(n) => diff.added += n,
            None => (diff.partial, diff.uncounted) = (true, diff.uncounted + 1),
        }
    }
    Some(diff)
}

/// One `--numstat` count: a number, or `-` for a binary file (changed, no line counts). Anything else is unreadable.
fn numstat_count(field: &[u8]) -> Option<u64> {
    if field == b"-" {
        return Some(0);
    }
    if field.is_empty() || !field.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(field).ok()?.parse().ok()
}

fn open_dir_nofollow(path: &Path) -> Option<File> {
    fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path).ok()
}

/// The lines a new file adds, matching `git diff --numstat --no-index /dev/null <file>`, or `None` when it was not counted
/// (the path is refused, a link is in the way, the file is too large, it is not a plain file, the read cap ran out, or
/// `deadline` passed while it was being read).
///
/// `rel` is the path git listed, as bytes, relative to `root`. Every directory part is opened with `O_NOFOLLOW`, so a
/// symlinked directory partway down is never entered. The last part is looked at with `fstatat(AT_SYMLINK_NOFOLLOW)`:
/// - a symlink counts 1 line (its target text, as git does) and is not followed;
/// - a plain file is reopened with `O_NOFOLLOW|O_NONBLOCK` and checked with `fstat`: its lines are its `\n` count, plus 1
///   when it is non-empty and does not end in one; a NUL in its first 8000 bytes means binary, which counts 0;
/// - a FIFO, socket or device is never opened.
fn count_new_file_at(root: &File, rel: &[u8], read_left: &mut u64, deadline: Instant) -> Option<u64> {
    if rel.is_empty() || rel.len() >= libc::PATH_MAX as usize || rel.last() == Some(&b'/') {
        return None;
    }
    let parts: Vec<&[u8]> = rel.split(|b| *b == b'/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == b"." || *p == b".." || p.contains(&0)) {
        return None;
    }
    let (last, dirs) = parts.split_last()?;
    let mut held: Option<File> = None;
    for d in dirs {
        let name = CString::new(*d).ok()?;
        let next = open_at(held.as_ref().unwrap_or(root), &name, libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK, 0).ok()?;
        held = Some(next);
    }
    let dir = held.as_ref().unwrap_or(root);
    let name = CString::new(*last).ok()?;
    // SAFETY: a valid directory fd, a NUL-terminated name and a zeroed out-struct, all live for this call.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatat(dir.as_raw_fd(), name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return None;
    }
    match st.st_mode & libc::S_IFMT {
        libc::S_IFLNK => Some(1),
        libc::S_IFREG => {
            let mut f = open_at(dir, &name, libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK, 0).ok()?;
            let size = f.metadata().ok().filter(|m| m.is_file())?.len();
            if size > PER_FILE_CAP || size > *read_left {
                return None;
            }
            count_lines(&mut f, size, read_left, deadline)
        }
        _ => None,
    }
}

/// `deadline` is checked before every chunk and before the count is accepted, so a slow read cannot outlast the budget:
/// past it the file is uncounted (`None`), never counted late.
fn count_lines(f: &mut File, size: u64, read_left: &mut u64, deadline: Instant) -> Option<u64> {
    let mut buf = [0u8; 64 * 1024];
    let (mut seen, mut lines, mut last) = (0u64, 0u64, b'\n');
    while seen < size {
        if Instant::now() >= deadline {
            return None;
        }
        let want = buf.len().min((size - seen) as usize);
        let n = f.read(&mut buf[..want]).ok()?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        // The probe is the first BINARY_PROBE bytes of the file, which may span chunks.
        let probe_end = BINARY_PROBE.saturating_sub(seen as usize).min(n);
        if chunk[..probe_end].contains(&0) {
            *read_left = read_left.saturating_sub(seen + n as u64);
            return Some(0);
        }
        lines += chunk.iter().filter(|b| **b == b'\n').count() as u64;
        last = chunk[n - 1];
        seen += n as u64;
    }
    *read_left = read_left.saturating_sub(seen);
    if Instant::now() >= deadline {
        return None;
    }
    Some(lines + u64::from(seen > 0 && last != b'\n'))
}

#[cfg(test)]
#[path = "effective_tests.rs"]
mod tests;
