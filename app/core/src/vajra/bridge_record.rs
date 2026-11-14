//! The run record's bridge requests (ADR-16 16.5a, PR 6h): the campaign and task-graph calls a core run makes. Each one
//! is a typed variant with a fixed argv that matches the script's own parser. Every value is checked against the
//! script's regex and cap here, so a value the script would clean, cut or read as an option is refused and nothing is
//! spawned. The record cleans the lead's text first (`record::one_line`). The app never opens the HMAC key and never
//! writes under `~/.claude/vajra`: the scripts do (this module and `desktop/record.rs` use no `std::fs`).
//!
//! Campaign calls run in `Bridge::record_cwd`, the core's own empty git dir. `vajra-campaign.sh` keys its active pointer
//! by the cwd's repository and `create` sets it (S8), so a core run moves only a pointer no user session reads.

use std::path::{Path, PathBuf};

use super::bridge::{Bridge, BridgeError, Request};

/// `vajra-campaign.sh close --status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseStatus {
    Done,
    Abandoned,
}

/// `vajra_tasks.py report <id> <status>`: the script's `REPORT_MAP` keys the record uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    InProgress,
    Done,
    Progress,
    Failed,
    Question,
    Blocked,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Done => "done",
            TaskStatus::Progress => "progress",
            TaskStatus::Failed => "failed",
            TaskStatus::Question => "question",
            TaskStatus::Blocked => "blocked",
        }
    }
}

/// One `--feature id|description` of `create`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feature {
    pub id: String,
    pub desc: String,
}

/// The script's caps (`clean_line` and `one_line` lengths). Bytes, so never more characters than the script keeps.
pub const GOAL_MAX: usize = 1000;
pub const FEATURE_MAX: usize = 500;
pub const EVIDENCE_MAX: usize = 300;
pub const LABEL_MAX: usize = 200;
pub const TITLE_MAX: usize = 200;
pub const SUMMARY_MAX: usize = 500;
pub const PROGRESS_MAX: usize = 300;
pub const PATH_MAX: usize = 300;
pub const REPORT_MAX: usize = 65536;
const MAX_FEATURES: usize = 64;
const MAX_FILES: usize = 64;
const MAX_DEPS: usize = 64;
const MAX_CHANGED: usize = 200;
/// `archive-evidence --setup`: at most this many commands, each at most this many characters.
pub const MAX_SETUP: usize = 5;
pub const SETUP_MAX: usize = 1000;

fn bad(what: &str, v: &str) -> BridgeError {
    let shown: String = v.chars().take(40).filter(|c| !c.is_control()).collect();
    BridgeError::Invalid(format!("bad {what}: {shown:?}"))
}

/// `^[a-z0-9][a-z0-9-]{0,63}$`: `sanitize_slug` keeps it as it is, and `vajra_tasks.py`'s `SLUG_RE` takes it.
pub fn slug(s: &str) -> Result<String, BridgeError> {
    agent(s).map_err(|_| bad("campaign slug", s))
}

