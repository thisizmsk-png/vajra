use super::*;
use crate::control::lead_settings as ls;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const ASK: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const RUN: &str = "20261106-0911";
const NONCE: &str = "0123456789abcdef0123456789abcdef";

fn header(ask: &str, run: &str) -> String {
    format!("[Vajra Desktop: question from the human, ask {ask}, run {run}. Answer with the read and control tools only.]\n")
}

fn no_red() -> Redactor {
    Redactor::default()
}

// ------------------------------------------------------------ ULID

#[test]
fn ulid_shape_and_known_value() {
    assert!(valid_ulid(ASK));
    for bad in ["", "01ARZ3NDEKTSV4RRFFQ69G5FA", "01ARZ3NDEKTSV4RRFFQ69G5FAVV", "01ARZ3NDEKTSV4RRFFQ69G5FAI", "01ARZ3NDEKTSV4RRFFQ69G5FAL", "01arz3ndektsv4rrffq69g5fav", "01ARZ3NDEKTSV4RRFFQ69G5FA\n"] {
        assert!(!valid_ulid(bad), "{bad:?}");
    }
    assert_eq!(ulid_from(0, "ffffffffffffffffffff").unwrap(), "0000000000ZZZZZZZZZZZZZZZZ");
    assert_eq!(ulid_from(0, "00000000000000000001").unwrap(), "00000000000000000000000001");
    let a = new_ulid(std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_000_000)).unwrap();
    let b = new_ulid(std::time::UNIX_EPOCH + std::time::Duration::from_millis(2_000_000)).unwrap();
    assert!(valid_ulid(&a) && valid_ulid(&b) && a < b, "time ordered: {a} {b}");
}

// ------------------------------------------------------------ framing

#[test]
fn ask_len_bounds() {
    let ok = |t: &str| frame(ASK, RUN, t, &no_red());
    assert!(ok("x").is_ok());
    assert!(ok(&"x".repeat(MAX_TEXT)).is_ok());
    assert!(ok(&"é".repeat(MAX_TEXT)).is_ok(), "the bound counts chars, not bytes");
    for bad in [String::new(), "x".repeat(MAX_TEXT + 1), "\u{1b}\u{0}\r".into(), "  \n\t ".into()] {
        assert_eq!(ok(&bad).unwrap_err().code(), "invalid_args", "{bad:?}");
    }
}

#[test]
fn ask_strips_controls_esc_bidi() {
    let dirty = "a\u{1b}[31mred\u{7f}\u{0}\r\nb\u{202e}c\u{2066}d\u{2028}e\u{2029}f\u{feff}g\u{85}h\u{9b}i\tj\u{200b}k";
    let f = frame(ASK, RUN, dirty, &no_red()).unwrap();
    // The ESC and the other controls go, the visible bracket text stays: it is just text without its ESC.
    assert_eq!(f.text(), "a[31mred\nbcdefghi\tjk");
    for c in f.text().chars() {
        assert!(!c.is_control() || c == '\n' || c == '\t', "{c:?}");
    }
}

#[test]
fn ask_redacts_nonce() {
    let red = Redactor::new(Some(NONCE));
    let f = frame(ASK, RUN, &format!("the key is {NONCE} ok"), &red).unwrap();
    assert!(!f.text().contains(NONCE) && f.text().contains("[redacted]"), "{}", f.text());
    // An ESC or an invisible character inside the nonce cannot smuggle it past the redaction.
    let split = format!("{}\u{1b}[0m{}\u{200b}{}", &NONCE[..10], &NONCE[10..20], &NONCE[20..]);
    let f = frame(ASK, RUN, &split, &red).unwrap();
    assert!(!f.text().contains(&NONCE[..10]) && !f.text().contains(&NONCE[20..]), "{}", f.text());
    // A plain control byte that is dropped must not join the halves of a nonce back together.
    let joined = format!("{}\u{1}{}", &NONCE[..16], &NONCE[16..]);
    let f = frame(ASK, RUN, &joined, &red).unwrap();
    assert!(!f.text().contains(&NONCE[..16]) && !f.text().contains(&NONCE[16..]), "{}", f.text());
    assert!(!String::from_utf8_lossy(f.as_bytes()).contains(NONCE));
}

