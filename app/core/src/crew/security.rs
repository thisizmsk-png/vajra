//! The workflow's security-word rule (`SEC_RE` in `workflows/vajra-deliver.js`), ported so the core flags the same text.
//! JS `\b` and `\w` are ASCII, so a word is a run of `[A-Za-z0-9_]`; a prefix entry (`auth\w*`) matches any word that
//! starts with it, an exact entry only that word. Matching is case-insensitive.

const EXACT: [&str; 14] = ["login", "logout", "password", "passwd", "jwt", "oauth", "saml", "sso", "csrf", "xss", "cors", "rbac", "acl", "sql"];
const EXACT_MORE: [&str; 1] = ["pii"];
const PREFIX: [&str; 12] = ["auth", "secret", "token", "session", "cookie", "crypt", "permission", "inject", "sanitiz", "upload", "payment", "webhook"];

/// Whether `text` names a security topic.
pub fn matches(text: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).any(|w| {
        let w = w.to_ascii_lowercase();
        EXACT.contains(&w.as_str()) || EXACT_MORE.contains(&w.as_str()) || PREFIX.iter().any(|p| w.starts_with(p))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared samples: the same lines are run through the workflow's own regex in the JS parity check below.
    const HITS: [&str; 9] = ["Add login", "fix the SESSION expiry", "rotate tokens", "OAuth callback", "sql layer", "authentication", "uploads dir", "a pii scrub", "x, jwt."];

    #[test]
    fn matches_the_workflows_words() {
        for s in HITS {
            assert!(matches(s), "{s}");
        }
        // "oauthish" is neither `oauth` nor an `auth\w*` word; "sqlite" is not `sql`; "reauth" does not start a word with auth.
        for s in ["tidy the README", "oauthish", "sqlite", "reauth", "accelerate", "", "the pi i"] {
            assert!(!matches(s), "{s}");
        }
    }

    #[test]
    fn the_word_list_is_the_workflows() {
        let js = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../workflows/vajra-deliver.js")).unwrap();
        let re = js.lines().find(|l| l.starts_with("const SEC_RE")).unwrap();
        let inner = re.split("\\b(").nth(1).unwrap().split(")\\b").next().unwrap();
        let mut ported: Vec<String> = EXACT.iter().chain(EXACT_MORE.iter()).map(|s| s.to_string()).chain(PREFIX.iter().map(|p| format!("{p}\\w*"))).collect();
        let mut theirs: Vec<String> = inner.split('|').map(str::to_string).collect();
        ported.sort();
        theirs.sort();
        // `auth\w*` and `oauth` etc. appear once each in both lists.
        assert_eq!(ported, theirs);
    }
}
