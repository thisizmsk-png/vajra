//! The core's only git writer (ADR-16 16.6). Beside the read-only bridge; no free-form arguments.
//!
//! Every call runs `/usr/bin/git` on the bridge's fixed PATH and scrubbed environment, with `--no-optional-locks`, the
//! fixed `-c` flags of `FIXED`, and:
//! - `GIT_CONFIG_GLOBAL=<the global baseline copy>` (taken once at app start, before any agent session) and
//!   `GIT_CONFIG_NOSYSTEM=1`, so a later edit to `~/.gitconfig` or a file it includes never reaches the core's git;
//! - `GIT_ATTR_NOSYSTEM=1`, `GIT_LFS_SKIP_SMUDGE=1`, `GIT_TERMINAL_PROMPT=0`, `GIT_LITERAL_PATHSPECS=1`,
//!   `GIT_NO_REPLACE_OBJECTS=1`, and `GIT_ATTR_SOURCE=<dispatch_head>` where git is 2.40 or later;
//! - on a worker's worktree, `GIT_DIR`, `GIT_COMMON_DIR` (the recorded one) and `GIT_WORK_TREE`, so git never reads the
//!   worktree's `.git` file or `commondir`.
//!
//! Before every call the global baseline copy is checked against the sha256 taken at load. Before every call on a repo
//! the repo's `<common>/config`, `config.worktree`, every file they include, and `info/attributes` are checked
//! against the repo baseline (`repo_config_changed`). Before every call on a ticket its per-worktree files, its common
//! directory's identity and its detached HEAD are checked with plain filesystem reads (`tampered`), and before any
//! call that reads worktree content a plain walk refuses new or changed attribute and module files and nested repos,
//! names case-folded. The repo baseline also fingerprints every filter, merge-driver and diff program the trusted
//! config names, as git's shell would resolve it, and any of them other than git-lfs keeps the repo from content calls
//! until the human accepts it (git runs them in the worktree, where a worker can plant the code they load). Every call that can run one stops the repo's other live agent
//! sessions first (`Quiesce`), so the checks and git's use see the same files. `snapshot` uses plumbing only, through a fresh private index under the core's data dir; it
//! never writes through the worker's HEAD or index.

use std::collections::{BTreeSet, HashMap};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::bridge::{run_process_env, Output, BASE_PATH};
use crate::control::sha256;

/// Every call's fixed config. `core.sshCommand=false` and `protocol.allow=never` stop every transport.
const FIXED: [&str; 11] = [
    "core.hooksPath=/dev/null",
    "core.fsmonitor=",
    "core.attributesFile=/dev/null",
    "core.sshCommand=false",
    "protocol.allow=never",
    "submodule.recurse=false",
    "gc.auto=0",
    "maintenance.auto=false",
    "rerere.enabled=false",
    "credential.helper=",
    "core.useReplaceRefs=false",
];

/// Calls that create a commit on a `vajra/` ref (N10).
const NO_SIGN: &str = "commit.gpgsign=false";

pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The identity used when the repo has none (16.6). The commit trailer names who did the work.
const FALLBACK_NAME: &str = "user.name=Vajra";
const FALLBACK_EMAIL: &str = "user.email=vajra@localhost";

/// Agent config files (16.4 rule 6): never committed unless the ask names them.
const AGENT_DIRS: [&str; 3] = [".claude", ".codex", ".husky"];
const AGENT_FILES: [&str; 4] = ["CLAUDE.md", "AGENTS.md", ".mcp.json", ".envrc"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitwError {
    /// A name, ref or path the core built is not allowed; nothing ran.
    Invalid(&'static str),
    /// `<work>` and `<data>` overlap.
    WorktreeRootInvalid,
    /// The repo's own config files differ from the repo baseline. Names only, never values.
    RepoConfigChanged { keys: Vec<String> },
    /// A tamper check failed; `what` names the file or check.
    Tampered { what: String },
    DepsConflict { sha: String },
    MergeConflict { sha: String },
    /// Identity, signing configured some other way, or the timeout: git's first error line.
    CommitFailed { first_line: String },
    /// The task ref, worktree or dir already exists.
    Exists,
    /// The capture check failed: the tree is kept, and these paths differ.
    Kept { paths: Vec<String> },
    Git { step: &'static str, first_line: String },
    Io(String),
}

impl GitwError {
    pub fn code(&self) -> &'static str {
        match self {
            GitwError::Invalid(_) => "invalid",
            GitwError::WorktreeRootInvalid => "worktree_root_invalid",
            GitwError::RepoConfigChanged { .. } => "repo_config_changed",
            GitwError::Tampered { .. } => "tampered",
            GitwError::DepsConflict { .. } => "deps_conflict",
            GitwError::MergeConflict { .. } => "merge_conflict",
            GitwError::CommitFailed { .. } => "commit_failed",
            GitwError::Exists => "exists",
            GitwError::Kept { .. } => "kept",
            GitwError::Git { .. } => "git_failed",
            GitwError::Io(_) => "io",
        }
    }
}

type R<T> = Result<T, GitwError>;

fn io(e: std::io::Error) -> GitwError {
    GitwError::Io(e.to_string())
}

fn tampered(what: impl Into<String>) -> GitwError {
    GitwError::Tampered { what: what.into() }
}

fn first_line(o: &Output) -> String {
    let l = o.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or(if o.timed_out { "timed out" } else { "" });
    l.chars().take(200).collect()
}

// ------------------------------------------------------------ roots

/// `<work>`: `~/Library/Application Support/VajraWorktrees`, a sibling of the app data dir that agents may write.
pub fn work_root(home: &Path) -> PathBuf {
    home.join("Library/Application Support/VajraWorktrees")
}

/// At startup: both roots exist (`<work>` is created 0700) and neither canonical path contains the other.
pub fn check_roots(data: &Path, work: &Path) -> R<()> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(work).map_err(io)?;
    let (d, w) = (data.canonicalize().map_err(io)?, work.canonicalize().map_err(io)?);
    if d.starts_with(&w) || w.starts_with(&d) {
        return Err(GitwError::WorktreeRootInvalid);
    }
    Ok(())
}

/// Create `root/rel` one component at a time. Each component must end up a real directory owned by us, never a
/// symlink: a planted link under `<work>` must not move a ticket's `tmp` or worktree elsewhere (#146 gate M4).
fn make_dirs(root: &Path, rel: &[&str]) -> R<PathBuf> {
    let mut cur = root.canonicalize().map_err(io)?;
    let uid = unsafe { libc::geteuid() };
    for c in rel {
        if !valid_name(c) && *c != "tmp" && *c != "tickets" {
            return Err(GitwError::Invalid("directory name"));
        }
        cur.push(c);
        match std::fs::DirBuilder::new().mode(0o700).create(&cur) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io(e)),
        }
        let m = std::fs::symlink_metadata(&cur).map_err(io)?;
        if m.file_type().is_symlink() || !m.is_dir() || m.uid() != uid {
            return Err(tampered(format!("{} is not a directory Vajra made", rel.join("/"))));
        }
    }
    Ok(cur)
}

/// Remove a core-built dir only while its recorded canonical path still resolves to itself under one of the roots, so
/// a parent swapped for a symlink never redirects the delete.
fn remove_own_dir(d: &Path, roots: &[&Path]) {
    let here = d.canonicalize().ok();
    if here.as_deref() == Some(d) && roots.iter().filter_map(|r| r.canonicalize().ok()).any(|r| d.starts_with(&r) && d != r) {
        let _ = std::fs::remove_dir_all(d);
    }
}

// ------------------------------------------------------------ hashing

/// A file's state for a tamper or baseline check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileHash {
    Absent,
    Unreadable,
    Sha(String),
}

const HASH_MAX: u64 = 4 << 20;

/// Read a file without blocking on a FIFO or growing without bound. `follow` is for the user's own global files (a
/// dotfiles link is normal there); repo and worktree files are opened with `O_NOFOLLOW`.
fn read_bounded(p: &Path, follow: bool) -> Result<Option<Vec<u8>>, ()> {
    use std::io::Read;
    let flags = libc::O_NONBLOCK | if follow { 0 } else { libc::O_NOFOLLOW };
    let f = match std::fs::OpenOptions::new().read(true).custom_flags(flags).open(p) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    if !f.metadata().map_err(|_| ())?.is_file() {
        return Err(());
    }
    let mut b = Vec::new();
    f.take(HASH_MAX + 1).read_to_end(&mut b).map_err(|_| ())?;
    if b.len() as u64 > HASH_MAX {
        return Err(());
    }
    Ok(Some(b))
}

pub fn hash_file(p: &Path, follow: bool) -> FileHash {
    match read_bounded(p, follow) {
        Ok(None) => FileHash::Absent,
        Ok(Some(b)) => FileHash::Sha(sha256::hex_digest(&b)),
        Err(()) => FileHash::Unreadable,
    }
}

// ------------------------------------------------------------ the git runner

/// Where a call runs: in a repo by `-C`, or pinned to a worktree.
enum At<'a> {
    Repo(&'a Path),
    Pinned(&'a Ticket),
}

struct Call<'a> {
    at: At<'a>,
    args: Vec<String>,
    index: Option<&'a Path>,
    /// Extra `-c` entries (identity, `commit.gpgsign=false`).
    config: Vec<&'static str>,
    timeout: Duration,
}

impl<'a> Call<'a> {
    fn new(at: At<'a>, args: &[&str]) -> Self {
        Call { at, args: args.iter().map(|s| s.to_string()).collect(), index: None, config: Vec::new(), timeout: CALL_TIMEOUT }
    }
    fn index(mut self, i: &'a Path) -> Self {
        self.index = Some(i);
        self
    }
    fn config(mut self, c: &[&'static str]) -> Self {
        self.config.extend_from_slice(c);
        self
    }
}

/// The global baseline (16.6 "The trusted git baseline"): one flattened copy of the user's global config with its
/// includes resolved, taken at app start and kept across restarts, plus the hashes of the files it came from (a card
/// note when they change, never a tamper signal).
///
/// The copy lives in `<data>/git-baseline/`, which a Claude worker with no OS sandbox could write. So its sha256 is
/// kept in memory (`Gitw` checks it before every call) and, across restarts, in `<control>/git-baseline.sha256`, which
/// the guard write-protects. A copy that no longer matches is `Tampered` and is never used: only the human re-takes it.
/// The copy itself lives in `<control>/git-baseline/` too, so the window between the core's check and git's own read
/// of `GIT_CONFIG_GLOBAL` needs a write the guard refuses (#146 re-check N4).
#[derive(Clone, Debug)]
pub struct GlobalBaseline {
    pub copy: PathBuf,
    /// The copy's sha256 when it was taken or loaded.
    pub sha: String,
    sources: Vec<(PathBuf, FileHash)>,
}

const BASELINE_TAMPERED: &str = "the global git baseline";

fn baseline_dir(data: &Path) -> PathBuf {
    data.join("control").join("git-baseline")
}

fn baseline_anchor(data: &Path) -> PathBuf {
    data.join("control").join("git-baseline.sha256")
}

impl GlobalBaseline {
    /// Load the stored baseline, or take one when there is none. Call before any agent session launches.
    pub fn take_or_load(git: &Path, data: &Path, home: &Path, xdg: Option<&Path>) -> R<GlobalBaseline> {
        let dir = baseline_dir(data);
        crate::paths::ensure_private_dir(&data.join("control")).map_err(io)?;
        crate::paths::ensure_private_dir(&dir).map_err(io)?;
        let copy = dir.join("global.config");
        let anchor = hash_file(&baseline_anchor(data), false);
        let stored = hash_file(&copy, false);
        match (&stored, &anchor) {
            (FileHash::Absent, FileHash::Absent) => Self::take(git, data, home, xdg),
            (FileHash::Sha(have), FileHash::Sha(_)) if read_bounded(&baseline_anchor(data), false).ok().flatten().is_some_and(|a| a.trim_ascii() == have.as_bytes()) => {
                let sources = read_bounded(&dir.join("sources.json"), false).ok().flatten().and_then(|b| parse_sources(&String::from_utf8_lossy(&b))).ok_or_else(|| tampered(BASELINE_TAMPERED))?;
                Ok(GlobalBaseline { copy, sha: have.clone(), sources })
            }
            // A copy without its anchor, an anchor without its copy, or a mismatch.
            _ => Err(tampered(BASELINE_TAMPERED)),
        }
    }