#[test]
fn frame_header_fields_validated() {
    let bad_asks = [
        "", "01ARZ3NDEKTSV4RRFFQ69G5FA\nV", "/1ARZ3NDEKTSV4RRFFQ69G5FAV", "01ARZ3NDEKTSV4RRFFQ69G5FAV\n/clear",
        "01ARZ3NDEKTSV4RRFFQ69G5FAI", "../../../../../../etc/passwd", "01ARZ3NDEKTSV4RRFFQ69G5FAVX",
    ];
    for a in bad_asks {
        assert_eq!(frame(a, RUN, "hi", &no_red()).unwrap_err().code(), "invalid_args", "ask {a:?}");
    }
    let bad_runs = ["", "20261106-091", "20261106-09111", "20261106\n0911", "/0261106-0911", "2026110-60911", "20261106_0911", "20261106-0911\n/clear"];
    for r in bad_runs {
        assert_eq!(frame(ASK, r, "hi", &no_red()).unwrap_err().code(), "invalid_args", "run {r:?}");
    }
}

/// xorshift64*: deterministic, so a failure reproduces from the printed seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn nasty(r: &mut Rng, hdr: &str) -> String {
    let pieces: [&str; 28] = [
        "/", "/clear", "/help", "!", "!echo x", "#", "# note", "-", "--version", "-p", "\n", "\r", "\r\n", "\u{1b}", "\u{1b}[201~",
        "\u{0}", "\u{feff}", "\u{2028}", "\u{202e}", "\u{2067}", " ", "\t", "  \n  ", "[", "a", "é", "中", "\u{7f}",
    ];
    let mut s = String::new();
    // Sometimes a whitespace-only or control-only prefix, sometimes a copy of the header itself.
    match r.below(6) {
        0 => s.push_str("   \n\t"),
        1 => s.push_str(hdr),
        2 => s.push_str(&hdr.replace("ask ", "ask X")),
        _ => {}
    }
    for _ in 0..r.below(40) {
        if r.below(9) == 0 {
            s.push_str(hdr);
        } else {
            s.push_str(pieces[r.below(pieces.len())]);
        }
    }
    if s.chars().count() > MAX_TEXT {
        s = s.chars().take(MAX_TEXT).collect();
    }
    s
}

#[test]
fn frame_cannot_be_bypassed() {
    let hdr = header(ASK, RUN);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut accepted, mut refused) = (0u32, 0u32);
    for case in 0..20_000u32 {
        let text = nasty(&mut rng, &hdr);
        let Ok(f) = frame(ASK, RUN, &text, &no_red()) else {
            refused += 1;
            continue;
        };
        accepted += 1;
        let b = f.as_bytes();
        let msg = format!("case {case}: {text:?}");
        // Every framed ask starts with the exact fixed header bytes.
        assert!(b.starts_with(hdr.as_bytes()), "{msg}");
        assert_eq!(b[0], b'[', "{msg}");
        // Its first newline is the header's own, so no byte of the text sits on line 1.
        assert_eq!(b.iter().position(|&c| c == b'\n'), Some(hdr.len() - 1), "{msg}");
        assert_eq!(f.header(), hdr, "{msg}");
        // Whatever follows is exactly the sanitised text, with no control character a terminal or CLI could act on.
        assert_eq!(&b[hdr.len()..], f.text().as_bytes(), "{msg}");
        assert!(f.text().chars().all(|c| !drop_char(c)), "{msg}");
        assert!(!f.text().trim().is_empty(), "{msg}");
        // Nothing in the whole body can end the line early or break out of a paste.
        assert!(!b.contains(&0x1b) && !b.contains(&b'\r') && !b.contains(&0), "{msg}");
    }
    assert!(accepted >= 10_000, "only {accepted} accepted ({refused} refused): the generator is too hostile to prove anything");
}

// ------------------------------------------------------------ Lead settings

static N: AtomicUsize = AtomicUsize::new(0);

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vajra-lead-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn hooks_json() -> serde_json::Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hooks/hooks.json");
    serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
}

