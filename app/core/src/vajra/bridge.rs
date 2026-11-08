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
    /// True when the repo's own config (local, worktree or command scope) defines a `clean`, `smudge` or `process` filter, or
    /// when that cannot be determined: `git add` would run that command, and a repo-configured helper must never run from the
    /// desktop (an agent can write the config and the attribute). Filters from the user's own global config are theirs and are
    /// not counted. Fails closed.
    pub fn repo_filters(&self, repo: &Path, timeout: Duration) -> bool {
        if timeout.is_zero() {
            return true;
        }
        let argv: Vec<String> = ["--no-pager", "-c", "core.fsmonitor=", "-c", "core.hooksPath=/dev/null", "-C"].map(String::from).into_iter()
            .chain([repo.to_string_lossy().into_owned()])
            .chain(["config", "--show-scope", "--get-regexp", r"^filter\..*\.(clean|smudge|process)$"].map(String::from))
            .collect();
        match run_process_env(&self.git, &argv, None, timeout, self.home.as_deref(), None, &[]) {
            Ok(o) if o.exit() == Some(1) => false, // no such key
            Ok(o) if o.exit() == Some(0) => o.stdout.lines().any(|l| matches!(l.split('\t').next(), Some("local" | "worktree" | "command") | None)),
            _ => true,
        }
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
        if self.repo_filters(repo, self.timeout) {
            return None;
        }
        let dir = std::env::temp_dir().join(format!("vajra-tree-{}-{}", std::process::id(), start.elapsed().as_nanos() ^ rand_tag()));
        private_dir(&dir)?;
        let idx = dir.join("index");
        let objects = dir.join("objects");
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
                let env: Vec<(&str, &Path)> = match (with_index, alt.as_deref()) {
                    (true, Some(alt)) => vec![("GIT_INDEX_FILE", idx.as_path()), ("GIT_OBJECT_DIRECTORY", objects.as_path()), ("GIT_ALTERNATE_OBJECT_DIRECTORIES", alt)],
                    _ => vec![],
                };
                run_process_env(&self.git, &argv, None, left, self.home.as_deref(), None, &env).ok().filter(|o| o.exit() == Some(0))
            };
            // verify-work excludes its walkthrough dir relative to the RUN's cwd (`<top>/<prefix>data/walkthroughs`), so the
            // exclusions below are prefixed with the cwd's path under the top level (empty at the root).
            let prefix = git(&["rev-parse", "--show-prefix"], false)?.stdout.trim().to_string();
            let root = git(&["rev-parse", "--show-toplevel"], false)?.stdout.trim().to_string();
            *top.borrow_mut() = Path::new(&root).canonicalize().ok().filter(|p| p.is_absolute())?;
            // The repo's object dir as an absolute path (`--git-path` answers relative to the top level for a plain checkout).
            let found = git(&["rev-parse", "--git-path", "objects"], false)?.stdout.trim().to_string();
            let found = if Path::new(&found).is_absolute() { PathBuf::from(found) } else { top.borrow().join(found) };
            *repo_objects.borrow_mut() = Some(found.canonicalize().ok()?);
            private_dir(&objects)?;
            if let Some(head) = git(&["rev-parse", "--verify", "-q", "HEAD"], false) {
                let head = head.stdout.trim().to_string();
                git(&["read-tree", &head], true)?;
            }
            let mut add = vec!["add".to_string(), "-A".into(), "--".into(), ".".into()];
            add.extend(exclude.iter().map(|d| format!(":(exclude){prefix}{d}")));
            let add: Vec<&str> = add.iter().map(String::as_str).collect();
            git(&add, true)?;
            let tree = git(&["write-tree"], true)?.stdout.trim().to_string();
            (tree.len() >= 40).then_some(tree)
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
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

fn scrubbed_env(cmd: &mut Command, home: Option<&Path>) {
    cmd.env_clear();
    cmd.env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin");
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
    let mut cmd = Command::new(program);
    cmd.args(argv).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    scrubbed_env(&mut cmd, home);
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
