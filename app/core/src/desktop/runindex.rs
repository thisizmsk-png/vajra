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
    /// Legacy field, read for compatibility and **not trusted**: this file is writable by a worker. The app keeps the
    /// run to Lead-session mapping in `<data>/control/lead/<run_id>/lead_session`, under the guarded control dir.
    pub lead_session_id: Option<String>,
    /// The adapter that ran it (`claude` or `codex`); absent in older lines, which were all `claude`. Like the rest of
    /// this file it is a hint, not trusted: it only picks which of the two known CLIs a resume offers by default.
    pub provider: Option<String>,
}

/// Serialises every read-modify-append of the index within the process, so a late metadata write cannot interleave
/// with a launch or resume.
static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Append one entry (file mode 0600, created on first use).
pub fn append(path: &Path, e: &Entry) -> io::Result<()> {
    let _g = WRITE.lock().unwrap_or_else(|p| p.into_inner());
    append_locked(path, e)
}

fn append_locked(path: &Path, e: &Entry) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut line = serde_json::json!({
        "campaign": e.campaign, "session_id": e.session_id, "run_id": e.run_id, "cwd": e.cwd.to_string_lossy(),
    });
    if let Some(l) = &e.lead_session_id {
        line["lead_session_id"] = serde_json::json!(l);
    }
    if let Some(p) = &e.provider {
        line["provider"] = serde_json::json!(p);
    }
    let mut f = OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
    f.write_all(format!("{line}\n").as_bytes())
}

/// The newest valid entry per campaign. A missing file is empty; garbage lines, a session id that is not a canonical
/// uuid and relative paths are skipped.
pub fn latest(path: &Path) -> HashMap<String, Entry> {
    entries(path).into_iter().map(|e| (e.campaign.clone(), e)).collect()
}

/// Every run id the index (its tail) has ever recorded, for any campaign: an id that was issued before a restart is
/// still taken after it.
pub fn run_ids(path: &Path) -> std::collections::HashSet<String> {
    entries(path).into_iter().map(|e| e.run_id).collect()
}

/// Every valid entry in the tail of the file, oldest first.
pub fn entries(path: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let Ok(mut f) = std::fs::File::open(path) else { return out };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(READ_CAP);
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(start)).is_err() || f.take(READ_CAP).read_to_end(&mut buf).is_err() {
        return out;
    }
    let text = String::from_utf8_lossy(&buf);
    // A cut at the front leaves a partial first line, which fails to parse and is skipped.
    out.extend(text.lines().filter_map(parse_line));
    out
}

/// One index line as an entry: all four fields present, a canonical session id, an absolute cwd; the Lead id only when
/// it is a canonical uuid.
fn parse_line(line: &str) -> Option<Entry> {
    let v = serde_json::from_str::<serde_json::Value>(line).ok()?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let (campaign, session_id, run_id, cwd) = (s("campaign")?, s("session_id")?, s("run_id")?, s("cwd")?);
    if !ids::valid_session_id(&session_id) || !Path::new(&cwd).is_absolute() {
        return None;
    }
    // A malformed Lead id reads as absent, never as an error: the line is still a valid launch record.
    let lead_session_id = s("lead_session_id").filter(|l| ids::valid_session_id(l));
    let provider = s("provider").filter(|p| crate::adapter::Engine::from_id(p).is_some());
    Some(Entry { campaign, session_id, run_id, cwd: PathBuf::from(cwd), lead_session_id, provider })
}

/// The longest single line the full reader looks at (a launch line is a few hundred bytes).
const LINE_CAP: usize = 64 * 1024;

/// Every valid entry in the WHOLE file, oldest first, for the one-time migration, which must not miss older launches.
/// Memory is bounded by streaming one line at a time (a line over `LINE_CAP` is skipped, not buffered) and the file is
/// opened without following a symlink or waiting on a FIFO. A file larger than `max_bytes` is an `InvalidData` error:
/// the caller refuses rather than act on input it could not read completely. A missing file is empty.
pub fn entries_all(path: &Path, max_bytes: u64) -> io::Result<Vec<Entry>> {
    use std::io::BufRead;
    use std::os::unix::fs::OpenOptionsExt;
    let f = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let meta = f.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
    }
    if meta.len() > max_bytes {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "launch index too large to read completely"));
    }
    let mut out = Vec::new();
    let mut r = io::BufReader::new(f.take(max_bytes + 1));
    let mut line = Vec::new();
    loop {
        line.clear();
        // read_until would buffer an unbounded line; cap each line by reading through `take`.
        let n = (&mut r).take(LINE_CAP as u64 + 1).read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if line.len() > LINE_CAP && line.last() != Some(&b'\n') {
            // Over-long line: drop the rest of it without keeping it.
            loop {
                let buf = r.fill_buf()?;
                if buf.is_empty() {
                    break;
                }
                match buf.iter().position(|b| *b == b'\n') {
                    Some(i) => {
                        r.consume(i + 1);
                        break;
                    }
                    None => {
                        let n = buf.len();
                        r.consume(n);
                    }
                }
            }
            continue;
        }
        if let Some(e) = parse_line(&String::from_utf8_lossy(&line)) {
            out.push(e);
        }
    }
    Ok(out)
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
        Entry { campaign: c.into(), session_id: s.into(), run_id: r.into(), cwd: PathBuf::from("/repo"), lead_session_id: None, provider: None }
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
    fn the_full_reader_sees_launches_older_than_the_tail_cap_and_skips_long_junk_lines() {
        let p = tmp("full");
        append(&p, &entry("old", S1, "r0")).unwrap();
        let pad = "x".repeat(2000);
        let mut f = OpenOptions::new().append(true).open(&p).unwrap();
        for _ in 0..800 {
            writeln!(f, "{{\"junk\":\"{pad}\"}}").unwrap();
        }
        writeln!(f, "{}", "y".repeat(LINE_CAP * 3)).unwrap(); // one over-long line
        drop(f);
        append(&p, &entry("new", S2, "r1")).unwrap();
        assert!(std::fs::metadata(&p).unwrap().len() > READ_CAP, "the index is bigger than the tail the normal reader sees");
        assert!(!latest(&p).contains_key("old"), "the tail reader misses it");
        let all = entries_all(&p, 64 * 1024 * 1024).unwrap();
        assert_eq!(all.iter().map(|e| e.campaign.as_str()).collect::<Vec<_>>(), vec!["old", "new"]);
        assert_eq!(entries_all(&p, 1024).unwrap_err().kind(), io::ErrorKind::InvalidData, "too large is refused, not truncated");
        assert!(entries_all(&p.with_file_name("none.jsonl"), 1024).unwrap().is_empty());
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
