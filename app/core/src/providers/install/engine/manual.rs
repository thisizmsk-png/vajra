//! The Terminal line for installing the engine by hand (red team G2 M1). A downloaded app has no engine in the skills
//! folder yet, so a line that runs the installer from there fails on a clean Mac. When this build pinned its bundled
//! engine, the line installs from that copy instead: it checks the copy's sha256 against the hash compiled into this
//! binary, unpacks it into the skills folder, then runs its installer. Built-in macOS tools only (`shasum`, `tar`,
//! `bash`), so nothing is downloaded and no repository is cloned.

use std::path::Path;

use super::bundle::RESOURCE;
use super::flow::INSTALLER;
use super::place::SKILL;
use super::Pin;

/// The skill folder as the line names it.
fn skill_dir() -> String {
    format!("~/.claude/skills/{SKILL}")
}

/// The line to show, given the user's home, the app's resources folder (None before the window sets it) and this
/// build's pin. `fallback` is the catalog's own command: it is kept when the skill folder already holds an installer
/// (a copy is there, so its own installer is the one to run) or a git checkout, and when there is no pinned bundle.
pub fn line(home: &Path, resources: Option<&Path>, pin: &Pin, fallback: &str) -> String {
    bundled(home, resources, pin).unwrap_or_else(|| fallback.to_string())
}

fn bundled(home: &Path, resources: Option<&Path>, pin: &Pin) -> Option<String> {
    let skill = home.join(".claude/skills").join(SKILL);
    if std::fs::symlink_metadata(skill.join(INSTALLER)).is_ok() || std::fs::symlink_metadata(skill.join(".git")).is_ok() {
        return None;
    }
    if !pin.tree_ok || pin.sha256.len() != 64 || !pin.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let tgz = resources?.join(RESOURCE);
    if !std::fs::symlink_metadata(&tgz).is_ok_and(|m| m.file_type().is_file()) {
        return None;
    }
    let tgz = tgz.to_str().filter(|p| quotable(p))?;
    let dir = skill_dir();
    Some(format!(
        "echo '{sha}  {tgz}' | shasum -a 256 -c && mkdir -p {dir} && tar -xzf '{tgz}' -C {dir} && bash {dir}/{INSTALLER}",
        sha = pin.sha256
    ))
}

/// A path that is safe inside single quotes on one pasted line, and in a `shasum -c` line: absolute, with no quote,
/// backslash or control character (a pasted newline would run what follows it).
fn quotable(p: &str) -> bool {
    p.starts_with('/') && !p.chars().any(|c| c == '\'' || c == '\\' || c.is_control())
}

#[cfg(test)]
#[path = "manual_tests.rs"]
mod tests;
