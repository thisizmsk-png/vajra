//! A core run's log on disk and live (W2): `<run dir>/log.jsonl`, one entry per line, owner-only, never through a link.
//!
//! - **Order.** `append` adds the entry in memory and queues it, both under the log's lock, so the file's order is
//!   the log's. One writer thread writes each line; only then does the UI hear of it (`run_log {run_id, seq}`), and
//!   `since` returns only written entries. Nothing here does file I/O on the caller's thread, so the protocol may
//!   append under its own lock.
//! - **Notices.** A critical entry (a question, a check result, a verdict, the status, a failed ticket) is synced and
//!   told at once; the rest are told once per 100 ms at most.
//! - **Anchors.** `anchor` waits for everything before it to be written and synced, then sends `log head <run> <seq>
//!   <hash>` through the run's signed record (#190's own progress write: ledgered, retried, read back), then logs
//!   `anchored`. The log has no signer of its own.
//! - **Restart.** Loading keeps the longest valid prefix. A torn last line (a crash mid-write) is the only thing cut,
//!   and a `recovered` entry says so. A break anywhere else is never repaired: the log stays broken, is never written
//!   again, and the run can't be verified on it.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::record::{Event, RunRecord};
use super::runlog::{self, Entry, Kind, Log, Refused, Source};
use crate::control::fsx;

pub const FILE: &str = "log.jsonl";
/// A log bigger than this isn't read (a run writes a few thousand entries at most).
pub const MAX_BYTES: u64 = 64 << 20;
/// Ordinary entries are told to the UI at most this often.
pub const COALESCE: Duration = Duration::from_millis(100);
/// The most entries one `since` answer carries.
pub const PAGE_MAX: usize = 500;

/// The most entries one notice carries. A notice for more carries the first `NOTICE_MAX`, and the UI, seeing it stop
/// short of `seq`, asks `since` for the rest: one call for a burst, none for ordinary traffic.
pub const NOTICE_MAX: usize = 64;

/// What the UI hears: the run's log is written up to `seq`, and `entries` are the written ones after `after` (in order,
/// at most `NOTICE_MAX`). A view holding exactly `after` applies them with no call; any other view asks `since`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub run_id: String,
    pub seq: u64,
    pub after: u64,
    pub entries: Vec<Entry>,
    /// The log's state as of this notice, so a caught-up view that makes no call still learns the log stopped (#233).
    pub io_error: Option<String>,
    pub broken: Option<runlog::Break>,
}

/// Tells the UI about newly written entries.
pub type Notify = Box<dyn Fn(&Notice) + Send + Sync>;
/// Told each anchor right after it is sent to the signed record: `(run id, anchor)`.
pub type OnAnchor = Box<dyn Fn(&str, &runlog::Anchor) + Send + Sync>;

/// Make `dir` a private directory of this user (mode 0700, owned by this uid, not a link) inside a parent that is a real
/// directory of this user, not a link. The parent is checked, never changed: the app makes its own run-log folder
/// private. Anything else is refused, never fixed (bhishma F8).
fn private_run_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let uid = unsafe { libc::getuid() };
    let refuse = |d: &Path, what: &str| io::Error::other(format!("{} is not {what}", d.display()));
    let parent = dir.parent().ok_or_else(|| refuse(dir, "inside a folder"))?;
    let pm = std::fs::symlink_metadata(parent)?;
    if !pm.is_dir() || pm.uid() != uid {
        return Err(refuse(parent, "a directory of this user"));
    }
    fsx::private_dir(dir)?;
    let m = std::fs::symlink_metadata(dir)?;
    if !m.is_dir() || m.uid() != uid || m.permissions().mode() & 0o777 != 0o700 {
        return Err(refuse(dir, "a private directory of this user"));
    }
    Ok(())
}

/// What loading found on disk.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Loaded {
    pub entries: Vec<Entry>,
    /// The position of the first whole line that isn't an entry.
    pub unreadable: Option<u64>,
    /// Bytes after the last newline: a torn last line.
    pub torn: u64,
    /// The length of the whole lines.
    pub good_len: u64,
}

