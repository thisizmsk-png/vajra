//! Read-only readers over Vajra's state files. Every open goes through the
//! allowlist.

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::allowlist::Allowlist;

/// How much of `audit.log` the tail reader returns.
pub const AUDIT_TAIL_BYTES: u64 = 256 * 1024;

/// A JSON file read. On a parse error `value` is the last good snapshot
/// (or `None` if there never was one) and `stale` is true.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub value: Option<Value>,
    pub stale: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CampaignSummary {
    pub slug: String,
    pub goal: String,
    pub status: String,
    pub created: String,
    pub updated: String,
    pub passing: usize,
    pub total: usize,
    pub last_integration: Option<Integration>,
    pub checkpoints: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Integration {
    pub branch: String,
    pub sha: String,
    pub from: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceState {
    /// The recorded file exists and the task graph's check said green.
    Verified,
    /// The check said not green.
    Unverified,
    /// The file is gone (worktree cleanup); the recorded hash is all that is left.
    FileGone,
    /// No evidence recorded.
    None,
}

pub struct Reader {
    allow: Allowlist,
    good: HashMap<PathBuf, Value>,
}

/// The most report text sent to the UI.
pub const REPORT_CAP: usize = 256 * 1024;

impl Reader {
    pub fn new(allow: Allowlist) -> Self {
        Self { allow, good: HashMap::new() }
    }

    fn read_string(&self, path: &Path) -> io::Result<String> {
        let mut s = String::new();
        self.allow.open_allowlisted(path)?.read_to_string(&mut s)?;
        Ok(s)
    }

    fn campaign_dir(&self, slug: &str) -> PathBuf {
        self.allow.campaigns_dir().join(slug)
    }

    /// Campaign directories only. Stray files (the root also holds
    /// `<uuid>.json`) and symlinks that leave the allowlist are skipped.
    pub fn campaigns(&self) -> io::Result<Vec<String>> {
        let root = self.allow.resolve(&self.allow.campaigns_dir())?;
        let mut out = Vec::new();
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            let Ok(name) = entry.file_name().into_string() else { continue };
            if name.starts_with('.') {
                continue;
            }
            match self.allow.resolve(&entry.path()) {
                Ok(real) if real.is_dir() => out.push(name),
                _ => {}
            }
        }
        out.sort();
        Ok(out)
    }

    /// Read and parse a JSON file, keeping the last good snapshot per path.
    pub fn json(&mut self, path: &Path) -> Snapshot {
        let parsed = self
            .read_string(path)
            .map_err(|_| ())
            .and_then(|s| serde_json::from_str::<Value>(&s).map_err(|_| ()));
        let key = path.to_path_buf();
        match parsed {
            Ok(v) => {
                self.good.insert(key, v.clone());
                Snapshot { value: Some(v), stale: false }
            }
            Err(()) => Snapshot { value: self.good.get(&key).cloned(), stale: true },
        }
    }

    pub fn campaign_json(&mut self, slug: &str) -> Snapshot {
        self.json(&self.campaign_dir(slug).join("campaign.json"))
    }

    pub fn features(&mut self, slug: &str) -> Snapshot {
        self.json(&self.campaign_dir(slug).join("features.json"))
    }

    pub fn tasks(&mut self, slug: &str) -> Snapshot {
        self.json(&self.campaign_dir(slug).join("tasks.json"))
    }

    /// `report.md` (E4), at most `REPORT_CAP` bytes; the flag says it was cut. The final component is never
    /// followed, so a symlinked report is refused like any other allowlist escape.
    pub fn report(&self, slug: &str) -> io::Result<(String, bool)> {
        let mut buf = Vec::new();
        self.allow
            .open_allowlisted(&self.campaign_dir(slug).join("report.md"))?
            .take(REPORT_CAP as u64 + 1)
            .read_to_end(&mut buf)?;
        let cut = buf.len() > REPORT_CAP;
        buf.truncate(REPORT_CAP);
        Ok((String::from_utf8_lossy(&buf).into_owned(), cut))
    }

    pub fn progress(&self, slug: &str) -> io::Result<String> {
        self.read_string(&self.campaign_dir(slug).join("progress.md"))
    }

    /// Names in `checkpoints/`, sorted. A missing dir is an empty list.
    pub fn checkpoints(&self, slug: &str) -> io::Result<Vec<String>> {
        let dir = self.campaign_dir(slug).join("checkpoints");
        let real = match self.allow.resolve(&dir) {
            Ok(r) => r,
            Err(super::allowlist::Denied::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(e) => return Err(e.into()),
        };
        let mut names = Vec::new();
        for entry in std::fs::read_dir(real)? {
            if let Ok(n) = entry?.file_name().into_string() {
                names.push(n);
            }
        }
        names.sort();
        Ok(names)
    }

    pub fn campaign(&mut self, slug: &str) -> io::Result<CampaignSummary> {
        let c = self.campaign_json(slug).value.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "campaign.json unreadable")
        })?;
        let feats = self.features(slug).value;
        let list = feats
            .as_ref()
            .and_then(|f| f.get("features"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let passing = list.iter().filter(|f| f["passes"] == Value::Bool(true)).count();
        let text = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let progress = self.progress(slug).unwrap_or_default();
        Ok(CampaignSummary {
            slug: text("slug"),
            goal: text("goal"),
            status: text("status"),
            created: text("created"),
            updated: text("updated"),
            passing,
            total: list.len(),
            last_integration: last_integration(&progress),
            checkpoints: self.checkpoints(slug)?,
        })
    }

    /// The last 256 KiB of `audit.log`, cut to whole lines.
    pub fn audit_tail(&self) -> io::Result<String> {
        let mut f = self.allow.open_allowlisted(&self.allow.vajra_dir().join("audit.log"))?;
        let len = f.metadata()?.len();
        let start = len.saturating_sub(AUDIT_TAIL_BYTES);
        f.seek(SeekFrom::Start(start))?;
        let mut buf = Vec::new();
        f.take(AUDIT_TAIL_BYTES).read_to_end(&mut buf)?;
        let text = String::from_utf8_lossy(&buf).into_owned();
        if start > 0 {
            // The first line is probably cut in half.
            return Ok(text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default());
        }
        Ok(text)
    }
}

/// The last `integration <branch> @ <sha> from <sha>` line of progress.md.
pub fn last_integration(progress: &str) -> Option<Integration> {
    progress.lines().rev().find_map(|line| {
        let rest = line.split_once("integration ")?.1;
        let (branch, rest) = rest.split_once(" @ ")?;
        let (sha, from) = rest.split_once(" from ")?;
        let ok = |s: &str| !s.is_empty() && !s.contains(char::is_whitespace);
        let (sha, from) = (sha.trim(), from.trim());
        (ok(branch) && ok(sha) && ok(from)).then(|| Integration {
            branch: branch.to_string(),
            sha: sha.to_string(),
            from: from.to_string(),
        })
    })
}

/// Derive the evidence state of a task. `green` is the result of
/// `vajra_tasks.py check-evidence` when it has been run. Only the path's
/// existence is tested; the file is never opened here.
pub fn evidence_state(evidence: Option<&Value>, green: Option<bool>) -> EvidenceState {
    let Some(ev) = evidence.filter(|e| e.is_object()) else {
        return EvidenceState::None;
    };
    let gone = ev
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|p| std::fs::symlink_metadata(p).is_err());
    if gone {
        EvidenceState::FileGone
    } else if green == Some(true) {
        EvidenceState::Verified
    } else {
        EvidenceState::Unverified
    }
}
