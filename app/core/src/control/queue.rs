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

/// `^[A-Za-z0-9-]{8,128}$`, the hook's own check.
pub fn valid_session_id(s: &str) -> bool {
    (8..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

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
    pub fn queued(&self, action_id: &str) -> Option<PathBuf> {
        find(&self.dir("queue"), |n| n.ends_with(&format!("-{action_id}.json")))
    }

    /// An inflight claim for `action_id`, if one exists.
    pub fn inflight(&self, action_id: &str) -> Option<PathBuf> {
        find(&self.dir("inflight"), |n| n.starts_with(&format!("{action_id}.")))
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

fn find(dir: &Path, pred: impl Fn(&str) -> bool) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.filter_map(Result::ok).map(|e| e.path()).find(|p| {
        p.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.starts_with('.') && pred(n))
    })
}
