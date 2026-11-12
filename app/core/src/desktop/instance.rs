//! One app per data folder and per account's worktree folder (6c-3, from the 6d2d security re-check).
//!
//! The start-up worker sweep kills processes that started before this app and carry a ticket TMPDIR under the shared
//! worktree folder. A second app (a development build beside the installed one, even with its own data folder) shares
//! that folder, so without these locks it would take the first app's live workers for an earlier life's. Only the app
//! that holds both locks sweeps and starts agents; a second one does neither.

use std::fs::File;
use std::path::Path;

/// Why a second instance starts no agent.
pub const OTHER_INSTANCE: &str = "Agents are off: another Vajra Desktop is open on this data folder or this account's worktrees.";
pub const OTHER_INSTANCE_FIX: &str = "Quit the other Vajra Desktop, then restart this one.";

/// An exclusive `flock` held for the app's life, on a 0600 file opened never through a link. The kernel drops it when
/// the process ends, however it ends, so a crash never leaves it held.
pub struct InstanceLock;

impl InstanceLock {
    /// The data folder's lock (`<data>/instance.lock`), or `None` when another process holds it or it can't be opened
    /// (fail closed: no sweep, no agent).
    pub fn take(data: &Path) -> Option<File> {
        Self::take_file(&data.join("instance.lock"))
    }

    /// The shared worktree folder's lock (`<work>/.instance.lock`): one app per account sweeps and runs workers there.
    pub fn take_work(home: &Path) -> Option<File> {
        let work = crate::vajra::gitw::work_root(home);
        crate::control::fsx::private_dir(&work).ok()?;
        Self::take_file(&work.join(".instance.lock"))
    }

    /// Both locks, or none: the data folder's first.
    pub fn take_both(data: &Path, home: &Path) -> Option<(File, File)> {
        Self::take(data).and_then(|d| Self::take_work(home).map(|w| (d, w)))
    }

    fn take_file(path: &Path) -> Option<File> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;
        let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path).ok()?;
        // SAFETY: the descriptor is open for this call; LOCK_NB never waits.
        (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_lock_refuses_a_second_taker_and_frees_on_drop() {
        let d = std::env::temp_dir().join(format!("vajra-instance-{}", std::process::id()));
        let (data, home) = (d.join("data"), d.join("home"));
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let held = InstanceLock::take_both(&data, &home).expect("the first taker holds both");
        // flock is per open file description: a second open in the same process is refused like another app's.
        assert!(InstanceLock::take(&data).is_none());
        assert!(InstanceLock::take_work(&home).is_none());
        // Another data folder, the same home: the worktree folder's lock still refuses it.
        let data2 = d.join("data-2");
        std::fs::create_dir_all(&data2).unwrap();
        assert!(InstanceLock::take_both(&data2, &home).is_none());
        drop(held);
        assert!(InstanceLock::take_both(&data2, &home).is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_link_in_the_locks_place_is_never_followed() {
        let d = std::env::temp_dir().join(format!("vajra-instance-link-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::os::unix::fs::symlink(d.join("elsewhere"), d.join("instance.lock")).unwrap();
        assert!(InstanceLock::take(&d).is_none());
        assert!(!d.join("elsewhere").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
