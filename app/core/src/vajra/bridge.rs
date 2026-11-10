//! Script bridge: the only place that runs programs. Fixed executables, argv
//! arrays (never a shell string), a timeout that kills the process group, a
//! 1 MiB output cap and a scrubbed environment.

use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const OUTPUT_CAP: usize = 1024 * 1024;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything the app may run.
#[derive(Clone, Debug)]
pub enum Request {
    CampaignVerify { slug: String },
    /// `vajra-campaign.sh verify-digest` with four hex lines on stdin (no slug, no file): is this signature valid for
    /// these hashes. The core hashes the bytes it holds; the script stays the only holder of the key.
    VerifyDigest { input: Vec<u8> },
    /// `jq -cS '[.features[] | {id, description}]'` over the held features.json on stdin: the exact canonicaliser the
    /// script uses for `featuresDefSha256`, with no file argument.
    FeaturesDef { features: Vec<u8> },
    CampaignProgress { slug: String, text: String },
    /// `check-evidence <path> [--head SHA] --json`, run in `repo`; or, for an
    /// archived bundle, `--archived <slug> --head <40-hex>` with no repo.
    CheckEvidence { path: PathBuf, head: Option<String>, archived: Option<String>, repo: Option<PathBuf> },
    KbSearch { query: String, k: u32, repo: PathBuf },
    Doctor,
    /// `args` starts with a read-only subcommand.
    Git { cwd: PathBuf, args: Vec<String> },
}

#[derive(Debug)]
pub enum BridgeError {
    /// The request failed validation; nothing was run.
    Invalid(String),
    Spawn(io::Error),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Invalid(m) => write!(f, "invalid request: {m}"),
            BridgeError::Spawn(e) => write!(f, "spawn failed: {e}"),
        }
    }
}

impl std::error::Error for BridgeError {}

#[derive(Debug)]
pub struct Output {
    /// None when the process was killed (timeout) or died on a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub timed_out: bool,
}

impl Output {
    /// The run finished whole: not killed by the timeout and not cut at `OUTPUT_CAP`.
    pub fn whole(&self) -> bool {
        !self.truncated && !self.timed_out
    }

    /// The exit code, only when the run was whole. Read this, never `code`, to decide what a run meant: the exit
    /// code and text of a cut or killed run say nothing reliable.
    pub fn exit(&self) -> Option<i32> {
        if self.whole() { self.code } else { None }
    }

    /// stdout, only when the run was whole (a cut buffer can still parse as JSON and mean the wrong thing).
    pub fn stdout_whole(&self) -> Option<&str> {
        self.whole().then_some(self.stdout.as_str())
    }
}

#[derive(Clone, Debug)]
pub struct Bridge {
    /// Dir holding `vajra-campaign.sh`, `vajra_tasks.py`, `vajra_kb.py`, `vajra-doctor.sh`.
    pub scripts_dir: PathBuf,
    pub python: PathBuf,
    pub git: PathBuf,
    pub timeout: Duration,
    /// The HOME the scripts see. `None` passes the process HOME through; the app sets it to the home it was opened with.
    pub home: Option<PathBuf>,
    /// Where `jq` is looked for (the first that is a file). A field so a test can say "no jq on this machine".
    pub jq_candidates: Vec<PathBuf>,
}

const GIT_READ_ONLY: [&str; 7] = ["--version", "rev-parse", "merge-base", "status", "log", "diff", "ls-files"];
const GIT_BAD_FLAGS: [&str; 6] = ["--output", "--ext-diff", "--textconv", "--exec-path", "-c", "--open-files-in-pager"];

impl Bridge {
    pub fn new(scripts_dir: PathBuf) -> Self {
        Self {
            scripts_dir,
            python: PathBuf::from("/usr/bin/python3"),
            git: PathBuf::from("/usr/bin/git"),
            timeout: DEFAULT_TIMEOUT,
            home: None,
            jq_candidates: ["/usr/bin/jq", "/opt/homebrew/bin/jq", "/usr/local/bin/jq", "/bin/jq"].iter().map(PathBuf::from).collect(),
        }
    }

