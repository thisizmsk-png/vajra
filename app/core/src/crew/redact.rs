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
                redact_wrapped(replace_ci(&s, n), n)
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
    // Matched in lower case (ASCII only, so the byte offsets are the same as in `plain`): a nonce an agent writes in
    // upper or mixed case is still the nonce.
    let (plain_lc, n) = (plain.to_ascii_lowercase(), n.to_ascii_lowercase());
    let n = n.as_str();
    let mut cut: Vec<(usize, usize)> = plain_lc.match_indices(n).map(|(a, m)| (a, a + m.len())).collect();
    if let Some(k) = (MIN_NONCE..n.len()).rev().find(|&k| plain_lc.ends_with(&n[..k])) {
        cut.push((plain.len() - k, plain.len()));
    }
    if let Some(k) = (MIN_NONCE..n.len()).rev().find(|&k| plain_lc.starts_with(&n[n.len() - k..])) {
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
    let n = n.to_ascii_lowercase();
    let lc = s.to_ascii_lowercase();
    if let Some(k) = (MIN_NONCE..n.len()).rev().find(|&k| lc.ends_with(&n[..k])) {
        s.truncate(s.len() - k);
        s.push_str(REDACTED);
    }
    let lc = s.to_ascii_lowercase();
    if let Some(k) = (MIN_NONCE..n.len()).rev().find(|&k| lc.starts_with(&n[n.len() - k..])) {
        s.replace_range(..k, REDACTED);
    }
    s
}

/// `s` with every ASCII-case-insensitive occurrence of `n` replaced by the marker.
fn replace_ci(s: &str, n: &str) -> String {
    let (lc, n) = (s.to_ascii_lowercase(), n.to_ascii_lowercase());
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    for (a, m) in lc.match_indices(n.as_str()) {
        out.push_str(&s[at..a]);
        out.push_str(REDACTED);
        at = a + m.len();
    }
    out.push_str(&s[at..]);
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    const N: &str = "0123456789abcdef0123456789abcdef";

    fn red() -> Redactor {
        Redactor::new(Some(N))
    }

    #[test]
    fn the_nonce_is_redacted_in_any_ascii_case() {
        let up = N.to_ascii_uppercase();
        let mixed: String = N.chars().enumerate().map(|(i, c)| if i % 3 == 0 { c.to_ascii_uppercase() } else { c }).collect();
        for form in [N.to_string(), up, mixed] {
            let line = format!("the nonce is {form} ok");
            assert_eq!(red().clean(&line), "the nonce is [redacted] ok", "{form}");
        }
    }

    #[test]
    fn an_uppercase_nonce_is_found_through_escapes_and_invisible_characters() {
        let up = N.to_ascii_uppercase();
        let split = format!("{}\u{200b}{}", &up[..16], &up[16..]);
        assert_eq!(red().clean(&format!("x {split} y")), "x [redacted] y");
        // The PTY view keeps colours but still shows no nonce character.
        let sgr = format!("x {}\u{1b}[0m{} y", &up[..16], &up[16..]);
        let out = red().redact(sgr);
        assert!(!out.to_ascii_lowercase().contains(&N[..8]) && out.contains(REDACTED), "{out:?}");
    }

    #[test]
    fn an_uppercase_fragment_cut_by_a_terminal_wrap_is_redacted_too() {
        let up = N.to_ascii_uppercase();
        assert_eq!(red().clean(&format!("tail {}", &up[..20])), "tail [redacted]");
        assert_eq!(red().clean(&format!("{} head", &up[12..])), "[redacted] head");
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        assert_eq!(red().clean("a sha 0123abcd and DEADBEEF stay"), "a sha 0123abcd and DEADBEEF stay");
    }
}
