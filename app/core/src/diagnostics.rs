//! A local, redacted diagnostics preview (FG-24).
//!
//! The text is built from the lifecycle report and the crash reconciliation only: process ids, folders and verdicts. It
//! never carries terminal output, a prompt, an environment or a label typed by the human. On top of that every line goes
//! through `redact`: the home directory becomes `~`, folders that usually hold secrets are replaced, and anything that
//! looks like a token is replaced. The result is bounded. It is handed to the UI to show and copy by the human; this
//! module has no network code and nothing here uploads (a test pins that).

use crate::lifecycle_report::Report;
use crate::owned::{Found, Verdict};

/// The preview never exceeds this many bytes.
pub const MAX_BYTES: usize = 16 * 1024;
const REDACTED: &str = "<redacted>";
const SENSITIVE_DIRS: &[&str] = &[".ssh", ".aws", ".gnupg", ".kube", ".docker", ".netrc", ".npmrc", ".pypirc", ".config/gcloud", ".config/gh", "Library/Keychains", ".env"];
const SECRET_KEYS: &[&str] = &["token", "secret", "password", "passwd", "apikey", "api_key", "api-key", "authorization", "credential", "private"];
const SECRET_PREFIXES: &[&str] = &["sk-", "sk_", "pk-", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "xoxa-", "xoxb-", "xoxp-", "xoxr-", "glpat-", "npm_"];

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '_' | '=' | '-')
}

fn looks_like_secret(w: &str) -> bool {
    let w = w.trim_matches(|c: char| !is_token_char(c) && c != '.');
    if SECRET_PREFIXES.iter().any(|p| w.starts_with(p) && w.len() >= p.len() + 8) {
        return true;
    }
    if w.starts_with("AKIA") && w.len() >= 16 && w.chars().all(|c| c.is_ascii_alphanumeric()) {
        return true;
    }
    if w.starts_with("eyJ") && w.matches('.').count() == 2 {
        return true;
    }
    w.len() >= 32 && w.chars().all(is_token_char) && w.chars().any(|c| c.is_ascii_digit()) && w.chars().any(|c| c.is_ascii_alphabetic())
}

fn sensitive_path(w: &str) -> bool {
    let p = w.replace('\\', "/");
    SENSITIVE_DIRS.iter().any(|d| p.split('/').collect::<Vec<_>>().windows(d.split('/').count()).any(|win| win.join("/") == *d))
}

