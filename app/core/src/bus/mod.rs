//! C2 event bus.
//!
//! Each stream (a PTY or a transcript) has a ring of recent lines with a byte
//! budget and a monotonic sequence number. Lines that fall off the ring are
//! appended to `spill/<stream>.log` (mode 0600). The spill dir has a total
//! size cap; past it, the least recently written stream's spill file goes.
//!
//! Subscribers get a bounded queue each. A producer never waits on a reader:
//! when a queue is full it is replaced by one `SnapshotNeeded` marker and
//! further events are dropped until the reader takes the marker and pulls a
//! `snapshot`. The renderer itself polls `tail` (pull, not push).

use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Synthetic load producer for tests and memory measurement (t8). Compiled
/// in all builds so integration tests and the bin can use it; nothing in the
/// app calls it.
pub mod synthetic;

#[cfg(test)]
mod tests;

/// Ring byte budget per stream.
pub const RING_BYTES: usize = 256 * 1024;
/// Total size cap of the spill dir.
pub const SPILL_CAP_BYTES: u64 = 1024 * 1024 * 1024;
/// Events queued per subscriber before it is dropped to a snapshot.
pub const QUEUE_CAP: usize = 1024;
/// Longer lines are truncated (at a char boundary) on push.
/// One index entry per this many lines (by seq).
const INDEX_EVERY: u64 = 128;
/// Most line text one `read_spill_before` call returns (the lines nearest the target are kept; fewer if they are long).
const BEFORE_BUDGET_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_LINE_BYTES: usize = 64 * 1024;
/// Bookkeeping charged per line on top of its text, so many tiny lines are
/// still bounded by the budget.
pub const LINE_OVERHEAD: usize = 32;
/// Longest accepted stream id.
pub const MAX_STREAM_ID: usize = 128;

#[derive(Clone, Debug)]
pub struct Config {
    pub ring_bytes: usize,
    pub spill_cap_bytes: u64,
    pub queue_cap: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ring_bytes: RING_BYTES,
            spill_cap_bytes: SPILL_CAP_BYTES,
            queue_cap: QUEUE_CAP,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub seq: u64,
    pub text: Arc<str>,
}

impl Line {
    fn cost(&self) -> usize {
        self.text.len() + LINE_OVERHEAD
    }
}

/// Result of `tail`.
#[derive(Debug, Default)]
pub struct Tail {
    /// Lines with `seq > after_seq` still in the ring, oldest first.
    pub lines: Vec<Line>,
    /// True when lines after `after_seq` already fell off the ring (they are
    /// in spill, or were dropped).
    pub gap: bool,
    /// Newest seq of the stream (0 when empty).
    pub head_seq: u64,
    /// Logical spill offsets for `read_spill`: [spill_start, spill_end).
    pub spill_start: u64,
    pub spill_end: u64,
}

/// Result of `read_spill_before`.
#[derive(Debug, Default)]
pub struct SpillBefore {
    /// Contiguous lines ending at the line just before the requested seq, oldest first.
    pub lines: Vec<Line>,
    /// True when no older line exists (the first line returned, or none, is the oldest the core still holds).
    pub at_start: bool,
    /// True when the spill was evicted while reading.
    pub gap: bool,
}

/// Result of `window`: the lines a reader is showing, and where they sit in the stream.
#[derive(Debug, Default)]
pub struct Window {
    /// At most the requested number of lines, oldest first.
    pub lines: Vec<Line>,
    /// Newest seq of the stream (0 when empty).
    pub head_seq: u64,
    /// Oldest seq the core still holds (spill or ring); 0 when the stream is empty. `head_seq - first_seq + 1` is the
    /// number of rows a scrollbar should stand for, unless `gap` says lines inside were lost.
    pub first_seq: u64,
    /// No older line is held than the first one returned.
    pub at_start: bool,
    /// The last line returned is the newest.
    pub at_end: bool,
    /// Lines the reader asked for were lost: the requested start was older than `first_seq`, the spill was evicted
    /// while reading, or there is a hole between the spill and the ring. Retry or mark the hole.
    pub gap: bool,
}