/// Parse the file's bytes: every whole line, and what trails the last newline.
pub fn parse(bytes: &[u8]) -> Loaded {
    let good_len = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let mut out = Loaded { torn: (bytes.len() - good_len) as u64, good_len: good_len as u64, ..Loaded::default() };
    // Only the final newline ends a line without starting one: every line before it, an empty one included, must be an
    // entry, so a blank line added inside the file breaks the log there (#222 review).
    let whole = bytes[..good_len].strip_suffix(b"\n").filter(|_| good_len > 0);
    for (i, line) in whole.into_iter().flat_map(|w| w.split(|&b| b == b'\n')).enumerate() {
        match serde_json::from_slice::<Entry>(line) {
            Ok(e) => out.entries.push(e),
            Err(_) => {
                out.unreadable = Some(i as u64 + 1);
                break;
            }
        }
    }
    out
}

/// Open the log for writing, never through a link, and only a regular file.
fn open_write(path: &Path) -> io::Result<std::fs::File> {
    let f = OpenOptions::new().write(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    if !f.metadata()?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
    }
    Ok(f)
}

/// Read the run's log, then check `required` (the last anchor this app session sent; F4), and only then cut a torn
/// last line, and only when everything before it holds. A log that breaks either check is left exactly as found.
fn load(path: &Path, run_id: &str, required: Option<&runlog::Anchor>) -> io::Result<(Log, u64)> {
    let bytes = match fsx::read_nofollow_capped(path, MAX_BYTES) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    let p = parse(&bytes);
    let mut log = Log::loaded(run_id, p.entries, p.unreadable);
    if let Some(a) = required {
        log.require(a);
    }
    let mut torn = 0;
    if p.torn > 0 && log.broken().is_none() {
        let f = open_write(path)?;
        f.set_len(p.good_len)?;
        f.sync_all()?;
        torn = p.torn;
    }
    Ok((log, torn))
}

/// A log this app doesn't hold open (a finished run), read for display or verification only: no writer thread, no
/// repair, no append. A torn last line is left on disk and simply not read; only a live `RunLog::open` cuts it.
pub struct Snapshot {
    log: Log,
    /// Bytes after the last newline, left as found.
    pub torn: u64,
}

impl Snapshot {
    pub fn read(dir: &Path, run_id: &str) -> Result<Self, String> {
        // Read-only: the folder must already be a real directory (not a link); nothing is created or changed.
        match std::fs::symlink_metadata(dir) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err("the run log's folder is not a directory".into()),
            Err(e) => return Err(format!("the run log can't be read: {e}")),
        }
        let bytes = fsx::read_nofollow_capped(&dir.join(FILE), MAX_BYTES).map_err(|e| format!("the run log can't be read: {e}"))?;
        let p = parse(&bytes);
        Ok(Snapshot { log: Log::loaded(run_id, p.entries, p.unreadable), torn: p.torn })
    }

    pub fn since(&self, after: u64, limit: usize) -> (Vec<Entry>, u64) {
        (self.log.since(after, limit.min(PAGE_MAX)).to_vec(), self.log.head_seq())
    }

    pub fn entries(&self) -> Vec<Entry> {
        self.log.entries().to_vec()
    }

    /// Require the chain to hold an anchor this app session sent (F4); a mismatch marks this snapshot broken there.
    pub fn require(&mut self, a: &runlog::Anchor) -> bool {
        self.log.require(a)
    }

    pub fn card(&self) -> LogCard {
        LogCard { head_seq: self.log.head_seq(), written_seq: self.log.head_seq(), broken: self.log.broken().cloned(), io_error: None, skipped_anchors: 0 }
    }
}

/// How long `open` waits for an earlier writer of the same file to finish its queue.
pub const CLAIM_WAIT: Duration = Duration::from_secs(10);

/// The files that have a live writer thread in this process. A writer holds its file's claim from before it reads the
/// file until after its last queued write, so a log dropped and reopened at once never has two writers (#222 review).
static WRITERS: Mutex<Option<std::collections::HashSet<PathBuf>>> = Mutex::new(None);
static RELEASED: std::sync::Condvar = std::sync::Condvar::new();

struct Claim(PathBuf);

