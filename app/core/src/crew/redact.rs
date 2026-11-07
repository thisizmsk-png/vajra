//! Agent text cleaning: the C3 ANSI sanitizer, then every remaining escape
//! and invisible format character is dropped (stream lines are plain text),
//! then the steer nonce is replaced. Stripping first means an escape or a
//! zero-width char inside the nonce cannot smuggle it past the replace.

use crate::sessions::ansi;

pub const REDACTED: &str = "[redacted]";
/// A nonce shorter than this is not a real C6 nonce; replacing it would
/// mangle ordinary text.
const MIN_NONCE: usize = 8;

#[derive(Clone, Debug, Default)]
pub struct Redactor {
    /// Every nonce this text may carry (a resume rotates it; old lines keep the old one).
    nonces: Vec<String>,
}

impl Redactor {
    pub fn new(nonce: Option<&str>) -> Self {
        let mut r = Self::default();
        r.add(nonce);
        r
    }

    pub fn add(&mut self, nonce: Option<&str>) {
        if let Some(n) = nonce.filter(|n| n.len() >= MIN_NONCE && !self.nonces.iter().any(|x| x == n)) {
            self.nonces.push(n.to_string());
        }
    }

    /// Take on every nonce `other` knows. A redactor only ever gains nonces.
    pub fn absorb(&mut self, other: &Redactor) {
        for n in &other.nonces {
            self.add(Some(n));
        }
    }

    /// One line of agent text in, one plain-text line out.
    pub fn clean(&self, text: &str) -> String {
        self.redact(strip(&ansi::sanitize(text)))
    }

    /// Only the nonce replacement (whole, and a fragment a terminal wrap left
    /// at either end), for text that keeps its colours: the PTY view. The terminal still shows a nonce that an SGR
    /// sequence or an invisible character splits, so the match runs on the visible text.
    pub fn redact(&self, s: String) -> String {
        self.nonces.iter().fold(s, |s, n| {
            if s.contains('\u{1b}') || s.chars().any(is_format) {
                redact_keeping_escapes(&s, n)
            } else {
                redact_wrapped(s.replace(n.as_str(), REDACTED), n)
            }
        })
    }
}

/// The nonce replaced in text that keeps its escapes. Escape sequences and invisible format characters are skipped
/// when matching (they are not drawn), the matched visible characters are dropped from the original and replaced by
/// one marker per match, and the escapes inside a match are kept (they carry colour only), so the output has no
/// nonce characters left and colours do not change.
fn redact_keeping_escapes(s: &str, n: &str) -> String {
    // Every character of `s` is either visible (part of `plain`) or an escape/format character.
    let mut toks: Vec<(usize, usize, bool)> = Vec::new();
    let mut plain = String::with_capacity(s.len());
    let mut plain_at: Vec<usize> = Vec::new(); // byte offset in `plain` of each visible token
    let mut it = s.char_indices();
    while let Some((i, c)) = it.next() {
        if c == '\u{1b}' {
            let mut end = i + 1;
            for (j, d) in it.by_ref() {
                end = j + d.len_utf8();
                if ('\u{40}'..='\u{7e}').contains(&d) && d != '[' {
                    break;
                }
            }
            toks.push((i, end, false));
        } else if is_format(c) {
            toks.push((i, i + c.len_utf8(), false));
        } else {
            toks.push((i, i + c.len_utf8(), true));
            plain_at.push(plain.len());
            plain.push(c);
        }
    }
    let mut cut: Vec<(usize, usize)> = plain.match_indices(n).map(|(a, m)| (a, a + m.len())).collect();
    if let Some(k) = (MIN_NONCE..n.len()).rev().find(|&k| plain.ends_with(&n[..k])) {
        cut.push((plain.len() - k, plain.len()));
    }
    if let Some(k) = (MIN_NONCE..n.len()).rev().find(|&k| plain.starts_with(&n[n.len() - k..])) {
        cut.push((0, k));
    }
    if cut.is_empty() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let (mut v, mut in_cut) = (0, false);
    for &(a, b, visible) in &toks {
        if !visible {
            out.push_str(&s[a..b]);
            continue;
        }
        let p = plain_at[v];
        v += 1;
        if cut.iter().any(|&(x, y)| p >= x && p < y) {
            if !in_cut {
                out.push_str(REDACTED);
                in_cut = true;
            }
        } else {
            in_cut = false;
            out.push_str(&s[a..b]);
        }
    }
    out
}

/// A terminal wrap can cut the nonce across two lines: a prefix of at least
/// `MIN_NONCE` chars at the end of a line, or a suffix at its start, is
/// replaced too.
fn redact_wrapped(mut s: String, n: &str) -> String {
    if let Some(k) = (MIN_NONCE..n.len()).rev().find(|&k| s.ends_with(&n[..k])) {
        s.truncate(s.len() - k);
        s.push_str(REDACTED);
    }
    if let Some(k) = (MIN_NONCE..n.len()).rev().find(|&k| s.starts_with(&n[n.len() - k..])) {
        s.replace_range(..k, REDACTED);
    }
    s
}

/// Drop the CSI sequences `ansi::sanitize` keeps (SGR, cursor) and format
/// characters (zero-width, bidi controls, soft hyphen, BOM).
fn strip(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // sanitize only leaves ESC '[' params final.
            for d in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&d) && d != '[' {
                    break;
                }
            }
            continue;
        }
        if is_format(c) {
            continue;
        }
        out.push(c);
    }
    out
}

/// One list of invisible characters for the whole core (tags, variation selectors, bidi, zero-width, separators, ...).
fn is_format(c: char) -> bool {
    crate::control::lead::invisible(c)
}
