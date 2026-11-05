//! ANSI sanitizer for agent output shown in the app.
//!
//! Allow-list: printable text, tab, carriage return, SGR (`CSI ... m`) and
//! cursor movement (`CSI ... A..H` and `f`). Everything else goes: other CSI
//! (erase, private modes, reports), every OSC (hyperlinks 8, clipboard 52,
//! titles 0/1/2), DCS, SOS, PM, APC, two-char escapes, other C0 controls and
//! all C1 controls (8-bit CSI/OSC/DCS/ST included).

/// Longest CSI parameter string kept; longer ones are dropped whole.
const MAX_CSI_PARAMS: usize = 64;

const ESC: char = '\x1b';
const BEL: char = '\x07';

fn is_c1(c: char) -> bool {
    ('\u{80}'..='\u{9f}').contains(&c)
}

fn keep_csi(params: &str, intermediates: bool, fin: char) -> bool {
    if intermediates || params.len() > MAX_CSI_PARAMS {
        return false;
    }
    match fin {
        'm' => params
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b';' || b == b':'),
        'A'..='H' | 'f' => params.bytes().all(|b| b.is_ascii_digit() || b == b';'),
        _ => false,
    }
}

pub fn sanitize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != ESC {
            if c == '\t' || c == '\r' || !(c.is_control() || is_c1(c)) {
                out.push(c);
            }
            continue;
        }
        match chars.next() {
            // CSI: params 0x30..=0x3F, intermediates 0x20..=0x2F, final 0x40..=0x7E.
            Some('[') => {
                let mut params = String::new();
                let mut intermediates = false;
                let mut fin = None;
                for c in chars.by_ref() {
                    match c {
                        '\x30'..='\x3f' if !intermediates => params.push(c),
                        '\x20'..='\x2f' => intermediates = true,
                        '\x40'..='\x7e' => {
                            fin = Some(c);
                            break;
                        }
                        // Malformed: abandon the sequence, drop this char too.
                        _ => break,
                    }
                }
                if let Some(fin) = fin {
                    if keep_csi(&params, intermediates, fin) {
                        out.push(ESC);
                        out.push('[');
                        out.push_str(&params);
                        out.push(fin);
                    }
                }
            }
            // String sequences: OSC, DCS, SOS, PM, APC. Skip to BEL (OSC) or
            // ST (ESC \ or U+009C); unterminated drops to the end.
            Some(']' | 'P' | 'X' | '^' | '_') => {
                while let Some(c) = chars.next() {
                    if c == BEL || c == '\u{9c}' {
                        break;
                    }
                    if c == ESC && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Charset designation: ESC ( 0 and friends take one more char.
            Some('(' | ')' | '*' | '+' | '-' | '.' | '/' | '#' | '%' | ' ') => {
                chars.next();
            }
            // Any other two-char escape (RIS, keypad, save/restore) or a
            // trailing ESC: drop.
            _ => {}
        }
    }
    out
}

#[cfg(test)]
#[path = "ansi_tests.rs"]
mod tests;