impl Claim {
    fn take(path: &Path) -> Result<Claim, String> {
        let deadline = Instant::now() + CLAIM_WAIT;
        let mut w = crate::lock(&WRITERS);
        loop {
            let set = w.get_or_insert_with(Default::default);
            if set.insert(path.to_path_buf()) {
                return Ok(Claim(path.to_path_buf()));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("the run log is still being written by an earlier writer".into());
            }
            w = RELEASED.wait_timeout(w, left).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(set) = crate::lock(&WRITERS).as_mut() {
            set.remove(&self.0);
        }
        RELEASED.notify_all();
    }
}

enum Msg {
    Write { entry: Box<Entry>, critical: bool },
    Anchor,
    Flush(mpsc::Sender<()>),
    Stop,
}

/// The log's state as the run card shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LogCard {
    pub head_seq: u64,
    pub written_seq: u64,
    pub broken: Option<runlog::Break>,
    /// The file couldn't be written or synced: entries after `written_seq` are in memory only, or not yet durable.
    pub io_error: Option<String>,
    /// Anchors asked for (a round's end, the run's end) that weren't sent because of `io_error`: whatever came after
    /// the last sent anchor is outside the signed prefix.
    pub skipped_anchors: u32,
}

struct Shared {
    path: PathBuf,
    log: Mutex<Log>,
    /// The last seq on disk; `since` answers up to here.
    written: Mutex<u64>,
    /// The last seq a notice covered; the next notice carries the written entries after it.
    told: Mutex<u64>,
    io_error: Mutex<Option<String>>,
    notify: Mutex<Option<Notify>>,
    on_anchor: Mutex<Option<OnAnchor>>,
    record: Mutex<Option<Arc<dyn RunRecord>>>,
    /// Anchors asked for but not sent because the log couldn't be made durable first.
    skipped_anchors: Mutex<u32>,
    #[cfg(test)]
    fail_sync: std::sync::atomic::AtomicBool,
    tx: Mutex<mpsc::Sender<Msg>>,
}

/// One run's live log. Dropping it stops its writer once everything queued is written.
pub struct RunLog {
    s: Arc<Shared>,
}

impl RunLog {
    /// Open (or create) the run's log in `dir`, resuming after its last good entry.
    pub fn open(dir: &Path, run_id: &str) -> Result<Self, String> {
        Self::open_requiring(dir, run_id, None)
    }

    /// Like `open`, but the chain must hold `required` (an anchor this app session sent) before anything is repaired or
    /// appended. If it doesn't, the log opens broken (`unanchored`) and its file is not touched.
    pub fn open_requiring(dir: &Path, run_id: &str, required: Option<&runlog::Anchor>) -> Result<Self, String> {
        private_run_dir(dir).map_err(|e| format!("the run log's folder can't be used: {e}"))?;
        let path = dir.join(FILE);
        // One writer per file: wait for an earlier writer of it to drain its queue and stop before reading it.
        let claim = Claim::take(&path)?;
        let (log, torn) = load(&path, run_id, required).map_err(|e| format!("the run log can't be read: {e}"))?;
        if log.broken().is_none() {
            // Create it now (0600, no link), so a planted file in its place is refused before any run event.
            fsx::open_append(&path).map_err(|e| format!("the run log can't be opened: {e}"))?;
        }
        let written = log.head_seq();
        let (tx, rx) = mpsc::channel();
        let s = Arc::new(Shared {
            path,
            log: Mutex::new(log),
            written: Mutex::new(written),
            told: Mutex::new(written),
            io_error: Mutex::new(None),
            notify: Mutex::new(None),
            on_anchor: Mutex::new(None),
            record: Mutex::new(None),
            skipped_anchors: Mutex::new(0),
            #[cfg(test)]
            fail_sync: Default::default(),
            tx: Mutex::new(tx),
        });
        let worker = Arc::clone(&s);
        std::thread::Builder::new()
            .name("vajra-runlog".into())
            .spawn(move || {
                // The claim is released only after the last queued write: the writer thread owns the file until then.
                let _claim = claim;
                worker.run(rx);
            })
            .map_err(|e| format!("the run log could not start: {e}"))?;
        let me = RunLog { s };
        if torn > 0 {
            let at_seq = me.head_seq();
            let _ = me.append(Source::Core, Kind::Recovered { dropped_bytes: torn, at_seq });
        }
        Ok(me)
    }

    pub fn set_notify(&self, n: Notify) {
        *crate::lock(&self.s.notify) = Some(n);
    }

