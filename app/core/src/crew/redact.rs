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

/// Provider error text made safe for an event, a log line or the UI (R19): control characters dropped, then URL query
/// strings and credentials, the account identifiers a provider echoes back (OpenRouter's `user_id`, Cloudflare's
/// `cf-ray`), bearer tokens and anything shaped like a key. Errs toward removing too much.
pub fn redact_provider_text(text: &str) -> String {
    let cleaned = Redactor::default().clean(text);
    let urls = redact_urls(&cleaned);
    let fields = FIELDS.iter().fold(urls, |s, f| redact_field(&s, f));
    redact_key_like(&fields)
}

const SECRET: &str = "<redacted>";

/// Fields whose value is a token (a run of token characters). An auth scheme word before it (`Bearer`, `Basic`, ...) is
/// skipped, so the credential after it is what goes.
const FIELDS: [&str; 20] = [
    "user_id", "cf-ray", "x-api-key", "api-key", "api_key", "apikey", "password", "passwd", "secret", "client_secret",
    "access_token", "refresh_token", "id_token", "token", "key", "bearer", "proxy-authorization", "authorization", "set-cookie", "cookie",
];
/// Header names whose whole value is redacted, to the closing quote if it sat in quotes (JSON) and else to the end of
/// the line, whatever the scheme (Basic, Token, Digest, a cookie list, ...). `set-cookie` and `proxy-authorization`
/// contain the shorter names, but are listed so their value is cut the same way.
const LINE_FIELDS: [&str; 4] = ["proxy-authorization", "authorization", "set-cookie", "cookie"];
const SCHEMES: [&str; 6] = ["bearer", "basic", "token", "digest", "negotiate", "apikey"];

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '+' | '=')
}

/// `https://host/path?query#frag` -> `https://host/path?<redacted>`; `user:pw@` before the host goes too.
fn redact_urls(s: &str) -> String {
    let lc = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    while let Some(i) = ["http://", "https://"].iter().filter_map(|p| lc[at..].find(p).map(|j| at + j)).min() {
        out.push_str(&s[at..i]);
        let end = s[i..].find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']' | '}')).map_or(s.len(), |j| i + j);
        let url = &s[i..end];
        let scheme = url.find("://").map_or(0, |j| j + 3);
        let (head, rest) = url.split_at(scheme);
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let rest = match rest[..authority_end].rfind('@') {
            Some(a) => &rest[a + 1..],
            None => rest,
        };
        out.push_str(head);
        match rest.find(['?', '#']) {
            Some(q) => {
                out.push_str(&rest[..q]);
                out.push('?');
                out.push_str(SECRET);
            }
            None => out.push_str(rest),
        }
        at = end;
    }
    out.push_str(&s[at..]);
    out
}

/// Replace the value after every case-insensitive `name` that is followed by `:` or `=` (after any closing quote), as in
/// `name: v`, `name=v` and `"name":"v"`; for `bearer`, the token after a space. A line field (`LINE_FIELDS`) loses its
/// whole value; any other loses one token, after an auth scheme word if there is one.
fn redact_field(s: &str, name: &str) -> String {
    let lc = s.to_ascii_lowercase();
    let quote = |c: char| c == '"' || c == '\'';
    let whole = LINE_FIELDS.contains(&name);
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    while let Some(j) = lc[at..].find(name) {
        let after = at + j + name.len();
        out.push_str(&s[at..after]);
        at = after;
        let rest = s[after..].trim_start_matches(quote);
        let sep = rest.chars().next();
        let ok = if name == "bearer" { sep == Some(' ') } else { matches!(sep, Some(':' | '=')) };
        if !ok {
            continue;
        }
        let before_value = rest[1..].trim_start_matches(' ');
        let opener = before_value.chars().next().filter(|c| quote(*c));
        let mut value = before_value.trim_start_matches(quote);
        let len = if whole {
            value.find(|c: char| c == '\n' || c == '\r' || Some(c) == opener).unwrap_or(value.len())
        } else {
            let word = value.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(0);
            if word > 0 && value[word..].starts_with(' ') && SCHEMES.contains(&value[..word].to_ascii_lowercase().as_str()) {
                value = value[word..].trim_start_matches(' ');
            }
            value.find(|c: char| !is_token_char(c)).unwrap_or(value.len())
        };
        if len == 0 {
            continue;
        }
        out.push_str(&s[after..s.len() - value.len()]);
        out.push_str(SECRET);
        at = s.len() - value.len() + len;
    }
    out.push_str(&s[at..]);
    out
}

/// Known key prefixes, or a long token that mixes letters and digits.
fn key_like(tok: &str) -> bool {
    const PREFIXES: [&str; 11] = ["sk-", "sk_", "pk-", "pk_", "ghp_", "gho_", "github_pat_", "xox", "akia", "aiza", "eyj"];
    let lc = tok.to_ascii_lowercase();
    if tok.len() >= 12 && PREFIXES.iter().any(|p| lc.starts_with(p)) {
        return true;
    }
    tok.len() >= 32 && tok.chars().any(|c| c.is_ascii_digit()) && tok.chars().any(|c| c.is_ascii_alphabetic())
}

