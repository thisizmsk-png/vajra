//! The app's own log: logs/app.log.
//!
//! The API takes only `&'static str` event and field names plus numbers, so
//! no agent text (prompts, transcripts, tool output) can reach the log.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Rotate app.log once a write would take it past this size.
pub const MAX_BYTES: u64 = 1024 * 1024;
/// Files kept in total: app.log, app.log.1, app.log.2.
pub const KEEP_FILES: usize = 3;

pub struct Logger {
    path: PathBuf,
    max_bytes: u64,
    state: Mutex<State>,
}

struct State {
    file: File,
    len: u64,
}

fn open_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

/// Event and field names must be `[a-z0-9_]+`; anything else is replaced so
/// a misused name can never smuggle text or newlines into the log.
fn safe_name(name: &'static str, fallback: &'static str) -> &'static str {
    let ok = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if ok { name } else { fallback }
}

impl Logger {
    /// Open `<logs_dir>/app.log` with the default 1 MB x 3 rotation.
    pub fn open(logs_dir: &Path) -> io::Result<Self> {
        Self::open_with_limit(logs_dir, MAX_BYTES)
    }

    /// Open with a custom rotation size (tests).
    pub fn open_with_limit(logs_dir: &Path, max_bytes: u64) -> io::Result<Self> {
        crate::paths::ensure_private_dir(logs_dir)?;
        let path = logs_dir.join("app.log");
        let file = open_file(&path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            path,
            max_bytes,
            state: Mutex::new(State { file, len }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn rotated(&self, n: usize) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    }

    /// app.log -> app.log.1 -> ... -> app.log.<KEEP_FILES-1>, oldest dropped.
    fn rotate(&self, state: &mut State) -> io::Result<()> {
        let oldest = self.rotated(KEEP_FILES - 1);
        match std::fs::remove_file(&oldest) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        for n in (1..KEEP_FILES - 1).rev() {
            let from = self.rotated(n);
            if from.exists() {
                std::fs::rename(&from, self.rotated(n + 1))?;
            }
        }
        std::fs::rename(&self.path, self.rotated(1))?;
        state.file = open_file(&self.path)?;
        state.len = 0;
        Ok(())
    }

    /// Write one line: `<unix_ms> <event> key=value ...`.
    pub fn log(&self, event: &'static str, fields: &[(&'static str, u64)]) -> io::Result<()> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut line = format!("{now_ms} {}", safe_name(event, "invalid_event"));
        for (key, value) in fields {
            line.push_str(&format!(" {}={value}", safe_name(key, "invalid_field")));
        }
        line.push('\n');

        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.len > 0 && state.len + line.len() as u64 > self.max_bytes {
            self.rotate(&mut state)?;
        }
        state.file.write_all(line.as_bytes())?;
        state.len += line.len() as u64;
        Ok(())
    }

    /// Log `cold_start ms=<ms since process start>` and return the ms.
    pub fn log_cold_start(&self, process_start: SystemTime) -> io::Result<u64> {
        let ms = SystemTime::now()
            .duration_since(process_start)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.log("cold_start", &[("ms", ms)])?;
        Ok(ms)
    }
}

/// When this process started, from the kernel (so it includes time spent
/// before main). Falls back to the first call if the kernel query fails.
pub fn process_start_time() -> SystemTime {
    static FALLBACK: OnceLock<SystemTime> = OnceLock::new();
    kernel_start_time().unwrap_or_else(|| *FALLBACK.get_or_init(SystemTime::now))
}

#[cfg(target_os = "macos")]
fn kernel_start_time() -> Option<SystemTime> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: info is a properly sized, writable proc_bsdinfo buffer.
    let got = unsafe {
        libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if got != size {
        return None;
    }
    Some(
        UNIX_EPOCH
            + Duration::from_secs(info.pbi_start_tvsec)
            + Duration::from_micros(info.pbi_start_tvusec),
    )
}

#[cfg(not(target_os = "macos"))]
fn kernel_start_time() -> Option<SystemTime> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vajra-desktop-log-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn log_rotation_writes_app_log_with_event_and_fields() {
        let dir = tmp("write");
        let log = Logger::open(&dir).unwrap();
        assert_eq!(log.path(), dir.join("app.log"));
        log.log("session_opened", &[("count", 3), ("bytes", 42)]).unwrap();
        let text = read(&dir.join("app.log"));
        let line = text.lines().next().unwrap();
        let mut parts = line.split(' ');
        parts.next().unwrap().parse::<u64>().unwrap();
        assert_eq!(parts.collect::<Vec<_>>(), ["session_opened", "count=3", "bytes=42"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn log_rotation_defaults_to_1mb_by_3() {
        assert_eq!(MAX_BYTES, 1_048_576);
        assert_eq!(KEEP_FILES, 3);
    }

    #[test]
    fn log_rotation_rolls_over_and_keeps_three_files() {
        let dir = tmp("rotate");
        let log = Logger::open_with_limit(&dir, 200).unwrap();
        for i in 0..200 {
            log.log("tick", &[("i", i)]).unwrap();
        }
        let names: Vec<String> = {
            let mut v: Vec<String> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            v.sort();
            v
        };
        assert_eq!(names, ["app.log", "app.log.1", "app.log.2"]);
        for name in &names {
            let len = std::fs::metadata(dir.join(name)).unwrap().len();
            assert!(len <= 200, "{name} is {len} bytes");
        }
        // Newest line is in app.log; older ones roll down.
        assert!(read(&dir.join("app.log")).contains("tick i=199"));
        assert!(!read(&dir.join("app.log.1")).contains("i=199"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn log_rotation_appends_across_reopen() {
        let dir = tmp("reopen");
        Logger::open(&dir).unwrap().log("first", &[]).unwrap();
        Logger::open(&dir).unwrap().log("second", &[]).unwrap();
        let text = read(&dir.join("app.log"));
        assert_eq!(text.lines().count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn log_rotation_rejects_event_names_that_are_not_identifiers() {
        let dir = tmp("names");
        let log = Logger::open(&dir).unwrap();
        log.log("user said: hello\nfake 0 line", &[("ok key", 1)]).unwrap();
        let text = read(&dir.join("app.log"));
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains(" invalid_event invalid_field=1"));
        assert!(!text.contains("hello"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn log_rotation_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("mode");
        let log = Logger::open(&dir).unwrap();
        log.log("x", &[]).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("app.log")), 0o600);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cold_start_logs_ms_since_process_start() {
        let dir = tmp("cold");
        let log = Logger::open(&dir).unwrap();
        let start = SystemTime::now() - Duration::from_millis(250);
        let ms = log.log_cold_start(start).unwrap();
        assert!((250..60_000).contains(&ms), "ms = {ms}");
        let text = read(&dir.join("app.log"));
        assert!(text.contains(&format!(" cold_start ms={ms}")), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cold_start_process_start_time_is_in_the_past() {
        let start = process_start_time();
        let age = SystemTime::now().duration_since(start).unwrap();
        assert!(age < Duration::from_secs(3600), "age = {age:?}");
    }
}