    pub fn with_home(mut self, home: PathBuf) -> Self {
        self.home = Some(home);
        self
    }

    /// The `jq` the bridge would run, if any.
    pub fn find_jq(&self) -> Option<PathBuf> {
        self.jq_candidates.iter().find(|p| p.is_file()).cloned()
    }

    /// Build `(program, argv, cwd)` for a request, or refuse it.
    pub fn plan(&self, req: &Request) -> Result<(PathBuf, Vec<String>, Option<PathBuf>), BridgeError> {
        let script = |n: &str| self.scripts_dir.join(n);
        Ok(match req {
            Request::CampaignVerify { slug } => {
                (script("vajra-campaign.sh"), vec!["verify".into(), name(slug)?], None)
            }
            Request::VerifyDigest { .. } => (script("vajra-campaign.sh"), vec!["verify-digest".into()], None),
            Request::FeaturesDef { .. } => (
                self.find_jq().ok_or_else(|| BridgeError::Invalid("jq not found".into()))?,
                vec!["-cS".into(), "[.features[] | {id, description}]".into()],
                None,
            ),
            Request::CampaignProgress { slug, text } => (
                script("vajra-campaign.sh"),
                vec!["progress".into(), text.clone(), "--campaign".into(), name(slug)?],
                None,
            ),
            Request::CheckEvidence { path, head, archived, repo } => {
                if !path.is_absolute() {
                    return Err(BridgeError::Invalid("evidence path must be absolute".into()));
                }
                // An archived bundle is verified against one exact commit: a missing or abbreviated head is refused here, not
                // learned from the script's exit 2.
                if archived.is_some() && !head.as_deref().is_some_and(crate::sessions::argv::is_full_sha) {
                    return Err(BridgeError::Invalid("an archived evidence check needs a full 40-hex head".into()));
                }
                let mut argv = vec![
                    script("vajra_tasks.py").to_string_lossy().into_owned(),
                    "check-evidence".into(),
                    path.to_string_lossy().into_owned(),
                ];
                if let Some(h) = head {
                    if !crate::sessions::argv::is_sha(h) {
                        return Err(BridgeError::Invalid(format!("bad head: {h:?}")));
                    }
                    argv.extend(["--head".into(), h.clone()]);
                }
                if let Some(slug) = archived {
                    argv.extend(["--archived".into(), name(slug)?]);
                }
                argv.push("--json".into());
                (self.python.clone(), argv, repo.clone())
            }
            Request::KbSearch { query, k, repo } => {
                if query.trim().is_empty() {
                    return Err(BridgeError::Invalid("empty query".into()));
                }
                (
                    self.python.clone(),
                    vec![
                        script("vajra_kb.py").to_string_lossy().into_owned(),
                        "search".into(),
                        "--format".into(),
                        "ids".into(),
                        "--k".into(),
                        k.to_string(),
                        "--".into(),
                        query.clone(),
                    ],
                    Some(repo.clone()),
                )
            }
            Request::Doctor => (script("vajra-doctor.sh"), vec![], None),
            Request::Git { cwd, args } => {
                let sub = args.first().map(String::as_str).unwrap_or("");
                if !GIT_READ_ONLY.contains(&sub) {
                    return Err(BridgeError::Invalid(format!("git subcommand not allowed: {sub}")));
                }
                if let Some(bad) = args.iter().find(|a| {
                    GIT_BAD_FLAGS.iter().any(|f| a == f || (f.starts_with("--") && a.starts_with(&format!("{f}="))))
                }) {
                    return Err(BridgeError::Invalid(format!("git flag not allowed: {bad}")));
                }
                // No repo-configured helper ever runs: no fsmonitor hook, and diff
                // skips diff.external and textconv drivers (a worktree's .git/config
                // may have been written by an agent).
                let mut argv: Vec<String> = ["--no-pager", "-c", "core.fsmonitor="].map(String::from).to_vec();
                argv.push(sub.to_string());
                if sub == "diff" {
                    argv.extend(["--no-ext-diff".to_string(), "--no-textconv".to_string()]);
                }
                argv.extend(args.iter().skip(1).cloned());
                (self.git.clone(), argv, Some(cwd.clone()))
            }
        })
    }

