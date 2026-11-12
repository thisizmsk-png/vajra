//! The engine's Claude Code workflows (`~/.claude/workflows/vajra-*.js`), written by the core from the verified bytes
//! (bhishma C3). The engine installer would otherwise `cp` them, which writes through a symlink or a hard link: a
//! `vajra-deliver.js` hard-linked to `settings.json` would get its contents replaced. So the core refuses any existing
//! one that isn't a single regular file, snapshots them all first, and writes each through a fresh temp file and a
//! rename in the held folder; once they match, the installer finds nothing to copy.

use std::collections::BTreeMap;

use super::bundle::Engine;
use super::fsat::{Dir, Kind, Rename};

pub const DIR: &str = "workflows";

/// The engine's workflows: `workflows/vajra-*.js` in the bundle, by file name.
pub fn shipped(engine: &Engine) -> BTreeMap<String, Vec<u8>> {
    engine
        .files
        .iter()
        .filter_map(|f| {
            let name = f.name.strip_prefix("workflows/")?;
            (name.starts_with("vajra-") && name.ends_with(".js") && !name.contains('/')).then(|| (name.to_string(), f.bytes.clone()))
        })
        .collect()
}

/// What `~/.claude/workflows` held before: each engine workflow's bytes, or None when it wasn't there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot(pub BTreeMap<String, Option<Vec<u8>>>);

/// Why the workflows can't be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// An existing one is a link, a hard link or not a regular file (`workflows_link`).
    Link,
    /// The folder isn't a private folder of the user's.
    NotPrivate,
    Io,
}

impl Refused {
    pub fn code(self) -> &'static str {
        match self {
            Refused::Link => "workflows_link",
            Refused::NotPrivate => "not_writable",
            Refused::Io => "io",
        }
    }
}

/// Open (or create, 0755) `~/.claude/workflows` under the held `claude` folder.
pub fn open(claude: &Dir, uid: u32) -> Result<Dir, Refused> {
    match claude.stat_at(DIR).map_err(|_| Refused::Io)? {
        None => claude.make_dir(DIR, 0o755).map_err(|_| Refused::Io),
        Some(s) if s.private_dir(uid) => claude.open_dir(DIR).map_err(|_| Refused::NotPrivate),
        Some(_) => Err(Refused::NotPrivate),
    }
}

/// Snapshot every name in `names` in `dir`, refusing a link, a hard link or anything but a regular file.
pub fn snapshot(dir: &Dir, names: impl IntoIterator<Item = String>) -> Result<Snapshot, Refused> {
    let mut out = BTreeMap::new();
    for n in names {
        let v = match dir.stat_at(&n).map_err(|_| Refused::Io)? {
            None => None,
            Some(s) if s.kind == Kind::File && s.nlink == 1 => Some(dir.read(&n, 16 << 20).map_err(|_| Refused::Link)?),
            Some(_) => return Err(Refused::Link),
        };
        out.insert(n, v);
    }
    Ok(Snapshot(out))
}

/// The names whose bytes would change.
pub fn changes(snap: &Snapshot, want: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    want.iter().filter(|(n, b)| snap.0.get(*n).and_then(|v| v.as_ref()) != Some(*b)).map(|(n, _)| n.clone()).collect()
}

/// Put `bytes` at `name` in `dir`: a new temp file, then a rename over the name (which replaces a regular file and
/// never writes through a link, since the rename replaces the directory entry itself).
fn put(dir: &Dir, name: &str, bytes: &[u8]) -> Result<(), Refused> {
    let tmp = format!(".{name}.vajra-tmp-{}", std::process::id());
    let _ = dir.remove_file(&tmp);
    dir.write_new(&tmp, bytes, 0o644).map_err(|_| Refused::Io)?;
    dir.rename(&tmp, dir, name, Rename::Replace).map_err(|_| {
        let _ = dir.remove_file(&tmp);
        Refused::Io
    })
}

/// Write every shipped workflow whose bytes differ.
pub fn write(dir: &Dir, want: &BTreeMap<String, Vec<u8>>, snap: &Snapshot) -> Result<(), Refused> {
    for n in changes(snap, want) {
        put(dir, &n, &want[&n])?;
    }
    dir.sync().map_err(|_| Refused::Io)
}

/// Put every snapshotted workflow back exactly: its old bytes, or removed when it wasn't there.
pub fn restore(dir: &Dir, snap: &Snapshot) -> Result<(), Refused> {
    for (n, v) in &snap.0 {
        match v {
            Some(b) => {
                if dir.read(n, 16 << 20).ok().as_deref() != Some(b.as_slice()) {
                    put(dir, n, b)?;
                }
            }
            None => match dir.stat_at(n).map_err(|_| Refused::Io)? {
                Some(s) if s.kind == Kind::File => dir.remove_file(n).map_err(|_| Refused::Io)?,
                Some(_) => return Err(Refused::Link),
                None => {}
            },
        }
    }
    dir.sync().map_err(|_| Refused::Io)
}

#[cfg(test)]
#[path = "workflows_tests.rs"]
mod tests;
