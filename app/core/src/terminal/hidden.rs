//! What the core keeps for a pane nobody is looking at: the newest bytes in a fixed ring, how many were left out, and
//! whether the program is on the alternate screen. Nothing here talks to the UI or the PTY; `mod.rs` wires it in.

/// The newest output a hidden pane keeps (256 KiB). Older output is counted, not kept.
pub const RING: usize = 256 * 1024;

/// A fixed ring of the newest bytes. The buffer is allocated at the first byte and freed when the tail is taken, so a
/// pane that is never hidden costs nothing and the worst case is `RING` per hidden pane.
#[derive(Default)]
pub struct Ring {
    buf: Vec<u8>,
    head: usize,
    len: usize,
    dropped: u64,
}

/// What a hidden pane hands over when it is shown again.
#[derive(Debug, PartialEq, Eq)]
pub struct Tail {
    pub bytes: Vec<u8>,
    /// Bytes read while hidden that are not in `bytes`.
    pub skipped: u64,
}

impl Ring {
    /// Append `data`, forgetting the oldest bytes when it does not fit.
    pub fn push(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if self.buf.is_empty() {
            self.buf = vec![0; RING];
        }
        if data.len() >= RING {
            self.dropped += (self.len + data.len() - RING) as u64;
            self.buf.copy_from_slice(&data[data.len() - RING..]);
            self.head = 0;
            self.len = RING;
            return;
        }
        let over = (self.len + data.len()).saturating_sub(RING);
        if over > 0 {
            self.dropped += over as u64;
            self.head = (self.head + over) % RING;
            self.len -= over;
        }
        let at = (self.head + self.len) % RING;
        let first = data.len().min(RING - at);
        self.buf[at..at + first].copy_from_slice(&data[..first]);
        self.buf[..data.len() - first].copy_from_slice(&data[first..]);
        self.len += data.len();
    }

    /// Put a taken tail back (it could not be delivered), counting what it had left out. Only valid on an emptied ring.
    pub fn restore(&mut self, tail: Tail) {
        self.push(&tail.bytes);
        self.dropped += tail.skipped;
    }

    /// The kept bytes in order, and how many were left out; the ring is empty (and its memory freed) afterwards.
    ///
    /// When older bytes were dropped the tail starts after the first newline, so it never begins in the middle of a
    /// line. If that would leave nothing (no newline, or only a final one: a progress bar that rewrites one line with
    /// CR, or binary) the tail starts at the first kept byte instead, since sending nothing would be worse.
    pub fn take(&mut self) -> Tail {
        let mut bytes = Vec::with_capacity(self.len);
        if self.len > 0 {
            let first = self.len.min(RING - self.head);
            bytes.extend_from_slice(&self.buf[self.head..self.head + first]);
            bytes.extend_from_slice(&self.buf[..self.len - first]);
        }
        let mut skipped = self.dropped;
        if self.dropped > 0 {
            if let Some(i) = bytes.iter().position(|b| *b == b'\n').filter(|i| i + 1 < bytes.len()) {
                bytes.drain(..=i);
                skipped += (i + 1) as u64;
            }
        }
        *self = Self::default();
        Tail { bytes, skipped }
    }
}

/// Follows whether the program is on the alternate screen (`ESC [ ? 1049 h`, `1047`, or `47`, and `l` to leave) in
/// bytes it is fed, including a sequence split across two reads.
#[derive(Default)]
pub struct AltTrack {
    pub on: bool,
    carry: [u8; CARRY],
    carried: usize,
}

const CARRY: usize = 7; // the longest sequence is 8 bytes; a split one has at most 7 in the first piece
const SEQS: [(&[u8], bool); 6] = [
    (b"\x1b[?1049h", true),
    (b"\x1b[?1049l", false),
    (b"\x1b[?1047h", true),
    (b"\x1b[?1047l", false),
    (b"\x1b[?47h", true),
    (b"\x1b[?47l", false),
];

impl AltTrack {
    pub fn new(on: bool) -> Self {
        Self { on, ..Self::default() }
    }

    pub fn feed(&mut self, data: &[u8]) {
        // A sequence that began in the previous read: look at the carried bytes plus the first few of this read. One that
        // lies wholly inside the carry was seen when it arrived.
        if self.carried > 0 {
            let mut w = [0u8; CARRY + 8];
            w[..self.carried].copy_from_slice(&self.carry[..self.carried]);
            let take = data.len().min(8);
            w[self.carried..self.carried + take].copy_from_slice(&data[..take]);
            let w = &w[..self.carried + take];
            for i in 0..self.carried {
                if let Some((on, n)) = seq_at(&w[i..]) {
                    if i + n > self.carried {
                        self.on = on;
                    }
                }
            }
        }
        let mut i = 0;
        while let Some(p) = data[i..].iter().position(|b| *b == 0x1b) {
            let at = i + p;
            if let Some((on, _)) = seq_at(&data[at..]) {
                self.on = on;
            }
            i = at + 1;
        }
        // Keep the last bytes of the stream, in case a sequence ends in the next read.
        if data.len() >= CARRY {
            self.carry.copy_from_slice(&data[data.len() - CARRY..]);
            self.carried = CARRY;
        } else {
            let mut all = [0u8; CARRY * 2];
            all[..self.carried].copy_from_slice(&self.carry[..self.carried]);
            all[self.carried..self.carried + data.len()].copy_from_slice(data);
            let n = self.carried + data.len();
            let keep = n.min(CARRY);
            self.carry[..keep].copy_from_slice(&all[n - keep..n]);
            self.carried = keep;
        }
    }
}

