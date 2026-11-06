//! Lines the app itself adds to Vajra's audit.log (only `resume`; deliveries
//! and stop blocks are written by the hook). The caller passes the audit.log
//! path explicitly: C6 never derives it from $HOME.

use std::io::{self, Write};
use std::path::Path;
use std::time::SystemTime;

use super::{fsx, time};

/// Append `TS VERDICT desktop-control reason="REASON"`, the hook's format.
/// The reason keeps printable ASCII only and loses double quotes, like the
/// hook's `audit()`, so it cannot break the line format.
pub fn append_line(audit_log: &Path, verdict: &str, reason: &str, now: SystemTime) -> io::Result<()> {
    let r: String = reason.chars().filter(|c| (' '..='~').contains(c) && *c != '"').collect();
    append_raw(audit_log, &format!("{} {verdict} desktop-control reason=\"{r}\"\n", time::iso_secs(now)))
}

/// The resume line: `TS ALLOW desktop-control reason="resume id=<action_id>"`.
pub fn append_resume(audit_log: &Path, action_id: &str, now: SystemTime) -> io::Result<()> {
    append_line(audit_log, "ALLOW", &format!("resume id={action_id}"), now)
}

/// Append one preformatted line (single write on an O_APPEND fd).
pub fn append_raw(audit_log: &Path, line: &str) -> io::Result<()> {
    fsx::open_append(audit_log)?.write_all(line.as_bytes())
}