#[test]
fn lead_settings_constant() {
    let t = tmp("const");
    let (home, repo, data) = (t.join("home"), t.join("repo"), t.join("data"));
    for d in [&home, &repo, &data] {
        std::fs::create_dir_all(d).unwrap();
    }
    let text = ls::settings(&home, &repo, &data).unwrap();
    assert_eq!(text, ls::settings(&home, &repo, &data).unwrap(), "deterministic");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let p = &v["permissions"];
    assert_eq!(p["defaultMode"], "dontAsk");
    assert_eq!(p["disableBypassPermissionsMode"], "disable");
    let allow: Vec<&str> = p["allow"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert_eq!(allow.join(","), crate::sessions::argv::LEAD_TOOLS);
    // Nothing else is configured.
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["hooks", "permissions"]);
    // The hooks are exactly the guard and observer entries of hooks/hooks.json, and not desktop-control.sh.
    let real = hooks_json();
    assert_eq!(v["hooks"]["PreToolUse"][0], real["hooks"]["PreToolUse"][0]);
    assert_eq!(v["hooks"]["PostToolUse"][0], real["hooks"]["PostToolUse"][0]);
    assert_eq!(v["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
    assert_eq!(v["hooks"]["PostToolUse"].as_array().unwrap().len(), 1);
    assert_eq!(v["hooks"].as_object().unwrap().len(), 2);
    assert!(!text.contains("desktop-control"));
    // The guard fails closed.
    assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["onFailure"], "block");
    // The self-test variant has no hooks and the same rules.
    let bare: serde_json::Value = serde_json::from_str(&ls::settings_no_hooks(&home, &repo, &data).unwrap()).unwrap();
    assert!(bare.get("hooks").is_none());
    assert_eq!(bare["permissions"], v["permissions"]);
}

#[test]
fn deny_rules_cover_the_key_and_credentials() {
    let t = tmp("cover");
    let (home, repo, data) = (t.join("home"), t.join("repo"), t.join("Application Support/VajraDesktop"));
    for d in [&home, &repo, &data] {
        std::fs::create_dir_all(d).unwrap();
    }
    let rules = ls::deny_rules(&home, &repo, &data).unwrap();
    let real_home = std::fs::canonicalize(&home).unwrap();
    let want = format!("Read(/{}/.claude/vajra/**)", real_home.display());
    assert!(rules.contains(&want), "{want} in {rules:#?}");
    for suffix in [".ssh/**", ".aws/**", ".netrc", ".claude.json", ".claude/settings*.json", "Library/Application Support/VajraDesktop/**"] {
        assert!(rules.iter().any(|r| r.ends_with(&format!("/{suffix})"))), "{suffix}");
    }
    for suffix in ["**/.env", "**/.env.*", "**/id_rsa*"] {
        assert!(rules.iter().any(|r| r.contains("/repo/") && r.ends_with(&format!("/{suffix})"))), "{suffix}");
    }
    assert!(rules.iter().any(|r| r.contains("Application Support/VajraDesktop/**)") && !r.contains("/home/")), "data dir rule");
}

/// Is `r` the one shape the CLI honours: `Read(//` then a non-slash, no `~`, no leading `**/`, no single slash?
fn double_slash_abs(r: &str) -> bool {
    let Some(body) = r.strip_prefix("Read(//") else { return false };
    body.ends_with(')')
        && !body.starts_with('/')
        && !body.contains('~')
        && !body.starts_with("**")
        && !body[..body.len() - 1].contains(['\n', '\0'])
}

#[test]
fn deny_rules_all_double_slash_abs() {
    // Unit: spaces, unicode, and a symlinked HOME.
    let t = tmp("dslash");
    let real = t.join("réal home");
    std::fs::create_dir_all(&real).unwrap();
    let link = t.join("link home");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let repo = t.join("repo 中");
    let data = t.join("da ta");
    for d in [&repo, &data] {
        std::fs::create_dir_all(d).unwrap();
    }
    let rules = ls::deny_rules(&link, &repo, &data).unwrap();
    assert!(rules.iter().all(|r| double_slash_abs(r)), "{rules:#?}");
    let probe = |base: &Path| format!("Read(//{}/.netrc)", base.to_str().unwrap().trim_start_matches('/'));
    assert!(rules.contains(&probe(&link)) && rules.contains(&probe(&std::fs::canonicalize(&real).unwrap())), "both forms of a symlinked HOME");
    let mut dedup = rules.clone();
    dedup.sort();
    dedup.dedup();
    assert_eq!(dedup.len(), rules.len(), "no duplicates");

    // Property: random HOME and repo paths (spaces, unicode, symlinks, the macOS /var twin).
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let names = ["a", "a b", "é", "中文", "x.y", "dir-1", "with space", "ünï"];
    let root = std::fs::canonicalize(&t).unwrap();
    for case in 0..150 {
        // Half the cases start from the canonical root, half from the as-given temp path (a /var → /private/var twin).
        let start = if rng.below(2) == 0 { root.clone() } else { t.clone() };
        let mut mk = |label: &str| {
            let mut p = start.join(format!("case{case}{label}"));
            for _ in 0..1 + rng.below(3) {
                p.push(names[rng.below(names.len())]);
            }
            std::fs::create_dir_all(&p).unwrap();
            p
        };
        let (h, r, d) = (mk("h"), mk("r"), mk("d"));
        let (h, r) = if rng.below(3) == 0 {
            let l = start.join(format!("case{case}-link"));
            std::os::unix::fs::symlink(&h, &l).unwrap();
            (l, r)
        } else {
            (h, r)
        };
        let rules = ls::deny_rules(&h, &r, &d).unwrap();
        assert!(rules.iter().all(|x| double_slash_abs(x)), "case {case}: {rules:#?}");
        for base in [&h, &r, &d] {
            let given = base.to_str().unwrap().trim_start_matches('/');
            let canon = std::fs::canonicalize(base).unwrap();
            let canon = canon.to_str().unwrap().trim_start_matches('/').to_string();
            let has = |b: &str| rules.iter().any(|x| x.starts_with(&format!("Read(//{b}/")));
            assert!(has(given), "case {case}: given form of {base:?}");
            assert_eq!(has(&canon), true, "case {case}: realpath form of {base:?}");
            // The twin is a distinct rule exactly when the canonical base differs.
            let twin_distinct = canon != given;
            let n_given = rules.iter().filter(|x| x.starts_with(&format!("Read(//{given}/"))).count();
            let n_canon = rules.iter().filter(|x| x.starts_with(&format!("Read(//{canon}/"))).count();
            assert!(n_given > 0 && (twin_distinct || n_given == n_canon), "case {case}");
        }
    }

    // Bases the CLI would read as glob syntax (or that break the rule's own syntax) are refused, never escaped.
    for bad in ["(", ")", "*", "?", "[", "{", "a\nb", "a\0b", "]", "}", "a\\b"] {
        let base = t.join(format!("x{bad}y"));
        assert_eq!(ls::deny_rules(&base, &repo, &data).unwrap_err().code(), "lead_guard_missing", "home {bad:?}");
        assert_eq!(ls::deny_rules(&link, &base, &data).unwrap_err().code(), "lead_guard_missing", "repo {bad:?}");
        assert_eq!(ls::deny_rules(&link, &repo, &base).unwrap_err().code(), "lead_guard_missing", "data {bad:?}");
        assert!(ls::settings(&base, &repo, &data).is_err());
    }
    for rel in ["", "relative/path", "~", "~/x", "/"] {
        assert!(ls::deny_rules(Path::new(rel), &repo, &data).is_err(), "{rel:?}");
    }
}

#[test]
fn deny_rules_not_injectable() {
    // Compile-time API check: `settings` takes three paths and nothing that carries rule strings.
    let _: fn(&Path, &Path, &Path) -> Result<String, LeadError> = ls::settings;
    let _: fn(&Path, &Path, &Path) -> Result<String, LeadError> = ls::settings_no_hooks;
    let _: fn(&Path, &Path, &Path) -> Result<Vec<String>, LeadError> = ls::deny_rules;
    // Grep test: no `"Read(` literal outside the generator (and these tests, which build negative controls).
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    assert!(files.len() > 20);
    let needle = ["\"Read", "("].concat();
    for f in files {
        let name = f.file_name().unwrap().to_str().unwrap();
        if matches!(name, "lead_settings.rs" | "lead_tests.rs" | "live_tests.rs") {
            continue;
        }
        assert!(!std::fs::read_to_string(&f).unwrap().contains(&needle), "{} builds a Read rule outside the generator", f.display());
    }
}