fn seq_at(d: &[u8]) -> Option<(bool, usize)> {
    SEQS.iter().find(|(s, _)| d.starts_with(s)).map(|(s, on)| (*on, s.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Vec<u8> {
        (0..n).flat_map(|i| format!("line {i:06}\r\n").into_bytes()).collect()
    }

    #[test]
    fn output_that_fits_is_kept_whole_and_nothing_is_skipped() {
        let mut r = Ring::default();
        r.push(b"hello ");
        r.push(b"world\r\n");
        assert_eq!(r.take(), Tail { bytes: b"hello world\r\n".to_vec(), skipped: 0 });
        assert!(r.len == 0 && r.dropped == 0);
    }

    #[test]
    fn past_the_limit_only_the_newest_bytes_stay_and_the_count_adds_up() {
        let mut r = Ring::default();
        let all = lines(60_000); // 14 bytes a line, about 840 KB
        for c in all.chunks(1000) {
            r.push(c);
        }
        let t = r.take();
        assert!(t.bytes.len() <= RING && t.bytes.len() > RING - 20, "{}", t.bytes.len());
        assert_eq!(t.skipped + t.bytes.len() as u64, all.len() as u64, "kept + skipped is everything");
        assert!(all.ends_with(&t.bytes), "the kept bytes are the newest ones, in order");
        assert!(t.bytes.starts_with(b"line "), "the tail starts on a whole line");
    }

    #[test]
    fn one_write_bigger_than_the_ring_keeps_its_end() {
        let mut r = Ring::default();
        r.push(b"old\n");
        let big = lines(40_000);
        r.push(&big);
        let t = r.take();
        assert_eq!(t.skipped + t.bytes.len() as u64, big.len() as u64 + 4);
        assert!(big.ends_with(&t.bytes));
    }

    #[test]
    fn a_tail_without_a_newline_starts_at_the_first_kept_byte() {
        let mut r = Ring::default();
        for _ in 0..40 {
            r.push(&vec![b'x'; 10_000]); // 400 KB of one line, like a progress bar rewriting itself
        }
        let t = r.take();
        assert_eq!(t.bytes.len(), RING, "sent from the start of the ring, not dropped");
        assert_eq!(t.skipped, 400_000 - RING as u64);
    }

    #[test]
    fn a_final_newline_alone_does_not_empty_the_tail() {
        let mut r = Ring::default();
        r.push(&vec![b'y'; RING + 10]);
        r.push(b"\n");
        let t = r.take();
        assert_eq!(t.bytes.len(), RING, "kept whole rather than trimmed to nothing");
        assert!(t.bytes.ends_with(b"\n"));
    }

    #[test]
    fn a_restored_tail_comes_out_again_with_its_count() {
        let mut r = Ring::default();
        r.push(&vec![b'x'; RING + 5]);
        r.push(b"\nlast\n");
        let first = r.take();
        let copy = Tail { bytes: first.bytes.clone(), skipped: first.skipped };
        r.restore(first);
        assert_eq!(r.take(), copy);
    }

    #[test]
    fn taking_empties_the_ring_and_frees_it() {
        let mut r = Ring::default();
        r.push(b"abc\n");
        let _ = r.take();
        assert!(r.len == 0 && r.dropped == 0 && r.buf.is_empty());
        assert_eq!(r.take(), Tail { bytes: vec![], skipped: 0 });
    }

    fn alt(chunks: &[&[u8]], start: bool) -> bool {
        let mut a = AltTrack::new(start);
        for c in chunks {
            a.feed(c);
        }
        a.on
    }

    #[test]
    fn the_alternate_screen_is_followed_through_enter_and_leave() {
        assert!(alt(&[b"x\x1b[?1049hy"], false));
        assert!(!alt(&[b"\x1b[?1049h", b"draw", b"\x1b[?1049l"], false));
        assert!(alt(&[b"\x1b[?47h"], false) && alt(&[b"\x1b[?1047h"], false));
        assert!(!alt(&[b"\x1b[?1047l"], true));
        assert!(alt(&[b"\x1b[31mred\x1b[0m"], true), "other sequences change nothing");
        assert!(!alt(&[b"plain"], false));
    }

    #[test]
    fn a_sequence_split_across_reads_is_still_seen() {
        for cut in 1..8 {
            let s = b"\x1b[?1049h";
            assert!(alt(&[&s[..cut], &s[cut..]], false), "enter split at {cut}");
            let l = b"\x1b[?1049l";
            assert!(!alt(&[&l[..cut], &l[cut..]], true), "leave split at {cut}");
        }
        assert!(alt(&[b"abc\x1b[?10", b"49hdef"], false));
        assert!(alt(&[b"\x1b", b"[", b"?", b"1049", b"h"], false), "even a byte at a time");
    }

    #[test]
    fn a_split_enter_then_a_whole_leave_ends_off() {
        assert!(!alt(&[b"\x1b[?10", b"49h\x1b[?1049l"], false));
    }
}