/// Result of `read_spill`.
#[derive(Debug, Default)]
pub struct SpillPage {
    pub lines: Vec<Line>,
    /// Offset to pass to the next `read_spill` call.
    pub next_offset: u64,
    /// True when the requested offset was evicted by the spill cap; reading
    /// restarted at the oldest spill still on disk.
    pub gap: bool,
    /// True when `next_offset` is the end of the spill.
    pub end: bool,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub lines: Vec<Line>,
    pub head_seq: u64,
    pub total_lines: u64,
    pub total_bytes: u64,
    /// Lines evicted from the ring that could not be spilled (disk error).
    pub spill_dropped: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Line {
        stream: Arc<str>,
        line: Line,
    },
    /// The queue overflowed; events were dropped. Pull a `snapshot`.
    SnapshotNeeded,
}

/// Memory and disk counters, for health checks and tests.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub streams: usize,
    pub ring_bytes: usize,
    pub ring_lines: usize,
    pub spill_bytes: u64,
    pub spill_dropped: u64,
    pub subscribers: usize,
    pub queued_events: usize,
}

#[derive(Debug)]
pub enum BusError {
    /// Stream ids must be 1..=128 bytes of `[A-Za-z0-9._-]`, not starting
    /// with a dot (they become spill file names).
    InvalidStream,
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid stream id")
    }
}

impl std::error::Error for BusError {}

pub fn valid_stream_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_STREAM_ID
        && !id.starts_with('.')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

fn truncate(text: &str) -> &str {
    if text.len() <= MAX_LINE_BYTES {
        return text;
    }
    let mut end = MAX_LINE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

struct Stream {
    ring: VecDeque<Line>,
    ring_bytes: usize,
    next_seq: u64,
    total_bytes: u64,
    spill: Option<File>,
    /// Logical offset of the current spill file's first byte.
    spill_base: u64,
    /// Physical length of the current spill file.
    spill_len: u64,
    /// `(seq, logical offset)` of every `INDEX_EVERY`th line in the current spill file: lets a reader that wants the
    /// lines just before some seq start near them instead of at the start of the file.
    spill_index: Vec<(u64, u64)>,
    /// Seq of the first line in the current spill file (0 while the file is empty).
    spill_first: u64,
    spill_disabled: bool,
    spill_dropped: u64,
    last_used: u64,
}

struct Inner {
    streams: HashMap<Arc<str>, Stream>,
    /// Spill files left by a previous process: (path, len). Evicted first.
    orphans: Vec<(PathBuf, u64)>,
    spill_total: u64,
    tick: u64,
    subs: Vec<Arc<SubQueue>>,
}

struct SubQueue {
    state: Mutex<SubState>,
    ready: Condvar,
}

struct SubState {
    events: VecDeque<Event>,
    overflowed: bool,
}

pub struct Bus {
    dir: PathBuf,
    cfg: Config,
    inner: Mutex<Inner>,
}

/// A bounded event queue. Dropping it unsubscribes.
pub struct Subscriber {
    q: Arc<SubQueue>,
}

use crate::lock;

fn open_spill(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    // Never follow a link and never truncate in open(): a planted symlink or hard link at this name must not make
    // the app destroy another file. Open (create if absent), check it is a private single-link regular file, then
    // truncate: a file with this name is from a previous process, whose seqs do not match this one's.
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let md = file.metadata()?;
    if !md.is_file() || md.nlink() != 1 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "spill path is not a plain single-link file"));
    }
    file.set_len(0)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

fn encode(line: &Line, out: &mut Vec<u8>) {
    // `<seq> <len>\n<text>\n`: the length prefix keeps text with newlines intact.
    out.extend_from_slice(format!("{} {}\n", line.seq, line.text.len()).as_bytes());
    out.extend_from_slice(line.text.as_bytes());
    out.push(b'\n');
}

/// Parse one record at the start of `buf`: (line, bytes consumed). `Ok(None)`
/// means the record is incomplete.
fn decode(buf: &[u8]) -> io::Result<Option<(Line, usize)>> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "not a spill record boundary");
    let Some(nl) = buf.iter().take(48).position(|&b| b == b'\n') else {
        return if buf.len() < 48 { Ok(None) } else { Err(bad()) };
    };
    let header = std::str::from_utf8(&buf[..nl]).map_err(|_| bad())?;
    let (seq, len) = header.split_once(' ').ok_or_else(bad)?;
    let seq: u64 = seq.parse().map_err(|_| bad())?;
    let len: usize = len.parse().map_err(|_| bad())?;
    if len > MAX_LINE_BYTES {
        return Err(bad());
    }
    let start = nl + 1;
    if buf.len() < start + len + 1 {
        return Ok(None);
    }
    if buf[start + len] != b'\n' {
        return Err(bad());
    }
    let text = String::from_utf8_lossy(&buf[start..start + len]);
    Ok(Some((
        Line {
            seq,
            text: Arc::from(text.as_ref()),
        },
        start + len + 1,
    )))
}

