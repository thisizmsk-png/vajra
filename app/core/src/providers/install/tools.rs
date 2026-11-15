//! Tool resolution (ADR-17 rule 3, bhishma condition 4): `brew`, and `npm` with the `node` it runs on, as absolute
//! canonical paths under the known roots only. A tool in a folder a worker could write is never used.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Where a tool may live.
#[derive(Debug, Clone)]
pub struct Roots {
    /// System and Homebrew prefixes: a tool's canonical path must be inside one.
    pub system: Vec<PathBuf>,
    /// Homebrew prefixes, the only places `brew` is looked for.
    pub homebrew: Vec<PathBuf>,
    /// nvm's version folders: one counts only when the login PATH names its `bin`.
    pub nvm: PathBuf,
    /// Volta: always "Install it yourself" (its shims choose a node at run time).
    pub volta: PathBuf,
}

impl Roots {
    pub fn of(home: &Path) -> Self {
        Roots {
            system: ["/usr", "/bin", "/sbin", "/opt/homebrew", "/usr/local"].map(PathBuf::from).to_vec(),
            homebrew: ["/opt/homebrew", "/usr/local"].map(PathBuf::from).to_vec(),
            nvm: home.join(".nvm/versions/node"),
            volta: home.join(".volta"),
        }
    }
}

/// A resolved tool, as the install sheet shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    pub path: PathBuf,
    /// "nvm" for an nvm node; None otherwise.
    pub label: Option<&'static str>,
    /// The file belongs to the user (a user-owned Homebrew prefix): the sheet says "owned by you".
    pub owned_by_you: bool,
}

/// npm run without its `#!/usr/bin/env node` line: `[node, npm-cli.js, ...]`, both from one prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Npm {
    pub node: Tool,
    pub cli: Tool,
    pub prefix: PathBuf,
}

impl Npm {
    pub fn argv(&self, args: &[&str]) -> Vec<OsString> {
        [self.node.path.clone().into_os_string(), self.cli.path.clone().into_os_string()].into_iter().chain(args.iter().map(OsString::from)).collect()
    }
}

/// A candidate that was not used, and why (shown on the sheet when nothing usable was found).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub path: PathBuf,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct Tools {
    pub brew: Option<Tool>,
    pub npm: Option<Npm>,
    pub refused: Vec<Refused>,
}

/// Resolve the tools for these roots and the user's login PATH (its folders, in order). Nothing is run.
pub fn resolve(roots: &Roots, login_path: &[PathBuf]) -> Tools {
    let canon = |v: &[PathBuf]| v.iter().filter_map(|p| p.canonicalize().ok()).collect::<Vec<_>>();
    let (system, homebrew) = (canon(&roots.system), canon(&roots.homebrew));
    let nvm = roots.nvm.canonicalize().ok();
    let volta = roots.volta.canonicalize().ok();
    let mut t = Tools::default();

    for hb in &roots.homebrew {
        let cand = hb.join("bin/brew");
        if std::fs::symlink_metadata(&cand).is_err() {
            continue;
        }
        match regular(&cand) {
            Some((path, owned)) if homebrew.iter().any(|r| path.starts_with(r)) => {
                t.brew = Some(Tool { path, label: None, owned_by_you: owned });
                break;
            }
            _ => t.refused.push(Refused { path: cand, reason: "outside_tool_roots" }),
        }
    }

    for dir in login_path {
        let cand = dir.join("npm");
        if std::fs::symlink_metadata(&cand).is_err() {
            continue;
        }
        let Some((cli, cli_owned)) = regular(&cand) else {
            t.refused.push(Refused { path: cand, reason: "unresolvable" });
            continue;
        };
        if volta.as_ref().is_some_and(|v| cli.starts_with(v) || cand.starts_with(v)) || cand.starts_with(&roots.volta) {
            // Volta picks a node at run time: Vajra can't show which one would run.
            t.refused.push(Refused { path: cand, reason: "volta" });
            break;
        }
        // The root the tool must stay inside, and its label.
        let (root, label) = match nvm.as_ref().and_then(|n| nvm_version(&cli, n)) {
            Some(version) => {
                if dir.canonicalize().ok() != Some(version.join("bin")) {
                    t.refused.push(Refused { path: cand, reason: "nvm_not_on_path" });
                    continue;
                }
                (version, Some("nvm"))
            }
            None => match system.iter().find(|r| cli.starts_with(r)) {
                Some(r) => (r.clone(), None),
                None => {
                    t.refused.push(Refused { path: cand, reason: "outside_tool_roots" });
                    continue;
                }
            },
        };
        let Some(prefix) = npm_prefix(&cli) else {
            t.refused.push(Refused { path: cand, reason: "not_npm_cli" });
            continue;
        };
        match regular(&prefix.join("bin/node")) {
            Some((node, node_owned)) if node.starts_with(&root) && prefix.starts_with(&root) => {
                t.npm = Some(Npm { node: Tool { path: node, label, owned_by_you: node_owned && label.is_none() }, cli: Tool { path: cli, label, owned_by_you: cli_owned && label.is_none() }, prefix });
                break;
            }
            _ => t.refused.push(Refused { path: cand, reason: "node_outside_prefix" }),
        }
    }
    t
}

/// The canonical path of `p` when it is a regular file, and whether the user owns it.
fn regular(p: &Path) -> Option<(PathBuf, bool)> {
    use std::os::unix::fs::MetadataExt;
    let c = p.canonicalize().ok()?;
    let m = std::fs::metadata(&c).ok()?;
    // SAFETY: geteuid has no preconditions.
    m.is_file().then(|| (c, m.uid() == unsafe { libc::geteuid() }))
}

/// `<nvm>/<version>` when `cli` is inside an nvm version folder.
fn nvm_version(cli: &Path, nvm: &Path) -> Option<PathBuf> {
    let rest = cli.strip_prefix(nvm).ok()?;
    let first = rest.components().next()?;
    Some(nvm.join(first))
}

/// The prefix of `<prefix>/lib/node_modules/npm/bin/npm-cli.js`.
fn npm_prefix(cli: &Path) -> Option<PathBuf> {
    let tail = ["npm-cli.js", "bin", "npm", "node_modules", "lib"];
    let mut p = cli;
    for want in tail {
        if p.file_name()? != want {
            return None;
        }
        p = p.parent()?;
    }
    Some(p.to_path_buf())
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
