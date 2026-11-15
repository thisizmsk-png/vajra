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
    good: HashMap<PathBuf, Good>,
    /// How many times `json` parsed a file (a cache hit does not count). Read by the tests.
    parses: u64,
}

/// The last good parse of a path, and which file it was parsed from.
struct Good {
    value: Value,
    /// `None`: the file was too fresh to be told apart from a later rewrite, so it is read again every time.
    sig: Option<Sig>,
}

/// Which file an open descriptor is and when it last changed. The change time moves on every write, rename, link
/// and `utimes`, and a writer cannot set it, so an equal signature on a settled file means the same bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Sig {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

/// A file changed less than this long ago is never answered from the cache: a second write inside one clock tick
/// could leave the same size and the same times.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(1);

impl Sig {
    /// The signature of a file that has not changed for `SETTLE`, by its own change time against the wall clock.
    fn settled(md: &std::fs::Metadata) -> Option<Self> {
        use std::os::unix::fs::MetadataExt;
        let changed = std::time::Duration::new(u64::try_from(md.ctime()).ok()?, u32::try_from(md.ctime_nsec()).ok()?);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?;
        (now.checked_sub(changed)? >= SETTLE).then(|| Self {
            dev: md.dev(),
            ino: md.ino(),
            len: md.size(),
            mtime: (md.mtime(), md.mtime_nsec()),
            ctime: (md.ctime(), md.ctime_nsec()),
        })
    }
}

impl Reader {
    pub fn new(allow: Allowlist) -> Self {
        Self { allow, good: HashMap::new(), parses: 0 }
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
    ///
    /// A machine that has never run a campaign has no campaigns folder yet. That is an empty list, not a failure:
    /// nothing is read. Only a path that is truly absent counts; a link that is present but leads nowhere, or
    /// outside the allowlist, is still refused below.
    pub fn campaigns(&self) -> io::Result<Vec<String>> {
        let dir = self.allow.campaigns_dir();
        if matches!(std::fs::symlink_metadata(&dir), Err(e) if e.kind() == io::ErrorKind::NotFound) {
            return Ok(Vec::new());
        }
        let root = self.allow.resolve(&dir)?;
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
    ///
    /// Every call opens the file through the allowlist, so every check runs every time. Only the read and the parse
    /// are skipped, and only when the open descriptor is the settled file the last good snapshot was parsed from.
    pub fn json(&mut self, path: &Path) -> Snapshot {
        let key = path.to_path_buf();
        let Ok(mut file) = self.allow.open_allowlisted(path) else {
            return Snapshot { value: self.good.get(&key).map(|g| g.value.clone()), stale: true };
        };
        // Taken before the read: a write that lands during the read moves the change time past this signature.
        let sig = file.metadata().ok().and_then(|md| Sig::settled(&md));
        if let Some(g) = self.good.get(&key)
            && sig.is_some()
            && g.sig == sig
        {
            return Snapshot { value: Some(g.value.clone()), stale: false };
        }
        let mut s = String::new();
        self.parses += 1;
        match file.read_to_string(&mut s).ok().and_then(|_| serde_json::from_str::<Value>(&s).ok()) {
            Some(v) => {
                self.good.insert(key, Good { value: v.clone(), sig });
                Snapshot { value: Some(v), stale: false }
            }
            None => Snapshot { value: self.good.get(&key).map(|g| g.value.clone()), stale: true },
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A temp folder that is removed when the test ends.
    struct Tmp(PathBuf);
    impl Tmp {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn reader() -> (Tmp, Reader, PathBuf) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let tmp = Tmp(std::env::temp_dir().join(format!("vajra-reader-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst))));
        let (claude, data) = (tmp.path().join(".claude"), tmp.path().join("data"));
        let camp = claude.join("vajra/campaigns/c1");
        std::fs::create_dir_all(&camp).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let r = Reader::new(Allowlist::new(&claude, &data).unwrap());
        let file = camp.canonicalize().unwrap().join("tasks.json");
        (tmp, r, file)
    }

    fn settle() {
        std::thread::sleep(SETTLE + std::time::Duration::from_millis(100));
    }

    #[test]
    fn a_settled_file_is_parsed_once_and_a_rewrite_of_the_same_size_is_seen() {
        let (_tmp, mut r, file) = reader();
        std::fs::write(&file, r#"{"tasks":[{"id":"t1","status":"aaaa"}]}"#).unwrap();
        settle();
        for _ in 0..5 {
            let s = r.tasks("c1");
            assert!(!s.stale);
            assert_eq!(s.value.unwrap()["tasks"][0]["status"], "aaaa");
        }
        assert_eq!(r.parses, 1, "an unchanged file must not be read and parsed again");
        // Rewritten in place: same inode, same length, and the old modification time put back.
        let before = std::fs::metadata(&file).unwrap().modified().unwrap();
        let mut f = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
        f.write_all(br#"{"tasks":[{"id":"t1","status":"bbbb"}]}"#).unwrap();
        f.set_modified(before).unwrap();
        drop(f);
        // Wait out the settle window first. Read inside it, the window alone forces the read and the signature is
        // never compared. After it, only the change time tells this file from the cached one: same inode, same
        // length, same modification time.
        settle();
        let parsed = r.parses;
        assert_eq!(r.tasks("c1").value.unwrap()["tasks"][0]["status"], "bbbb");
        assert_eq!(r.parses, parsed + 1, "a same-size in-place rewrite with the old mtime must be seen by its change time");
        // Settled again and cached.
        r.tasks("c1");
        assert_eq!(r.parses, parsed + 1);
        // Replaced by rename (how the engine writes): a new inode. Same length, and the old mtime put back, so the
        // inode and the change time are all that differ.
        let before = std::fs::metadata(&file).unwrap().modified().unwrap();
        let next = file.with_file_name("tasks.json.tmp");
        std::fs::write(&next, r#"{"tasks":[{"id":"t1","status":"cccc"}]}"#).unwrap();
        std::fs::OpenOptions::new().write(true).open(&next).unwrap().set_modified(before).unwrap();
        std::fs::rename(&next, &file).unwrap();
        settle();
        let parsed = r.parses;
        assert_eq!(r.tasks("c1").value.unwrap()["tasks"][0]["status"], "cccc");
        assert_eq!(r.parses, parsed + 1, "a file replaced by rename must be read again once it has settled");
    }

    #[test]
    fn a_fresh_file_is_read_every_time() {
        let (_tmp, mut r, file) = reader();
        std::fs::write(&file, r#"{"tasks":[]}"#).unwrap();
        r.tasks("c1");
        r.tasks("c1");
        assert_eq!(r.parses, 2, "a file changed inside the settle window is never answered from the cache");
    }

    #[test]
    fn a_cached_file_that_is_no_longer_allowed_is_stale_and_a_broken_one_keeps_the_last_good() {
        let (tmp, mut r, file) = reader();
        std::fs::write(&file, r#"{"tasks":[1]}"#).unwrap();
        settle();
        assert!(!r.tasks("c1").stale);
        assert!(!r.tasks("c1").stale);
        assert_eq!(r.parses, 1);
        // A second hard link: the open is refused, so the answer is stale even though the bytes did not change.
        let link = tmp.path().join("data/link");
        std::fs::hard_link(&file, &link).unwrap();
        assert!(r.tasks("c1").stale);
        std::fs::remove_file(&link).unwrap();
        std::fs::write(&file, "{ not json").unwrap();
        let s = r.tasks("c1");
        assert!(s.stale);
        assert_eq!(s.value.unwrap()["tasks"][0], 1);
    }
}
