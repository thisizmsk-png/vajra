//! Transcript tailer. Reads new bytes of one allowlisted jsonl file per poll,
//! holds a partial last line until its newline arrives, and restarts from
//! offset 0 when the file shrinks or is replaced (different inode).

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use serde_json::Value;

use super::record::{self, LineKind, StreamLine};
use super::redact::Redactor;
use crate::bus::Bus;
use crate::vajra::allowlist::{Allowlist, Denied};

/// Bytes read per poll; the rest waits for the next poll.
pub const READ_CAP: u64 = 256 * 1024;
/// A record longer than this is skipped whole and shown as `OVERSIZED`, so a
/// huge tool result never pins memory or splits into unparseable fragments.
pub const PARTIAL_CAP: usize = 1024 * 1024;
pub const OVERSIZED: &str = "[record over 1 MiB not shown]";

#[derive(Debug, Default)]
pub struct Chunk {
    pub lines: Vec<String>,
    /// The file shrank or was replaced since the last poll.
    pub truncated: bool,
}

pub struct Tailer {
    allow: Allowlist,
    path: PathBuf,
    offset: u64,
    ino: Option<u64>,
    partial: Vec<u8>,
    /// Inside an oversized record: drop bytes until its newline.
    skipping: bool,
}

impl Tailer {
    pub fn new(allow: Allowlist, path: PathBuf) -> Self {
        Self { allow, path, offset: 0, ino: None, partial: Vec::new(), skipping: false }
    }

    pub fn poll(&mut self) -> io::Result<Chunk> {
        let mut file = match self.allow.open_allowlisted(&self.path) {
            Ok(f) => f,
            Err(Denied::Io(e)) if e.kind() == io::ErrorKind::NotFound => return Ok(Chunk::default()),
            Err(d) => return Err(d.into()),
        };
        let meta = file.metadata()?;
        let mut chunk = Chunk::default();
        if meta.len() < self.offset || self.ino.is_some_and(|i| i != meta.ino()) {
            chunk.truncated = true;
            self.offset = 0;
            self.partial.clear();
            self.skipping = false;
        }
        self.ino = Some(meta.ino());
        self.read_new(&mut file, &mut chunk)?;
        Ok(chunk)
    }

    fn read_new(&mut self, file: &mut File, chunk: &mut Chunk) -> io::Result<()> {
        file.seek(SeekFrom::Start(self.offset))?;
        let mut buf = Vec::new();
        let n = file.take(READ_CAP).read_to_end(&mut buf)?;
        self.offset += n as u64;
        let mut rest = buf.as_slice();
        while let Some(i) = rest.iter().position(|&b| b == b'\n') {
            if self.skipping {
                self.skipping = false;
                chunk.lines.push(OVERSIZED.to_string());
            } else if self.partial.len() + i > PARTIAL_CAP {
                self.partial.clear();
                chunk.lines.push(OVERSIZED.to_string());
            } else {
                self.partial.extend_from_slice(&rest[..i]);
                chunk.lines.push(take_line(&mut self.partial));
            }
            rest = &rest[i + 1..];
        }
        if !self.skipping {
            if self.partial.len() + rest.len() > PARTIAL_CAP {
                self.partial = Vec::new();
                self.skipping = true;
            } else {
                self.partial.extend_from_slice(rest);
            }
        }
        Ok(())
    }
}

fn take_line(partial: &mut Vec<u8>) -> String {
    if partial.last() == Some(&b'\r') {
        partial.pop();
    }
    let s = String::from_utf8_lossy(partial).into_owned();
    partial.clear();
    s
}

/// A tailed transcript: tailer + record parser + schema fingerprint +
/// nonce redaction. Once a record fails the fingerprint, the rest of the
/// file is shown raw (design.md 7, "Transcript format change").
pub struct Transcript {
    tailer: Tailer,
    redactor: Redactor,
    unrecognized: bool,
    prompt_first_line: Option<String>,
    git_branch: Option<String>,
    cwd: Option<String>,
}

impl Transcript {
    pub fn new(tailer: Tailer, redactor: Redactor) -> Self {
        Self { tailer, redactor, unrecognized: false, prompt_first_line: None, git_branch: None, cwd: None }
    }

    /// The steer nonce rotates on resume: later lines redact the current one.
    pub fn set_redactor(&mut self, redactor: Redactor) {
        self.redactor = redactor;
    }

    pub fn poll(&mut self) -> io::Result<Vec<StreamLine>> {
        let chunk = self.tailer.poll()?;
        if chunk.truncated {
            self.unrecognized = false;
        }
        let mut out = Vec::new();
        for line in &chunk.lines {
            if line.trim().is_empty() {
                continue;
            }
            if line == OVERSIZED {
                out.push(StreamLine { kind: LineKind::Raw, text: line.clone() });
                continue;
            }
            if !self.unrecognized {
                match serde_json::from_str::<Value>(line).ok().and_then(|v| record::parse(&v).map(|r| (v, r))) {
                    Some((v, lines)) => {
                        self.note_identity(&v);
                        out.extend(lines.into_iter().flat_map(|l| self.clean(l)));
                        continue;
                    }
                    None => self.unrecognized = true,
                }
            }
            out.extend(self.clean(StreamLine { kind: LineKind::Raw, text: line.clone() }));
        }
        Ok(out)
    }

    /// Poll and push each line to `bus` under `stream`; returns lines pushed.
    pub fn pump(&mut self, bus: &Bus, stream: &str) -> io::Result<usize> {
        let lines = self.poll()?;
        for l in &lines {
            bus.push(stream, &l.display()).map_err(io::Error::other)?;
        }
        Ok(lines.len())
    }

    pub fn format_unrecognized(&self) -> bool {
        self.unrecognized
    }

    pub fn banner(&self) -> Option<&'static str> {
        self.unrecognized.then_some("Transcript format not recognized; showing raw view")
    }

    /// First line of the first user prompt, cleaned and redacted.
    pub fn prompt_first_line(&self) -> Option<&str> {
        self.prompt_first_line.as_deref()
    }

    pub fn git_branch(&self) -> Option<&str> {
        self.git_branch.as_deref()
    }

    /// The worker's cwd from its records (a stop's fallback match key).
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    fn note_identity(&mut self, v: &Value) {
        if self.cwd.is_none() {
            self.cwd = v.get("cwd").and_then(Value::as_str).filter(|s| s.starts_with('/')).map(str::to_string);
        }
        if self.git_branch.is_none() {
            self.git_branch = v.get("gitBranch").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        }
        if self.prompt_first_line.is_none()
            && let Some(p) = record::user_prompt(v)
        {
            self.prompt_first_line = p.lines().next().map(|l| self.redactor.clean(l));
        }
    }

    /// Multi-line text becomes one StreamLine per line, each cleaned.
    fn clean(&self, l: StreamLine) -> Vec<StreamLine> {
        l.text
            .split('\n')
            .map(|t| StreamLine { kind: l.kind, text: self.redactor.clean(t) })
            .filter(|s| !s.text.is_empty())
            .collect()
    }
}
