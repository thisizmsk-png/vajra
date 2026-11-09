//! The Lead's `--settings` file (ADR-13 Decision 2a), generated and never hand-written.
//!
//! The Lead keeps Read, Grep and Glob, and `--setting-sources ""` drops every user, project and local setting,
//! including the Vajra guard. This file puts the guard back (hooks copied from `hooks/hooks.json`) and adds
//! `permissions.deny` for the HMAC key, Vajra state, app control files and credential paths.
//!
//! Every deny rule is `Read(//<absolute path>)`. The vajra-14 spike found that a rule with a single leading `/`
//! fails open (the Read succeeded and returned the canary); only the double-slash form denies. So rules come from
//! one fixed table of (base, suffix) pairs and are built here, from canonical absolute bases, and nowhere else.
//! `settings()` takes no rule list, and nothing reads rules from user input or config.

use std::path::Path;

use serde_json::json;

use super::lead::LeadError;
use crate::sessions::argv::LEAD_TOOLS;

/// Rules under `$HOME`. A suffix ending in `/**` covers a directory, anything else a file or a name pattern.
pub const HOME_RULES: [&str; 17] = [
    ".claude/vajra/**",
    ".claude/skills/**",
    ".claude/settings*.json",
    ".claude/.credentials*",
    ".claude.json",
    "Library/Application Support/VajraDesktop/**",
    ".ssh/**",
    ".aws/**",
    ".gnupg/**",
    ".kube/**",
    ".config/gh/**",
    ".docker/config.json",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".pgpass",
    ".git-credentials",
];

/// Rules under the run's repo.
pub const REPO_RULES: [&str; 5] = ["**/.env", "**/.env.*", "**/id_rsa*", "**/id_ed25519*", "**/id_ecdsa*"];

/// Rules under the app data dir (it can differ from the default `$HOME` location through `VAJRA_DESKTOP_DATA`).
pub const DATA_RULES: [&str; 1] = ["**"];

/// Rules under every other project's folder (ADR-15 D8): one project's Lead never reads another project's code or
/// secrets into its transcript.
pub const OTHER_PROJECT_RULES: [&str; 1] = ["**"];

/// The hook entries, byte for byte what `hooks/hooks.json` registers for the guard and the Atman observer.
/// `desktop-control.sh` is left out on purpose: it serves workers only, and the Lead has no `agent_id`.
pub const PRE_TOOL_USE_MATCHER: &str = "Bash|Write|Edit|MultiEdit|NotebookEdit|Read|Grep|Glob|LS|mcp__vajra-desktop__.*";
pub const GUARD_COMMAND: &str = "bash \"$HOME/.claude/skills/vajra/hooks/pre-tool-use.sh\"";
pub const OBSERVER_COMMAND: &str = "bash \"$HOME/.claude/skills/vajra/hooks/post-tool-use.sh\"";

fn guard_missing(why: impl Into<String>) -> LeadError {
    LeadError::GuardMissing(why.into())
}

/// Characters that would change what a rule means: glob syntax, parens (the rule's own delimiter), a newline or
/// NUL, and a backslash. A base holding one is refused, never escaped.
fn base_ok(s: &str) -> bool {
    s.starts_with('/') && s.len() > 1 && !s.contains(['(', ')', '*', '?', '[', ']', '{', '}', '\\', '\n', '\r', '\0'])
}

/// The base as given (trailing slashes dropped) and its realpath when that differs. A path that cannot be
/// canonicalised (it does not exist yet) has only its given form.
fn forms(base: &Path, what: &str) -> Result<Vec<String>, LeadError> {
    let given = base.to_str().ok_or_else(|| guard_missing(format!("{what} is not valid UTF-8")))?;
    let given = given.trim_end_matches('/');
    if !base_ok(given) {
        return Err(guard_missing(format!("{what} is not a plain absolute path")));
    }
    let mut out = vec![given.to_string()];
    if let Ok(real) = std::fs::canonicalize(base) {
        let real = real.to_str().ok_or_else(|| guard_missing(format!("{what} is not valid UTF-8")))?.trim_end_matches('/').to_string();
        if !base_ok(&real) {
            return Err(guard_missing(format!("{what} resolves to a path that is not plain")));
        }
        if real != out[0] {
            out.push(real);
        }
    }
    Ok(out)
}