    pub fn run(&self, req: &Request) -> Result<Output, BridgeError> {
        let (program, argv, cwd) = self.plan(req)?;
        let stdin = match req {
            Request::VerifyDigest { input } => Some(input.as_slice()),
            Request::FeaturesDef { features } => Some(features.as_slice()),
            _ => None,
        };
        run_process_full(&program, &argv, cwd.as_deref(), self.timeout, self.home.as_deref(), stdin)
    }
}

impl Bridge {
    /// True when the repo's own config (any scope but the user's global one) defines a `clean`, `smudge` or `process`
    /// filter, or when that cannot be determined: `git add` would run that command, and a repo-configured helper must never
    /// run from the desktop (an agent can write the config and the attribute). Filters from the user's own global config are
    /// theirs and are not counted. Fails closed.
    pub fn repo_filters(&self, repo: &Path, timeout: Duration) -> bool {
        self.frozen_config(repo, timeout).is_none()
    }

    /// The repo's whole effective config, read ONCE (includes expanded), or `None` when it defines a filter outside the
    /// user's global config or cannot be read. `tree_id` gives exactly these entries to `git add` through a private git dir
    /// with no config file of its own (#113 review): a filter written into the repo's config after this read is never seen.
    /// Left out: the include directives (already expanded), the keys every command sets itself (`core.fsmonitor`,
    /// `core.hooksPath`), the repository layout ones (`core.worktree`, `core.bare`) and the format ones
    /// (`core.repositoryformatversion`, `extensions.*`, applied by `private_git_dir`).
    fn frozen_config(&self, repo: &Path, timeout: Duration) -> Option<FrozenConfig> {
        if timeout.is_zero() {
            return None;
        }
        let argv: Vec<String> = ["--no-pager", "-c", "core.fsmonitor=", "-c", "core.hooksPath=/dev/null", "-C"].map(String::from).into_iter()
            .chain([repo.to_string_lossy().into_owned()])
            .chain(["config", "--list", "--show-scope", "-z"].map(String::from))
            .collect();
        let out = run_process_env(&self.git, &argv, None, timeout, self.home.as_deref(), None, &[]).ok().filter(|o| o.exit() == Some(0) && !o.truncated)?;
        parse_frozen(&out.stdout)
    }