    /// The human re-takes a tampered baseline from the live global config: only while no Vajra agent session runs.
    pub fn retake(git: &Path, data: &Path, home: &Path, xdg: Option<&Path>, agents_running: bool) -> R<GlobalBaseline> {
        if agents_running {
            return Err(GitwError::Invalid("a Vajra agent session is running"));
        }
        Self::take(git, data, home, xdg)
    }

    fn take(git: &Path, data: &Path, home: &Path, xdg: Option<&Path>) -> R<GlobalBaseline> {
        let dir = baseline_dir(data);
        crate::paths::ensure_private_dir(&data.join("control")).map_err(io)?;
        crate::paths::ensure_private_dir(&dir).map_err(io)?;
        let flat = flatten_global(git, home, xdg)?;
        let copy = dir.join("global.config");
        write_private(&copy, flat.as_bytes())?;
        let sources = global_sources(git, home, xdg);
        write_private(&dir.join("sources.json"), render_sources(&sources).as_bytes())?;
        let sha = sha256::hex_digest(flat.as_bytes());
        // The anchor last: a crash before it leaves a copy with no anchor, which only the human re-takes.
        write_private(&baseline_anchor(data), format!("{sha}\n").as_bytes())?;
        Ok(GlobalBaseline { copy, sha, sources })
    }

    /// The global files whose bytes differ from when the baseline was taken (for the card note
    /// "Your git settings changed since Vajra started").
    pub fn changed_files(&self, git: &Path, home: &Path, xdg: Option<&Path>) -> Vec<PathBuf> {
        let now = global_sources(git, home, xdg);
        let mut out: Vec<PathBuf> = now.iter().filter(|(p, h)| self.sources.iter().find(|(q, _)| q == p).map(|(_, g)| g) != Some(h)).map(|(p, _)| p.clone()).collect();
        out.extend(self.sources.iter().filter(|(p, _)| !now.iter().any(|(q, _)| q == p)).map(|(p, _)| p.clone()));
        out
    }

    /// The config key names that differ between the baseline and the live global config. Names only.
    pub fn changed_keys(&self, git: &Path, home: &Path, xdg: Option<&Path>) -> Vec<String> {
        let live = flatten_entries(git, home, xdg).unwrap_or_default();
        let base = read_config_file(git, &self.copy, home).unwrap_or_default();
        diff_keys(&base, &live)
    }

    /// "Use my new git settings": human-only, and only while no Vajra agent session runs anywhere. Not reachable from
    /// MCP or the socket: the service exposes no command for it.
    pub fn replace(&mut self, git: &Path, home: &Path, xdg: Option<&Path>, agents_running: bool) -> R<()> {
        if agents_running {
            return Err(GitwError::Invalid("a Vajra agent session is running"));
        }
        let data = self.copy.parent().and_then(Path::parent).and_then(Path::parent).ok_or(GitwError::Invalid("baseline dir"))?.to_path_buf();
        *self = Self::take(git, &data, home, xdg)?;
        Ok(())
    }
}

/// The keys whose effect calls for a highlight on the settings-changed card.
pub fn highlighted(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.starts_with("filter.") || k.ends_with(".driver") || k.starts_with("gpg.") || k.starts_with("include") || ["core.hookspath", "core.fsmonitor", "core.sshcommand"].contains(&k.as_str())
}

fn diff_keys(a: &[(String, String)], b: &[(String, String)]) -> Vec<String> {
    let set = |v: &[(String, String)]| -> BTreeSet<(String, String)> { v.iter().cloned().collect() };
    let (sa, sb) = (set(a), set(b));
    let keys: BTreeSet<String> = sa.symmetric_difference(&sb).map(|(k, _)| k.clone()).collect();
    keys.into_iter().collect()
}

/// Write through a fresh temp file and rename. The temp name is unpredictable and opened `O_EXCL|O_NOFOLLOW`, so a
/// planted file or symlink at it fails the write instead of redirecting it.
fn write_private(p: &Path, bytes: &[u8]) -> R<()> {
    use std::io::Write;
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut name = p.file_name().ok_or(GitwError::Invalid("file name"))?.to_os_string();
    name.push(format!(".tmp{}-{n}-{nanos:08x}", std::process::id()));
    let tmp = p.with_file_name(name);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).custom_flags(libc::O_NOFOLLOW).mode(0o600).open(&tmp).map_err(io)?;
    let r = f.write_all(bytes).and_then(|()| f.sync_all()).and_then(|()| std::fs::rename(&tmp, p));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r.map_err(io)
}

fn render_sources(s: &[(PathBuf, FileHash)]) -> String {
    let v: Vec<serde_json::Value> = s
        .iter()
        .map(|(p, h)| serde_json::json!({"path": p, "hash": match h { FileHash::Absent => "absent".to_string(), FileHash::Unreadable => "unreadable".into(), FileHash::Sha(x) => x.clone() }}))
        .collect();
    serde_json::Value::Array(v).to_string()
}

fn parse_sources(t: &str) -> Option<Vec<(PathBuf, FileHash)>> {
    let v: serde_json::Value = serde_json::from_str(t).ok()?;
    v.as_array()?
        .iter()
        .map(|e| {
            let p = PathBuf::from(e["path"].as_str()?);
            let h = match e["hash"].as_str()? {
                "absent" => FileHash::Absent,
                "unreadable" => FileHash::Unreadable,
                x => FileHash::Sha(x.into()),
            };
            Some((p, h))
        })
        .collect()
}

fn xdg_config(home: &Path, xdg: Option<&Path>) -> PathBuf {
    xdg.map(Path::to_path_buf).unwrap_or_else(|| home.join(".config")).join("git/config")
}

/// Run git to read config only (no repo, no filters): the user's HOME and XDG, no system file.
fn config_read(git: &Path, home: &Path, xdg: Option<&Path>, args: &[&str]) -> Option<Output> {
    let argv: Vec<String> = ["--no-pager", "-c", "core.fsmonitor=", "-c", "core.hooksPath=/dev/null"].iter().chain(args).map(|s| s.to_string()).collect();
    let xdg_env = xdg.map(|x| x.to_path_buf()).unwrap_or_else(|| home.join(".config"));
    let env: [(&str, &Path); 1] = [("XDG_CONFIG_HOME", &xdg_env)];
    // Run outside any repository so only the named scope is read.
    run_process_env(git, &argv, Some(Path::new("/")), CALL_TIMEOUT, Some(home), None, &env).ok().filter(|o| o.whole())
}

/// `git config --list -z` output (`key LF value NUL`, or `key NUL` for a bare key).
fn parse_list_z(s: &str) -> Vec<(String, String)> {
    s.split('\0').filter(|x| !x.is_empty()).map(|item| match item.split_once('\n') {
        Some((k, v)) => (k.to_string(), v.to_string()),
        // A bare key (no `=`) means true; rendering it empty would read as false.
        None => (item.to_string(), "true".to_string()),
    }).collect()
}

/// The live global config, includes resolved, as entries.
fn flatten_entries(git: &Path, home: &Path, xdg: Option<&Path>) -> R<Vec<(String, String)>> {
    let o = config_read(git, home, xdg, &["config", "--global", "--includes", "--list", "-z"]).ok_or_else(|| GitwError::Io("git config did not finish".into()))?;
    // Exit 1 or 128 with no global file at all is an empty config, never a failure to take the baseline.
    if o.exit() != Some(0) && !o.stdout.is_empty() {
        return Err(GitwError::Git { step: "config", first_line: first_line(&o) });
    }
    Ok(parse_list_z(&o.stdout).into_iter().filter(|(k, _)| !k.to_ascii_lowercase().starts_with("include")).collect())
}

fn read_config_file(git: &Path, file: &Path, home: &Path) -> Option<Vec<(String, String)>> {
    let f = file.to_str()?;
    let o = config_read(git, home, None, &["config", "--file", f, "--no-includes", "--list", "-z"])?;
    (o.exit() == Some(0) || o.stdout.is_empty()).then(|| parse_list_z(&o.stdout))
}

/// Render entries as a git config file (subsections and values quoted and escaped).
fn render_config(entries: &[(String, String)]) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t");
    let mut out = String::new();
    for (k, v) in entries {
        let Some((section, rest)) = k.split_once('.') else { continue };
        let (sub, name) = match rest.rsplit_once('.') {
            Some((s, n)) => (Some(s), n),
            None => (None, rest),
        };
        if section.is_empty() || name.is_empty() || !section.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            continue;
        }
        match sub {
            Some(s) => out.push_str(&format!("[{section} \"{}\"]\n", esc(s))),
            None => out.push_str(&format!("[{section}]\n")),
        }
        out.push_str(&format!("\t{name} = \"{}\"\n", esc(v)));
    }
    out
}

fn flatten_global(git: &Path, home: &Path, xdg: Option<&Path>) -> R<String> {
    Ok(render_config(&flatten_entries(git, home, xdg)?))
}

/// `~/.gitconfig`, the XDG file and every include target (following git's path rules, to depth 10), each hashed.
fn global_sources(git: &Path, home: &Path, xdg: Option<&Path>) -> Vec<(PathBuf, FileHash)> {
    config_sources(git, home, xdg, &[home.join(".gitconfig"), xdg_config(home, xdg)], true).0
}

/// At most this many config files in one include graph. A repo graph with more is refused, never trusted in part.
const INCLUDE_MAX: usize = 64;