    /// Tests: make every sync from now on fail.
    #[cfg(test)]
    pub fn fail_syncs(&self) {
        self.s.fail_sync.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn set_on_anchor(&self, f: OnAnchor) {
        *crate::lock(&self.s.on_anchor) = Some(f);
    }

    /// Require the chain to hold an anchor this app session sent (on reopen; F4). A mismatch breaks the log there.
    pub fn require(&self, a: &runlog::Anchor) -> bool {
        crate::lock(&self.s.log).require(a)
    }

    /// The signed record anchors go to. Without one, `anchor` logs nothing and the log stays unsigned.
    pub fn set_record(&self, r: Arc<dyn RunRecord>) {
        *crate::lock(&self.s.record) = Some(r);
    }

    /// Append one event: in memory now, on disk soon after (in order). Cheap; safe under the caller's lock.
    pub fn append(&self, source: Source, kind: Kind) -> Result<u64, Refused> {
        let mut log = crate::lock(&self.s.log);
        let critical = kind.critical();
        let entry = Box::new(log.append(source, kind, super::state::now_ms())?.clone());
        let seq = entry.seq;
        // Sent under the log's lock: the queue's order is the log's.
        let _ = crate::lock(&self.s.tx).send(Msg::Write { entry, critical });
        Ok(seq)
    }

    /// Anchor the head into the signed record once everything appended so far is written.
    pub fn anchor(&self) {
        let _ = crate::lock(&self.s.tx).send(Msg::Anchor);
    }

    /// Wait until everything queued so far has been written (and anchors sent).
    pub fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        if crate::lock(&self.s.tx).send(Msg::Flush(tx)).is_ok() {
            let _ = rx.recv();
        }
    }

    pub fn head_seq(&self) -> u64 {
        crate::lock(&self.s.log).head_seq()
    }

    /// The written entries after `after`, at most `limit` (capped at `PAGE_MAX`), and the written head.
    pub fn since(&self, after: u64, limit: usize) -> (Vec<Entry>, u64) {
        let written = *crate::lock(&self.s.written);
        let log = crate::lock(&self.s.log);
        let room = usize::try_from(written.saturating_sub(after)).unwrap_or(usize::MAX);
        (log.since(after, limit.min(PAGE_MAX).min(room)).to_vec(), written)
    }

    /// Every entry in memory (for verification against the signed record).
    pub fn entries(&self) -> Vec<Entry> {
        crate::lock(&self.s.log).entries().to_vec()
    }

    pub fn run_id(&self) -> String {
        crate::lock(&self.s.log).run_id().to_string()
    }

    pub fn card(&self) -> LogCard {
        let log = crate::lock(&self.s.log);
        LogCard {
            head_seq: log.head_seq(),
            written_seq: *crate::lock(&self.s.written),
            broken: log.broken().cloned(),
            io_error: crate::lock(&self.s.io_error).clone(),
            skipped_anchors: *crate::lock(&self.s.skipped_anchors),
        }
    }
}

impl Drop for RunLog {
    fn drop(&mut self) {
        let _ = crate::lock(&self.s.tx).send(Msg::Stop);
    }
}