    /// The whole-tree id of `repo`'s working tree (tracked edits and untracked files included, `exclude`d paths, relative to `repo`, left out),
    /// computed the way `vajra-verify-work.sh` records `git.worktreeTree`: HEAD read into a throwaway index, `add -A`, `write-tree`.
    /// No hook runs: `core.hooksPath=/dev/null` on every command (writing the temporary index would otherwise fire a repo's
    /// `post-index-change`, and `-c` outranks a repo-configured `hooksPath`).
    /// The repo's index, working tree and object store are never written: every new object (a blob per modified or untracked
    /// file, then the trees) goes to a temp object dir (`GIT_OBJECT_DIRECTORY`) that is removed afterwards, while the repo's
    /// own objects are read through `GIT_ALTERNATE_OBJECT_DIRECTORIES`. The tree id is the same as if they were written
    /// there. `None` if any step fails, the total `timeout` runs out, or the repo's own config defines an executable filter
    /// (`repo_filters`: `add` would run it). Same git hardening as `Request::Git`.
    pub fn tree_id(&self, repo: &Path, exclude: &[&str]) -> Option<String> {
        let start = Instant::now();
        // The one read of the config this snapshot uses; the steps below never read the repo's config files again.
        let frozen = self.frozen_config(repo, self.timeout)?;
        let dir = std::env::temp_dir().join(format!("vajra-tree-{}-{}", std::process::id(), start.elapsed().as_nanos() ^ rand_tag()));
        private_dir(&dir)?;
        let idx = dir.join("index");
        let objects = dir.join("objects");
        let gitdir = dir.join("git");
        let repo_objects: std::cell::RefCell<Option<PathBuf>> = std::cell::RefCell::new(None);
        // Every step runs from the repository's top level, as verify-work does (`--show-toplevel`): the run's cwd may be a
        // subdirectory, and `add -A -- .` from there would leave dirty files elsewhere at their HEAD content.
        let top: std::cell::RefCell<PathBuf> = std::cell::RefCell::new(repo.to_path_buf());
        let result = (|| {
            let git = |args: &[&str], with_index: bool| -> Option<Output> {
                let left = self.timeout.checked_sub(start.elapsed()).filter(|d| !d.is_zero())?;
                let argv: Vec<String> = ["--no-pager", "-c", "core.fsmonitor=", "-c", "core.hooksPath=/dev/null", "-C"].map(String::from).into_iter()
                    .chain([top.borrow().to_string_lossy().into_owned()])
                    .chain(args.iter().map(|a| a.to_string()))
                    .collect();
                let alt = repo_objects.borrow().clone();
                let work = top.borrow().clone();
                // The index steps run on the private git dir: the repo's own config, `info/` and hooks are not read, only
                // the frozen config entries (command scope), with no system or global file either.
                let mut env: Vec<(String, std::ffi::OsString)> = match (with_index, alt.as_deref()) {
                    (true, Some(alt)) => vec![
                        ("GIT_DIR".into(), gitdir.clone().into()),
                        ("GIT_WORK_TREE".into(), work.into()),
                        ("GIT_INDEX_FILE".into(), idx.clone().into()),
                        ("GIT_OBJECT_DIRECTORY".into(), objects.clone().into()),
                        ("GIT_ALTERNATE_OBJECT_DIRECTORIES".into(), alt.into()),
                        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
                        ("GIT_CONFIG_COUNT".into(), frozen.entries.len().to_string().into()),
                    ],
                    (true, None) => return None,
                    _ => vec![],
                };
                if with_index {
                    for (i, (k, v)) in frozen.entries.iter().enumerate() {
                        env.push((format!("GIT_CONFIG_KEY_{i}"), k.into()));
                        env.push((format!("GIT_CONFIG_VALUE_{i}"), v.into()));
                    }
                }
                let env: Vec<(&str, &Path)> = env.iter().map(|(k, v)| (k.as_str(), Path::new(v))).collect();
                run_process_env(&self.git, &argv, None, left, self.home.as_deref(), None, &env).ok().filter(|o| o.exit() == Some(0))
            };
            let root = git(&["rev-parse", "--show-toplevel"], false)?.stdout.trim().to_string();
            *top.borrow_mut() = Path::new(&root).canonicalize().ok().filter(|p| p.is_absolute())?;
            // The repo's object dir as an absolute path (`--git-path` answers relative to the top level for a plain checkout).
            let found = git(&["rev-parse", "--git-path", "objects"], false)?.stdout.trim().to_string();
            let found = if Path::new(&found).is_absolute() { PathBuf::from(found) } else { top.borrow().join(found) };
            *repo_objects.borrow_mut() = Some(found.canonicalize().ok()?);
            private_dir(&objects)?;
            // The repo's ignore and attribute files under its git dir decide which files are added and how, as for
            // verify-work; copied (they name filters but define none: the definitions are the frozen config's).
            let mut info: Vec<(&str, Vec<u8>)> = Vec::new();
            for name in ["exclude", "attributes"] {
                let p = git(&["rev-parse", "--git-path", &format!("info/{name}")], false)?.stdout.trim().to_string();
                let p = if Path::new(&p).is_absolute() { PathBuf::from(p) } else { top.borrow().join(p) };
                // Repo-controlled: read without following a link, blocking on a FIFO or growing without bound; a file
                // that is refused fails the tree closed (the caller then shows `unverified`), never a guess (#134 review).
                if let Some(b) = read_info_file(&p).ok()? {
                    info.push((name, b));
                }
            }
            private_git_dir(&gitdir, &frozen, &info)?;
            if let Some(head) = git(&["rev-parse", "--verify", "-q", "HEAD"], false) {
                let head = head.stdout.trim().to_string();
                git(&["read-tree", &head], true)?;
            }
            let mut add = vec!["add".to_string(), "-A".into(), "--".into(), ".".into()];
            // verify-work excludes its walkthrough base directory only when that directory lies inside the repository (its
            // `case "$BASE_ABS/" in "$TOP"/*`), as a path relative to the top level. The same rule here: an `exclude` entry is an
            // absolute directory or one relative to the run's directory (`repo`), and one outside the repository leaves nothing out.
            let top_dir = top.borrow().clone();
            add.extend(exclude.iter().filter_map(|d| {
                let abs = if Path::new(d).is_absolute() { PathBuf::from(d) } else { repo.join(d) };
                let abs = abs.canonicalize().unwrap_or(abs);
                abs.strip_prefix(&top_dir).ok().filter(|r| !r.as_os_str().is_empty()).map(|r| format!(":(exclude){}", r.display()))
            }));
            let add: Vec<&str> = add.iter().map(String::as_str).collect();
            #[cfg(test)]
            BEFORE_ADD.with(|h| if let Some(f) = h.borrow().as_ref() { f() });
            git(&add, true)?;
            let tree = git(&["write-tree"], true)?.stdout.trim().to_string();
            (tree.len() >= 40).then_some(tree)
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
}

/// A repo config as read once by `frozen_config`: the entries `git add` gets, and the repository format.
#[derive(Debug, Default)]
struct FrozenConfig {
    entries: Vec<(String, String)>,
    /// `extensions.objectformat` (`sha1` or `sha256`), when the repo sets it.
    object_format: Option<String>,
}

/// Parse `git config --list --show-scope -z` (`scope NUL key LF value NUL`, or `scope NUL key NUL` for a key with no
/// value). `None` on a filter command outside the global scope, an object format this code does not know, or bad output.
fn parse_frozen(out: &str) -> Option<FrozenConfig> {
    let mut f = FrozenConfig::default();
    let mut parts = out.split('\0');
    while let Some(scope) = parts.next() {
        if scope.is_empty() {
            // The final NUL leaves one empty piece; anything after it is bad output.
            return parts.next().is_none().then_some(f);
        }
        let item = parts.next()?;
        let (key, value) = match item.split_once('\n') {
            Some((k, v)) => (k, v),
            None => (item, "true"),
        };
        let lower = key.to_ascii_lowercase();
        let (section, rest) = lower.split_once('.')?;
        let name = rest.rsplit('.').next()?;
        if section == "filter" && matches!(name, "clean" | "smudge" | "process") && scope != "global" {
            return None;
        }
        match (section, rest) {
            ("include" | "includeif", _) => continue,
            ("core", "fsmonitor" | "hookspath" | "worktree" | "bare" | "repositoryformatversion") => continue,
            ("extensions", "objectformat") => {
                let v = value.to_ascii_lowercase();
                if v != "sha1" && v != "sha256" {
                    return None;
                }
                f.object_format = Some(v);
                continue;
            }
            ("extensions", _) => continue,
            _ => {}
        }
        f.entries.push((key.to_string(), value.to_string()));
    }
    Some(f)
}

/// A git dir of the app's own under the snapshot's temp dir (mode 0700): `HEAD`, `refs/`, an `objects/` placeholder
/// (`GIT_OBJECT_DIRECTORY` is the real one), `info/` with the copied files, and a config file holding only the
/// repository format. No hooks dir.
fn private_git_dir(gitdir: &Path, frozen: &FrozenConfig, info: &[(&str, Vec<u8>)]) -> Option<()> {
    private_dir(gitdir)?;
    for d in ["refs", "objects", "info"] {
        private_dir(&gitdir.join(d))?;
    }
    std::fs::write(gitdir.join("HEAD"), b"ref: refs/heads/vajra-tree\n").ok()?;
    let config = match &frozen.object_format {
        Some(fmt) => format!("[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = {fmt}\n"),
        None => "[core]\n\trepositoryformatversion = 0\n".to_string(),
    };
    std::fs::write(gitdir.join("config"), config).ok()?;
    for (name, bytes) in info {
        std::fs::write(gitdir.join("info").join(name), bytes).ok()?;
    }
    Some(())
}

#[cfg(test)]
thread_local! {
    /// Tests: run between the config check and `git add` in `tree_id` (a worker rewriting the config in that window).
    pub(crate) static BEFORE_ADD: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

/// The most of a repository's `info/exclude` or `info/attributes` that `tree_id` reads; a larger one is refused.
const INFO_FILE_MAX: u64 = 1 << 20;

/// A repository's info file, read so that the repository cannot block or grow this process through it: opened read-only
/// with `O_NOFOLLOW | O_NONBLOCK` (never through a symbolic link at the last component, never waiting on a FIFO), the
/// OPENED handle must be a regular file, and at most `INFO_FILE_MAX` bytes are read. `Ok(None)`: there is no such file.
/// `Err`: refused (a link, a FIFO or device, too large, unreadable).
fn read_info_file(p: &Path) -> Result<Option<Vec<u8>>, ()> {
    use std::os::unix::fs::OpenOptionsExt;
    let f = match std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(p) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    if !f.metadata().map_err(|_| ())?.is_file() {
        return Err(());
    }
    let mut b = Vec::new();
    f.take(INFO_FILE_MAX + 1).read_to_end(&mut b).map_err(|_| ())?;
    if b.len() as u64 > INFO_FILE_MAX {
        return Err(());
    }
    Ok(Some(b))
}

/// Create `path` readable by its owner alone (mode 0700, before any object is written into it): `git add` stores the
/// repository's modified and untracked file contents there, and the umask alone would leave it open to other local users.
fn private_dir(path: &Path) -> Option<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path).ok()
}

fn rand_tag() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

/// Slugs and task ids: `[A-Za-z0-9][A-Za-z0-9._-]*`, so never an option.
fn name(s: &str) -> Result<String, BridgeError> {
    let ok = s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && s.len() <= 128;
    if ok { Ok(s.to_string()) } else { Err(BridgeError::Invalid(format!("bad name: {s:?}"))) }
}

/// The fixed PATH every bridged program runs with (NFR-SEC-2). `vajra-campaign.sh` holds the HMAC key and calls
/// `openssl`, `jq`, `awk`, `tr` and `mktemp` by bare name, and a same-uid worker can write user dirs such as
/// `~/.local/bin`, so a bridged program never sees a user dir ahead of these. Never the resolved login-shell PATH.
pub const BASE_PATH: [&str; 6] = ["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/opt/homebrew/bin", "/usr/local/bin"];

/// `BASE_PATH` as one PATH value: what every `Request` runs with.
pub fn bridge_path() -> std::ffi::OsString {
    BASE_PATH.join(":").into()
}

/// The claude version check's PATH: the user's resolved dirs in their own order, which is the order the PTY sessions and
/// the Lead give `claude` (`userpath::joined`), so an npm `claude`'s `#!/usr/bin/env node` picks the same `node` here
/// as at run time; then any system dir they lack. Relative entries are dropped (never the cwd). Keyless only: it is
/// never used for a script that holds or is given the HMAC key.
pub fn probe_path(user: &[PathBuf]) -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for d in user.iter().cloned().chain(BASE_PATH.map(PathBuf::from)) {
        if d.is_absolute() && !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    std::env::join_paths(&dirs).unwrap_or_else(|_| bridge_path())
}

fn scrubbed_env(cmd: &mut Command, home: Option<&Path>, path: std::ffi::OsString) {
    cmd.env_clear();
    cmd.env("PATH", path);
    cmd.env("LC_ALL", "C.UTF-8");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    if let Some(home) = home.map(|h| h.as_os_str().to_owned()).or_else(|| std::env::var_os("HOME")) {
        cmd.env("HOME", home);
    }
}

/// Read `r` fully, keeping at most `cap` bytes and draining the rest so the
/// child never blocks on a full pipe.
fn read_capped(mut r: impl Read, cap: usize) -> (Vec<u8>, bool) {
    let (mut kept, mut over) = (Vec::new(), false);
    let mut buf = [0u8; 8192];
    while let Ok(n) = r.read(&mut buf) {
        if n == 0 {
            break;
        }
        let room = cap.saturating_sub(kept.len());
        kept.extend_from_slice(&buf[..n.min(room)]);
        over |= n > room;
    }
    (kept, over)
}

fn kill_group(pid: u32) {
    // SAFETY: killpg on a pgid this process created; failure (ESRCH) is fine.
    unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
}

pub fn run_process(
    program: &Path,
    argv: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<Output, BridgeError> {
    run_process_with_home(program, argv, cwd, timeout, None)
}

pub fn run_process_with_home(
    program: &Path,
    argv: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
    home: Option<&Path>,
) -> Result<Output, BridgeError> {
    run_process_full(program, argv, cwd, timeout, home, None)
}

/// As `run_process_with_home`, optionally feeding `stdin` (written on its own thread so a child that stops reading
/// can never block the caller; the pipe is closed when the bytes are written).
pub fn run_process_full(
    program: &Path,
    argv: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
    home: Option<&Path>,
    stdin: Option<&[u8]>,
) -> Result<Output, BridgeError> {
    run_process_env(program, argv, cwd, timeout, home, stdin, &[])
}

/// As `run_process_full`, with extra environment variables on top of the scrubbed environment.
#[allow(clippy::too_many_arguments)]
pub fn run_process_env(
    program: &Path,
    argv: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
    home: Option<&Path>,
    stdin: Option<&[u8]>,
    env: &[(&str, &Path)],
) -> Result<Output, BridgeError> {
    // The fixed system PATH only: every `Request` (the key-holding campaign script among them) runs through here.
    run_process_inner(program, argv, cwd, timeout, home, stdin, env, bridge_path())
}

/// As `run_process`, on `probe_path(path)`: the user's dirs in their runtime order, then the system dirs. For the
/// claude version check only (an npm `claude` must find the same `node` it runs with); never for a script that holds
/// or is given the HMAC key.
pub fn run_process_on_path(
    program: &Path,
    argv: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
    path: &[PathBuf],
) -> Result<Output, BridgeError> {
    run_process_inner(program, argv, cwd, timeout, None, None, &[], probe_path(path))
}

#[allow(clippy::too_many_arguments)]
fn run_process_inner(
    program: &Path,
    argv: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
    home: Option<&Path>,
    stdin: Option<&[u8]>,
    env: &[(&str, &Path)],
    path: std::ffi::OsString,
) -> Result<Output, BridgeError> {
    let mut cmd = Command::new(program);
    cmd.args(argv).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    scrubbed_env(&mut cmd, home, path);
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.process_group(0);
    let mut child = cmd.spawn().map_err(BridgeError::Spawn)?;
    let pid = child.id();
    let (out, err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let bytes = bytes.to_vec();
        thread::spawn(move || {
            use std::io::Write;
            let _ = pipe.write_all(&bytes); // a child that exits early closes the pipe; that is not an error here
        });
    }
    let t_out = thread::spawn(move || read_capped(out, OUTPUT_CAP));
    let t_err = thread::spawn(move || read_capped(err, OUTPUT_CAP));

    let start = Instant::now();
    let (mut code, mut timed_out) = (None, false);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                code = status.code();
                break;
            }
            Ok(None) if start.elapsed() >= timeout => {
                timed_out = true;
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(_) => break,
        }
    }
    // Kill leftovers (and on timeout the child itself) so the pipes close.
    kill_group(pid);
    let _ = child.wait();
    let (so, to) = t_out.join().unwrap_or_default();
    let (se, te) = t_err.join().unwrap_or_default();
    Ok(Output {
        code,
        stdout: String::from_utf8_lossy(&so).into_owned(),
        stderr: String::from_utf8_lossy(&se).into_owned(),
        truncated: to || te,
        timed_out,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KbHit {
    pub id: String,
    pub kind: String,
    pub title: String,
}

/// One line of `vajra_kb.py search --format ids`, matching design 4.1:
/// `^kb-[0-9a-f]{6}\s+(decision|fact|lesson|delivery|pitfall)\s+\S.*$`.
pub fn parse_kb_line(line: &str) -> Option<KbHit> {
    let hex = line.strip_prefix("kb-")?.get(..6)?;
    if !hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    let rest = &line[9..];
    let after_id = rest.trim_start();
    if after_id.len() == rest.len() {
        return None; // \s+ needs at least one space
    }
    let kind_end = after_id.find(char::is_whitespace)?;
    let kind = &after_id[..kind_end];
    if !["decision", "fact", "lesson", "delivery", "pitfall"].contains(&kind) {
        return None;
    }
    let title = after_id[kind_end..].trim_start();
    if title.is_empty() {
        return None;
    }
    Some(KbHit { id: format!("kb-{hex}"), kind: kind.to_string(), title: title.to_string() })
}

/// Hits from search output. Non-matching lines are dropped; empty output
/// means "no results".
pub fn parse_kb_hits(stdout: &str) -> Vec<KbHit> {
    stdout.lines().filter_map(parse_kb_line).collect()
}

#[cfg(test)]
mod private_dir_tests {
    use super::private_dir;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_private_dir_is_owner_only_even_under_an_open_umask() {
        let base = std::env::temp_dir().join(format!("vajra-private-dir-{}", std::process::id()));
        // SAFETY: umask only changes this process's creation mask; restored right after the directory exists.
        let old = unsafe { libc::umask(0) };
        let made = private_dir(&base);
        unsafe { libc::umask(old) };
        assert!(made.is_some());
        let mode = std::fs::metadata(&base).unwrap().permissions().mode() & 0o777;
        let _ = std::fs::remove_dir(&base);
        assert_eq!(mode, 0o700, "a directory that holds repository contents must not be readable by other users");
        // A plain create_dir under the same umask would have left it open: that is what this guards.
    }
}

#[cfg(test)]
mod frozen_config_tests {
    use super::parse_frozen;

    #[test]
    fn a_filter_outside_the_global_scope_refuses_and_a_global_one_is_kept() {
        assert!(parse_frozen("local\0filter.evil.clean\ncat\0").is_none());
        assert!(parse_frozen("worktree\0filter.evil.process\nx\0").is_none());
        assert!(parse_frozen("command\0FILTER.Evil.Smudge\nx\0").is_none(), "keys compare without case");
        assert!(parse_frozen("unknown\0filter.evil.clean\nx\0").is_none(), "a scope git cannot name is not the user's");
        let f = parse_frozen("global\0filter.lfs.clean\ngit-lfs clean -- %f\0").unwrap();
        assert_eq!(f.entries, vec![("filter.lfs.clean".to_string(), "git-lfs clean -- %f".to_string())]);
    }

    #[test]
    fn includes_layout_and_forced_keys_are_left_out_and_formats_are_checked() {
        let f = parse_frozen(
            "local\0include.path\n/tmp/x\0local\0includeif.gitdir:/x.path\n/y\0local\0core.hookspath\n/h\0local\0core.fsmonitor\n/m\0\
             local\0core.worktree\n/w\0local\0core.bare\nfalse\0local\0core.repositoryformatversion\n1\0local\0extensions.worktreeconfig\ntrue\0\
             local\0extensions.objectformat\nsha256\0local\0core.autocrlf\ninput\0local\0x.flag\0",
        )
        .unwrap();
        assert_eq!(f.object_format.as_deref(), Some("sha256"));
        assert_eq!(f.entries, vec![("core.autocrlf".to_string(), "input".to_string()), ("x.flag".to_string(), "true".to_string())]);
        assert!(parse_frozen("local\0extensions.objectformat\nmd5\0").is_none());
        assert!(parse_frozen("local\0core.autocrlf\ninput\0trailing").is_none(), "output not ending in NUL is refused");
        assert!(parse_frozen("").unwrap().entries.is_empty());
    }
}