impl Bus {
    /// Open a bus spilling into `spill_dir` (created at 0700). Spill files
    /// already there count toward the cap and are evicted first.
    pub fn open(spill_dir: &Path, cfg: Config) -> io::Result<Self> {
        crate::paths::ensure_private_dir(spill_dir)?;
        let mut orphans = Vec::new();
        let mut spill_total = 0;
        for entry in std::fs::read_dir(spill_dir)? {
            let entry = entry?;
            let path = entry.path();
            let meta = entry.metadata()?;
            if meta.is_file() && path.extension().is_some_and(|e| e == "log") {
                spill_total += meta.len();
                orphans.push((path, meta.len()));
            }
        }
        Ok(Self {
            dir: spill_dir.to_path_buf(),
            cfg,
            inner: Mutex::new(Inner {
                streams: HashMap::new(),
                orphans,
                spill_total,
                tick: 0,
                subs: Vec::new(),
            }),
        })
    }

    fn spill_path(&self, stream: &str) -> PathBuf {
        self.dir.join(format!("{stream}.log"))
    }

    /// Append a line and return its seq. Never waits on a subscriber.
    pub fn push(&self, stream: &str, text: &str) -> Result<u64, BusError> {
        if !valid_stream_id(stream) {
            return Err(BusError::InvalidStream);
        }
        let text: Arc<str> = Arc::from(truncate(text));
        let mut guard = lock(&self.inner);
        let inner = &mut *guard;
        inner.tick += 1;
        let tick = inner.tick;

        let (key, line, evicted) = {
            if !inner.streams.contains_key(stream) {
                let path = self.spill_path(stream);
                if let Some(i) = inner.orphans.iter().position(|(p, _)| *p == path) {
                    let (_, len) = inner.orphans.swap_remove(i);
                    inner.spill_total -= len;
                }
                inner.streams.insert(
                    Arc::from(stream),
                    Stream {
                        ring: VecDeque::new(),
                        ring_bytes: 0,
                        next_seq: 1,
                        total_bytes: 0,
                        spill: None,
                        spill_base: 0,
                        spill_len: 0,
                        spill_index: Vec::new(),
                        spill_first: 0,
                        spill_disabled: false,
                        spill_dropped: 0,
                        last_used: 0,
                    },
                );
            }
            let key = inner
                .streams
                .get_key_value(stream)
                .expect("inserted")
                .0
                .clone();
            let s = inner.streams.get_mut(stream).expect("inserted");
            let line = Line {
                seq: s.next_seq,
                text,
            };
            s.next_seq += 1;
            s.total_bytes += line.text.len() as u64;
            s.last_used = tick;
            s.ring_bytes += line.cost();
            s.ring.push_back(line.clone());
            let mut evicted = Vec::new();
            while s.ring_bytes > self.cfg.ring_bytes && s.ring.len() > 1 {
                let old = s.ring.pop_front().expect("len > 1");
                s.ring_bytes -= old.cost();
                evicted.push(old);
            }
            (key, line, evicted)
        };

        if !evicted.is_empty() {
            self.spill(inner, &key, &evicted);
        }

        inner.subs.retain(|q| Arc::strong_count(q) > 1);
        for q in &inner.subs {
            q.offer(
                Event::Line {
                    stream: key.clone(),
                    line: line.clone(),
                },
                self.cfg.queue_cap,
            );
        }
        Ok(line.seq)
    }

    fn spill(&self, inner: &mut Inner, key: &Arc<str>, lines: &[Line]) {
        let path = self.spill_path(key);
        let s = inner.streams.get_mut(key).expect("stream exists");
        if s.spill_disabled {
            s.spill_dropped += lines.len() as u64;
            return;
        }
        let mut buf = Vec::new();
        let mut marks: Vec<(u64, u64)> = Vec::new();
        for line in lines {
            if line.seq % INDEX_EVERY == 0 {
                marks.push((line.seq, s.spill_base + s.spill_len + buf.len() as u64));
            }
            encode(line, &mut buf);
        }
        let wrote = (|| -> io::Result<()> {
            if s.spill.is_none() {
                s.spill = Some(open_spill(&path)?);
            }
            s.spill.as_mut().expect("opened").write_all(&buf)
        })();
        match wrote {
            Ok(()) => {
                if s.spill_len == 0 {
                    s.spill_first = lines.first().map_or(0, |l| l.seq);
                }
                s.spill_len += buf.len() as u64;
                s.spill_index.extend(marks);
                inner.spill_total += buf.len() as u64;
            }
            Err(_) => {
                // ENOSPC and friends: keep the ring, stop spilling this stream.
                s.spill_disabled = true;
                s.spill_dropped += lines.len() as u64;
                s.spill = None;
            }
        }
        self.enforce_cap(inner);
    }