impl Shared {
    fn run(self: Arc<Self>, rx: mpsc::Receiver<Msg>) {
        let mut file: Option<std::fs::File> = None;
        // An ordinary entry written but not yet told: tell it by this time.
        let mut due: Option<Instant> = None;
        loop {
            let m = match due {
                Some(d) => match rx.recv_timeout(d.saturating_duration_since(Instant::now())) {
                    Ok(m) => m,
                    Err(RecvTimeoutError::Timeout) => {
                        due = None;
                        self.tell();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match rx.recv() {
                    Ok(m) => m,
                    Err(_) => break,
                },
            };
            match m {
                Msg::Flush(done) => {
                    // Everything queued by now, the `anchored` entries an anchor queued included, is handled first.
                    let mut stop = false;
                    let mut waiting = vec![done];
                    while let Ok(m) = rx.try_recv() {
                        match m {
                            Msg::Flush(d) => waiting.push(d),
                            Msg::Stop => stop = true,
                            m => self.handle(&mut file, &mut due, m),
                        }
                    }
                    if due.take().is_some() {
                        self.tell();
                    }
                    for d in waiting {
                        let _ = d.send(());
                    }
                    if stop {
                        break;
                    }
                }
                Msg::Stop => {
                    if due.is_some() {
                        self.tell();
                    }
                    break;
                }
                m => self.handle(&mut file, &mut due, m),
            }
        }
    }

    fn handle(&self, file: &mut Option<std::fs::File>, due: &mut Option<Instant>, m: Msg) {
        match m {
            Msg::Write { entry, critical } => {
                let had_error = crate::lock(&self.io_error).is_some();
                if self.write(file, &entry, critical) {
                    if critical {
                        *due = None;
                        self.tell();
                    } else {
                        due.get_or_insert_with(|| Instant::now() + COALESCE);
                    }
                } else if !had_error {
                    // The log just stopped: tell it now, with whatever was written before, so the UI shows it.
                    *due = None;
                    self.tell_state(true);
                }
            }
            Msg::Anchor => self.anchor(file),
            Msg::Flush(_) | Msg::Stop => {}
        }
    }

    /// Write one entry. Only the next seq is written, so a write error stops the file at the last good line.
    fn write(&self, file: &mut Option<std::fs::File>, e: &Entry, sync: bool) -> bool {
        if crate::lock(&self.io_error).is_some() {
            return false;
        }
        let res = (|| -> io::Result<()> {
            if file.is_none() {
                *file = Some(fsx::open_append(&self.path)?);
            }
            let f = file.as_mut().expect("opened");
            let mut line = serde_json::to_vec(e).map_err(io::Error::other)?;
            line.push(b'\n');
            f.write_all(&line)?;
            if sync {
                self.sync(f)?;
            }
            Ok(())
        })();
        match res {
            Ok(()) => {
                *crate::lock(&self.written) = e.seq;
                true
            }
            Err(err) => {
                *crate::lock(&self.io_error) = Some(format!("the run log stopped at seq {}: {err}", e.seq - 1));
                false
            }
        }
    }

    fn tell(&self) {
        self.tell_state(false);
    }

    /// Tell the UI what is newly written. `state_changed`: also tell when nothing new is written (the log just
    /// stopped), so a caught-up view hears it even if no later entry is ever written.
    fn tell_state(&self, state_changed: bool) {
        let seq = *crate::lock(&self.written);
        let mut told = crate::lock(&self.told);
        if seq <= *told && !state_changed {
            return;
        }
        let io_error = crate::lock(&self.io_error).clone();
        let notice = {
            let log = crate::lock(&self.log);
            // Only written entries: the log in memory may already hold later ones the writer hasn't reached.
            let room = usize::try_from(seq.saturating_sub(*told)).unwrap_or(usize::MAX);
            let entries = log.since(*told, NOTICE_MAX.min(room)).to_vec();
            Notice { run_id: log.run_id().to_string(), seq, after: *told, entries, io_error, broken: log.broken().cloned() }
        };
        *told = (*told).max(seq);
        drop(told);
        if let Some(n) = crate::lock(&self.notify).as_ref() {
            n(&notice);
        }
    }

    /// Everything before this message is written: sync it, send its head to the signed record, then log that.
    fn sync(&self, f: &std::fs::File) -> io::Result<()> {
        #[cfg(test)]
        if self.fail_sync.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(io::Error::other("sync failed (test)"));
        }
        f.sync_data()
    }

    fn anchor(&self, file: &mut Option<std::fs::File>) {
        let Some(record) = crate::lock(&self.record).clone() else { return };
        if crate::lock(&self.io_error).is_some() {
            *crate::lock(&self.skipped_anchors) += 1;
            return;
        }
        if let Some(f) = file.as_ref()
            && let Err(e) = self.sync(f)
        {
            // The durability barrier failed: the anchor is skipped, and that is shown, never silent (#222 review). The
            // log stops here, so finalization and verification see a write error, not a cleanly finished log.
            let written = *crate::lock(&self.written);
            *crate::lock(&self.io_error) = Some(format!("the run log couldn't be synced to anchor seq {written}: {e}"));
            *crate::lock(&self.skipped_anchors) += 1;
            self.tell_state(true);
            return;
        }
        let written = *crate::lock(&self.written);
        let (run, anchor) = {
            let log = crate::lock(&self.log);
            let Some(e) = written.checked_sub(1).and_then(|i| log.entries().get(i as usize)) else { return };
            (log.run_id().to_string(), runlog::Anchor { seq: e.seq, hash: e.hash.clone() })
        };
        record.send(Event::Progress(runlog::anchor_text(&run, &anchor)));
        // Kept in the app's memory for its life (F4): the anchors this session sent are the only ones it confirms.
        if let Some(f) = crate::lock(&self.on_anchor).as_ref() {
            f(&run, &anchor);
        }
        let hash = anchor.hash;
        // Queued behind anything appended since: the order on disk stays the log's.
        let mut log = crate::lock(&self.log);
        if let Ok(e) = log.append(Source::Core, Kind::Anchored { seq: written, hash }, super::state::now_ms()) {
            let entry = Box::new(e.clone());
            let _ = crate::lock(&self.tx).send(Msg::Write { entry, critical: false });
        }
    }
}