/// The config files and every `include.path` / `includeIf.*.path` target below them (to depth 10), each hashed,
/// whatever the include condition says. `follow_roots` is false for repo files (opened `O_NOFOLLOW`); include targets
/// may be a dotfiles link. The second value is why the list is partial: the graph holds more than `INCLUDE_MAX` files,
/// or an include path `include_target` can't resolve. A caller that trusts the list must then refuse (#146 codex).
fn config_sources(git: &Path, home: &Path, xdg: Option<&Path>, roots: &[PathBuf], follow_roots: bool) -> (Vec<(PathBuf, FileHash)>, Option<String>) {
    let mut out: Vec<(PathBuf, FileHash)> = Vec::new();
    let mut partial = None;
    // Breadth-first from every root: a file is first reached at its shallowest depth, so the visited check never
    // skips a shorter path git reads (a file first met at the depth cap, unexpanded, and then included directly)
    // (#146 codex).
    let mut queue: std::collections::VecDeque<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    while let Some((p, depth)) = queue.pop_front() {
        if out.iter().any(|(q, _)| q == &p) {
            continue;
        }
        if out.len() >= INCLUDE_MAX {
            partial = Some(format!("more than {INCLUDE_MAX} config files"));
            break;
        }
        let h = hash_file(&p, depth > 0 || follow_roots);
        let present = matches!(h, FileHash::Sha(_));
        out.push((p.clone(), h));
        if !present || depth >= 10 {
            continue;
        }
        let Some(f) = p.to_str() else {
            // Its includes can't be read back through git's argv: partial, never skipped (#146 bhishma delta).
            partial = Some("a config path that is not UTF-8".into());
            continue;
        };
        let Some(o) = config_read(git, home, xdg, &["config", "--file", f, "--no-includes", "--get-regexp", "-z", "^include(if\\..*)?\\.path$"]) else { continue };
        for (_, target) in parse_list_z(&o.stdout) {
            // Output is decoded lossily: a target that wasn't UTF-8 names a different file than git reads.
            if target.contains('\u{FFFD}') {
                partial = Some("an include path that is not UTF-8".into());
                continue;
            }
            match include_target(&target, &p, home) {
                Some(t) => queue.push_back((t, depth + 1)),
                None => partial = Some("a ~user or %(prefix) path Vajra can't resolve".into()),
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    (out, partial)
}

/// An include path read the way git's pathname rules read it: `~` and `~/` are HOME, `~user` and `~user/` that user's
/// home from the passwd database (as git's `getpwnam`), an absolute path is itself, anything else is relative to the
/// including file. A `~user` with no passwd entry and `%(prefix)/` (git's install prefix) are `None`: the caller
/// refuses the graph rather than hash a path git wouldn't read (#146 codex).
fn include_target(target: &str, from: &Path, home: &Path) -> Option<PathBuf> {
    if let Some(rest) = target.strip_prefix('~') {
        let (user, tail) = rest.split_once('/').unwrap_or((rest, ""));
        let base = if user.is_empty() { home.to_path_buf() } else { user_home(user)? };
        // A home that isn't UTF-8 can't be handed to git's argv to read its own includes.
        base.to_str()?;
        return Some(if tail.is_empty() { base } else { base.join(tail) });
    }
    if target.starts_with("%(prefix)/") {
        return None;
    }
    Some(if Path::new(target).is_absolute() { PathBuf::from(target) } else { from.parent().unwrap_or(Path::new("/")).join(target) })
}

#[cfg(test)]
thread_local! {
    /// Tests: `~user` homes that `user_home` returns instead of the passwd database's (no test touches a real home).
    pub(crate) static USER_HOMES: std::cell::RefCell<Vec<(String, PathBuf)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A user's home directory from the passwd database.
fn user_home(name: &str) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(d) = USER_HOMES.with(|u| u.borrow().iter().find(|(n, _)| n == name).map(|(_, d)| d.clone())) {
        return Some(d);
    }
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(name).ok()?;
    // SAFETY: a zeroed `passwd` is a valid out-parameter; `buf` outlives the call and `pw`'s pointers into it, which
    // are copied out before it drops; `getpwnam_r` is the reentrant form.
    unsafe {
        let mut pw: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 16 * 1024];
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        if libc::getpwnam_r(c.as_ptr(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut found) != 0 || found.is_null() || pw.pw_dir.is_null() {
            return None;
        }
        let dir = std::ffi::CStr::from_ptr(pw.pw_dir).to_bytes();
        (!dir.is_empty()).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(dir)))
    }
}

// ------------------------------------------------------------ quiescing other sessions (M3)

/// While the core's git reads or writes a repo's content (filters and drivers can run), every other live Vajra agent
/// session on that repo is stopped, so none can change the repo's config, attributes or the programs they name
/// between the core's check and git's use of them (#146 gate M3). The checks run again once they are stopped.
pub trait Quiesce: Send + Sync + std::fmt::Debug {
    /// Stop every live agent session on the repo with this common dir; they resume when the `Pause` drops.
    fn pause(&self, common: &Path) -> Pause;
}

/// Resumes what `Quiesce::pause` stopped, on drop (also on an early `?` return or a panic).
pub struct Pause(Option<Box<dyn FnOnce() + Send>>);

impl Pause {
    pub fn new(resume: impl FnOnce() + Send + 'static) -> Pause {
        Pause(Some(Box::new(resume)))
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

/// Live pauses on one repo, and every session id stopped for them.
type PauseCount = (usize, Vec<String>);

/// How often a held pause stops its sessions again.
const RESTOP_MS: u64 = 3;

/// The live session ids on the repo with this common dir.
pub type SessionsOnRepo = Arc<dyn Fn(&Path) -> Vec<String> + Send + Sync>;

/// The agent sessions the core started itself, each against the common dir its ticket pinned (#151 codex P1). A core
/// worker runs in a linked worktree outside the repo's folder, so a cwd match would never name it: it is registered
/// before its process starts and dropped when the attempt ends.
#[derive(Debug, Default)]
pub struct RepoSessions(Mutex<HashMap<String, PathBuf>>);

impl RepoSessions {
    pub fn register(&self, session: &str, common: &Path) {
        crate::lock(&self.0).insert(session.to_string(), common.to_path_buf());
    }

    pub fn unregister(&self, session: &str) {
        crate::lock(&self.0).remove(session);
    }

    /// The registered sessions on the repo with this common dir.
    pub fn on(&self, common: &Path) -> Vec<String> {
        crate::lock(&self.0).iter().filter(|(_, c)| c.as_path() == common).map(|(s, _)| s.clone()).collect()
    }
}

/// The production `Quiesce`: `on_repo` names the live session ids on a repo (common dir), and the supervisor stops
/// their process groups.
///
/// Pauses are counted per repo (#146 review): two core calls on one repo can overlap (two tickets' snapshots, or a
/// snapshot and a `merge_deps`), and the sessions must stay stopped until the last of them ends. The first pause on a
/// repo stops its sessions; while any pause is held, sessions started on the repo are stopped too; only the last drop
/// resumes them all.
pub struct SessionQuiesce {
    sup: Arc<crate::sessions::Supervisor>,
    on_repo: SessionsOnRepo,
    /// Per common dir: how many pauses are live, and every session id stopped for them.
    held: Arc<Mutex<HashMap<PathBuf, PauseCount>>>,
}

impl SessionQuiesce {
    pub fn new(sup: Arc<crate::sessions::Supervisor>, on_repo: impl Fn(&Path) -> Vec<String> + Send + Sync + 'static) -> SessionQuiesce {
        SessionQuiesce { sup, on_repo: Arc::new(on_repo), held: Arc::default() }
    }
}

impl std::fmt::Debug for SessionQuiesce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionQuiesce")
    }
}

/// Stop every live session `on_repo` names for `key` that isn't stopped yet, while the repo still has a held pause.
/// `on_repo` runs outside the `held` lock.
fn stop_new(sup: &crate::sessions::Supervisor, on_repo: &SessionsOnRepo, held: &Mutex<HashMap<PathBuf, PauseCount>>, key: &Path) -> bool {
    let ids = on_repo(key);
    let mut m = crate::lock(held);
    let Some((_, stopped)) = m.get_mut(key) else { return false };
    let new: Vec<String> = ids.into_iter().filter(|id| !stopped.contains(id)).collect();
    stopped.extend(sup.pause(&new));
    true
}

impl Quiesce for SessionQuiesce {
    fn pause(&self, common: &Path) -> Pause {
        let key = common.to_path_buf();
        let first = {
            let mut m = crate::lock(&self.held);
            let (n, _) = m.entry(key.clone()).or_default();
            *n += 1;
            *n == 1
        };
        // The guard owns the increment before anything below can panic (`on_repo`, a thread spawn): an unwind drops it,
        // so the count never leaks and no session stays stopped for the life of the app (#146 bhishma delta).
        let guard = {
            let (sup, held, key) = (self.sup.clone(), self.held.clone(), key.clone());
            Pause::new(move || {
                let mut m = crate::lock(&held);
                let last = m.get_mut(&key).map(|(n, _)| {
                    *n -= 1;
                    *n == 0
                });
                if last == Some(true) {
                    if let Some((_, stopped)) = m.remove(&key) {
                        sup.resume(&stopped);
                    }
                }
            })
        };
        stop_new(&self.sup, &self.on_repo, &self.held, &key);
        if first {
            // While the repo is paused, every few ms: stop any session started on it since (a single long pause still
            // catches it), and stop the stopped ones again, since a group the kernel continued (an exit that leaves it
            // newly orphaned while stopped) must not run on. Both run under the `held` lock the last drop's resume
            // takes, so nothing is stopped after the resume.
            let (sup, held, on_repo) = (self.sup.clone(), self.held.clone(), self.on_repo.clone());
            let spawned = std::thread::Builder::new().name("vajra-gitw-restop".into()).spawn(move || loop {
                std::thread::sleep(Duration::from_millis(RESTOP_MS));
                if !stop_new(&sup, &on_repo, &held, &key) {
                    break;
                }
                let m = crate::lock(&held);
                let Some((_, stopped)) = m.get(&key) else { break };
                sup.restop(stopped);
            });
            // No thread (the system refused one): the sessions stopped above stay stopped for this pause, just without
            // the re-stop. Never a panic with the count held.
            drop(spawned);
        }
        guard
    }
}

// ------------------------------------------------------------ trusted programs (L4)

/// A program the trusted config runs (a filter, merge driver or diff program), resolved the way git's shell finds it:
/// git's exec path, then `BASE_PATH`. `at` fingerprints the binary it resolved to; ctime moves on any write, chmod or
/// rename, and a user can't set it back (#146 gate L4).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Program {
    word: String,
    at: Option<Fingerprint>,
}

/// (canonical path, dev, ino, size, mtime, mtime ns, ctime, ctime ns).
type Fingerprint = (PathBuf, u64, u64, u64, i64, i64, i64, i64);

/// Config keys whose value is a command git may run inside the core's calls.
fn program_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    let Some((section, rest)) = k.split_once('.') else { return false };
    let name = rest.rsplit('.').next().unwrap_or("");
    rest.contains('.') && match section {
        "filter" => matches!(name, "clean" | "smudge" | "process"),
        "merge" => name == "driver",
        "diff" => matches!(name, "textconv" | "command"),
        _ => false,
    }
}

/// A trusted-config command the core runs without asking: `git-lfs` (bare, or by absolute path) running
/// `filter-process`, `clean` or `smudge`, with nothing a shell would treat specially. Every other filter, merge driver
/// or diff program may load code from the worktree (git runs it with the worktree as its cwd: `python3 -m x`, `node`,
/// `sh ./x`), so the human has to accept it first (#146 re-check N2).
fn vetted_program(cmd: &str) -> bool {
    let mut w = cmd.split_ascii_whitespace();
    let prog = w.next().unwrap_or("");
    let lfs = prog == "git-lfs" || (prog.starts_with('/') && prog.ends_with("/git-lfs"));
    lfs && matches!(w.next(), Some("filter-process" | "clean" | "smudge"))
        && !cmd.chars().any(|c| ";|&<>()`$\\\"'*?[]{}~\n".contains(c))
}

/// The program a shell command runs first: the first word, or a quoted first word.
fn first_word(cmd: &str) -> Option<String> {
    let c = cmd.trim_start();
    let w = match c.chars().next()? {
        q @ ('"' | '\'') => c[1..].split(q).next()?.to_string(),
        _ => c.split(|ch: char| ch.is_whitespace() || ";|&<>()`$".contains(ch)).next()?.to_string(),
    };
    (!w.is_empty()).then_some(w)
}

fn fingerprint(p: &Path) -> Option<Fingerprint> {
    let c = p.canonicalize().ok()?;
    let m = std::fs::metadata(&c).ok()?;
    m.is_file().then(|| (c, m.dev(), m.ino(), m.size(), m.mtime(), m.mtime_nsec(), m.ctime(), m.ctime_nsec()))
}

// ------------------------------------------------------------ the repo baseline

#[derive(Clone, Debug)]
struct Held {
    config: FileHash,
    attributes: FileHash,
    /// `<common>/config.worktree` and every include target of the two config files; why not, when the include graph
    /// can't be followed whole (`config_sources`), which refuses every content call.
    others: Result<Vec<(PathBuf, FileHash)>, String>,
    /// The programs the effective trusted config (global baseline plus repo) can run.
    programs: Vec<Program>,
    /// The config keys naming a program outside `vetted_program`: they may run code from the worktree, so the repo
    /// gets no content call until the human accepts them (`accept_programs`).
    unvetted: Vec<String>,
    programs_accepted: bool,
    /// The baseline config's entries, kept only to name the keys that changed.
    entries: Vec<(String, String)>,
    runs: usize,
}

/// The repo baselines (16.6): taken at a run's Start before its lead launches, shared by concurrent runs on the repo,
/// released when the last of them ends.
#[derive(Debug, Default)]
pub struct RepoBaselines {
    held: Mutex<HashMap<PathBuf, Held>>,
}

impl RepoBaselines {
    /// Hold a baseline for this common dir. When none is held, one is taken only if no Vajra agent session runs on the
    /// repo; a held one is reused and never retaken.
    pub fn acquire(&self, gw: &Gitw, common: &Path, agents_on_repo: bool) -> R<()> {
        let mut m = crate::lock(&self.held);
        if let Some(h) = m.get_mut(common) {
            h.runs += 1;
            return Ok(());
        }
        if agents_on_repo {
            return Err(GitwError::Invalid("a Vajra agent session is running on this repo"));
        }
        m.insert(common.to_path_buf(), Held { runs: 1, ..take_repo(gw, common) });
        Ok(())
    }

    pub fn release(&self, common: &Path) {
        let mut m = crate::lock(&self.held);
        if let Some(h) = m.get_mut(common) {
            h.runs -= 1;
            if h.runs == 0 {
                m.remove(common);
            }
        }
    }