    fn enforce_cap(&self, inner: &mut Inner) {
        while inner.spill_total > self.cfg.spill_cap_bytes {
            if let Some((path, len)) = inner.orphans.pop() {
                let _ = std::fs::remove_file(path);
                inner.spill_total -= len;
                continue;
            }
            let Some(key) = inner
                .streams
                .iter()
                .filter(|(_, s)| s.spill_len > 0)
                .min_by_key(|(_, s)| s.last_used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            let path = self.spill_path(&key);
            let s = inner.streams.get_mut(&key).expect("found");
            s.spill = None;
            let _ = std::fs::remove_file(path);
            s.spill_base += s.spill_len;
            inner.spill_total -= s.spill_len;
            s.spill_len = 0;
            s.spill_first = 0;
            s.spill_index.clear();
        }
    }

    /// Lines with `seq > after_seq` from the ring, up to `max_bytes` of text
    /// (always at least one line, so a reader makes progress).
    pub fn tail(&self, stream: &str, after_seq: u64, max_bytes: usize) -> Tail {
        let inner = lock(&self.inner);
        let Some(s) = inner.streams.get(stream) else {
            return Tail::default();
        };
        let mut tail = Tail {
            head_seq: s.next_seq - 1,
            spill_start: s.spill_base,
            spill_end: s.spill_base + s.spill_len,
            ..Tail::default()
        };
        let Some(first) = s.ring.front().map(|l| l.seq) else {
            return tail;
        };
        tail.gap = after_seq.saturating_add(1) < first;
        let skip = after_seq.saturating_add(1).saturating_sub(first);
        let mut bytes = 0;
        for line in s
            .ring
            .iter()
            .skip(usize::try_from(skip).unwrap_or(usize::MAX))
        {
            if !tail.lines.is_empty() && bytes + line.text.len() > max_bytes {
                break;
            }
            bytes += line.text.len();
            tail.lines.push(line.clone());
        }
        tail
    }

    /// The newest lines of the ring that fit in `max_bytes` of text, oldest first (always at least one line when the
    /// ring is not empty). `tail` walks forward from the oldest retained line; an inspector wants the other end.
    pub fn recent(&self, stream: &str, max_bytes: usize) -> Vec<Line> {
        let inner = lock(&self.inner);
        let Some(s) = inner.streams.get(stream) else {
            return Vec::new();
        };
        let mut bytes = 0;
        let mut out: Vec<Line> = Vec::new();
        for line in s.ring.iter().rev() {
            if !out.is_empty() && bytes + line.text.len() > max_bytes {
                break;
            }
            bytes += line.text.len();
            out.push(line.clone());
        }
        out.reverse();
        out
    }

    /// Read spilled lines from logical `offset` (start with `Tail::spill_start`),
    /// up to `max_bytes` of text (at least one line when any remain).
    pub fn read_spill(&self, stream: &str, offset: u64, max_bytes: usize) -> io::Result<SpillPage> {
        let (base, len) = {
            let inner = lock(&self.inner);
            match inner.streams.get(stream) {
                Some(s) => (s.spill_base, s.spill_len),
                None => {
                    return Ok(SpillPage {
                        end: true,
                        ..SpillPage::default()
                    });
                }
            }
        };
        let gap = offset < base;
        let start = offset.max(base);
        let end = base + len;
        if start >= end {
            return Ok(SpillPage {
                next_offset: end,
                gap,
                end: true,
                ..SpillPage::default()
            });
        }
        let want = (max_bytes.max(MAX_LINE_BYTES) + 64) as u64;
        let mut buf = Vec::new();
        {
            let mut file = File::open(self.spill_path(stream))?;
            file.seek(SeekFrom::Start(start - base))?;
            file.take(want.min(end - start)).read_to_end(&mut buf)?;
        }
        let mut page = SpillPage {
            gap,
            ..SpillPage::default()
        };
        let mut pos = 0;
        let mut bytes = 0;
        while pos < buf.len() {
            let Some((line, used)) = decode(&buf[pos..])? else {
                break;
            };
            if !page.lines.is_empty() && bytes + line.text.len() > max_bytes {
                break;
            }
            bytes += line.text.len();
            pos += used;
            page.lines.push(line);
        }
        // The file may have been evicted (and restarted) while we read.
        let base_now = lock(&self.inner)
            .streams
            .get(stream)
            .map_or(base, |s| s.spill_base);
        if base_now != base {
            return Ok(SpillPage {
                next_offset: base_now,
                gap: true,
                ..SpillPage::default()
            });
        }
        page.next_offset = start + pos as u64;
        page.end = page.next_offset >= end;
        Ok(page)
    }

    /// Up to `max_lines` lines immediately before `before_seq`, oldest first, from the spill and (for the newest part)
    /// the in-memory ring, so they are adjacent to a reader whose window starts at `before_seq`. Starts at the nearest
    /// index entry at or before `before_seq - max_lines`, so the cost is about `max_lines` lines, not the spill's size.
    pub fn read_spill_before(&self, stream: &str, before_seq: u64, max_lines: usize) -> io::Result<SpillBefore> {
        self.read_spill_before_with(stream, before_seq, max_lines, &|| {})
    }

    /// `between` runs after the ring and index were captured and before the spill is read (tests force an eviction there).
    pub(crate) fn read_spill_before_with(&self, stream: &str, before_seq: u64, max_lines: usize, between: &dyn Fn()) -> io::Result<SpillBefore> {
        let max_lines = max_lines.max(1);
        let (base, start, ring): (u64, u64, Vec<Line>) = {
            let inner = lock(&self.inner);
            let Some(s) = inner.streams.get(stream) else {
                return Ok(SpillBefore { at_start: true, ..SpillBefore::default() });
            };
            let want = before_seq.saturating_sub(max_lines as u64 + 1);
            let start = s.spill_index.iter().rev().find(|(q, _)| *q <= want).map_or(s.spill_base, |(_, o)| *o);
            let ring = s.ring.iter().filter(|l| l.seq < before_seq).cloned().collect();
            (s.spill_base, start, ring)
        };
        // Keep the lines nearest `before_seq`: when their text outgrows the byte budget the OLDEST are dropped (never the
        // newest), so the result is always adjacent to the reader's window, with fewer lines if they are long.
        between();
        let mut lines: std::collections::VecDeque<Line> = std::collections::VecDeque::new();
        let mut held = 0usize;
        let mut trimmed = false;
        let mut offset = start;
        loop {
            let page = self.read_spill(stream, offset, 64 * 1024)?;
            if page.gap && offset != base {
                // The spill was evicted while we read. The lines the ring still holds are real and adjacent to the
                // window: return them (the reader marks the hole), and say `gap` so it knows to retry.
                let skip = ring.len().saturating_sub(max_lines);
                return Ok(SpillBefore { lines: ring.into_iter().skip(skip).collect(), at_start: false, gap: true });
            }
            let mut reached = false;
            for l in page.lines {
                if l.seq >= before_seq {
                    reached = true;
                    break;
                }
                held += l.text.len();
                lines.push_back(l);
                while held > BEFORE_BUDGET_BYTES && lines.len() > 1 {
                    held -= lines.pop_front().map_or(0, |o| o.text.len());
                    trimmed = true;
                }
            }
            if reached || page.end {
                break;
            }
            offset = page.next_offset;
        }
        // The ring continues where the spill ends.
        let last = lines.back().map_or(0, |l| l.seq);
        for l in ring.into_iter().filter(|l| l.seq > last) {
            held += l.text.len();
            lines.push_back(l);
        }
        while held > BEFORE_BUDGET_BYTES && lines.len() > 1 {
            held -= lines.pop_front().map_or(0, |o| o.text.len());
            trimmed = true;
        }
        // Nothing older exists only if we began at the first byte of the spill and nothing was cut off the front.
        let began_at_start = start == base;
        let at_start = began_at_start && !trimmed && lines.len() <= max_lines;
        let mut lines: Vec<Line> = lines.into();
        if lines.len() > max_lines {
            lines.drain(..lines.len() - max_lines);
        }
        Ok(SpillBefore { lines, at_start, gap: false })
    }

    /// The rows a reader is showing, and nothing else (#45): up to `max_lines` lines (and about `max_bytes` of text, at
    /// least one line) starting at seq `from`, or the newest ones when `from` is `None`. Lines come from the spill and
    /// the ring alike, so the reader never has to hold the stream: it asks for the window it is looking at. Memory is
    /// bounded by the request, and the spill is read from the nearest index entry, so the cost is about the window.
    pub fn window(&self, stream: &str, from: Option<u64>, max_lines: usize, max_bytes: usize) -> io::Result<Window> {
        self.window_with(stream, from, max_lines, max_bytes, &|| {})
    }

    /// `between` runs after the bounds were captured and before the spill is read (tests force an eviction there).
    pub(crate) fn window_with(&self, stream: &str, from: Option<u64>, max_lines: usize, max_bytes: usize, between: &dyn Fn()) -> io::Result<Window> {
        let max_lines = max_lines.max(1);
        let (head, first, ring_front, ring, start): (u64, u64, u64, Vec<Line>, Option<u64>) = {
            let inner = lock(&self.inner);
            let Some(s) = inner.streams.get(stream) else {
                return Ok(Window { at_start: true, at_end: true, ..Window::default() });
            };
            let head = s.next_seq - 1;
            if head == 0 {
                return Ok(Window { at_start: true, at_end: true, ..Window::default() });
            }
            let ring_front = s.ring.front().map_or(s.next_seq, |l| l.seq);
            let first = if s.spill_len > 0 { s.spill_first.max(1) } else { ring_front };
            let want = match from {
                Some(f) => f.max(first),
                None => (head + 1).saturating_sub(max_lines as u64).max(first),
            };
            let ring = s.ring.iter().filter(|l| l.seq >= want).take(max_lines).cloned().collect();
            let start = (want < ring_front && s.spill_len > 0).then(|| s.spill_index.iter().rev().find(|(q, _)| *q <= want).map_or(s.spill_base, |(_, o)| *o));
            (head, first, ring_front, ring, start)
        };
        let want = match from {
            Some(f) => f.max(first),
            None => (head + 1).saturating_sub(max_lines as u64).max(first),
        };
        let mut gap = from.is_some_and(|f| f < first);
        between();
        let mut lines: Vec<Line> = Vec::new();
        let mut held = 0usize;
        let mut budget_stop = false;
        if let Some(mut offset) = start {
            'spill: loop {
                let page = self.read_spill(stream, offset, 64 * 1024)?;
                if page.gap {
                    // The spill was evicted while we read (including the part at the captured base): the ring lines
                    // captured for this request are real, so return them, cut to the same bounds a normal answer
                    // obeys, and say `gap` so the reader retries from the new bounds.
                    let mut out = ring;
                    out.truncate(max_lines);
                    let out = fit_bytes(out, from.is_some(), max_bytes);
                    return Ok(Window { head_seq: head, first_seq: ring_front, at_start: false, at_end: out.last().is_none_or(|l| l.seq >= head), lines: out, gap: true });
                }
                for l in page.lines {
                    if l.seq < want {
                        continue;
                    }
                    if l.seq >= ring_front || lines.len() >= max_lines {
                        break 'spill;
                    }
                    // `from` reads forward: stop at the budget. A newest window trims the front after the ring joins.
                    if from.is_some() && !lines.is_empty() && held + l.text.len() > max_bytes {
                        // The budget ends the whole window: the ring must not be joined after a cut, or rows far
                        // from the spill's last row would follow it and read as a data-loss gap.
                        budget_stop = true;
                        break 'spill;
                    }
                    held += l.text.len();
                    lines.push(l);
                }
                if page.end {
                    break;
                }
                offset = page.next_offset;
            }
        }
        let last = lines.last().map(|l| l.seq);
        for l in ring {
            if budget_stop || lines.len() >= max_lines {
                break;
            }
            if last.is_some_and(|q| l.seq <= q) {
                continue;
            }
            if from.is_some() && !lines.is_empty() && held + l.text.len() > max_bytes {
                break;
            }
            held += l.text.len();
            lines.push(l);
        }
        if from.is_none() {
            // The newest lines within the budget: the oldest are dropped, never the newest.
            while held > max_bytes && lines.len() > 1 {
                held -= lines.remove(0).text.len();
            }
        }
        // A hole inside the window (the spill stopped before the ring started) is reported, not hidden.
        if lines.windows(2).any(|w| w[1].seq != w[0].seq + 1) {
            gap = true;
        }
        Ok(Window {
            head_seq: head,
            first_seq: first,
            at_start: lines.first().is_none_or(|l| l.seq <= first) && from.is_none_or(|_| !lines.is_empty()),
            at_end: lines.last().is_none_or(|l| l.seq >= head),
            lines,
            gap,
        })
    }

