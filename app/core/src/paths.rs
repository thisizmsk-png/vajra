//! Where Vajra Desktop keeps its own files.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

/// Environment variable that overrides the data dir.
pub const DATA_ENV: &str = "VAJRA_DESKTOP_DATA";

/// Resolve the data dir from an optional override and the home dir.
/// An empty override is ignored.
pub fn resolve_data_dir(env_override: Option<&OsStr>, home: &Path) -> PathBuf {
    match env_override {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home.join("Library/Application Support/VajraDesktop"),
    }
}

/// Create `dir` (and parents) and set it to mode 0700, tightening an
/// existing dir too.
pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// The data dir for this process (VAJRA_DESKTOP_DATA, else under $HOME),
/// created at mode 0700.
pub fn data_dir() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    let dir = resolve_data_dir(std::env::var_os(DATA_ENV).as_deref(), Path::new(&home));
    ensure_private_dir(&dir)?;
    Ok(dir)
}

/// The logs dir inside a data dir.
pub fn logs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vajra-desktop-paths-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn data_dir_defaults_to_application_support() {
        let got = resolve_data_dir(None, Path::new("/Users/alice"));
        assert_eq!(
            got,
            PathBuf::from("/Users/alice/Library/Application Support/VajraDesktop")
        );
    }

    #[test]
    fn data_dir_env_override_wins() {
        let got = resolve_data_dir(Some(OsStr::new("/tmp/vd")), Path::new("/Users/alice"));
        assert_eq!(got, PathBuf::from("/tmp/vd"));
    }

    #[test]
    fn data_dir_empty_override_is_ignored() {
        let got = resolve_data_dir(Some(OsStr::new("")), Path::new("/Users/alice"));
        assert_eq!(
            got,
            PathBuf::from("/Users/alice/Library/Application Support/VajraDesktop")
        );
    }

    #[test]
    fn data_dir_is_created_at_mode_0700() {
        let dir = tmp("create").join("nested");
        ensure_private_dir(&dir).unwrap();
        assert!(dir.is_dir());
        assert_eq!(mode(&dir), 0o700);
        std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn data_dir_existing_permissions_are_tightened() {
        let dir = tmp("tighten");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn data_dir_logs_live_under_logs() {
        assert_eq!(logs_dir(Path::new("/d")), PathBuf::from("/d/logs"));
    }
}