/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`: a feature id (`valid_feature_id`) and a task id (`ID_RE`), whole.
pub fn ident(s: &str) -> Result<String, BridgeError> {
    let ok = s.len() <= 64
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if ok { Ok(s.to_string()) } else { Err(bad("id", s)) }
}

/// `^[a-z0-9][a-z0-9-]{0,63}$`: `AGENT_RE`.
pub fn agent(s: &str) -> Result<String, BridgeError> {
    let ok = s.len() <= 64
        && s.bytes().next().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok { Ok(s.to_string()) } else { Err(bad("agent", s)) }
}

/// `^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$`: `BRANCH_RE`.
fn branch(s: &str) -> Result<String, BridgeError> {
    let ok = s.len() <= 200
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'));
    if ok { Ok(s.to_string()) } else { Err(bad("branch", s)) }
}

/// `SAFE_PATH` with no `..` part: repo-relative, never `/`, `~` or `-` first, and no `,` (the list separator).
pub fn file(s: &str) -> Result<String, BridgeError> {
    let ok = !s.is_empty()
        && s.len() <= PATH_MAX
        && !s.starts_with(['/', '~', '-'])
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'@' | b'+' | b'-' | b'/' | b'*'))
        && !s.split('/').any(|p| p == "..");
    if ok { Ok(s.to_string()) } else { Err(bad("path", s)) }
}

/// `SHA_RE` (`^[0-9a-f]{7,40}$`), or exactly 40 when `full`.
fn sha(s: &str, full: bool) -> Result<String, BridgeError> {
    let ok = (if full { s.len() == 40 } else { (7..=40).contains(&s.len()) }) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if ok { Ok(s.to_string()) } else { Err(bad("commit", s)) }
}

/// Bidi embedding, override and isolate controls: refused in signed text a card shows.
fn bidi(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// One line the script keeps as it is: within `cap` bytes, no control character (C1 too), no bidi control, no leading
/// `-` (an option to bash's positional parsers and to argparse), and no leading, trailing, doubled or non-space
/// whitespace (`one_line` would change it). `required`: not empty.
pub fn text(s: &str, cap: usize, required: bool) -> Result<String, BridgeError> {
    let ok = s.len() <= cap
        && !s.chars().any(|c| c.is_control() || bidi(c) || (c.is_whitespace() && c != ' '))
        && !s.starts_with('-')
        && s.trim() == s
        && !s.contains("  ")
        && (!required || !s.is_empty());
    if ok { Ok(s.to_string()) } else { Err(bad("text", s)) }
}

/// The record's cleaner for the lead's text: whitespace runs and control or bidi characters become one space, a
/// leading `-` is dropped, and the result is cut to `cap` bytes on a character boundary. `text` accepts what it returns.
pub fn one_line(s: &str, cap: usize) -> String {
    let spaced: String = s.chars().map(|c| if c.is_control() || bidi(c) || c.is_whitespace() { ' ' } else { c }).collect();
    let words = spaced.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
    let mut out = words.trim_start_matches(['-', ' ']).to_string();
    if out.len() > cap {
        let mut end = cap;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out.trim_end().to_string()
}

/// An absolute path of at most `PATH_MAX` bytes, one line (`valid_worktree`, and the evidence path).
fn abs(p: &Path) -> Result<String, BridgeError> {
    let s = p.to_string_lossy();
    if p.is_absolute() && s.len() <= PATH_MAX && !s.chars().any(|c| c.is_control() || bidi(c)) && p.to_str().is_some() {
        Ok(s.into_owned())
    } else {
        Err(bad("path", &s))
    }
}

/// FNV-1a 32 of `s`, as the workflow's `fnv()` and the script's `fnv32` (a copy check, not a security property).
pub fn fnv32(s: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

fn base64ish(s: &str) -> bool {
    let body = s.trim_end_matches('=');
    !body.is_empty() && s.len() - body.len() <= 2 && body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

fn list(items: &[String], max: usize, each: impl Fn(&str) -> Result<String, BridgeError>) -> Result<String, BridgeError> {
    if items.len() > max {
        return Err(BridgeError::Invalid(format!("at most {max} items")));
    }
    Ok(items.iter().map(|i| each(i)).collect::<Result<Vec<_>, _>>()?.join(","))
}

fn s(v: &str) -> String {
    v.to_string()
}

type Planned = (PathBuf, Vec<String>, Option<PathBuf>);

impl Bridge {
    /// `(program, argv, cwd)` for a record request, or `None` when `req` isn't one.
    pub(crate) fn plan_record(&self, req: &Request) -> Option<Result<Planned, BridgeError>> {
        if !req.is_record() {
            return None;
        }
        Some(self.plan_record_inner(req))
    }

    fn plan_record_inner(&self, req: &Request) -> Result<Planned, BridgeError> {
        let campaign = |argv: Vec<String>| -> Result<Planned, BridgeError> {
            let cwd = self.record_cwd.clone().ok_or_else(|| BridgeError::Invalid("the run record has no working directory".into()))?;
            Ok((self.scripts_dir.join("vajra-campaign.sh"), argv, Some(cwd)))
        };
        // `-I`: no user site-packages or `.pth` file runs inside the script that reads the key.
        let tasks = |head: Vec<String>, slug_: &str, cwd: PathBuf| -> Result<Planned, BridgeError> {
            let mut argv = vec![s("-I"), self.scripts_dir.join("vajra_tasks.py").to_string_lossy().into_owned()];
            argv.extend(head);
            argv.extend([s("--campaign"), slug(slug_)?]);
            Ok((self.python.clone(), argv, Some(cwd)))
        };
        let root = || PathBuf::from("/");
        match req {
            Request::CampaignCreate { slug: sl, goal, features } => {
                if features.is_empty() || features.len() > MAX_FEATURES {
                    return Err(BridgeError::Invalid(format!("a campaign needs 1 to {MAX_FEATURES} features")));
                }
                let mut argv = vec![s("create"), slug(sl)?, s("--goal"), text(goal, GOAL_MAX, true)?];
                for f in features {
                    argv.extend([s("--feature"), format!("{}|{}", ident(&f.id)?, text(&f.desc, FEATURE_MAX, false)?)]);
                }
                campaign(argv)
            }
            Request::CampaignShow { slug: sl } => campaign(vec![s("show"), slug(sl)?, s("--json")]),
            Request::CampaignList => campaign(vec![s("list")]),
            Request::ProcessList => Ok((PathBuf::from("/bin/ps"), vec![s("-A"), s("-o"), s("command=")], Some(root()))),
            Request::CampaignPass { slug: sl, feature, evidence } => {
                campaign(vec![s("pass"), ident(feature)?, s("--evidence"), text(evidence, EVIDENCE_MAX, true)?, s("--campaign"), slug(sl)?])
            }
            Request::CampaignCheckpoint { slug: sl, label } => campaign(vec![s("checkpoint"), text(label, LABEL_MAX, true)?, s("--campaign"), slug(sl)?]),
            Request::CampaignClose { slug: sl, status } => {
                let st = if *status == CloseStatus::Done { "done" } else { "abandoned" };
                campaign(vec![s("close"), slug(sl)?, s("--status"), s(st)])
            }
            Request::CampaignReport { slug: sl, bytes, check, base64 } => {
                if !(1..=REPORT_MAX).contains(bytes) {
                    return Err(BridgeError::Invalid(format!("a report is 1 to {REPORT_MAX} bytes")));
                }
                if !base64ish(base64) || base64.len() > (REPORT_MAX / 3 + 2) * 4 {
                    return Err(BridgeError::Invalid("the report is not base64, or too long".into()));
                }
                if fnv32(base64) != *check {
                    return Err(BridgeError::Invalid("the report's check is not the FNV-1a 32 of its base64".into()));
                }
                campaign(vec![s("report"), s("--campaign"), slug(sl)?, s("--bytes"), bytes.to_string(), s("--check"), check.clone(), s("--base64"), base64.clone()])
            }
            Request::CampaignUnlockStale { slug: sl } => campaign(vec![s("unlock"), slug(sl)?, s("--if-stale")]),
            Request::CampaignArchiveEvidence { slug: sl, dir, head, setup } => {
                // `vajra_tasks.py`'s own limits: at most 5 setup commands, each 1 to 1000 characters.
                if setup.len() > MAX_SETUP || setup.iter().any(|c| c.is_empty() || c.chars().count() > SETUP_MAX || c.chars().any(char::is_control)) {
                    return Err(BridgeError::Invalid(format!("an archive takes at most {MAX_SETUP} setup commands, each 1 to {SETUP_MAX} characters on one line")));
                }
                let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if name != sl.as_str() {
                    return Err(BridgeError::Invalid("the evidence folder must be named for the campaign".into()));
                }
                let setup = serde_json::to_string(setup).map_err(|_| BridgeError::Invalid("the setup commands can't be encoded".into()))?;
                campaign(vec![s("archive-evidence"), slug(sl)?, abs(dir)?, sha(head, true)?, setup])
            }
            Request::TasksAdd { slug: sl, id, title, owner, deps, files, acceptance, branch: br } => {
                let mut argv = vec![s("add"), ident(id)?, s("--title"), text(title, TITLE_MAX, true)?, s("--owner"), agent(owner)?];
                if !deps.is_empty() {
                    argv.extend([s("--deps"), list(deps, MAX_DEPS, ident)?]);
                }
                if files.is_empty() {
                    return Err(BridgeError::Invalid("a task needs its files".into()));
                }
                argv.extend([s("--files"), list(files, MAX_FILES, file)?]);
                for c in acceptance {
                    argv.extend([s("--acceptance"), ident(c)?]);
                }
                argv.extend([s("--branch"), branch(br)?]);
                tasks(argv, sl, root())
            }
            Request::TasksDep { slug: sl, id, dep, remove } => {
                let mut argv = vec![s("dep"), ident(id)?, ident(dep)?];
                if *remove {
                    argv.push(s("--remove"));
                }
                tasks(argv, sl, root())
            }
            Request::TasksValidate { slug: sl } => tasks(vec![s("validate")], sl, root()),
            Request::TasksList { slug: sl } => {
                let (p, mut argv, cwd) = tasks(vec![s("list")], sl, root())?;
                argv.push(s("--json"));
                Ok((p, argv, cwd))
            }
            Request::TasksClaim { slug: sl, id, worker, worktree } => {
                // The workflow's order: `claim <id> <owner> --stacked --campaign <slug> --worktree <dir>`.
                let (p, mut argv, cwd) = tasks(vec![s("claim"), ident(id)?, agent(worker)?, s("--stacked")], sl, root())?;
                if let Some(w) = worktree.as_deref() {
                    argv.extend([s("--worktree"), abs(w)?]);
                }
                Ok((p, argv, cwd))
            }
            Request::TasksReport { slug: sl, id, status, summary } => {
                let (p, mut argv, cwd) = tasks(vec![s("report"), ident(id)?, s(status.as_str())], sl, root())?;
                if let Some(m) = summary.as_deref() {
                    argv.extend([s("--summary"), text(m, SUMMARY_MAX, false)?]);
                }
                Ok((p, argv, cwd))
            }
            Request::TasksClose { slug: sl, id, evidence, head, commit, branch: br, base_commit, changed, repo } => {
                if !repo.is_absolute() {
                    return Err(BridgeError::Invalid("the repository path must be absolute".into()));
                }
                let changed = list(changed, MAX_CHANGED, |p| text(p, PATH_MAX, true).and_then(|p| if p.contains(',') { Err(bad("path", &p)) } else { Ok(p) }))?;
                let argv = vec![
                    s("close"),
                    ident(id)?,
                    s("--evidence"),
                    abs(evidence)?,
                    s("--head"),
                    sha(head, true)?,
                    s("--commit"),
                    sha(commit, false)?,
                    s("--branch"),
                    branch(br)?,
                    s("--base-commit"),
                    sha(base_commit, false)?,
                    s("--changed"),
                    changed,
                    // The ledger reads `result` (`closed` or `already done`) from the JSON answer (bhishma, #190).
                    s("--json"),
                ];
                tasks(argv, sl, repo.clone())
            }
            _ => unreachable!("is_record lists every variant above"),
        }
    }
}

impl Request {
    /// A run record request (this file's variants).
    pub fn is_record(&self) -> bool {
        matches!(
            self,
            Request::CampaignCreate { .. }
                | Request::CampaignShow { .. }
                | Request::CampaignList
                | Request::ProcessList
                | Request::CampaignPass { .. }
                | Request::CampaignCheckpoint { .. }
                | Request::CampaignClose { .. }
                | Request::CampaignReport { .. }
                | Request::CampaignUnlockStale { .. }
                | Request::CampaignArchiveEvidence { .. }
                | Request::TasksAdd { .. }
                | Request::TasksDep { .. }
                | Request::TasksValidate { .. }
                | Request::TasksList { .. }
                | Request::TasksClaim { .. }
                | Request::TasksReport { .. }
                | Request::TasksClose { .. }
        )
    }

    /// A write to a campaign or its task graph: on its timeout it gets SIGTERM and `KILL_GRACE` before SIGKILL, so the
    /// script's EXIT trap can free its lock (16.5a "Kills and locks"). `CampaignProgress` is a write too.
    pub fn is_campaign_write(&self) -> bool {
        (self.is_record() && !matches!(self, Request::CampaignShow { .. } | Request::CampaignList | Request::ProcessList | Request::TasksValidate { .. } | Request::TasksList { .. }))
            || matches!(self, Request::CampaignProgress { .. })
    }
}