    /// The last `max_lines` ring lines plus counters, for a subscriber that
    /// got `SnapshotNeeded`.
    pub fn snapshot(&self, stream: &str, max_lines: usize) -> Option<Snapshot> {
        let inner = lock(&self.inner);
        let s = inner.streams.get(stream)?;
        let skip = s.ring.len().saturating_sub(max_lines);
        Some(Snapshot {
            lines: s.ring.iter().skip(skip).cloned().collect(),
            head_seq: s.next_seq - 1,
            total_lines: s.next_seq - 1,
            total_bytes: s.total_bytes,
            spill_dropped: s.spill_dropped,
        })
    }

    /// Stream ids, sorted.
    pub fn streams(&self) -> Vec<Arc<str>> {
        let mut ids: Vec<_> = lock(&self.inner).streams.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Forget a stream and delete its spill file (run deletion).
    pub fn remove_stream(&self, stream: &str) {
        let mut inner = lock(&self.inner);
        if let Some(s) = inner.streams.remove(stream) {
            inner.spill_total -= s.spill_len;
            drop(s.spill);
            let _ = std::fs::remove_file(self.spill_path(stream));
        }
    }

    pub fn subscribe(&self) -> Subscriber {
        let q = Arc::new(SubQueue {
            state: Mutex::new(SubState {
                events: VecDeque::new(),
                overflowed: false,
            }),
            ready: Condvar::new(),
        });
        lock(&self.inner).subs.push(q.clone());
        Subscriber { q }
    }

    pub fn stats(&self) -> Stats {
        let mut inner = lock(&self.inner);
        inner.subs.retain(|q| Arc::strong_count(q) > 1);
        let mut st = Stats {
            streams: inner.streams.len(),
            spill_bytes: inner.spill_total,
            subscribers: inner.subs.len(),
            ..Stats::default()
        };
        for s in inner.streams.values() {
            st.ring_bytes += s.ring_bytes;
            st.ring_lines += s.ring.len();
            st.spill_dropped += s.spill_dropped;
        }
        for q in &inner.subs {
            st.queued_events += lock(&q.state).events.len();
        }
        st
    }
}

impl SubQueue {
    fn offer(&self, event: Event, cap: usize) {
        let mut st = lock(&self.state);
        if st.overflowed {
            return;
        }
        if st.events.len() >= cap {
            st.events.clear();
            st.events.push_back(Event::SnapshotNeeded);
            st.overflowed = true;
        } else {
            st.events.push_back(event);
        }
        drop(st);
        self.ready.notify_one();
    }