/// `Read(//<abs without its leading slash>/<suffix>)`.
fn rule(base: &str, suffix: &str) -> String {
    format!("Read(//{}/{suffix})", &base[1..])
}

/// Every deny rule for this machine, deduped, in table order. A base that is not a plain absolute path is refused
/// (`lead_guard_missing`). When a base differs from its realpath (macOS `/var` → `/private/var`, a symlinked HOME
/// or repo) both forms are emitted: the spike did not show the twin is needed, but emitting it costs nothing.
pub fn deny_rules(home: &Path, repo: &Path, data: &Path) -> Result<Vec<String>, LeadError> {
    deny_rules_with(home, repo, data, &[])
}

/// `deny_rules` plus `Read(//<root>/**)` for each of `others`, the folders of the app's other projects (ADR-15 D8). The
/// caller leaves out its own project and any folder that holds it. A root that is not a plain absolute path is
/// refused like any other base: the Lead fails closed rather than run without that rule.
pub fn deny_rules_with(home: &Path, repo: &Path, data: &Path, others: &[std::path::PathBuf]) -> Result<Vec<String>, LeadError> {
    deny_rules_iso(home, repo, data, others, &[])
}

/// `deny_rules_with` plus the secret-file rules (`REPO_RULES`) for each of `secret_dirs`: other projects' folders that hold the Lead's
/// own folder (a project nested inside another one), which cannot be denied whole without denying the Lead itself.
pub fn deny_rules_iso(home: &Path, repo: &Path, data: &Path, others: &[std::path::PathBuf], secret_dirs: &[std::path::PathBuf]) -> Result<Vec<String>, LeadError> {
    deny_rules_for_iso(home, std::slice::from_ref(&repo.to_path_buf()), data, others, secret_dirs)
}

/// `deny_rules_with` where the project has more than one folder (its main checkout plus worktrees or run folders
/// elsewhere): the repo secret-file rules (`.env`, `id_rsa*`, ...) are generated under EVERY one of `repos`, so a
/// worktree outside the main checkout keeps them. At least one repo is required.
pub fn deny_rules_for(home: &Path, repos: &[std::path::PathBuf], data: &Path, others: &[std::path::PathBuf]) -> Result<Vec<String>, LeadError> {
    deny_rules_for_iso(home, repos, data, others, &[])
}

/// `deny_rules_for` plus secret-file rules for `secret_dirs` (see `deny_rules_iso`).
pub fn deny_rules_for_iso(home: &Path, repos: &[std::path::PathBuf], data: &Path, others: &[std::path::PathBuf], secret_dirs: &[std::path::PathBuf]) -> Result<Vec<String>, LeadError> {
    if repos.is_empty() {
        return Err(LeadError::GuardMissing("no project folder to guard".into()));
    }
    let mut out: Vec<String> = Vec::new();
    let mut push = |r: String| {
        if !out.contains(&r) {
            out.push(r);
        }
    };
    let bases: Vec<(&Path, &str, &[&str])> = std::iter::once((home, "HOME", &HOME_RULES[..]))
        .chain(repos.iter().map(|r| (r.as_path(), "repo", &REPO_RULES[..])))
        .chain(std::iter::once((data, "data dir", &DATA_RULES[..])))
        .collect();
    for (base, what, table) in bases {
        for form in forms(base, what)? {
            for suffix in table {
                push(rule(&form, suffix));
            }
        }
    }
    for other in others {
        for form in forms(other, "another project's folder")? {
            for suffix in OTHER_PROJECT_RULES {
                push(rule(&form, suffix));
            }
        }
    }
    for dir in secret_dirs {
        for form in forms(dir, "the folder of a project holding this one")? {
            for suffix in REPO_RULES {
                push(rule(&form, suffix));
            }
        }
    }
    Ok(out)
}

