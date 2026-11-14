//! Vajra Desktop core.
// Test code may spawn directly; the lib's non-test build is linted on its own, so every spawn in the app itself still
// has to go through `spawnlock` (clippy.toml).
#![cfg_attr(test, allow(clippy::disallowed_methods))]

pub mod adapter;
pub mod ids;
pub mod lifecycle_report;
pub mod logging;
pub mod nap;
pub mod owned;
pub mod paths;
pub mod preferences;
pub mod procid;
pub mod procsweep;

pub mod bus;
pub mod diagnostics;
pub mod control;
pub mod crew;
pub mod desktop;
pub mod projects;
pub mod providers;
pub mod sessions;
pub mod spawnlock;
pub mod terminal;
pub mod userpath;
pub mod vajra;

/// Lock a mutex even if a panic elsewhere poisoned it: one failed thread must
/// not wedge every caller.
pub(crate) fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