/// Home directory to `~`, only at a path boundary (`/Users/sam` is not a prefix of `/Users/samantha`).
fn tilde(s: &str, home: &str) -> String {
    let home = home.trim_end_matches('/');
    if home.len() < 2 {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(home) {
        let after = &rest[i + home.len()..];
        let boundary = after.chars().next().is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.')));
        out.push_str(&rest[..i]);
        out.push_str(if boundary { "~" } else { home });
        rest = after;
    }
    out.push_str(rest);
    out
}

pub fn redact(text: &str, home: &str) -> String {
    let text = tilde(text, home);
    text.lines()
        .map(|line| {
            let words: Vec<&str> = line.split(' ').collect();
            let mut out: Vec<String> = Vec::with_capacity(words.len());
            let mut hide_next = false;
            for w in words {
                let lower = w.to_ascii_lowercase();
                if hide_next && !w.is_empty() {
                    // everything after "Bearer" or "password:" on the line: a credential can span several words
                    if out.last().map(String::as_str) != Some(REDACTED) {
                        out.push(REDACTED.into());
                    }
                } else if let Some((k, _)) = w.split_once(['=', ':']).filter(|(k, v)| !v.is_empty() && SECRET_KEYS.iter().any(|s| k.to_ascii_lowercase().contains(s))) {
                    out.push(format!("{k}={REDACTED}"));
                } else if matches!(lower.as_str(), "bearer" | "basic") || (SECRET_KEYS.iter().any(|s| lower.trim_end_matches([':', '=']).ends_with(s)) && (lower.ends_with(':') || lower.ends_with('='))) {
                    out.push(w.into());
                    hide_next = true;
                } else if sensitive_path(w) {
                    out.push("<sensitive-path>".into());
                } else if looks_like_secret(w) {
                    out.push(REDACTED.into());
                } else {
                    out.push(w.into());
                }
            }
            out.join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn bound(mut s: String) -> String {
    const CUT: &str = "\n[truncated]";
    if s.len() > MAX_BYTES {
        let mut n = MAX_BYTES - CUT.len();
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        s.truncate(n);
        s.push_str(CUT);
    }
    s
}

/// The preview text. `home` is this user's home directory, used only to shorten paths.
pub fn preview(r: &Report, found: &[Found], home: &str) -> String {
    let mut t = String::from("Vajra lifecycle diagnostics (local, redacted)\n");
    t.push_str(&format!("If the window closes: {} terminals and {} runs keep running.\n", r.close_keeps.terminals, r.close_keeps.runs));
    t.push_str(&format!("If Vajra quits: {} terminals and {} runs end; nothing resumes by itself.\n", r.quit_ends.terminals, r.quit_ends.runs));
    t.push_str(&format!("Archiving a worker leaves {} terminals open.\n", r.archive_leaves.len()));
    t.push_str("Terminals:\n");
    for d in &r.terminals {
        t.push_str(&format!("  id {} pid {} {} {}{}\n", d.id, d.pid, if d.alive { "running" } else { "ended" }, d.cwd.display(), if d.in_worktree { " (worktree)" } else { "" }));
    }
    t.push_str("Runs:\n");
    for d in &r.runs {
        t.push_str(&format!("  pid {} group {}\n", d.pid, d.pid));
    }
    t.push_str("Found after the last launch:\n");
    for f in found {
        let v = match &f.verdict {
            Verdict::Owned { members } => format!("owned, {members} processes, can be ended"),
            Verdict::Gone => "gone".into(),
            Verdict::Reused => "pid reused by another program, left alone".into(),
            Verdict::Unproven => "pid still held but can't be checked yet, kept for another try".into(),
        };
        t.push_str(&format!("  pid {} {} {}\n", f.owner.pid, f.owner.cwd.display(), v));
    }
    t.push_str("Limits: only shells Vajra recorded with their start time can be recovered; nothing else is ever signalled.\n");
    bound(redact(&t, home))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle_report::{report, RunDiag};
    use crate::owned::Owner;
    use crate::terminal::TermInfo;

    const HOME: &str = "/Users/sam";

    #[test]
    fn given_a_path_under_home_when_redacted_then_it_shows_tilde_and_no_other_user_is_touched() {
        assert_eq!(redact("cwd /Users/sam/code/app", HOME), "cwd ~/code/app");
        assert_eq!(redact("cwd /Users/samantha/code", HOME), "cwd /Users/samantha/code");
        assert_eq!(redact("home /Users/sam", HOME), "home ~");
    }

    #[test]
    fn given_secret_folders_when_redacted_then_the_path_is_hidden() {
        for p in ["/Users/sam/.ssh/id_ed25519", "/Users/sam/.aws/credentials", "/Users/sam/.config/gcloud/x", "/Users/sam/Library/Keychains/login.keychain-db", "/p/.env"] {
            assert_eq!(redact(&format!("see {p}"), HOME), "see <sensitive-path>", "{p}");
        }
        assert_eq!(redact("see /Users/sam/code/.environment", HOME), "see ~/code/.environment");
    }

    #[test]
    fn given_tokens_when_redacted_then_none_survive() {
        let secrets = [
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "github_pat_11ABCDEFG0abcdefghijklmnopqrstuv",
            "xoxb-1234567890-abcdefghij",
            "AKIAIOSFODNN7EXAMPLE",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r",
            "0123456789abcdef0123456789abcdef01234567",
        ];
        for s in secrets {
            let out = redact(&format!("value {s} end"), HOME);
            assert!(!out.contains(s), "{s} leaked: {out}");
            assert!(out.contains(REDACTED));
        }
    }

    #[test]
    fn given_key_value_and_bearer_forms_when_redacted_then_the_value_goes_and_the_key_stays() {
        assert_eq!(redact("API_TOKEN=abc123", HOME), "API_TOKEN=<redacted>");
        assert_eq!(redact("password: hunter2", HOME), "password: <redacted>");
        assert_eq!(redact("Authorization: Bearer abc.def", HOME), "Authorization: <redacted>");
        assert_eq!(redact("curl -H Bearer xyz", HOME), "curl -H Bearer <redacted>");
    }

    #[test]
    fn given_ordinary_text_when_redacted_then_it_is_unchanged() {
        let t = "id 3 pid 4242 running ~/code/app (worktree)";
        assert_eq!(redact(t, HOME), t);
    }

    #[test]
    fn given_a_huge_report_when_previewed_then_it_is_bounded_and_says_so() {
        let terms: Vec<TermInfo> = (0..2000).map(|i| TermInfo { id: i, pid: 10 + i, cwd: format!("/Users/sam/{}", "d".repeat(50)).into(), root: "/Users/sam".into(), rows: 1, cols: 1, alive: true, sign_in: None }).collect();
        let p = preview(&report(&terms, &[]), &[], HOME);
        assert!(p.len() <= MAX_BYTES, "{}", p.len());
        assert!(p.ends_with("[truncated]"));
    }

    #[test]
    fn the_preview_has_the_facts_and_none_of_the_home_path() {
        let terms = [TermInfo { id: 1, pid: 77, cwd: "/Users/sam/code/app/wt".into(), root: "/Users/sam/code/app".into(), rows: 1, cols: 1, alive: true, sign_in: None }];
        let runs = [RunDiag { stream: "s1".into(), pid: 900 }];
        let found = [Found { owner: Owner::shell(55, 1, "/Users/sam/.ssh".into()), verdict: Verdict::Reused }];
        let p = preview(&report(&terms, &runs), &found, HOME);
        assert!(p.contains("pid 77") && p.contains("pid 900") && p.contains("pid 55") && p.contains("left alone"));
        assert!(!p.contains("/Users/sam"), "{p}");
        assert!(p.contains("~/code/app/wt") && p.contains("<sensitive-path>"));
    }

    #[test]
    fn the_module_has_no_network_or_process_spawning() {
        let src = include_str!("diagnostics.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for bad in ["std::net", "reqwest", "hyper", "TcpStream", "http://", "https://", "Command::new", "std::fs::write"] {
            assert!(!code.contains(bad), "{bad}");
        }
    }
}