fn render(rules: &[String], hooks: bool) -> String {
    let allow: Vec<&str> = LEAD_TOOLS.split(',').collect();
    let mut v = json!({
        "permissions": {
            "defaultMode": "dontAsk",
            "disableBypassPermissionsMode": "disable",
            "allow": allow,
            "deny": rules,
        }
    });
    if hooks {
        v["hooks"] = json!({
            "PreToolUse": [{
                "matcher": PRE_TOOL_USE_MATCHER,
                "hooks": [{"type": "command", "command": GUARD_COMMAND, "timeout": 10, "onFailure": "block"}],
            }],
            "PostToolUse": [{
                "matcher": "*",
                "hooks": [{"type": "command", "command": OBSERVER_COMMAND, "async": true, "timeout": 30}],
            }],
        });
    }
    let mut s = serde_json::to_string_pretty(&v).expect("a json! value serialises");
    s.push('\n');
    s
}

/// The Lead's settings file, as the bytes the core writes: the guard hooks and the generated deny rules.
pub fn settings(home: &Path, repo: &Path, data: &Path) -> Result<String, LeadError> {
    settings_with(home, repo, data, &[])
}

/// `settings` with the other projects' folders denied too (ADR-15 D8).
pub fn settings_with(home: &Path, repo: &Path, data: &Path, others: &[std::path::PathBuf]) -> Result<String, LeadError> {
    Ok(render(&deny_rules_with(home, repo, data, others)?, true))
}

/// `settings_with` for a project with several folders (see `deny_rules_for`).
pub fn settings_for(home: &Path, repos: &[std::path::PathBuf], data: &Path, others: &[std::path::PathBuf]) -> Result<String, LeadError> {
    Ok(render(&deny_rules_for(home, repos, data, others)?, true))
}

/// `settings_no_hooks_with` for a project with several folders.
pub fn settings_no_hooks_for(home: &Path, repos: &[std::path::PathBuf], data: &Path, others: &[std::path::PathBuf]) -> Result<String, LeadError> {
    Ok(render(&deny_rules_for(home, repos, data, others)?, false))
}

/// `settings_with` plus secret-file rules for the folders that hold this project (`deny_rules_iso`).
pub fn settings_iso(home: &Path, repo: &Path, data: &Path, others: &[std::path::PathBuf], secret_dirs: &[std::path::PathBuf]) -> Result<String, LeadError> {
    Ok(render(&deny_rules_iso(home, repo, data, others, secret_dirs)?, true))
}

/// `settings_iso` with the hooks removed (the startup self-test).
pub fn settings_no_hooks_iso(home: &Path, repo: &Path, data: &Path, others: &[std::path::PathBuf], secret_dirs: &[std::path::PathBuf]) -> Result<String, LeadError> {
    Ok(render(&deny_rules_iso(home, repo, data, others, secret_dirs)?, false))
}

/// `settings_for` plus secret-file rules for `secret_dirs`.
pub fn settings_for_iso(home: &Path, repos: &[std::path::PathBuf], data: &Path, others: &[std::path::PathBuf], secret_dirs: &[std::path::PathBuf]) -> Result<String, LeadError> {
    Ok(render(&deny_rules_for_iso(home, repos, data, others, secret_dirs)?, true))
}

/// `settings_no_hooks_for` plus secret-file rules for `secret_dirs`.
pub fn settings_no_hooks_for_iso(home: &Path, repos: &[std::path::PathBuf], data: &Path, others: &[std::path::PathBuf], secret_dirs: &[std::path::PathBuf]) -> Result<String, LeadError> {
    Ok(render(&deny_rules_for_iso(home, repos, data, others, secret_dirs)?, false))
}

/// The same file with the hooks removed. Used by the startup self-test and the live deny-rule tests, which must
/// prove the deny layer on its own. Never passed to a real ask.
pub fn settings_no_hooks(home: &Path, repo: &Path, data: &Path) -> Result<String, LeadError> {
    settings_no_hooks_with(home, repo, data, &[])
}

/// `settings_no_hooks` with the other projects' folders denied too.
pub fn settings_no_hooks_with(home: &Path, repo: &Path, data: &Path, others: &[std::path::PathBuf]) -> Result<String, LeadError> {
    Ok(render(&deny_rules_with(home, repo, data, others)?, false))
}