    /// The human accepts the repo's new config: only with no Vajra agent session on the repo.
    pub fn accept_new(&self, gw: &Gitw, common: &Path, agents_on_repo: bool) -> R<()> {
        if agents_on_repo {
            return Err(GitwError::Invalid("a Vajra agent session is running on this repo"));
        }
        let mut m = crate::lock(&self.held);
        let runs = m.get(common).map(|h| h.runs).ok_or(GitwError::Invalid("no baseline is held for this repo"))?;
        m.insert(common.to_path_buf(), Held { runs, ..take_repo(gw, common) });
        Ok(())
    }

    /// The human accepts the programs the repo's trusted config runs that Vajra can't vet ("This program may run code
    /// from the worktree"): only with no Vajra agent session on the repo. A later change to them is checked as usual.
    pub fn accept_programs(&self, common: &Path, agents_on_repo: bool) -> R<()> {
        if agents_on_repo {
            return Err(GitwError::Invalid("a Vajra agent session is running on this repo"));
        }
        let mut m = crate::lock(&self.held);
        m.get_mut(common).ok_or(GitwError::Invalid("no baseline is held for this repo"))?.programs_accepted = true;
        Ok(())
    }

    /// After the human replaced the global baseline (only with no agent session running): every held repo's program
    /// list is taken again from the new effective config, so a program the new baseline names is fingerprinted and,
    /// when unvetted, needs the human's accept. An earlier accept carries over only when the unvetted set is unchanged
    /// (#146 R1). The git calls run outside the lock.
    fn refresh_programs(&self, gw: &Gitw) {
        let commons: Vec<PathBuf> = crate::lock(&self.held).keys().cloned().collect();
        for c in commons {
            let (programs, unvetted) = gw.programs(&c);
            if let Some(h) = crate::lock(&self.held).get_mut(&c) {
                h.programs_accepted &= h.unvetted == unvetted;
                h.programs = programs;
                h.unvetted = unvetted;
            }
        }
    }

    /// The program keys the human has to accept before the repo gets content calls (for the card). Names only.
    pub fn unvetted_programs(&self, common: &Path) -> Vec<String> {
        crate::lock(&self.held).get(common).filter(|h| !h.programs_accepted).map(|h| h.unvetted.clone()).unwrap_or_default()
    }

    /// Before a content call: the unvetted-program gate, then the hashes.
    fn check(&self, gw: &Gitw, common: &Path) -> R<()> {
        self.check_as(gw, common, true)
    }

    /// Before a call that runs no filter or driver (`discard`'s `worktree remove --force`, `branch -D`): the hashes
    /// only. An unvetted program can't run there, so it doesn't block them (#146 claude).
    fn check_meta(&self, gw: &Gitw, common: &Path) -> R<()> {
        self.check_as(gw, common, false)
    }

    fn check_as(&self, gw: &Gitw, common: &Path, content: bool) -> R<()> {
        // A copy, so the git calls below never run under the lock (#146 claude): a card read or another repo's check
        // isn't stuck behind them.
        let h = crate::lock(&self.held).get(common).cloned().ok_or(GitwError::Invalid("no repo baseline is held; start the run first"))?;
        #[cfg(test)]
        IN_CHECK.with(|c| if let Some(f) = c.borrow().as_ref() { f() });
        if content && !h.unvetted.is_empty() && !h.programs_accepted {
            return Err(GitwError::RepoConfigChanged { keys: h.unvetted.iter().map(|k| format!("program {k}")).collect() });
        }
        let partial = |why: &str| GitwError::RepoConfigChanged { keys: vec![format!("include.path ({why})")] };
        let held_others = h.others.as_ref().map_err(|w| partial(w))?;
        let config = hash_file(&common.join("config"), false);
        let attributes = hash_file(&common.join("info/attributes"), false);
        let others = repo_others(gw, common).map_err(|w| partial(&w))?;
        let programs: Vec<Program> = h.programs.iter().map(|p| gw.program(&p.word)).collect();
        if config == h.config && attributes == h.attributes && others == *held_others && programs == h.programs {
            return Ok(());
        }
        let mut keys = if config == h.config { Vec::new() } else { diff_keys(&h.entries, &read_config_file(&gw.git, &common.join("config"), &gw.home).unwrap_or_default()) };
        if attributes != h.attributes {
            keys.push("info/attributes".into());
        }
        if others != *held_others {
            let wt = common.join("config.worktree");
            let changed = |p: &PathBuf| others.iter().find(|(q, _)| q == p) != held_others.iter().find(|(q, _)| q == p);
            let paths: BTreeSet<&PathBuf> = others.iter().chain(held_others).map(|(p, _)| p).filter(|p| changed(p)).collect();
            if paths.contains(&wt) {
                keys.push("config.worktree".into());
            }
            if paths.iter().any(|p| **p != wt) {
                keys.push("include.path".into());
            }
        }
        for (now, then) in programs.iter().zip(&h.programs) {
            if now != then {
                keys.push(format!("program {}", now.word));
            }
        }
        if keys.is_empty() {
            keys.push("config".into());
        }
        Err(GitwError::RepoConfigChanged { keys })
    }
}

/// `config.worktree` and the include targets: everything the repo's config sources but the two files hashed by name.
/// Why not, when the include graph can't be followed whole.
fn repo_others(gw: &Gitw, common: &Path) -> Result<Vec<(PathBuf, FileHash)>, String> {
    let config = common.join("config");
    let (mut v, partial) = repo_sources(gw, common);
    v.retain(|(p, _)| *p != config);
    partial.map_or(Ok(v), Err)
}

/// `<common>/config`, `config.worktree` and every file they include, under any condition.
fn repo_sources(gw: &Gitw, common: &Path) -> (Vec<(PathBuf, FileHash)>, Option<String>) {
    config_sources(&gw.git, &gw.home, None, &[common.join("config"), common.join("config.worktree")], false)
}

fn take_repo(gw: &Gitw, common: &Path) -> Held {
    let (progs, unvetted) = gw.programs(common);
    Held {
        config: hash_file(&common.join("config"), false),
        attributes: hash_file(&common.join("info/attributes"), false),
        others: repo_others(gw, common),
        programs: progs,
        unvetted,
        programs_accepted: false,
        entries: read_config_file(&gw.git, &common.join("config"), &gw.home).unwrap_or_default(),
        runs: 0,
    }
}

// ------------------------------------------------------------ tickets

/// A common directory's identity, recorded at `worktree_add` and checked with plain filesystem reads.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CommonId {
    path: PathBuf,
    dev: u64,
    ino: u64,
    config_dev: u64,
    config_ino: u64,
}

fn common_id(p: &Path) -> Option<CommonId> {
    let path = p.canonicalize().ok()?;
    let d = std::fs::symlink_metadata(&path).ok()?;
    let c = std::fs::symlink_metadata(path.join("config")).ok()?;
    (d.is_dir() && c.is_file()).then(|| CommonId { dev: d.dev(), ino: d.ino(), config_dev: c.dev(), config_ino: c.ino(), path })
}

/// One core worktree: a worker's ticket, or the run's integration tree.
#[derive(Clone, Debug)]
pub struct Ticket {
    pub project: String,
    pub run: String,
    /// The task id, or `integration`.
    pub task: String,
    pub round: u32,
    pub repo: PathBuf,
    pub wt: PathBuf,
    pub gitdir: PathBuf,
    common: CommonId,
    /// `<data>/tickets/<project>/<run>/<task>-r<n>`: the private index and lists. Agents can't write there.
    pub private: PathBuf,
    /// `<work>/<project>/<run>/tmp/<task>-r<n>`: the seat's only other writable root, and its `TMPDIR`.
    pub tmp: PathBuf,
    dotgit: FileHash,
    commondir: FileHash,
    config_worktree: FileHash,
    /// The HEAD the worker started from (after `merge_deps`), detached.
    pub dispatch_head: String,
    /// An integration tree's HEAD is the integration branch, not detached.
    integration: bool,
    /// The top-level dependency folders `clone_deps` cloned in (N7): untracked at `dispatch_head` and git-ignored.
    pub deps: Vec<String>,
}

impl Ticket {
    pub fn common_dir(&self) -> &Path {
        &self.common.path
    }
    pub fn task_ref(&self) -> String {
        format!("refs/heads/vajra/{}/task-{}-r{}", self.run, self.task, self.round)
    }
}

/// A task commit's change set against `dispatch_head`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffStat {
    pub files: Vec<String>,
    pub added: u64,
    pub removed: u64,
}

/// The dependency folders N7 clones, where they exist and are git-ignored.
pub const DEP_DIRS: [&str; 4] = ["node_modules", ".venv", "target", "vendor"];

/// One folder `clone_deps` considered; `error` is the card's reason when the clone failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepClone {
    pub name: String,
    pub error: Option<String>,
}

/// `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)`: a copy-on-write clone of a whole tree; `dst` must not exist.
/// A partial clone left by a failure is removed.
fn clonefile(src: &Path, dst: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    #[cfg(test)]
    if FAIL_CLONE.with(|c| c.get()) {
        return Err("the copy failed (test)".into());
    }
    let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|_| "a path holds a NUL".to_string());
    let (s, d) = (c(src)?, c(dst)?);
    // SAFETY: both are valid NUL-terminated paths that outlive the call.
    let r = unsafe { libc::clonefile(s.as_ptr(), d.as_ptr(), 0x0001 | 0x0002) };
    if r == 0 {
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    if std::fs::symlink_metadata(dst).is_ok_and(|m| m.is_dir()) {
        let _ = std::fs::remove_dir_all(dst);
    }
    Err(if e.raw_os_error() == Some(libc::EXDEV) { "the folder is on another volume, so it can't be cloned".into() } else { format!("the copy failed: {e}") })
}

/// A finished snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// `None` when nothing in the task's globs changed.
    pub commit: Option<String>,
    pub task_ref: Option<String>,
    pub files: Vec<String>,
    pub leftovers: Vec<String>,
}

/// `[A-Za-z0-9][A-Za-z0-9._-]{0,63}`, no `..`: safe as a path component and inside a ref name.
fn valid_name(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !s.contains("..")
        && !s.ends_with(".lock")
        && !s.ends_with('.')
}

