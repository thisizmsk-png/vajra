//! C6 control service (design.md 5): the per-session steering nonce, the
//! queue/inflight/acks/stops layout the `desktop-control.sh` hook reads, the
//! append-only control ledger, human-only stops, the owner-only Unix socket
//! and the `--mcp` stdio server the Lead chat session talks to.

pub mod audit;
pub mod fsx;
pub mod ledger;
pub mod lead;
pub mod lead_settings;
pub mod mcp;
pub mod nonce;
pub mod queue;
pub mod reconcile;
pub mod service;
pub mod sha256;
pub mod socket;
pub mod time;

use std::path::{Path, PathBuf};

/// The app socket inside the data dir. Keep the full path under 104 bytes
/// (sun_path); the default `~/Library/Application Support/VajraDesktop` is.
pub fn socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join("app.sock")
}
