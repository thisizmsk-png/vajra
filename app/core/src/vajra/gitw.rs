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
//! Before every call on a repo the repo's `<common>/config` and `info/attributes` are checked against the repo
//! baseline (`repo_config_changed`), and before every call on a ticket its per-worktree files, its common directory's
//! identity and its detached HEAD are checked with plain filesystem reads (`tampered`). `snapshot` uses plumbing only,
//! through a fresh private index under the core's data dir; it never writes through the worker's HEAD or index.

use std::collections::{BTreeSet, HashMap};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::bridge::{run_process_env, Output};
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
#[derive(Clone, Debug)]
pub struct GlobalBaseline {
    pub copy: PathBuf,
    sources: Vec<(PathBuf, FileHash)>,
}

impl GlobalBaseline {
    /// Load the stored baseline, or take one when there is none. Call before any agent session launches.
    pub fn take_or_load(git: &Path, data: &Path, home: &Path, xdg: Option<&Path>) -> R<GlobalBaseline> {
        let dir = data.join("git-baseline");
        crate::paths::ensure_private_dir(&dir).map_err(io)?;
        let copy = dir.join("global.config");
        let sources = dir.join("sources.json");
        if copy.is_file() {
            if let Some(s) = std::fs::read_to_string(&sources).ok().and_then(|t| parse_sources(&t)) {
                return Ok(GlobalBaseline { copy, sources: s });
            }
        }
        Self::take(git, &dir, home, xdg)
    }

