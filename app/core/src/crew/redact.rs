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
    nonce: Option<String>,
}

impl Redactor {
    pub fn new(nonce: Option<&str>) -> Self {
        Self { nonce: nonce.filter(|n| n.len() >= MIN_NONCE).map(str::to_string) }
    }

    /// One line of agent text in, one plain-text line out.
    pub fn clean(&self, text: &str) -> String {
        let s = strip(&ansi::sanitize(text));
        match &self.nonce {
            Some(n) => s.replace(n.as_str(), REDACTED),
            None => s,
        }
    }
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

fn is_format(c: char) -> bool {
    matches!(c,
        '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}' | '\u{feff}')
}
