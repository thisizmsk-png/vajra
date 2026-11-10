//! The crew roles a proposal seat can take (ADR-16 16.1), as data. The seven writer roles map the workflow's
//! `ALL_WRITERS` agents; `verifier`, `reviewer` and `security_reviewer` are the read-only seats.

/// What a seat may do in its worktree (the "role kind" of ADR-16 16.4, as far as 6a needs it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The run's lead: read-only, no network, drives the run through the run protocol.
    Lead,
    /// Edits and commits in its own worktree.
    Writer,
    /// Read, Grep, Glob and Bash; no Edit or Write. The core never snapshots or commits its worktree.
    Checker,
}

pub const VERIFIER: &str = "verifier";
pub const REVIEWER: &str = "reviewer";
pub const SECURITY_REVIEWER: &str = "security_reviewer";

pub const ROLES: [(&str, Kind); 10] = [
    ("backend", Kind::Writer),
    ("frontend", Kind::Writer),
    ("tests", Kind::Writer),
    ("docs", Kind::Writer),
    ("platform", Kind::Writer),
    ("numerics", Kind::Writer),
    ("analysis", Kind::Writer),
    (VERIFIER, Kind::Checker),
    (REVIEWER, Kind::Checker),
    (SECURITY_REVIEWER, Kind::Checker),
];

pub fn is_role(role: &str) -> bool {
    ROLES.iter().any(|(r, _)| *r == role)
}

pub fn kind_of(role: &str) -> Option<Kind> {
    ROLES.iter().find(|(r, _)| *r == role).map(|(_, k)| *k)
}

/// The role of one of the deliver workflow's writer agents (`ALL_WRITERS` in `workflows/vajra-deliver.js`).
pub fn role_of(writer_id: &str) -> Option<&'static str> {
    Some(match writer_id {
        "bhima" | "sepoy-backend-1" => "backend",
        "nakula" | "sepoy-frontend-1" => "frontend",
        "sepoy-test-1" => "tests",
        "abhimanyu" => "docs",
        "hanuman" => "platform",
        "karna" => "numerics",
        "sahadeva" => "analysis",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys of the `ALL_WRITERS` object literal in the workflow.
    fn workflow_writers() -> Vec<String> {
        let js = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../workflows/vajra-deliver.js")).unwrap();
        let body = js.split("const ALL_WRITERS = {").nth(1).unwrap().split("\n}").next().unwrap();
        body.lines().filter_map(|l| l.trim().split(':').next()).map(|k| k.trim_matches('\'').to_string()).filter(|k| !k.is_empty()).collect()
    }

    #[test]
    fn every_workflow_writer_has_a_writer_role() {
        let keys = workflow_writers();
        assert!(keys.len() >= 9, "parsed {keys:?}");
        for k in &keys {
            let r = role_of(k).unwrap_or_else(|| panic!("{k} has no role"));
            assert_eq!(kind_of(r), Some(Kind::Writer), "{k}");
        }
    }

    #[test]
    fn every_writer_role_is_used_by_a_workflow_writer() {
        let used: Vec<_> = workflow_writers().iter().filter_map(|k| role_of(k)).collect();
        for (r, k) in ROLES {
            assert!(k == Kind::Checker || used.contains(&r), "{r}");
        }
    }

    #[test]
    fn the_checker_roles_are_not_writers() {
        for r in [VERIFIER, REVIEWER, SECURITY_REVIEWER] {
            assert_eq!(kind_of(r), Some(Kind::Checker));
        }
        assert!(!is_role("architect") && role_of("arjuna").is_none());
    }
}
