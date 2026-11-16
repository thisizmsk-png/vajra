//! The Terminal line for installing the engine by hand (red team G2 M1). A downloaded app has no engine in the skills
//! folder yet, so a line that runs the installer from there fails on a clean Mac. When this build pinned its bundled
//! engine, the line installs from that copy instead: it checks the copy's sha256 against the hash compiled into this
//! binary, unpacks it into a skills folder it has just made, then runs its installer. Built-in macOS tools only, each by
//! its absolute system path (never found through PATH), so nothing is downloaded and no repository is cloned.
//!
//! What the line guarantees: the bundle matches this build's pin when it is read, and the folder it unpacks into did
//! not exist when it was pasted (a plain `mkdir`, no `-p`: a folder, file or symlink already there stops it). It is not
//! the one-click path: no held fds, no re-hash around the installer, no lock (bhishma on #283).

use std::path::Path;

use super::bundle::RESOURCE;
use super::flow::INSTALLER;
use super::place::SKILL;
use super::Pin;

/// The line to show, and whether a leftover folder is in the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub command: String,
    /// The skills folder exists but holds no installer (left by an earlier try, or something else): the user should
    /// remove it, then the bundled line is offered. The catalog's command is shown meanwhile.
    pub leftover: bool,
}

/// The line, given the user's home, the app's resources folder (None before the window sets it) and this build's pin.
/// `fallback` is the catalog's own command. It is kept whenever the skills folder exists in any form (a copy runs its
/// own installer; nothing is ever unpacked over what is there) and when there is no pinned bundle.
pub fn line(home: &Path, resources: Option<&Path>, pin: &Pin, fallback: &str) -> Line {
    let skill = home.join(".claude/skills").join(SKILL);
    if std::fs::symlink_metadata(&skill).is_ok() {
        // `metadata` follows a symlinked folder to the copy it points at, as the catalog's command would.
        let has_installer = std::fs::metadata(skill.join(INSTALLER)).is_ok_and(|m| m.is_file());
        return Line { command: fallback.to_string(), leftover: !has_installer };
    }
    Line { command: bundled(resources, pin).unwrap_or_else(|| fallback.to_string()), leftover: false }
}

fn bundled(resources: Option<&Path>, pin: &Pin) -> Option<String> {
    if !pin.tree_ok || pin.sha256.len() != 64 || !pin.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let tgz = resources?.join(RESOURCE);
    if !std::fs::symlink_metadata(&tgz).is_ok_and(|m| m.file_type().is_file()) {
        return None;
    }
    command(tgz.to_str()?, &pin.sha256)
}

/// The longest engine command the setup list may carry: `parseSetupList` (setupContract.ts) refuses the whole list
/// past it. Counted here in UTF-8 bytes, which is never fewer than the UI's UTF-16 count, so a line that fits here
/// fits there. The line names the bundle's path twice, so an app installed at a very deep path gets the catalog line
/// instead of a list the UI can't read (Codex P2 on #283).
pub const MAX_COMMAND: usize = 1024;

/// The bundled line for the tarball at `tgz` pinned to `sha256`, or None when the path can't be quoted safely or the
/// line would not fit the setup list.
pub(crate) fn command(tgz: &str, sha256: &str) -> Option<String> {
    if !quotable(tgz) {
        return None;
    }
    let dir = format!("~/.claude/skills/{SKILL}");
    let line = format!(
        "echo '{sha256}  {tgz}' | /usr/bin/shasum -a 256 -c && /bin/mkdir -p ~/.claude/skills && /bin/mkdir {dir} && /usr/bin/tar -xzf '{tgz}' -C {dir} && /bin/bash {dir}/{INSTALLER}"
    );
    (line.len() <= MAX_COMMAND).then_some(line)
}

/// A path that is safe inside single quotes on one pasted line, and in a `shasum -c` line: absolute, with no quote,
/// backslash or control character (a pasted newline would run what follows it), and nothing that renders invisibly or
/// reorders the line: Unicode line and paragraph separators, bidi controls and zero-width marks.
fn quotable(p: &str) -> bool {
    p.starts_with('/') && !p.chars().any(|c| c == '\'' || c == '\\' || c.is_control() || invisible(c))
}

fn invisible(c: char) -> bool {
    matches!(c, '\u{00ad}' | '\u{034f}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}')
}

#[cfg(test)]
#[path = "manual_tests.rs"]
mod tests;