    fn pop(st: &mut SubState) -> Option<Event> {
        let ev = st.events.pop_front()?;
        if ev == Event::SnapshotNeeded {
            st.overflowed = false;
        }
        Some(ev)
    }
}

impl Subscriber {
    pub fn try_recv(&self) -> Option<Event> {
        SubQueue::pop(&mut lock(&self.q.state))
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        let st = lock(&self.q.state);
        let (mut st, _) = self
            .q
            .ready
            .wait_timeout_while(st, timeout, |s| s.events.is_empty())
            .unwrap_or_else(|e| e.into_inner());
        SubQueue::pop(&mut st)
    }

    pub fn len(&self) -> usize {
        lock(&self.q.state).events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Cut `lines` to about `max_bytes` of text, keeping at least one: from the front when the window reads forward from a
/// seq (the newest are left out), from the back otherwise (the oldest are left out).
fn fit_bytes(mut lines: Vec<Line>, forward: bool, max_bytes: usize) -> Vec<Line> {
    let mut held: usize = lines.iter().map(|l| l.text.len()).sum();
    if forward {
        while held > max_bytes && lines.len() > 1 {
            held -= lines.pop().map_or(0, |l| l.text.len());
        }
    } else {
        while held > max_bytes && lines.len() > 1 {
            held -= lines.remove(0).text.len();
        }
    }
    lines
}