    fn take(git: &Path, dir: &Path, home: &Path, xdg: Option<&Path>) -> R<GlobalBaseline> {
        let flat = flatten_global(git, home, xdg)?;
        let copy = dir.join("global.config");
        write_private(&copy, flat.as_bytes())?;
        let sources = global_sources(git, home, xdg);
        write_private(&dir.join("sources.json"), render_sources(&sources).as_bytes())?;
        Ok(GlobalBaseline { copy, sources })
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
        let dir = self.copy.parent().ok_or(GitwError::Invalid("baseline dir"))?.to_path_buf();
        *self = Self::take(git, &dir, home, xdg)?;
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

fn write_private(p: &Path, bytes: &[u8]) -> R<()> {
    use std::io::Write;
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(io)?;
    f.write_all(bytes).map_err(io)?;
    f.sync_all().map_err(io)?;
    std::fs::rename(&tmp, p).map_err(io)
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
        None => (item.to_string(), String::new()),
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
    let mut out: Vec<(PathBuf, FileHash)> = Vec::new();
    let mut queue: Vec<(PathBuf, usize)> = vec![(home.join(".gitconfig"), 0), (xdg_config(home, xdg), 0)];
    while let Some((p, depth)) = queue.pop() {
        if out.iter().any(|(q, _)| q == &p) || out.len() >= 64 {
            continue;
        }
        let h = hash_file(&p, true);
        let present = matches!(h, FileHash::Sha(_));
        out.push((p.clone(), h));
        if !present || depth >= 10 {
            continue;
        }
        let Some(f) = p.to_str() else { continue };
        let Some(o) = config_read(git, home, xdg, &["config", "--file", f, "--no-includes", "--get-regexp", "-z", "^include(if\\..*)?\\.path$"]) else { continue };
        for (_, target) in parse_list_z(&o.stdout) {
            let t = if let Some(rest) = target.strip_prefix("~/") {
                home.join(rest)
            } else if Path::new(&target).is_absolute() {
                PathBuf::from(&target)
            } else {
                p.parent().unwrap_or(Path::new("/")).join(&target)
            };
            queue.push((t, depth + 1));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// ------------------------------------------------------------ the repo baseline

#[derive(Clone, Debug)]
struct Held {
    config: FileHash,
    attributes: FileHash,
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

    fn check(&self, gw: &Gitw, common: &Path) -> R<()> {
        let m = crate::lock(&self.held);
        let h = m.get(common).ok_or(GitwError::Invalid("no repo baseline is held; start the run first"))?;
        let config = hash_file(&common.join("config"), false);
        let attributes = hash_file(&common.join("info/attributes"), false);
        if config == h.config && attributes == h.attributes {
            return Ok(());
        }
        let mut keys = if config == h.config { Vec::new() } else { diff_keys(&h.entries, &read_config_file(&gw.git, &common.join("config"), &gw.home).unwrap_or_default()) };
        if attributes != h.attributes {
            keys.push("info/attributes".into());
        }
        if keys.is_empty() {
            keys.push("config".into());
        }
        Err(GitwError::RepoConfigChanged { keys })
    }
}

fn take_repo(gw: &Gitw, common: &Path) -> Held {
    Held {
        config: hash_file(&common.join("config"), false),
        attributes: hash_file(&common.join("info/attributes"), false),
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
}

impl Ticket {
    pub fn common_dir(&self) -> &Path {
        &self.common.path
    }
    pub fn task_ref(&self) -> String {
        format!("refs/heads/vajra/{}/task-{}-r{}", self.run, self.task, self.round)
    }
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
    pub repos: RepoBaselines,
    /// git 2.40 or later: `GIT_ATTR_SOURCE` is honoured.
    attr_source: bool,
    #[cfg(test)]
    pub checks: std::sync::atomic::AtomicBool,
}

impl Gitw {
    /// `check_roots` must have passed. `global` is the `GlobalBaseline` copy.
    pub fn new(git: PathBuf, home: PathBuf, data: PathBuf, work: PathBuf, global: PathBuf) -> Self {
        let mut g = Gitw {
            git, home, data, work, global, repos: RepoBaselines::default(), attr_source: false,
            #[cfg(test)]
            checks: std::sync::atomic::AtomicBool::new(true),
        };
        g.attr_source = g.version().is_some_and(|v| v >= (2, 40));
        g
    }

    fn version(&self) -> Option<(u32, u32)> {
        let o = run_process_env(&self.git, &["--version".into()], None, CALL_TIMEOUT, Some(&self.home), None, &[]).ok()?;
        let v = o.stdout.split_whitespace().nth(2)?;
        let mut it = v.split('.').map(|x| x.parse::<u32>().ok());
        Some((it.next()??, it.next()??))
    }

    fn checks_on(&self) -> bool {
        #[cfg(test)]
        return self.checks.load(std::sync::atomic::Ordering::SeqCst);
        #[cfg(not(test))]
        true
    }

    fn run(&self, c: Call<'_>) -> R<Output> {
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
        self.repos.check(self, &common)?;
        let leaf = if integration { if round == 1 { "integration".to_string() } else { format!("integration-r{round}") } } else { format!("{task}-r{round}") };
        let run_dir = self.work.join(project).join(run);
        let wt = run_dir.join(&leaf);
        let tmp = run_dir.join("tmp").join(&leaf);
        let private = self.data.join("tickets").join(project).join(run).join(&leaf);
        if wt.exists() || private.exists() {
            return Err(GitwError::Exists);
        }
        for d in [&run_dir, &tmp, &private] {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(d).map_err(io)?;
        }
        let wt_s = wt.to_str().ok_or(GitwError::Invalid("worktree path"))?;
        self.ok("worktree add", Call::new(At::Repo(&repo), &["worktree", "add", "--detach", "--", wt_s, from]))?;
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
        let t = Ticket {
            project: project.into(),
            run: run.into(),
            task: task.into(),
            round,
            repo,
            wt: wt.clone(),
            common: common_id(&common).ok_or_else(|| tampered("the common git directory"))?,
            private: private.canonicalize().map_err(io)?,
            tmp: tmp.canonicalize().map_err(io)?,
            dotgit: hash_file(&wt.join(".git"), false),
            commondir: hash_file(&gitdir.join("commondir"), false),
            config_worktree: hash_file(&gitdir.join("config.worktree"), false),
            gitdir,
            dispatch_head: from.into(),
            integration,
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

    /// 16.6 `snapshot`: plumbing only, through a fresh private index. `globs` are the task's files; `named` the agent
    /// config files the ask names; `worker` the trailer's "<account> <model>".
    pub fn snapshot(&self, t: &Ticket, globs: &[String], named: &[String], worker: &str) -> R<Snapshot> {
        if t.integration {
            return Err(GitwError::Invalid("the integration tree is never snapshotted"));
        }
        if worker.is_empty() || worker.len() > 200 || worker.chars().any(|c| c.is_control()) {
            return Err(GitwError::Invalid("worker label"));
        }
        // A repo config change makes the finishing ticket tampered (16.6).
        self.preflight(t).map_err(|e| match e {
            GitwError::RepoConfigChanged { keys } => tampered(format!("repo config: {}", keys.join(", "))),
            e => e,
        })?;
        let idx = self.fresh_index(t, "index")?;
        let changed = self.changed(t, &idx)?;
        if self.checks_on() {
            for p in &changed {
                let base = p.rsplit('/').next().unwrap_or(p);
                if base == ".gitattributes" || base == ".gitmodules" {
                    return Err(tampered(p.clone()));
                }
                // An embedded repository shows as `dir/`; a `.git` component anywhere is a nested repo.
                if p.ends_with('/') || p.split('/').any(|c| c == ".git") {
                    return Err(tampered(format!("nested git repository at {p}")));
                }
            }
        }
        let (files, leftovers): (Vec<String>, Vec<String>) = changed.into_iter().partition(|p| globs.iter().any(|g| glob_match(g, p)) && (!agent_config(p) || named.contains(p)));
        if files.is_empty() {
            return Ok(Snapshot { commit: None, task_ref: None, files, leftovers });
        }
        let task_ref = t.task_ref();
        vajra_ref(&task_ref)?;
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
        let zero = "0".repeat(commit.len());
        let u = self.run(Call::new(At::Pinned(t), &["update-ref", &task_ref, &commit, &zero]))?;
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

    /// `checkout -B vajra/<run>/integration[-r<n>] <base>`, then `merge --no-ff` each commit, checking before each merge
    /// that HEAD is exactly that branch. Returns the integration head.
    pub fn integrate(&self, it: &Ticket, base: &str, commits: &[String]) -> R<String> {
        if !it.integration {
            return Err(GitwError::Invalid("not an integration tree"));
        }
        if !valid_oid(base) || !commits.iter().all(|c| valid_oid(c)) {
            return Err(GitwError::Invalid("commit ids"));
        }
        let r = Self::integration_ref(it);
        vajra_ref(&r)?;
        let branch = r.trim_start_matches("refs/heads/");
        self.preflight(it)?;
        self.ok("checkout", Call::new(At::Pinned(it), &["checkout", "-q", "-B", branch, base]))?;
        let mut cfg = vec![NO_SIGN];
        cfg.extend(self.identity(it)?);
        for c in commits {
            self.preflight(it)?;
            let head = self.ok("symbolic-ref", Call::new(At::Pinned(it), &["symbolic-ref", "-q", "HEAD"]))?;
            if head.stdout.trim() != r {
                return Err(tampered("the integration tree's HEAD"));
            }
            let o = self.run(Call::new(At::Pinned(it), &["merge", "--no-ff", "--no-edit", "--no-verify", "--", c]).config(&cfg))?;
            if o.exit() != Some(0) {
                let _ = self.run(Call::new(At::Pinned(it), &["merge", "--abort"]));
                return Err(GitwError::MergeConflict { sha: c.clone() });
            }
        }
        Ok(self.ok("rev-parse", Call::new(At::Pinned(it), &["rev-parse", "--verify", "HEAD"]))?.stdout.trim().to_string())
    }

    /// The capture check (16.6 Retention): the tamper checks pass, the whole worktree staged the way `snapshot` staged
    /// it equals the task commit's tree, and the commit is an ancestor of the integration branch.
    pub fn capture_check(&self, t: &Ticket, commit: &str, integration_ref: &str) -> R<()> {
        if !valid_oid(commit) {
            return Err(GitwError::Invalid("commit id"));
        }
        vajra_ref(integration_ref)?;
        self.preflight(t)?;
        let idx = self.fresh_index(t, "capture-index")?;
        self.ok("add", Call::new(At::Pinned(t), &["add", "-A", "--", "."]).index(&idx))?;
        let cap = self.ok("write-tree", Call::new(At::Pinned(t), &["write-tree"]).index(&idx))?.stdout.trim().to_string();
        let want = self.ok("rev-parse", Call::new(At::Pinned(t), &["rev-parse", "--verify", &format!("{commit}^{{tree}}")]))?.stdout.trim().to_string();
        if cap != want {
            let d = self.ok("diff-tree", Call::new(At::Pinned(t), &["diff-tree", "-r", "--name-only", "-z", "--no-renames", &want, &cap]))?;
            return Err(GitwError::Kept { paths: d.stdout.split('\0').filter(|s| !s.is_empty()).map(String::from).collect() });
        }
        let a = self.run(Call::new(At::Pinned(t), &["merge-base", "--is-ancestor", commit, integration_ref]))?;
        if a.exit() != Some(0) {
            return Err(GitwError::Kept { paths: vec!["(the task commit is not integrated)".into()] });
        }
        Ok(())
    }

    /// Remove a ticket's worktree once the capture check passes (`--force`: the plumbing snapshot leaves it dirty).
    pub fn worktree_remove(&self, t: &Ticket, commit: &str, integration_ref: &str) -> R<()> {
        self.capture_check(t, commit, integration_ref)?;
        self.remove(t)
    }

    /// The human discarded the ticket (after a confirm): remove it without the capture check.
    pub fn discard(&self, t: &Ticket) -> R<()> {
        self.repos.check(self, &t.common.path)?;
        self.remove(t)
    }

    fn remove(&self, t: &Ticket) -> R<()> {
        let wt = t.wt.to_str().ok_or(GitwError::Invalid("worktree path"))?;
        self.ok("worktree remove", Call::new(At::Repo(&t.repo), &["worktree", "remove", "--force", "--", wt]))?;
        for d in [&t.tmp, &t.private] {
            // Only the core-built dirs under <work> and <data>.
            if d.starts_with(&self.work) || d.starts_with(&self.data) {
                let _ = std::fs::remove_dir_all(d);
            }
        }
        Ok(())
    }

    /// `git branch -D vajra/...`: only a task branch under `refs/heads/vajra/`.
    pub fn branch_delete(&self, repo: &Path, r: &str) -> R<()> {
        vajra_ref(r)?;
        let common = self.common_of(repo)?;
        self.repos.check(self, &common)?;
        self.ok("branch -D", Call::new(At::Repo(repo), &["branch", "-D", "--", r.trim_start_matches("refs/heads/")]))?;
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: one more path for `add` that is not in the staged list (forces the tree check).
    pub(crate) static EXTRA_ADD: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// An agent config file (16.4 rule 6), at any depth for the named files and dirs; `.github/workflows/` at the root.
pub fn agent_config(p: &str) -> bool {
    let comps: Vec<&str> = p.split('/').collect();
    let base = comps.last().copied().unwrap_or("");
    AGENT_FILES.contains(&base) || comps[..comps.len().saturating_sub(1)].iter().any(|c| AGENT_DIRS.contains(c)) || p.starts_with(".github/workflows/")
}

/// A task glob: `**` matches any number of path segments, `*` and `?` stay inside one segment, a trailing `/` means
/// the whole directory. Literal otherwise.
pub fn glob_match(pat: &str, path: &str) -> bool {
    let pat = if let Some(d) = pat.strip_suffix('/') { format!("{d}/**") } else { pat.to_string() };
    if Path::new(&pat).components().any(|c| matches!(c, Component::ParentDir | Component::RootDir)) {
        return false;
    }
    let p: Vec<&str> = pat.split('/').collect();
    let s: Vec<&str> = path.split('/').collect();
    fn segs(p: &[&str], s: &[&str]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((&"**", rest)) => (0..=s.len()).any(|i| segs(rest, &s[i..])),
            Some((h, rest)) => !s.is_empty() && seg(h.as_bytes(), s[0].as_bytes()) && segs(rest, &s[1..]),
        }
    }
    fn seg(p: &[u8], s: &[u8]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((b'*', rest)) => (0..=s.len()).any(|i| seg(rest, &s[i..])),
            Some((b'?', rest)) => !s.is_empty() && seg(rest, &s[1..]),
            Some((c, rest)) => s.first() == Some(c) && seg(rest, &s[1..]),
        }
    }
    segs(&p, &s)
}

#[cfg(test)]
#[path = "gitw_tests.rs"]
mod tests;
