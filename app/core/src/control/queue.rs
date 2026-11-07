//! The per-session control layout the hook reads (design.md 5.3):
//!
//! ```text
//! control/<session_id>/          0700
//!   nonce                        0600, see nonce.rs
//!   queue/<seq:06>-<id>.json     message|redirect, consumed once by the hook
//!   inflight/<id>.<pid>          claimed by the hook, before ack
//!   stops/<sha256_16(key)>.json  sticky stop flag, cleared only by a human
//!   acks/<id>.json               written by the hook
//!   expired/<seq:06>-<id>.json   moved here by the app, never delivered
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{fsx, sha256};

pub const SUBDIRS: [&str; 5] = ["queue", "inflight", "stops", "acks", "expired"];

/// One rule for every boundary (a canonical uuid, which the hook's
/// `^[A-Za-z0-9-]{8,128}$` also accepts).
pub use crate::ids::valid_session_id;

/// The stop file name for a worker key: the first 16 hex of its SHA-256.
pub fn stop_name(key: &str) -> String {
    format!("{}.json", &sha256::hex_digest(key.as_bytes())[..16])
}

#[derive(Clone, Debug)]
pub struct SessionDir {
    pub root: PathBuf,
}

impl SessionDir {
    /// Create (or tighten) `<control_root>/<session_id>` and its subdirs, all 0700.
    pub fn create(control_root: &Path, session_id: &str) -> io::Result<Self> {
        if !valid_session_id(session_id) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad session id"));
        }
        let root = control_root.join(session_id);
        fsx::private_dir(&root)?;
        for d in SUBDIRS {
            fsx::private_dir(&root.join(d))?;
        }
        Ok(Self { root })
    }

    pub fn dir(&self, sub: &str) -> PathBuf {
        self.root.join(sub)
    }

    pub fn stop_path(&self, key: &str) -> PathBuf {
        self.dir("stops").join(stop_name(key))
    }

    pub fn ack_path(&self, action_id: &str) -> PathBuf {
        self.dir("acks").join(format!("{action_id}.json"))
    }

    /// The queue file for `action_id`, if still queued.
    pub fn queued(&self, action_id: &str) -> io::Result<Option<PathBuf>> {
        find(&self.dir("queue"), |n| n.ends_with(&format!("-{action_id}.json")))
    }

    /// An inflight claim for `action_id`, if one exists.
    pub fn inflight(&self, action_id: &str) -> io::Result<Option<PathBuf>> {
        find(&self.dir("inflight"), |n| n.starts_with(&format!("{action_id}.")))
    }

    /// The expired copy of `action_id`'s queue file, if the app moved it there.
    pub fn expired(&self, action_id: &str) -> io::Result<Option<PathBuf>> {
        find(&self.dir("expired"), |n| n.ends_with(&format!("-{action_id}.json")))
    }

    /// Remove the `.<name>.<pid>.tmp` files a crash between `write_atomic`'s write and rename left in queue/. The hook
    /// skips dot files, so none is live when a session starts. Best effort: a file that will not go is left for next time.
    pub fn sweep_temp(&self) {
        let Ok(rd) = fsx::read_dir_nofollow(&self.dir("queue")) else { return };
        for e in rd.flatten() {
            let name = e.file_name();
            let Some(n) = name.to_str() else { continue };
            if n.starts_with('.') && n.ends_with(".tmp") && e.file_type().is_ok_and(|t| !t.is_dir()) {
                let _ = fs::remove_file(e.path());
            }
        }
    }

    /// The file names in queue/ and inflight/ right now, listed once so a tick settling many messages of one session
    /// does not scan both directories per message. Taken before any ack is read, which keeps the check order
    /// (queue, inflight, ack): a file only ever leaves queue/ and a claim only ever appears after it, so a stale
    /// listing can only say "still queued" (retried next tick), never "gone" for a file that is not.
    pub fn listing(&self) -> io::Result<Listing> {
        Ok(Listing { queue: names(&self.dir("queue"))?, inflight: names(&self.dir("inflight"))? })
    }

    /// The highest `<seq>` prefix in queue/ and expired/.
    pub fn max_seq(&self) -> u64 {
        ["queue", "expired"]
            .iter()
            .filter_map(|d| fs::read_dir(self.dir(d)).ok())
            .flatten()
            .filter_map(|e| e.ok()?.file_name().to_str()?.split('-').next()?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
    }
}

/// The first entry of `dir` that `pred` accepts. A scan that fails is an error, never "not found": settling on a
/// failed scan would call a file absent that is still there. A directory that does not exist has no entries.
/// A one-pass listing of a session's queue/ and inflight/ (see `SessionDir::listing`).
pub struct Listing {
    queue: Vec<String>,
    inflight: Vec<String>,
}

impl Listing {
    pub fn queued(&self, action_id: &str) -> bool {
        let suffix = format!("-{action_id}.json");
        self.queue.iter().any(|n| n.ends_with(&suffix))
    }

    pub fn inflight(&self, action_id: &str) -> bool {
        let prefix = format!("{action_id}.");
        self.inflight.iter().any(|n| n.starts_with(&prefix))
    }
}

fn names(dir: &Path) -> io::Result<Vec<String>> {
    let rd = match fsx::read_dir_nofollow(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for e in rd {
        if let Some(n) = e?.file_name().to_str().filter(|n| !n.starts_with('.')) {
            out.push(n.to_string());
        }
    }
    Ok(out)
}

fn find(dir: &Path, pred: impl Fn(&str) -> bool) -> io::Result<Option<PathBuf>> {
    let rd = match fsx::read_dir_nofollow(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    for e in rd {
        let p = e?.path();
        if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.starts_with('.') && pred(n)) {
            return Ok(Some(p));
        }
    }
    Ok(None)
}
