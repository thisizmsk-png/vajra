//! Vajra Desktop core.

pub mod adapter;
pub mod ids;
pub mod logging;
pub mod mobile_m0;
pub mod nap;
pub mod paths;

pub mod bus;
pub mod control;
pub mod crew;
pub mod desktop;
pub mod projects;
pub mod sessions;
pub mod terminal;
pub mod userpath;
pub mod vajra;

/// Lock a mutex even if a panic elsewhere poisoned it: one failed thread must
/// not wedge every caller.
pub(crate) fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
