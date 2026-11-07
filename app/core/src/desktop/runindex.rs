//! The app's own record of which Claude session ran which campaign, kept in its data dir. Campaign state does not
//! hold the session id, and `claude --resume` needs it, so without this a campaign started earlier could only be
//! continued in a fresh session. One JSON line per launch or resume, appended; the newest line per campaign wins.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::ids;

/// Only the tail of the file is read: a newer line always supersedes an older one.
const READ_CAP: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub campaign: String,
    pub session_id: String,
    pub run_id: String,
    pub cwd: PathBuf,
    /// The per-run Lead chat session (ADR-13). Optional: lines written before it existed, and every launch or resume
    /// line, carry none. A line that does carry one names the same run, so a newer launch line supersedes it.
    pub lead_session_id: Option<String>,
}

/// Append one entry (file mode 0600, created on first use).
pub fn append(path: &Path, e: &Entry) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut line = serde_json::json!({
        "campaign": e.campaign, "session_id": e.session_id, "run_id": e.run_id, "cwd": e.cwd.to_string_lossy(),
    });
    if let Some(l) = &e.lead_session_id {
        line["lead_session_id"] = serde_json::json!(l);
    }
    let mut f = OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
    f.write_all(format!("{line}\n").as_bytes())
}

/// The newest valid entry per campaign. A missing file is empty; garbage lines, a session id that is not a canonical
/// uuid and relative paths are skipped.
pub fn latest(path: &Path) -> HashMap<String, Entry> {
    let mut out = HashMap::new();
    let Ok(mut f) = std::fs::File::open(path) else { return out };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(READ_CAP);
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(start)).is_err() || f.take(READ_CAP).read_to_end(&mut buf).is_err() {
        return out;
    }
    let text = String::from_utf8_lossy(&buf);
    // A cut at the front leaves a partial first line, which fails to parse and is skipped.
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let (Some(campaign), Some(session_id), Some(run_id), Some(cwd)) = (s("campaign"), s("session_id"), s("run_id"), s("cwd")) else {
            continue;
        };
        if !ids::valid_session_id(&session_id) || !Path::new(&cwd).is_absolute() {
            continue;
        }
        // A malformed Lead id reads as absent, never as an error: the line is still a valid launch record.
        let lead_session_id = s("lead_session_id").filter(|l| ids::valid_session_id(l));
        out.insert(campaign.clone(), Entry { campaign, session_id, run_id, cwd: PathBuf::from(cwd), lead_session_id });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const S1: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
    const S2: &str = "1a8fad5b-d9cb-469f-a165-70867728950f";

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vajra-runindex-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("runs.jsonl")
    }

    fn entry(c: &str, s: &str, r: &str) -> Entry {
        Entry { campaign: c.into(), session_id: s.into(), run_id: r.into(), cwd: PathBuf::from("/repo"), lead_session_id: None }
    }

    #[test]
    fn the_newest_entry_per_campaign_wins() {
        let p = tmp("newest");
        append(&p, &entry("a", S1, "20261106-1000")).unwrap();
        append(&p, &entry("b", S2, "20261106-1001")).unwrap();
        append(&p, &entry("a", S1, "20261106-1100")).unwrap();
        let m = latest(&p);
        assert_eq!(m.len(), 2);
        assert_eq!(m["a"].run_id, "20261106-1100");
        assert_eq!(m["b"].session_id, S2);
    }

    #[test]
    fn a_missing_file_is_empty_and_garbage_lines_are_skipped() {
        let p = tmp("garbage");
        assert!(latest(&p).is_empty());
        append(&p, &entry("a", S1, "r1")).unwrap();
        let mut f = OpenOptions::new().append(true).open(&p).unwrap();
        for bad in [
            "not json\n",
            "{\"campaign\":\"x\"}\n",
            "{\"campaign\":\"x\",\"session_id\":\"nope\",\"run_id\":\"r\",\"cwd\":\"/r\"}\n",
            "{\"campaign\":\"x\",\"session_id\":\"0f8fad5b-d9cb-469f-a165-70867728950e\",\"run_id\":\"r\",\"cwd\":\"relative\"}\n",
        ] {
            f.write_all(bad.as_bytes()).unwrap();
        }
        let m = latest(&p);
        assert_eq!(m.keys().collect::<Vec<_>>(), vec!["a"]);
    }

    #[test]
    fn lead_session_id_round_trips_and_old_lines_still_read() {
        let p = tmp("lead");
        // A line written before the field existed.
        append(&p, &entry("old", S1, "r0")).unwrap();
        assert_eq!(latest(&p)["old"].lead_session_id, None);
        assert!(!std::fs::read_to_string(&p).unwrap().contains("lead_session_id"), "absent stays absent on disk");
        // The Lead line carries it, and the newest line wins.
        let mut e = entry("c", S1, "r1");
        e.lead_session_id = Some(S2.into());
        append(&p, &e).unwrap();
        assert_eq!(latest(&p)["c"].lead_session_id.as_deref(), Some(S2));
        assert_eq!(latest(&p)["c"].session_id, S1);
        // A later launch line (no Lead id) supersedes it.
        append(&p, &entry("c", S1, "r2")).unwrap();
        assert_eq!(latest(&p)["c"].lead_session_id, None);
    }

    #[test]
    fn a_malformed_lead_session_id_reads_as_absent() {
        let p = tmp("lead-bad");
        let mut f = OpenOptions::new().create(true).append(true).open(&p).unwrap();
        f.write_all(b"{\"campaign\":\"x\",\"session_id\":\"0f8fad5b-d9cb-469f-a165-70867728950e\",\"run_id\":\"r\",\"cwd\":\"/r\",\"lead_session_id\":\"--last\"}\n").unwrap();
        let m = latest(&p);
        assert_eq!(m["x"].lead_session_id, None);
        assert_eq!(m["x"].run_id, "r");
    }

    #[test]
    fn the_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let p = tmp("mode");
        append(&p, &entry("a", S1, "r1")).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn only_the_tail_is_read_and_a_cut_first_line_is_ignored() {
        let p = tmp("tail");
        append(&p, &entry("old", S1, "r0")).unwrap();
        let pad = "x".repeat(2000);
        let mut f = OpenOptions::new().append(true).open(&p).unwrap();
        for _ in 0..800 {
            writeln!(f, "{{\"junk\":\"{pad}\"}}").unwrap();
        }
        drop(f);
        append(&p, &entry("new", S2, "r1")).unwrap();
        let m = latest(&p);
        assert!(m.contains_key("new") && !m.contains_key("old"), "{:?}", m.keys());
    }
}