fn redact_key_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut tok = String::new();
    let flush = |tok: &mut String, out: &mut String| {
        if !tok.is_empty() {
            out.push_str(if key_like(tok) { SECRET } else { tok });
            tok.clear();
        }
    };
    for c in s.chars() {
        if is_token_char(c) {
            tok.push(c);
        } else {
            flush(&mut tok, &mut out);
            out.push(c);
        }
    }
    flush(&mut tok, &mut out);
    out
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

    // Fixtures use invented ids only.
    const USER: &str = "user_SENTINEL0000TESTONLY";
    const RAY: &str = "0000deadbeef0000-XXX";

    #[test]
    fn provider_text_loses_user_id_and_cf_ray() {
        let raw = format!("{{\"error\":{{\"code\":401,\"user_id\":\"{USER}\"}}}} cf-ray: {RAY} ok");
        let out = redact_provider_text(&raw);
        assert!(!out.contains(USER) && !out.contains(RAY), "{out}");
        assert!(out.contains("\"code\":401") && out.ends_with(" ok"), "{out}");
        for form in [format!("user_id={USER} end"), format!("User_Id: {USER} end"), format!("CF-Ray:{RAY}")] {
            let out = redact_provider_text(&form);
            assert!(!out.contains(USER) && !out.contains(RAY), "{form} -> {out}");
        }
    }

    #[test]
    fn provider_text_loses_key_like_strings_and_bearer_tokens() {
        for secret in ["sk-or-v1-FAKEFAKEFAKEFAKE0000", "ghp_FAKEFAKEFAKEFAKEFAKE", "0123456789abcdef0123456789abcdef0123"] {
            let out = redact_provider_text(&format!("bad key {secret}, retry"));
            assert!(!out.contains(secret) && out.contains("bad key") && out.contains(", retry"), "{out}");
        }
        let out = redact_provider_text("authorization: Bearer FAKEtoken123 failed");
        assert!(!out.contains("FAKEtoken123"), "{out}");
        for scheme in ["Basic dXNlcjpwYXNzd29yZA==", "Token abc", "Digest username=x, realm=y", "Negotiate YIIB"] {
            for header in ["Authorization", "authorization", "Proxy-Authorization", "AUTHORIZATION"] {
                let out = redact_provider_text(&format!("{header}: {scheme}\nnext line"));
                assert!(!out.contains("dXNlcj") && !out.contains("abc") && !out.contains("username=x") && !out.contains("realm") && !out.contains("YIIB"), "{out}");
                // Newlines are stripped first, so the whole rest of the text goes: safe, if more than needed.
                assert!(out.starts_with(header) && out.contains(SECRET), "{out}");
            }
        }
        // In JSON the value ends at its closing quote.
        let out = redact_provider_text(r#"{"authorization":"Basic dXNlcjpwYXNzd29yZA==","code":401}"#);
        assert!(!out.contains("dXNlcj") && out.contains(r#""code":401"#), "{out}");
        let out = redact_provider_text("sent Bearer FAKEtoken123");
        assert!(!out.contains("FAKEtoken123"), "{out}");
    }

    #[test]
    fn provider_text_loses_cookies_and_other_secret_fields() {
        for (raw, secret) in [
            ("Cookie: session=SENTINELCOOKIE; other=1\nx", "SENTINELCOOKIE"),
            ("set-cookie: sid=SENTINELCOOKIE; HttpOnly", "SENTINELCOOKIE"),
            ("x-api-key: SENTINELKEY", "SENTINELKEY"),
            ("body api_key=SENTINELKEY&x=1", "SENTINELKEY"),
            ("{\"password\":\"SENTINELPW\"}", "SENTINELPW"),
            ("client_secret=SENTINELPW", "SENTINELPW"),
            ("access_token: Bearer SENTINELTOK", "SENTINELTOK"),
            ("refresh_token=SENTINELTOK", "SENTINELTOK"),
            ("{\"key\":\"SENTINELKEY\"}", "SENTINELKEY"),
            ("body key=SENTINELKEY&x=1", "SENTINELKEY"),
            ("GET https://h.test/p?api_key=SENTINELKEY&key=SENTINELKEY&token=SENTINELKEY", "SENTINELKEY"),
        ] {
            let out = redact_provider_text(raw);
            assert!(!out.contains(secret), "{raw} -> {out}");
        }
    }

    #[test]
    fn provider_text_loses_url_queries_and_credentials() {
        let out = redact_provider_text("GET https://user:pw@api.example.test/v1/key?token=FAKEVALUE&a=b#frag failed; see https://docs.example.test/x.");
        assert!(!out.contains("FAKEVALUE") && !out.contains("user:pw") && !out.contains("a=b"), "{out}");
        assert!(out.contains("https://api.example.test/v1/key?<redacted>") && out.contains("https://docs.example.test/x."), "{out}");
    }

    #[test]
    fn provider_text_keeps_ordinary_error_words() {
        let t = "unexpected status 429 Too Many Requests, last status: 429 (request 7 of 30)";
        assert_eq!(redact_provider_text(t), t);
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        assert_eq!(red().clean("a sha 0123abcd and DEADBEEF stay"), "a sha 0123abcd and DEADBEEF stay");
    }
}
