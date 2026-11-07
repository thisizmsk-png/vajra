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

#[derive(Clone, Debug)]
pub struct Bridge {
    /// Dir holding `vajra-campaign.sh`, `vajra_tasks.py`, `vajra_kb.py`, `vajra-doctor.sh`.
    pub scripts_dir: PathBuf,
    pub python: PathBuf,
    pub git: PathBuf,
    pub timeout: Duration,
    /// The HOME the scripts see. `None` passes the process HOME through; the app sets it to the home it was opened with.
    pub home: Option<PathBuf>,
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
        }
    }

    pub fn with_home(mut self, home: PathBuf) -> Self {
        self.home = Some(home);
        self
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
                find_jq().ok_or_else(|| BridgeError::Invalid("jq not found".into()))?,
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
                let mut argv = vec![
                    script("vajra_tasks.py").to_string_lossy().into_owned(),
                    "check-evidence".into(),
                    path.to_string_lossy().into_owned(),
                ];
                if let Some(h) = head {
                    if !(7..=40).contains(&h.len()) || !h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
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

fn find_jq() -> Option<PathBuf> {
    ["/usr/bin/jq", "/opt/homebrew/bin/jq", "/usr/local/bin/jq", "/bin/jq"].iter().map(PathBuf::from).find(|p| p.is_file())
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
    let mut cmd = Command::new(program);
    cmd.args(argv).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    scrubbed_env(&mut cmd, home);
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