fn valid_oid(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A ref the core may write: under `refs/heads/vajra/`, each component a `valid_name`.
pub fn vajra_ref(r: &str) -> R<()> {
    let rest = r.strip_prefix("refs/heads/vajra/").ok_or(GitwError::Invalid("ref is outside refs/heads/vajra/"))?;
    if rest.split('/').all(valid_name) { Ok(()) } else { Err(GitwError::Invalid("ref name")) }
}

/// The git writer.
#[derive(Debug)]
pub struct Gitw {
    pub git: PathBuf,
    pub home: PathBuf,
    pub data: PathBuf,
    pub work: PathBuf,
    /// The global baseline copy every call reads as its global config.
    pub global: PathBuf,
    /// Its sha256, checked before every call (16.6, T4).
    global_sha: Mutex<String>,
    pub repos: RepoBaselines,
    /// git 2.40 or later: `GIT_ATTR_SOURCE` is honoured.
    attr_source: bool,
    /// Where git's shell finds a filter program: git's exec path, then `BASE_PATH`.
    prog_dirs: Vec<PathBuf>,
    quiesce: Arc<dyn Quiesce>,
    #[cfg(test)]
    pub checks: std::sync::atomic::AtomicBool,
}

impl Gitw {
    /// `check_roots` must have passed. `quiesce` stops the other live sessions on a repo during content calls.
    pub fn new(git: PathBuf, home: PathBuf, data: PathBuf, work: PathBuf, global: &GlobalBaseline, quiesce: Arc<dyn Quiesce>) -> Self {
        let mut g = Gitw {
            git, home, data, work, global: global.copy.clone(), global_sha: Mutex::new(global.sha.clone()), repos: RepoBaselines::default(), attr_source: false,
            prog_dirs: Vec::new(), quiesce,
            #[cfg(test)]
            checks: std::sync::atomic::AtomicBool::new(true),
        };
        g.attr_source = g.version().is_some_and(|v| v >= (2, 40));
        let exec = run_process_env(&g.git, &["--exec-path".into()], None, CALL_TIMEOUT, Some(&g.home), None, &[]).ok().map(|o| PathBuf::from(o.stdout.trim()));
        g.prog_dirs = exec.into_iter().filter(|p| p.is_absolute()).chain(BASE_PATH.iter().map(PathBuf::from)).collect();
        g
    }

    /// Resolve one program word the way git's shell would, and fingerprint what it finds. `~/` expands against HOME
    /// (#146 re-check N5); a relative word with a `/` resolves to nothing, and is never vetted.
    fn program(&self, word: &str) -> Program {
        let found = if let Some(rest) = word.strip_prefix("~/") {
            Some(self.home.join(rest))
        } else if word.contains('/') {
            Path::new(word).is_absolute().then(|| PathBuf::from(word))
        } else {
            self.prog_dirs.iter().map(|d| d.join(word)).find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.mode() & 0o111 != 0))
        };
        Program { word: word.to_string(), at: found.as_deref().and_then(fingerprint) }
    }

    /// The programs the effective config of this repo names (global baseline plus repo config), and the keys whose
    /// command `vetted_program` doesn't pass. Config only: no filter runs to read it. A config that can't be read is
    /// one unvetted entry, so it fails closed.
    fn programs(&self, common: &Path) -> (Vec<Program>, Vec<String>) {
        let o = match self.run(Call::new(At::Repo(common), &["config", "--list", "-z"])) {
            Ok(o) if o.exit() == Some(0) && o.whole() => o,
            _ => return (Vec::new(), vec!["config".into()]),
        };
        let mut entries: Vec<(String, String)> = parse_list_z(&o.stdout).into_iter().filter(|(k, _)| program_key(k)).collect();
        // That list evaluates `includeIf` against the common dir. A ticket's calls run with the worktree's own gitdir and
        // branch, where other conditions hold (`gitdir:.../worktrees/`, `onbranch:`). So every program any repo config
        // file defines, whatever its include condition, is listed and fingerprinted too (#146 codex).
        for (f, h) in repo_sources(self, common).0 {
            if !matches!(h, FileHash::Sha(_)) {
                continue;
            }
            match read_config_file(&self.git, &f, &self.home) {
                Some(e) => entries.extend(e.into_iter().filter(|(k, _)| program_key(k))),
                None => return (Vec::new(), vec!["config".into()]),
            }
        }
        let words: BTreeSet<String> = entries.iter().filter_map(|(_, v)| first_word(v)).collect();
        let unvetted: BTreeSet<String> = entries.iter().filter(|(_, v)| !vetted_program(v)).map(|(k, _)| k.clone()).collect();
        (words.iter().map(|w| self.program(w)).collect(), unvetted.into_iter().collect())
    }

    fn version(&self) -> Option<(u32, u32)> {
        let o = run_process_env(&self.git, &["--version".into()], None, CALL_TIMEOUT, Some(&self.home), None, &[]).ok()?;
        let v = o.stdout.split_whitespace().nth(2)?;
        let mut it = v.split('.').map(|x| x.parse::<u32>().ok());
        Some((it.next()??, it.next()??))
    }

    /// After the human replaced or re-took the global baseline.
    pub fn use_global(&self, g: &GlobalBaseline) -> R<()> {
        if g.copy != self.global {
            return Err(GitwError::Invalid("a different baseline copy"));
        }
        *crate::lock(&self.global_sha) = g.sha.clone();
        self.repos.refresh_programs(self);
        Ok(())
    }

    fn checks_on(&self) -> bool {
        #[cfg(test)]
        return self.checks.load(std::sync::atomic::Ordering::SeqCst);
        #[cfg(not(test))]
        true
    }

    fn run(&self, c: Call<'_>) -> R<Output> {
        // The copy git is about to read as its global config is the one taken at app start, byte for byte.
        if hash_file(&self.global, false) != FileHash::Sha(crate::lock(&self.global_sha).clone()) {
            return Err(tampered(BASELINE_TAMPERED));
        }
        let mut argv: Vec<String> = vec!["--no-pager".into(), "--no-optional-locks".into()];
        for kv in FIXED.iter().chain(c.config.iter()) {
            argv.extend(["-c".into(), kv.to_string()]);
        }
        let mut env: Vec<(&str, PathBuf)> = vec![
            ("GIT_CONFIG_GLOBAL", self.global.clone()),
            ("GIT_ATTR_NOSYSTEM", "1".into()),
            ("GIT_LFS_SKIP_SMUDGE", "1".into()),
            ("GIT_LITERAL_PATHSPECS", "1".into()),
            ("GIT_NO_REPLACE_OBJECTS", "1".into()),
            // An accepted Python filter does not put its cwd (the worktree) first on `sys.path` (3.11 and later).
            ("PYTHONSAFEPATH", "1".into()),
            ("PYTHONNOUSERSITE", "1".into()),
            ("PYTHONDONTWRITEBYTECODE", "1".into()),
        ];
        let cwd = match c.at {
            At::Repo(r) => {
                argv.extend(["-C".into(), r.to_string_lossy().into_owned()]);
                None
            }
            At::Pinned(t) => {
                env.push(("GIT_DIR", t.gitdir.clone()));
                env.push(("GIT_COMMON_DIR", t.common.path.clone()));
                env.push(("GIT_WORK_TREE", t.wt.clone()));
                if self.attr_source {
                    env.push(("GIT_ATTR_SOURCE", t.dispatch_head.clone().into()));
                }
                Some(t.wt.clone())
            }
        };
        if let Some(i) = c.index {
            env.push(("GIT_INDEX_FILE", i.to_path_buf()));
        }
        argv.extend(c.args);
        let env: Vec<(&str, &Path)> = env.iter().map(|(k, v)| (*k, v.as_path())).collect();
        run_process_env(&self.git, &argv, cwd.as_deref(), c.timeout, Some(&self.home), None, &env).map_err(|e| GitwError::Io(e.to_string()))
    }

    /// Run and require exit 0.
    fn ok(&self, step: &'static str, c: Call<'_>) -> R<Output> {
        let o = self.run(c)?;
        if o.exit() == Some(0) { Ok(o) } else { Err(GitwError::Git { step, first_line: first_line(&o) }) }
    }

    /// The repo's canonical common dir.
    /// Tests: the repo pause every content call takes first, held until the `Pause` drops.
    #[cfg(test)]
    pub(crate) fn hold_repo(&self, common: &Path) -> Pause {
        self.quiesce.pause(common)
    }

    pub fn common_of(&self, repo: &Path) -> R<PathBuf> {
        let o = self.ok("rev-parse", Call::new(At::Repo(repo), &["rev-parse", "--path-format=absolute", "--git-common-dir"]))?;
        PathBuf::from(o.stdout.trim()).canonicalize().map_err(io)
    }

    fn top_of(&self, repo: &Path) -> R<PathBuf> {
        let o = self.ok("rev-parse", Call::new(At::Repo(repo), &["rev-parse", "--show-toplevel"]))?;
        PathBuf::from(o.stdout.trim()).canonicalize().map_err(io)
    }

    /// The identity flags when the repo has no `user.email`.
    fn identity(&self, t: &Ticket) -> R<Vec<&'static str>> {
        let o = self.run(Call::new(At::Pinned(t), &["config", "--get", "user.email"]))?;
        Ok(if o.exit() == Some(0) && !o.stdout.trim().is_empty() { vec![] } else { vec![FALLBACK_NAME, FALLBACK_EMAIL] })
    }

    // -------------------------------------------------------- tamper checks

    /// Every check that needs no change list, before every call on the ticket. Plain filesystem reads first: git
    /// would already be reading a redirected common dir.
    pub fn preflight(&self, t: &Ticket) -> R<()> {
        self.repos.check(self, &t.common.path)?;
        if !self.checks_on() {
            return Ok(());
        }
        if hash_file(&t.wt.join(".git"), false) != t.dotgit {
            return Err(tampered(".git"));
        }
        if hash_file(&t.gitdir.join("commondir"), false) != t.commondir {
            return Err(tampered("commondir"));
        }
        if common_id(&t.common.path).as_ref() != Some(&t.common) {
            return Err(tampered("the common git directory"));
        }
        if hash_file(&t.gitdir.join("config.worktree"), false) != t.config_worktree {
            return Err(tampered("config.worktree"));
        }
        if t.integration {
            return Ok(());
        }
        match read_bounded(&t.gitdir.join("HEAD"), false) {
            Ok(Some(b)) if b == format!("{}\n", t.dispatch_head).as_bytes() => {}
            _ => return Err(tampered("HEAD")),
        }
        // Detached: `symbolic-ref -q HEAD` exits 1.
        if self.run(Call::new(At::Pinned(t), &["symbolic-ref", "-q", "HEAD"]))?.exit() != Some(1) {
            return Err(tampered("HEAD is not detached"));
        }
        Ok(())
    }

    // -------------------------------------------------------- operations

    #[allow(clippy::too_many_arguments)]
    fn add_tree(&self, repo: &Path, project: &str, run: &str, task: &str, round: u32, from: &str, integration: bool) -> R<Ticket> {
        if ![project, run, task].iter().all(|n| valid_name(n)) || !(1..=99).contains(&round) {
            return Err(GitwError::Invalid("project, run, task or round"));
        }
        if !valid_oid(from) {
            return Err(GitwError::Invalid("from must be a full commit id"));
        }
        let repo = self.top_of(repo)?;
        let common = self.common_of(&repo)?;
        // `worktree add` checks files out: smudge filters can run.
        let _quiet = self.quiesce.pause(&common);
        self.repos.check(self, &common)?;
        let leaf = if integration { if round == 1 { "integration".to_string() } else { format!("integration-r{round}") } } else { format!("{task}-r{round}") };
        let (work, data) = (self.work.canonicalize().map_err(io)?, self.data.canonicalize().map_err(io)?);
        let tmp = work.join(project).join(run).join("tmp").join(&leaf);
        let private = data.join("tickets").join(project).join(run).join(&leaf);
        let run_dir = make_dirs(&work, &[project, run])?;
        let wt = run_dir.join(&leaf);
        let there = |p: &Path| std::fs::symlink_metadata(p).is_ok();
        if there(&wt) || there(&private) {
            return Err(GitwError::Exists);
        }
        let wt_s = wt.to_str().ok_or(GitwError::Invalid("worktree path"))?.to_string();
        self.ok("worktree add", Call::new(At::Repo(&repo), &["worktree", "add", "--detach", "--", &wt_s, from]))?;
        // From here every error rolls back: the worktree is unregistered and removed, and the dirs this call made are
        // gone, so a retry of the same ticket starts clean (never a permanent `Exists`).
        let made = (|| {
            #[cfg(test)]
            if FAIL_AFTER_ADD.with(|f| f.get()) {
                return Err(tampered("test: fail after worktree add"));
            }
            let tmp = make_dirs(&work, &[project, run, "tmp", &leaf])?;
            let private = make_dirs(&data, &["tickets", project, run, &leaf])?;
            self.verify_tree(repo.clone(), &common, &wt, &tmp, &private, (project, run, task, round), from, integration)
        })();
        if made.is_err() {
            let _ = self.run(Call::new(At::Repo(&repo), &["worktree", "remove", "--force", "--", &wt_s]));
            let _ = self.run(Call::new(At::Repo(&repo), &["worktree", "prune"]));
            remove_own_dir(&tmp, &[&work]);
            remove_own_dir(&private, &[&data]);
        }
        made
    }

    /// Read back a just-added worktree's `.git` file and `commondir`, and record what the tamper checks compare.
    #[allow(clippy::too_many_arguments)]
    fn verify_tree(&self, repo: PathBuf, common: &Path, wt: &Path, tmp: &Path, private: &Path, (project, run, task, round): (&str, &str, &str, u32), from: &str, integration: bool) -> R<Ticket> {
        let wt = wt.canonicalize().map_err(io)?;
        let dotgit = read_bounded(&wt.join(".git"), false).ok().flatten().ok_or_else(|| tampered(".git"))?;
        let gitdir = std::str::from_utf8(&dotgit).ok().and_then(|s| s.strip_prefix("gitdir: ")).map(|s| PathBuf::from(s.trim_end())).ok_or_else(|| tampered(".git"))?;
        let gitdir = gitdir.canonicalize().map_err(io)?;
        if !gitdir.starts_with(common.join("worktrees")) {
            return Err(tampered("the worktree's git dir is outside the common dir"));
        }
        let cd = read_bounded(&gitdir.join("commondir"), false).ok().flatten().ok_or_else(|| tampered("commondir"))?;
        let resolved = gitdir.join(String::from_utf8_lossy(&cd).trim_end()).canonicalize().map_err(io)?;
        if resolved != common {
            return Err(tampered("commondir"));
        }
        // Every core-built dir is under its own root and outside the repo (#146 gate M4).
        let (tmp, private) = (tmp.canonicalize().map_err(io)?, private.canonicalize().map_err(io)?);
        let (work, data) = (self.work.canonicalize().map_err(io)?, self.data.canonicalize().map_err(io)?);
        let outside_repo = |p: &Path| !p.starts_with(&repo) && !p.starts_with(common);
        if !(wt.starts_with(&work) && tmp.starts_with(&work) && private.starts_with(&data) && [&wt, &tmp, &private].iter().all(|p| outside_repo(p))) {
            return Err(tampered("a ticket directory is outside its root"));
        }
        let common = common.to_path_buf();
        let t = Ticket {
            project: project.into(),
            run: run.into(),
            task: task.into(),
            round,
            repo,
            wt: wt.clone(),
            common: common_id(&common).ok_or_else(|| tampered("the common git directory"))?,
            private,
            tmp,
            dotgit: hash_file(&wt.join(".git"), false),
            commondir: hash_file(&gitdir.join("commondir"), false),
            config_worktree: hash_file(&gitdir.join("config.worktree"), false),
            gitdir,
            dispatch_head: from.into(),
            integration,
            deps: Vec::new(),
        };
        Ok(t)
    }

    /// `git worktree add --detach <work>/<project>/<run>/<task>-r<n> <from>`, after the repo baseline check.
    pub fn worktree_add(&self, repo: &Path, project: &str, run: &str, task: &str, round: u32, from: &str) -> R<Ticket> {
        let t = self.add_tree(repo, project, run, task, round, from, false)?;
        self.preflight(&t)?;
        Ok(t)
    }

    /// Merge the dependency commits into a fresh ticket before its worker starts; its HEAD after this is `dispatch_head`.
    pub fn merge_deps(&self, t: &mut Ticket, commits: &[String]) -> R<()> {
        if !commits.iter().all(|c| valid_oid(c)) {
            return Err(GitwError::Invalid("commit ids"));
        }
        let _quiet = self.quiesce.pause(&t.common.path);
        self.preflight(t)?;
        if commits.is_empty() {
            return Ok(());
        }
        let mut cfg = vec![NO_SIGN];
        cfg.extend(self.identity(t)?);
        let mut args = vec!["merge", "--no-ff", "--no-edit", "--no-verify", "--"];
        args.extend(commits.iter().map(String::as_str));
        let o = self.run(Call::new(At::Pinned(t), &args).config(&cfg))?;
        if o.exit() != Some(0) {
            let _ = self.run(Call::new(At::Pinned(t), &["merge", "--abort"]));
            return Err(GitwError::DepsConflict { sha: commits.join(",") });
        }
        let head = self.ok("rev-parse", Call::new(At::Pinned(t), &["rev-parse", "--verify", "HEAD"]))?.stdout.trim().to_string();
        if !valid_oid(&head) {
            return Err(GitwError::Git { step: "rev-parse", first_line: "no HEAD".into() });
        }
        t.dispatch_head = head;
        self.preflight(t)
    }

    /// N7: APFS clones (`clonefile`, copy-on-write) of the repo's dependency folders into a fresh ticket, before its
    /// `dispatch_head` is final. A folder is cloned only when it is a real directory at the repo's top level, has no
    /// path tracked at `dispatch_head` and is git-ignored there. One whose clone fails (another volume, S7) is reported
    /// so the card can say so, and the worker starts without it. Returns one entry per folder considered.
    pub fn clone_deps(&self, t: &mut Ticket) -> Vec<DepClone> {
        let mut out = Vec::new();
        if t.integration || self.preflight(t).is_err() {
            return out;
        }
        for name in DEP_DIRS {
            let src = t.repo.join(name);
            if !std::fs::symlink_metadata(&src).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            let tracked = self.run(Call::new(At::Pinned(t), &["ls-tree", "-z", "--name-only", &t.dispatch_head, "--", name]));
            if !matches!(&tracked, Ok(o) if o.exit() == Some(0) && o.stdout.is_empty()) {
                continue;
            }
            let dst = t.wt.join(name);
            let error = clonefile(&src, &dst).err();
            if error.is_none() {
                // Ignored at `dispatch_head`'s rules: git lists the whole folder as one ignored directory. (Read after the
                // clone: git answers only for a path that exists. `check-ignore` refuses literal pathspecs.) No filter runs.
                let ls = self.run(Call::new(At::Pinned(t), &["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory", "--", name]));
                if !matches!(&ls, Ok(o) if o.exit() == Some(0) && o.stdout == format!("{name}/\0")) {
                    let _ = std::fs::remove_dir_all(&dst);
                    continue;
                }
                t.deps.push(name.to_string());
            }
            out.push(DepClone { name: name.to_string(), error });
        }
        out
    }

    /// The "unchanged tree" test (16.5): every tamper check passes (so HEAD is still `dispatch_head`, detached), and
    /// nothing differs from `dispatch_head` but ignored files, read through a fresh private index with the baseline
    /// config, after the walk rule. The bridge's `git status` would read the user's live global config, which a
    /// worker can edit. `Ok(false)` when something changed; a tamper is an error.
    pub fn unchanged(&self, t: &Ticket) -> R<bool> {
        let _quiet = self.quiesce.pause(&t.common.path);
        self.preflight(t)?;
        self.walk_rule(t)?;
        let idx = self.fresh_index(t, "status-index")?;
        Ok(self.changed(t, &idx)?.is_empty())
    }

    /// The task commit's line counts against `dispatch_head`: tree to tree, so no worktree file is read and no filter
    /// runs, and lines from merged deps are never the worker's (16.10's base). Binary files count 0.
    pub fn diff_stat(&self, t: &Ticket, commit: &str) -> R<DiffStat> {
        if !valid_oid(commit) {
            return Err(GitwError::Invalid("commit id"));
        }
        self.preflight(t)?;
        let o = self.ok("diff-tree", Call::new(At::Pinned(t), &["diff-tree", "-r", "--numstat", "-z", "--no-renames", &t.dispatch_head, commit]))?;
        if !o.whole() {
            return Err(GitwError::Git { step: "diff-tree", first_line: "output too large".into() });
        }
        let mut d = DiffStat::default();
        for rec in o.stdout.split('\0').filter(|s| !s.is_empty()) {
            let mut f = rec.splitn(3, '\t');
            let (a, r, p) = (f.next().unwrap_or(""), f.next().unwrap_or(""), f.next().unwrap_or(""));
            d.added += a.parse::<u64>().unwrap_or(0);
            d.removed += r.parse::<u64>().unwrap_or(0);
            d.files.push(p.to_string());
        }
        Ok(d)
    }

    /// The paths an integration changed from `base` to `head`, tree to tree (no worktree file is read, no filter runs).
    pub fn diff_paths(&self, it: &Ticket, base: &str, head: &str) -> R<Vec<String>> {
        if !valid_oid(base) || !valid_oid(head) {
            return Err(GitwError::Invalid("commit ids"));
        }
        self.preflight(it)?;
        let o = self.ok("diff-tree", Call::new(At::Pinned(it), &["diff-tree", "-r", "--name-only", "-z", "--no-renames", base, head]))?;
        if !o.whole() {
            return Err(GitwError::Git { step: "diff-tree", first_line: "output too large".into() });
        }
        Ok(o.stdout.split('\0').filter(|s| !s.is_empty()).map(String::from).collect())
    }

    /// Fresh private index at `name`, loaded from `dispatch_head`.
    fn fresh_index(&self, t: &Ticket, name: &str) -> R<PathBuf> {
        let idx = t.private.join(name);
        match std::fs::remove_file(&idx) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
        self.ok("read-tree", Call::new(At::Pinned(t), &["read-tree", &t.dispatch_head]).index(&idx))?;
        Ok(idx)
    }

    /// The paths that differ from `dispatch_head` in the worktree (tracked changes and deletions, plus untracked files
    /// that are not ignored), read through a fresh private index so nothing the worker staged counts.
    fn changed(&self, t: &Ticket, idx: &Path) -> R<Vec<String>> {
        // Exit 1 only says some entries need updating.
        let _ = self.run(Call::new(At::Pinned(t), &["update-index", "-q", "--refresh"]).index(idx))?;
        let tracked = self.ok("diff-files", Call::new(At::Pinned(t), &["diff-files", "--name-only", "-z", "--ignore-submodules=all"]).index(idx))?;
        let others = self.ok("ls-files", Call::new(At::Pinned(t), &["ls-files", "--others", "--exclude-standard", "-z"]).index(idx))?;
        if !tracked.whole() || !others.whole() {
            return Err(GitwError::Git { step: "diff", first_line: "output too large".into() });
        }
        let set: BTreeSet<String> = tracked.stdout.split('\0').chain(others.stdout.split('\0')).filter(|s| !s.is_empty()).map(String::from).collect();
        Ok(set.into_iter().collect())
    }

    /// Before any git call on the ticket that reads file content (`update-index --refresh`, `add`): a plain filesystem
    /// walk of the whole worktree, ignored files included, names case-folded. Every `.gitattributes` or `.gitmodules`
    /// must be the regular file `dispatch_head` tracks at that exact path with the same bytes, and there is no `.git`
    /// below the root. No filter can have run yet, on any git version (#146 gate M1).
    fn walk_rule(&self, t: &Ticket) -> R<()> {
        if !self.checks_on() {
            return Ok(());
        }
        // The clone skip relies on `GIT_ATTR_SOURCE` pinning attributes to `dispatch_head`. Git before 2.40 reads the
        // worktree's own, so there a clone is walked like any other folder (#149 bhishma M1).
        let skip: &[String] = if self.attr_source { &t.deps } else { &[] };
        let found = special_files(&t.wt, skip)?;
        if found.is_empty() {
            return Ok(());
        }
        let ls = self.ok("ls-tree", Call::new(At::Pinned(t), &["ls-tree", "-r", "-z", "--full-tree", &t.dispatch_head]))?;
        if !ls.whole() {
            return Err(GitwError::Git { step: "ls-tree", first_line: "output too large".into() });
        }
        let tracked: HashMap<&str, (&str, &str)> = ls
            .stdout
            .split('\0')
            .filter_map(|e| {
                let (meta, path) = e.split_once('\t')?;
                let mut m = meta.split(' ');
                let (mode, kind, oid) = (m.next()?, m.next()?, m.next()?);
                (kind == "blob" && git_special(path.rsplit('/').next().unwrap_or(path)).is_some()).then_some((path, (mode, oid)))
            })
            .collect();
        for (rel, kind) in found {
            if kind == ".git" {
                return Err(tampered(format!("nested git repository at {rel}")));
            }
            let Some((mode, oid)) = tracked.get(rel.as_str()) else { return Err(tampered(rel)) };
            let disk = read_bounded(&t.wt.join(&rel), false).ok().flatten();
            let blob = self.ok("cat-file", Call::new(At::Pinned(t), &["cat-file", "blob", oid]))?;
            if !matches!(*mode, "100644" | "100755") || !blob.whole() || disk.as_deref() != Some(blob.stdout.as_bytes()) {
                return Err(tampered(rel));
            }
        }
        Ok(())
    }

    /// No created or changed `.gitattributes` (any depth), `.gitmodules` or `.lfsconfig`, and no nested repository,
    /// before any `add`. Names compare as the volume compares them.
    fn name_rule(&self, wt: &Path, changed: &[String]) -> R<()> {
        if !self.checks_on() {
            return Ok(());
        }
        for p in changed {
            let c = canonical(Some(wt), p.trim_end_matches('/'));
            let base = c.rsplit('/').next().unwrap_or(&c);
            if matches!(git_special(base), Some(".gitattributes" | ".gitmodules" | ".lfsconfig")) {
                return Err(tampered(p.clone()));
            }
            // An embedded repository shows as `dir/`; a `.git` component anywhere is a nested repo.
            if p.ends_with('/') || c.split('/').any(|x| x == ".git") {
                return Err(tampered(format!("nested git repository at {p}")));
            }
        }
        Ok(())
    }

    /// 16.6 `snapshot`: plumbing only, through a fresh private index. `globs` are the task's files; `named` the agent
    /// config files the ask names; `worker` the trailer's "<account> <model>".
    pub fn snapshot(&self, t: &Ticket, globs: &[String], named: &[String], worker: &str) -> R<Snapshot> {
        if t.integration {
            return Err(GitwError::Invalid("the integration tree is never snapshotted"));
        }
        if worker.is_empty() || worker.len() > 200 || worker.chars().any(|c| c.is_control()) {
            return Err(GitwError::Invalid("worker label"));
        }
        let _quiet = self.quiesce.pause(&t.common.path);
        // A repo config change makes the finishing ticket tampered (16.6).
        self.preflight(t).map_err(|e| match e {
            GitwError::RepoConfigChanged { keys } => tampered(format!("repo config: {}", keys.join(", "))),
            e => e,
        })?;
        self.walk_rule(t)?;
        let idx = self.fresh_index(t, "index")?;
        let changed = self.changed(t, &idx)?;
        self.name_rule(&t.wt, &changed)?;
        let (files, leftovers): (Vec<String>, Vec<String>) =
            changed.into_iter().partition(|p| globs.iter().any(|g| glob_match(g, p)) && (!agent_config_in(Some(&t.wt), p) || named.contains(p)) && !escaping_link(&t.wt, p));
        if files.is_empty() {
            return Ok(Snapshot { commit: None, task_ref: None, files, leftovers });
        }
        let task_ref = t.task_ref();
        vajra_ref(&task_ref)?;
        // Refs live in the common dir: a planted `<task ref> -> refs/heads/release` would make the create-only write
        // create `release` (#146 codex). Refused when symbolic, and written only with `--no-deref`.
        self.plain_ref(t, &task_ref, "the task ref")?;
        let mut list = files.join("\0").into_bytes();
        #[cfg(test)]
        EXTRA_ADD.with(|e| if let Some(x) = e.borrow().as_ref() { list.extend(b"\0"); list.extend(x.as_bytes()) });
        list.push(0);
        let list_file = t.private.join("paths");
        write_private(&list_file, &list)?;
        let lf = format!("--pathspec-from-file={}", list_file.display());
        self.ok("add", Call::new(At::Pinned(t), &["add", "-A", &lf, "--pathspec-file-nul"]).index(&idx))?;
        let tree = self.ok("write-tree", Call::new(At::Pinned(t), &["write-tree"]).index(&idx))?.stdout.trim().to_string();
        let dt = self.ok("diff-tree", Call::new(At::Pinned(t), &["diff-tree", "-r", "--name-only", "-z", "--no-renames", &t.dispatch_head, &tree]))?;
        if let Some(extra) = dt.stdout.split('\0').filter(|s| !s.is_empty()).find(|p| !files.iter().any(|f| f == p)) {
            return Err(tampered(format!("the tree holds {extra}, outside the staged list")));
        }
        let msg = format!("vajra: {} task {} round {}\n\nVajra-Worker: {}\n", t.run, t.task, t.round, worker);
        let mut cfg = vec![NO_SIGN];
        cfg.extend(self.identity(t)?);
        let o = self.run(Call::new(At::Pinned(t), &["commit-tree", &tree, "-p", &t.dispatch_head, "-m", &msg]).config(&cfg))?;
        if o.exit() != Some(0) {
            return Err(GitwError::CommitFailed { first_line: first_line(&o) });
        }
        let commit = o.stdout.trim().to_string();
        // Checked again before the ref exists. Every other live session on the repo has been stopped since before the
        // first check; a setsid daemon, an exited worker's leftover or another repo's worker can escape that pause
        // (ADR 16.6, an accepted residual), so this check is the second line.
        self.preflight(t).map_err(|e| match e {
            GitwError::RepoConfigChanged { keys } => tampered(format!("repo config: {}", keys.join(", "))),
            e => e,
        })?;
        self.plain_ref(t, &task_ref, "the task ref")?;
        #[cfg(test)]
        BEFORE_TASK_REF.with(|b| if let Some(f) = b.borrow().as_ref() { f() });
        let zero = "0".repeat(commit.len());
        let u = self.run(Call::new(At::Pinned(t), &["update-ref", "--no-deref", "--", &task_ref, &commit, &zero]))?;
        if u.exit() != Some(0) {
            return Err(GitwError::Exists);
        }
        Ok(Snapshot { commit: Some(commit), task_ref: Some(task_ref), files, leftovers })
    }

    /// The run's own integration tree (no worker is given it), from `base`.
    pub fn integration_tree(&self, repo: &Path, project: &str, run: &str, round: u32, base: &str) -> R<Ticket> {
        let t = self.add_tree(repo, project, run, "integration", round, base, true)?;
        self.preflight(&t)?;
        Ok(t)
    }

    /// The integration branch name of a tree from `integration_tree`.
    pub fn integration_ref(t: &Ticket) -> String {
        if t.round == 1 { format!("refs/heads/vajra/{}/integration", t.run) } else { format!("refs/heads/vajra/{}/integration-r{}", t.run, t.round) }
    }

    /// Refuse a symbolic `vajra/<run>/integration[-r<n>]`, set it to `base` with `update-ref --no-deref`, `merge --no-ff`
    /// each commit on a detached HEAD (checked before each merge), then move the branch to the result from exactly
    /// `base`, again with `--no-deref`. Returns the integration head.
    pub fn integrate(&self, it: &Ticket, base: &str, commits: &[String]) -> R<String> {
        if !it.integration {
            return Err(GitwError::Invalid("not an integration tree"));
        }
        if !valid_oid(base) || !commits.iter().all(|c| valid_oid(c)) {
            return Err(GitwError::Invalid("commit ids"));
        }
        let r = Self::integration_ref(it);
        vajra_ref(&r)?;
        let _quiet = self.quiesce.pause(&it.common.path);
        self.preflight(it)?;
        // Refs live in the common dir, which any worker on the repo can write. A planted `<r> -> refs/heads/main` would
        // make any write through `r` move main (#146 codex), so `r` is refused when symbolic, written only with
        // `--no-deref`, and never checked out: the merges run on a detached HEAD.
        self.plain_ref(it, &r, "the integration branch")?;
        // From the value it has now (none: the zero id), checked again right before the write: a directory swapped in
        // between by a process the pause can't reach makes the write fail, never move a ref elsewhere (#146 bhishma
        // delta).
        let now = self.run(Call::new(At::Pinned(it), &["rev-parse", "-q", "--verify", "--end-of-options", &r]))?;
        let old = match now.exit() {
            Some(0) if valid_oid(now.stdout.trim()) => now.stdout.trim().to_string(),
            Some(1) => "0".repeat(base.len()),
            _ => return Err(tampered("the integration branch")),
        };
        self.plain_ref(it, &r, "the integration branch")?;
        #[cfg(test)]
        BEFORE_REF_WRITE.with(|b| if let Some(f) = b.borrow().as_ref() { f() });
        if self.run(Call::new(At::Pinned(it), &["update-ref", "--no-deref", "-m", "vajra integrate", "--", &r, base, &old]))?.exit() != Some(0) {
            return Err(tampered("the integration branch"));
        }
        self.ok("checkout", Call::new(At::Pinned(it), &["checkout", "-q", "--detach", base]))?;
        let mut prev = base.to_string();
        if self.detached(it).is_err() || self.head(it)? != prev {
            return Err(tampered("the integration tree's HEAD"));
        }
        let mut cfg = vec![NO_SIGN];
        cfg.extend(self.identity(it)?);
        for c in commits {
            #[cfg(test)]
            BEFORE_MERGE.with(|b| if let Some(f) = b.borrow().as_ref() { f() });
            self.preflight(it)?;
            if self.detached(it).is_err() || self.head(it)? != prev {
                return Err(tampered("the integration tree's HEAD"));
            }
            let o = self.run(Call::new(At::Pinned(it), &["merge", "--no-ff", "--no-edit", "--no-verify", "--", c]).config(&cfg))?;
            if o.exit() != Some(0) {
                let _ = self.run(Call::new(At::Pinned(it), &["merge", "--abort"]));
                return Err(GitwError::MergeConflict { sha: c.clone() });
            }
            let now = self.head(it)?;
            if now != prev {
                // A new merge commit: first parent the previous head, second parent this commit.
                let ps = self.ok("rev-parse", Call::new(At::Pinned(it), &["rev-parse", "HEAD^1", "HEAD^2"]))?.stdout;
                let ps: Vec<&str> = ps.split_whitespace().collect();
                if ps != [prev.as_str(), c.as_str()] {
                    return Err(tampered("the integration tree's HEAD"));
                }
            } else if self.run(Call::new(At::Pinned(it), &["merge-base", "--is-ancestor", c, &prev]))?.exit() != Some(0) {
                // "Already up to date" is only right when the commit is already in.
                return Err(tampered("the integration tree's HEAD"));
            }
            prev = now;
        }
        if self.detached(it).is_err() || self.head(it)? != prev {
            return Err(tampered("the integration tree's HEAD"));
        }
        // From exactly `base`: a branch a worker moved meanwhile is refused, never overwritten.
        self.plain_ref(it, &r, "the integration branch")?;
        let u = self.run(Call::new(At::Pinned(it), &["update-ref", "--no-deref", "-m", "vajra integrate", "--", &r, &prev, base]))?;
        if u.exit() != Some(0) {
            return Err(tampered("the integration branch"));
        }
        self.plain_ref(it, &r, "the integration branch")?;
        Ok(prev)
    }

    /// `r` is a plain ref or none: `symbolic-ref -q` exits 1. `what` names it in the tamper reason. Every directory on
    /// its path under the common dir is a real directory or absent: a planted directory symlink (`refs/heads/vajra/<run>
    /// -> ../feat`) would carry even a `--no-deref` write to another branch's file (#146 bhishma delta).
    fn plain_ref(&self, t: &Ticket, r: &str, what: &str) -> R<()> {
        self.plain_ref_in(&t.common.path, At::Pinned(t), r, what)
    }

    /// `plain_ref` for a repo with no ticket (`branch_delete`).
    fn plain_ref_in(&self, common: &Path, at: At<'_>, r: &str, what: &str) -> R<()> {
        let mut d = common.to_path_buf();
        let parts: Vec<&str> = r.split('/').collect();
        for part in &parts[..parts.len().saturating_sub(1)] {
            d.push(part);
            match std::fs::symlink_metadata(&d) {
                Ok(m) if m.is_dir() => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                _ => return Err(tampered(format!("{what}'s directory is not a plain directory"))),
            }
        }
        if self.run(Call::new(at, &["symbolic-ref", "-q", "--", r]))?.exit() == Some(1) { Ok(()) } else { Err(tampered(format!("{what} is a symbolic ref"))) }
    }

    /// HEAD is detached: `symbolic-ref -q HEAD` exits 1.
    fn detached(&self, t: &Ticket) -> R<()> {
        if self.run(Call::new(At::Pinned(t), &["symbolic-ref", "-q", "HEAD"]))?.exit() == Some(1) { Ok(()) } else { Err(tampered("the integration tree's HEAD")) }
    }

    fn head(&self, t: &Ticket) -> R<String> {
        let h = self.ok("rev-parse", Call::new(At::Pinned(t), &["rev-parse", "--verify", "HEAD"]))?.stdout.trim().to_string();
        if valid_oid(&h) { Ok(h) } else { Err(GitwError::Git { step: "rev-parse", first_line: "no HEAD".into() }) }
    }

    /// The capture check (16.6 Retention): the tamper checks pass, the whole worktree staged the way `snapshot` staged
    /// it equals the task commit's tree, and the commit is an ancestor of the integration branch.
    pub fn capture_check(&self, t: &Ticket, commit: &str, integration_ref: &str) -> R<()> {
        if !valid_oid(commit) {
            return Err(GitwError::Invalid("commit id"));
        }
        vajra_ref(integration_ref)?;
        let _quiet = self.quiesce.pause(&t.common.path);
        self.preflight(t)?;
        self.walk_rule(t)?;
        let idx = self.fresh_index(t, "capture-index")?;
        // The same name rule as `snapshot`, before this `add` too (git before 2.40 has no `GIT_ATTR_SOURCE`).
        self.name_rule(&t.wt, &self.changed(t, &idx)?)?;
        self.ok("add", Call::new(At::Pinned(t), &["add", "-A", "--", "."]).index(&idx))?;
        let cap = self.ok("write-tree", Call::new(At::Pinned(t), &["write-tree"]).index(&idx))?.stdout.trim().to_string();
        let want = self.ok("rev-parse", Call::new(At::Pinned(t), &["rev-parse", "--verify", &format!("{commit}^{{tree}}")]))?.stdout.trim().to_string();
        if cap != want {
            let d = self.ok("diff-tree", Call::new(At::Pinned(t), &["diff-tree", "-r", "--name-only", "-z", "--no-renames", &want, &cap]))?;
            return Err(GitwError::Kept { paths: d.stdout.split('\0').filter(|s| !s.is_empty()).map(String::from).collect() });
        }
        // Read through no planted symlink or symbolic ref (#146 bhishma delta).
        self.plain_ref(t, integration_ref, "the integration branch")?;
        let a = self.run(Call::new(At::Pinned(t), &["merge-base", "--is-ancestor", commit, integration_ref]))?;
        if a.exit() != Some(0) {
            return Err(GitwError::Kept { paths: vec!["(the task commit is not integrated)".into()] });
        }
        Ok(())
    }

    /// Remove a ticket's worktree once the capture check passes (`--force`: the plumbing snapshot leaves it dirty).
    pub fn worktree_remove(&self, t: &Ticket, commit: &str, integration_ref: &str) -> R<()> {
        // One pause from the check until the tree is gone: a session resumed in between could write a file the check
        // never saw, and `--force` would delete it (#146 codex). The check's own pause nests inside this one.
        let _quiet = self.quiesce.pause(&t.common.path);
        self.capture_check(t, commit, integration_ref)?;
        self.remove(t)
    }

    /// The human discarded the ticket (after a confirm): remove it without the capture check.
    pub fn discard(&self, t: &Ticket) -> R<()> {
        self.repos.check_meta(self, &t.common.path)?;
        self.remove(t)
    }

    fn remove(&self, t: &Ticket) -> R<()> {
        let wt = t.wt.to_str().ok_or(GitwError::Invalid("worktree path"))?;
        self.ok("worktree remove", Call::new(At::Repo(&t.repo), &["worktree", "remove", "--force", "--", wt]))?;
        // Only the core-built dirs, still where they were made.
        remove_own_dir(&t.tmp, &[&self.work]);
        remove_own_dir(&t.private, &[&self.data]);
        Ok(())
    }

    /// `git branch -D vajra/...`: only a task branch under `refs/heads/vajra/`.
    pub fn branch_delete(&self, repo: &Path, r: &str) -> R<()> {
        vajra_ref(r)?;
        let common = self.common_of(repo)?;
        // Paused, and never through a planted directory symlink or symbolic ref: `branch -D` would delete a ref of the
        // same name elsewhere, with its reflog (#146 bhishma delta).
        let _quiet = self.quiesce.pause(&common);
        self.repos.check_meta(self, &common)?;
        self.plain_ref_in(&common, At::Repo(repo), r, "the task branch")?;
        self.ok("branch -D", Call::new(At::Repo(repo), &["branch", "-D", "--", r.trim_start_matches("refs/heads/")]))?;
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: runs inside the repo baseline check, after the held copy is taken and before its git calls.
    pub(crate) static IN_CHECK: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
    /// Tests: every `clonefile` fails.
    pub(crate) static FAIL_CLONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    /// Tests: fail right after `worktree add` succeeded (the rollback path).
    pub(crate) static FAIL_AFTER_ADD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    /// Tests: one more path for `add` that is not in the staged list (forces the tree check).
    pub(crate) static EXTRA_ADD: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    /// Tests: runs before each `integrate` merge (a worker moving the branch in the common dir).
    pub(crate) static BEFORE_MERGE: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    /// Tests: runs right before `integrate` first writes the integration ref, after its last check.
    pub(crate) static BEFORE_REF_WRITE: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    /// Tests: runs right before `snapshot` writes the task ref, after its last check.
    pub(crate) static BEFORE_TASK_REF: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

/// Names git or an agent harness reads specially. A worktree component that the volume treats as one of these is
/// that name.
const SPECIAL_GIT: [&str; 4] = [".gitattributes", ".gitmodules", ".git", ".lfsconfig"];
const PROTECTED: [&str; 13] = [".claude", ".codex", ".husky", "CLAUDE.md", "AGENTS.md", ".mcp.json", ".envrc", ".github", "workflows", ".gitattributes", ".gitmodules", ".git", ".lfsconfig"];

/// A name as a case-insensitive volume compares it, approximately: full case mapping (upper, then lower, so `ſ` and
/// the Kelvin sign become ASCII `s` and `k`), with combining marks dropped. It over-matches rather than under-matches
/// (APFS keeps accents distinct); `on_volume` backs it with the volume's own answer (#146 re-check N1).
fn fold(name: &str) -> String {
    name.chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .filter(|c| !matches!(*c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F))
        .collect()
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || fold(a) == fold(b)
}

/// The protected name an existing entry `dir/name` is, by the volume's own rules: `dir/<protected>` is the same
/// directory entry (dev and ino). Catches every fold or normalization APFS applies, whatever `fold` misses.
fn on_volume(dir: &Path, name: &str, names: &[&'static str]) -> Option<&'static str> {
    let m = std::fs::symlink_metadata(dir.join(name)).ok()?;
    names.iter().copied().find(|n| *n != name && std::fs::symlink_metadata(dir.join(n)).is_ok_and(|x| x.dev() == m.dev() && x.ino() == m.ino()))
}

/// `p` with every component the volume would read as a protected name spelled as that name. `wt` is the worktree
/// for the volume check; components that don't exist keep the `fold` answer.
fn canonical(wt: Option<&Path>, p: &str) -> String {
    let mut dir = wt.map(Path::to_path_buf);
    let mut out: Vec<String> = Vec::new();
    for c in p.split('/') {
        let named = PROTECTED.iter().copied().find(|n| same_name(n, c)).or_else(|| dir.as_deref().and_then(|d| on_volume(d, c, &PROTECTED)));
        out.push(named.map(String::from).unwrap_or_else(|| c.to_string()));
        dir = dir.map(|d| d.join(c));
    }
    out.join("/")
}

/// An agent config file (16.4 rule 6), at any depth for the named files and dirs; `.github/workflows/` at the root.
/// Names compare the way a case-insensitive volume compares them (`canonical`), so `.Claude/`, `.huſky/` and
/// `AGENTſ.md` count.
pub fn agent_config(p: &str) -> bool {
    agent_config_in(None, p)
}

fn agent_config_in(wt: Option<&Path>, p: &str) -> bool {
    let c = canonical(wt, p);
    let comps: Vec<&str> = c.split('/').collect();
    let base = comps.last().copied().unwrap_or("");
    AGENT_FILES.contains(&base) || comps[..comps.len().saturating_sub(1)].iter().any(|c| AGENT_DIRS.contains(c)) || c.starts_with(".github/workflows/")
}

/// A name git reads as attributes, submodules, LFS config or a repository (see `fold`).
fn git_special(name: &str) -> Option<&'static str> {
    SPECIAL_GIT.into_iter().find(|x| same_name(x, name))
}

/// Every `.gitattributes`, `.gitmodules`, `.lfsconfig` or `.git` in the worktree (as the volume compares names), as `(relative path, canonical name)`,
/// by plain `lstat` walk: no git, no symlink followed, ignored files included. The worktree's own `.git` file at the
/// root is skipped (the tamper checks hash it).
///
/// `deps` are the top-level dependency clones the core made: a real directory there is not walked. They are untracked
/// at `dispatch_head` and git-ignored, so an attributes file inside one only names ignored paths no core call stages,
/// and a worker that un-ignores one makes its files show as changed, where `name_rule` still refuses them (#146
/// claude: a package that ships `.gitattributes` or a git checkout is not tampering). Only where git honours
/// `GIT_ATTR_SOURCE`: the caller passes no `deps` on older git, where a worktree attributes file would apply.
fn special_files(wt: &Path, deps: &[String]) -> R<Vec<(String, &'static str)>> {
    const MAX_ENTRIES: usize = 5_000_000;
    let (mut out, mut seen) = (Vec::new(), 0usize);
    let mut stack: Vec<(PathBuf, String)> = vec![(wt.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        // What `dir/<special>` is on this volume, once per dir: an entry with the same dev and ino is that name.
        let ids: Vec<(&'static str, u64, u64)> = SPECIAL_GIT.iter().filter_map(|n| std::fs::symlink_metadata(dir.join(n)).ok().map(|m| (*n, m.dev(), m.ino()))).collect();
        for e in std::fs::read_dir(&dir).map_err(io)? {
            let e = e.map_err(io)?;
            seen += 1;
            if seen > MAX_ENTRIES {
                return Err(GitwError::Invalid("the worktree holds too many files to check"));
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let by_volume = || {
                let m = e.metadata().ok()?;
                ids.iter().find(|(_, d, i)| *d == m.dev() && *i == m.ino()).map(|(n, _, _)| *n)
            };
            if let Some(k) = git_special(&name).or_else(by_volume) {
                if !(r == ".git" && k == ".git" && name == ".git") {
                    out.push((r, k));
                }
                continue;
            }
            if e.file_type().map_err(io)?.is_dir() {
                if rel.is_empty() && deps.contains(&name) {
                    continue;
                }
                stack.push((e.path(), r));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// A symlink whose target is absolute or climbs out of the worktree (#146 gate L3): never committed, a leftover.
fn escaping_link(wt: &Path, rel: &str) -> bool {
    let Ok(m) = std::fs::symlink_metadata(wt.join(rel)) else { return false };
    if !m.file_type().is_symlink() {
        return false;
    }
    let Ok(target) = std::fs::read_link(wt.join(rel)) else { return true };
    let mut depth = rel.split('/').count() as i64 - 1;
    for c in target.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            Component::RootDir | Component::Prefix(_) => return true,
        }
    }
    false
}

/// The longest task glob accepted.
pub const MAX_GLOB: usize = 256;
/// The longest path matched (longer than any macOS path, `PATH_MAX` 1024).
const MAX_MATCH_PATH: usize = 4096;

/// A task glob: `**` matches any number of path segments, `*` and `?` stay inside one segment, a trailing `/` means
/// the whole directory. Literal otherwise.
pub fn glob_match(pat: &str, path: &str) -> bool {
    // The plan's globs and the worker's file names are both untrusted: matching is linear-time per segment pair and
    // polynomial overall (a DP table for `**`), so no pattern can make it backtrack without end.
    if pat.len() > MAX_GLOB || path.len() > MAX_MATCH_PATH {
        return false;
    }
    let pat = if let Some(d) = pat.strip_suffix('/') { format!("{d}/**") } else { pat.to_string() };
    if Path::new(&pat).components().any(|c| matches!(c, Component::ParentDir | Component::RootDir)) {
        return false;
    }
    let p: Vec<&str> = pat.split('/').collect();
    let s: Vec<&str> = path.split('/').collect();
    // dp[i][j]: p[i..] matches s[j..]. Filled from the end, so each cell is computed once.
    let (np, ns) = (p.len(), s.len());
    let mut dp = vec![vec![false; ns + 1]; np + 1];
    dp[np][ns] = true;
    for i in (0..np).rev() {
        for j in (0..=ns).rev() {
            dp[i][j] = if p[i] == "**" {
                dp[i + 1][j] || (j < ns && dp[i][j + 1])
            } else {
                j < ns && dp[i + 1][j + 1] && seg_match(p[i].as_bytes(), s[j].as_bytes())
            };
        }
    }
    dp[0][0]
}

/// One segment, `*` and `?` inside it: the iterative match that backtracks only to the last `*`, O(len(p) * len(s)).
fn seg_match(p: &[u8], s: &[u8]) -> bool {
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while j < s.len() {
        if i < p.len() && p[i] == b'*' {
            star = Some((i, j));
            i += 1;
        } else if i < p.len() && (p[i] == b'?' || p[i] == s[j]) {
            i += 1;
            j += 1;
        } else if let Some((si, sj)) = star {
            i = si + 1;
            j = sj + 1;
            star = Some((si, sj + 1));
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&c| c == b'*')
}

#[cfg(test)]
#[path = "gitw_tests.rs"]
mod tests;
