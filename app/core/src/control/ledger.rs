//! `control/ledger.jsonl` (design.md 5.6): append-only, one JSON line per
//! state change of an action. The type only offers `append`; the file is
//! opened O_APPEND | O_NOFOLLOW at 0600 and earlier lines are never rewritten.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::fsx;

pub struct Ledger {
    path: PathBuf,
    file: File,
}

impl Ledger {
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self { path: path.to_path_buf(), file: fsx::open_append(path)? })
    }

    /// Test only: appends fail from now on (the file is reopened read-only).
    #[cfg(test)]
    pub(crate) fn break_for_test(&mut self) {
        self.file = File::open(&self.path).unwrap();
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One record, one line, one write.
    pub fn append(&mut self, rec: &Value) -> io::Result<()> {
        let mut line = serde_json::to_vec(rec).map_err(io::Error::other)?;
        line.push(b'\n');
        self.file.write_all(&line)
    }

    /// Every parseable record, in order. A torn or foreign line is skipped.
    pub fn read_all(path: &Path) -> io::Result<Vec<Value>> {
        let body = match fsx::read_nofollow(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            r => r?,
        };
        Ok(String::from_utf8_lossy(&body)
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(Value::is_object)
            .collect())
    }
}
