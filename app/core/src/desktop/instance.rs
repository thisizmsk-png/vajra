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

/// An exclusive `flock` held for the app's life on the folder itself (the data folder, the worktree folder), opened
/// never through a link. The kernel drops it when the process ends, however it ends, so a crash never leaves it held.
///
/// The folder, not a file in it: a lock file can be unlinked and replaced by any process of the user (a worker), and a
/// second app would then lock the new file and think it is alone. A folder can't be unlinked while anything is in it,
/// and these two never empty while the app runs (its data; the live workers' worktrees). After the lock is taken, the
/// locked inode must still be the one the path names, or the lock is refused: a folder swapped in between the open and
/// the lock is never trusted. What this can't stop is a process of the same user renaming the whole folder away and
/// making a new one; that also moves every live worktree or the app's data out from under the first app.
pub struct InstanceLock;

impl InstanceLock {
    /// The data folder's lock, or `None` when another process holds it or it can't be opened (fail closed: no sweep,
    /// no agent).
    pub fn take(data: &Path) -> Option<File> {
        Self::take_dir(data)
    }

    /// The shared worktree folder's lock: one app per account sweeps and runs workers there.
    pub fn take_work(home: &Path) -> Option<File> {
        let work = crate::vajra::gitw::work_root(home);
        crate::control::fsx::private_dir(&work).ok()?;
        Self::take_dir(&work)
    }

    /// Both locks, or none: the data folder's first.
    pub fn take_both(data: &Path, home: &Path) -> Option<(File, File)> {
        Self::take(data).and_then(|d| Self::take_work(home).map(|w| (d, w)))
    }

    fn take_dir(path: &Path) -> Option<File> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;
        let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW).open(path).ok()?;
        // SAFETY: the descriptor is open for this call; LOCK_NB never waits.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return None;
        }
        still_named(&f, path).then_some(f)
    }
}

/// The open folder `f` is still the one `path` names (no link followed): same device, same inode.
fn still_named(f: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (f.metadata(), std::fs::symlink_metadata(path)) {
        (Ok(held), Ok(named)) => named.is_dir() && held.dev() == named.dev() && held.ino() == named.ino(),
        _ => false,
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
    fn a_folder_reached_through_a_link_is_never_locked() {
        let d = std::env::temp_dir().join(format!("vajra-instance-link-{}", std::process::id()));
        std::fs::create_dir_all(d.join("real")).unwrap();
        std::os::unix::fs::symlink(d.join("real"), d.join("data")).unwrap();
        assert!(InstanceLock::take(&d.join("data")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// bhishma Low (6c-3): a worker that unlinks and replaces whatever lock-looking file sits in either folder while the
    /// first app runs still leaves the second app refused. A lock on a file would fail here: the second app would lock
    /// the replacement.
    #[test]
    fn unlinking_and_replacing_files_in_the_folders_does_not_free_the_lock() {
        let d = std::env::temp_dir().join(format!("vajra-instance-unlink-{}", std::process::id()));
        let (data, home) = (d.join("data"), d.join("home"));
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let held = InstanceLock::take_both(&data, &home).expect("the first taker holds both");
        let work = crate::vajra::gitw::work_root(&home);
        for f in [data.join("instance.lock"), work.join(".instance.lock")] {
            let _ = std::fs::remove_file(&f);
            std::fs::write(&f, "").unwrap();
        }
        assert!(InstanceLock::take_both(&data, &home).is_none(), "the second app is still refused");
        let data2 = d.join("data-2");
        std::fs::create_dir_all(&data2).unwrap();
        assert!(InstanceLock::take_both(&data2, &home).is_none(), "another data folder, the same home: still refused");
        drop(held);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A folder swapped in after it was opened is not the one locked: the lock is refused, never trusted.
    #[test]
    fn a_held_descriptor_whose_folder_was_swapped_is_not_still_named() {
        use std::os::unix::fs::OpenOptionsExt;
        let d = std::env::temp_dir().join(format!("vajra-instance-swap-{}", std::process::id()));
        std::fs::create_dir_all(d.join("data")).unwrap();
        let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW).open(d.join("data")).unwrap();
        assert!(still_named(&f, &d.join("data")));
        std::fs::rename(d.join("data"), d.join("moved")).unwrap();
        std::fs::create_dir_all(d.join("data")).unwrap();
        assert!(!still_named(&f, &d.join("data")), "a new folder at the path is not the locked one");
        let _ = std::fs::remove_dir_all(&d);
    }
}
