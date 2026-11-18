//! The Files panel's commands: one project's folder tree, a file's content, the files a run changed and one file's diff.
//! Read-only, and Tauri commands only (no MCP tool), so only the person at the window reaches them.
//!
//! The UI names a project by its store id and, optionally, one of that project's run worktrees by its folder name; the
//! core picks the root from that. Every path is relative to the root and goes through `files::Scope` (see there for
//! the rules). The app's data folder and the agent home (which holds the engine state and its signing key) are denied
//! under every root, so a project added at the home folder still cannot show them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{App, Res, fail, invalid};
use crate::files::{self, Content, Refused, Scope, check_rel};
use crate::projects::contract::valid_project_id;
use crate::vajra::bridge::Request;

/// Where a run keeps its worker worktrees, below the project root.
const WORKTREES: &str = ".claude/worktrees";
/// Worktrees listed for one project at most.
const WORKTREES_MAX: usize = 200;
/// Changed files sent at most.
const CHANGED_MAX: usize = 2000;
/// Paths "Open file..." searches at most, and folders a walk outside git opens at most.
const INDEX_MAX: usize = 50_000;
const WALK_DIRS_MAX: usize = 5_000;
/// A worktree folder name: what `git worktree add` under the engine produces.
fn valid_worktree_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 128 && n != "." && n != ".." && n.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn refused(r: Refused) -> Value {
    r.to_json()
}

/// One changed file: `M` modified, `A` added (new or untracked), `D` deleted, `R` renamed (`from` is the old path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub status: char,
    pub from: Option<String>,
}

impl App {
    /// The project's recorded root, canonical.
    fn files_project_root(&self, project_id: &str) -> Result<PathBuf, Value> {
        if !valid_project_id(project_id) {
            return Err(invalid("project_id", "not a project id"));
        }
        let p = self.projects.get(project_id).map_err(|_| fail("not_found", "project not found"))?;
        Path::new(&p.root).canonicalize().map_err(|_| fail("project_missing", "the project folder is gone"))
    }

    /// The root a call reads from: the project's folder, or `<root>/.claude/worktrees/<worktree>` when one is named. A
    /// worktree must be a real folder directly in that place (resolved, so a link that leads elsewhere is refused).
    fn files_root(&self, project_id: &str, worktree: Option<&str>) -> Result<(PathBuf, PathBuf), Value> {
        let project = self.files_project_root(project_id)?;
        let Some(name) = worktree else { return Ok((project.clone(), project)) };
        if !valid_worktree_name(name) {
            return Err(invalid("worktree", "not a worktree name"));
        }
        let parent = project.join(WORKTREES);
        let real = parent.join(name).canonicalize().map_err(|_| fail("not_found", "no such worktree"))?;
        if real.parent() != Some(parent.as_path()) || !real.is_dir() {
            return Err(fail("outside_root", "the worktree leads outside the project"));
        }
        Ok((real, project))
    }

    fn files_scope(&self, root: &Path) -> Result<Scope, Value> {
        let key = self.claude.join("vajra").join(crate::vajra::allowlist::DENIED_NAME);
        Scope::new(root, &[self.data.clone(), self.claude.clone()], &key).map_err(refused)
    }

    /// Run a read-only git command in `cwd`; stdout bytes when it finished whole with exit 0.
    fn files_git(&self, cwd: &Path, args: &[&str]) -> Option<Vec<u8>> {
        let o = self.bridge.run(&Request::Git { cwd: cwd.to_path_buf(), args: args.iter().map(|a| a.to_string()).collect() }).ok()?;
        (o.exit() == Some(0)).then_some(o.stdout_raw)
    }

    /// `files_roots`: the project's root and its run worktrees, each with its branch.
    pub fn files_roots(&self, project_id: &str) -> Res {
        let project = self.files_project_root(project_id)?;
        let name = self.projects.get(project_id).map(|p| p.name).unwrap_or_default();
        let mut worktrees = Vec::new();
        if let Ok(rd) = std::fs::read_dir(project.join(WORKTREES)) {
            let mut names: Vec<String> = rd
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| valid_worktree_name(n))
                .collect();
            names.sort();
            for n in names.into_iter().take(WORKTREES_MAX) {
                let dir = project.join(WORKTREES).join(&n);
                let branch = self.files_git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]).map(|b| String::from_utf8_lossy(&b).trim().to_string());
                worktrees.push(json!({"name": n, "branch": branch}));
            }
        }
        Ok(json!({"project_id": project_id, "name": name, "root": project.to_string_lossy(), "worktrees": worktrees}))
    }

    /// `files_list`: one folder's entries. Each has `ignored` (git's ignore rules); ignored ones are left out unless
    /// `show_ignored`. Outside a git repository nothing is ignored.
    pub fn files_list(&self, project_id: &str, worktree: Option<&str>, path: &str, show_ignored: bool) -> Res {
        let (root, _) = self.files_root(project_id, worktree)?;
        let scope = self.files_scope(&root)?;
        let listing = scope.list(path).map_err(refused)?;
        let ignored = self.files_ignored(&root, path);
        let mut entries = Vec::new();
        for e in listing.entries {
            let rel = if path.is_empty() { e.name.clone() } else { format!("{path}/{}", e.name) };
            let ign = ignored.iter().any(|i| i == &rel);
            if ign && !show_ignored {
                continue;
            }
            entries.push(json!({"name": e.name, "path": rel, "kind": e.kind, "link": e.link, "size": e.size, "ignored": ign}));
        }
        Ok(json!({"path": path, "entries": entries, "truncated": listing.truncated, "skipped": listing.skipped}))
    }

    /// The entries directly in folder `rel` that git ignores (folders too), relative to the root.
    fn files_ignored(&self, root: &Path, rel: &str) -> Vec<String> {
        let spec = if rel.is_empty() { ":(literal).".to_string() } else { format!(":(literal){rel}/") };
        let Some(out) = self.files_git(root, &["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory", "--", &spec]) else {
            return Vec::new();
        };
        out.split(|&b| b == 0)
            .filter_map(|p| std::str::from_utf8(p).ok())
            .map(|p| p.trim_end_matches('/'))
            .filter(|p| !p.is_empty())
            .map(String::from)
            .collect()
    }

    /// `files_index`: every file path in the tree, for "Open file..." (fuzzy find by path). In a git repository it is
    /// what git tracks plus new files it does not ignore (`truncated` when git's answer was cut); in a folder git says is
    /// not a work tree, a walk of the folders without links. Both capped.
    pub fn files_index(&self, project_id: &str, worktree: Option<&str>) -> Res {
        let (root, _) = self.files_root(project_id, worktree)?;
        let scope = self.files_scope(&root)?;
        let deny: Vec<PathBuf> = [&self.data, &self.claude].iter().filter_map(|d| d.canonicalize().ok()).collect();
        let denied = |p: &str| {
            let p = root.join(p);
            deny.iter().any(|d| p.starts_with(d))
        };
        let mut paths: Vec<String> = Vec::new();
        let mut truncated = false;
        // A git folder always gets git's list, even a partial one (output over the bridge's cap, or slow): the plain walk
        // below knows no ignore rules, so it is only for a folder git says is not a work tree.
        let in_git = self.files_git(&root, &["rev-parse", "--is-inside-work-tree"]).is_some_and(|b| b.trim_ascii() == b"true");
        if in_git {
            let o = self
                .bridge
                .run(&Request::Git { cwd: root.clone(), args: ["ls-files", "-z", "--cached", "--others", "--exclude-standard"].map(String::from).to_vec() })
                .map_err(|e| fail("git_failed", e))?;
            let partial = o.truncated || o.timed_out;
            if o.code != Some(0) && !partial {
                return Err(fail("git_failed", "git could not list this tree"));
            }
            truncated |= partial;
            let mut records: Vec<&[u8]> = o.stdout_raw.split(|&b| b == 0).collect();
            // A cut answer's last record may be a cut path.
            if partial {
                records.pop();
            }
            for p in records.into_iter().filter_map(|p| std::str::from_utf8(p).ok()).filter(|p| !p.is_empty()) {
                if check_rel(p).is_err() || denied(p) {
                    continue;
                }
                if paths.len() == INDEX_MAX {
                    truncated = true;
                    break;
                }
                paths.push(p.to_string());
            }
            paths.sort();
            paths.dedup();
        } else {
            // Breadth first, folders only through `Scope::list` (so every rule applies), never through a link.
            let mut queue = std::collections::VecDeque::from([String::new()]);
            let mut visited = 0usize;
            while let Some(dir) = queue.pop_front() {
                visited += 1;
                if visited > WALK_DIRS_MAX {
                    truncated = true;
                    break;
                }
                let Ok(l) = scope.list(&dir) else { continue };
                for e in l.entries {
                    let rel = if dir.is_empty() { e.name } else { format!("{dir}/{}", e.name) };
                    match (e.kind, e.link) {
                        ("dir", false) => queue.push_back(rel),
                        ("file", _) if paths.len() < INDEX_MAX => paths.push(rel),
                        ("file", _) => truncated = true,
                        _ => {}
                    }
                }
            }
        }
        Ok(json!({"paths": paths, "truncated": truncated}))
    }

    /// `files_read`: a file's content. `kind` is `text`, `image` (base64 with its `mime`), `binary` or `too_large`
    /// (with the `cap`); only the first two carry content.
    pub fn files_read(&self, project_id: &str, worktree: Option<&str>, path: &str) -> Res {
        let (root, _) = self.files_root(project_id, worktree)?;
        let scope = self.files_scope(&root)?;
        let r = scope.read(path).map_err(refused)?;
        Ok(match r.content {
            Content::Text(text) => json!({"path": path, "size": r.size, "kind": "text", "text": text}),
            Content::Image { mime, bytes } => {
                json!({"path": path, "size": r.size, "kind": "image", "mime": mime, "base64": crate::providers::install::plan::base64(&bytes)})
            }
            Content::Binary => json!({"path": path, "size": r.size, "kind": "binary"}),
            Content::TooLarge { cap } => json!({"path": path, "size": r.size, "kind": "too_large", "cap": cap}),
        })
    }

    /// The base a tree's changes are counted from: for a worktree, where it left the project's HEAD; for the project
    /// itself, its HEAD (so its changes are what is not committed yet).
    fn files_base(&self, root: &Path, project: &Path) -> Option<String> {
        let text = |b: Vec<u8>| String::from_utf8(b).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let head = self.files_git(project, &["rev-parse", "HEAD"]).and_then(text)?;
        if root == project {
            return Some(head);
        }
        self.files_git(root, &["merge-base", "HEAD", &head]).and_then(text)
    }

    /// `files_changed`: the files changed in the tree since its base (committed or not, plus new untracked files).
    pub fn files_changed(&self, project_id: &str, worktree: Option<&str>) -> Res {
        let (root, project) = self.files_root(project_id, worktree)?;
        let Some(base) = self.files_base(&root, &project) else {
            return Ok(json!({"base": null, "files": [], "truncated": false}));
        };
        // `--relative`: only paths under this folder, relative to it (a project may sit below the repository's top).
        let mut changes = self
            .files_git(&root, &["diff", "--name-status", "-z", "--no-renames", "--relative", &base, "--"])
            .map(|b| parse_name_status(&b))
            .unwrap_or_default();
        if let Some(u) = self.files_git(&root, &["ls-files", "-z", "--others", "--exclude-standard"]) {
            changes.extend(u.split(|&b| b == 0).filter_map(|p| std::str::from_utf8(p).ok()).filter(|p| !p.is_empty()).map(|p| Change { path: p.into(), status: 'A', from: None }));
        }
        // A path inside a protected folder is not even named.
        changes.retain(|c| check_rel(&c.path).is_ok());
        let mut seen = HashMap::new();
        changes.retain(|c| seen.insert(c.path.clone(), ()).is_none());
        let truncated = changes.len() > CHANGED_MAX;
        let files: Vec<Value> = changes.into_iter().take(CHANGED_MAX).map(|c| json!({"path": c.path, "status": c.status.to_string(), "from": c.from})).collect();
        Ok(json!({"base": base, "files": files, "truncated": truncated}))
    }

    /// `files_diff`: one file's diff against the tree's base (as `files_changed` counts it). A new untracked file has no
    /// diff (`untracked: true`); the UI shows its content as all added.
    pub fn files_diff(&self, project_id: &str, worktree: Option<&str>, path: &str) -> Res {
        let (root, project) = self.files_root(project_id, worktree)?;
        if check_rel(path).map_err(refused)?.is_empty() {
            return Err(invalid("path", "names no file"));
        }
        // Git never opens a working file here. The current side is read by `Scope` (every files_read rule: inside the
        // root, outside the denied folders, no links, a regular file, not the key by inode). The base side is the blob
        // at the base commit, from git's object store, used only to compute line marks and then wiped: no base content
        // is sent. A key committed through a hard link lives on in a blob with its own identity, and the app may not
        // read the key to recognise it (NFR-SEC-2), so earlier content stays hidden until the engine-side check ships.
        let scope = self.files_scope(&root)?;
        let new = match scope.read(path) {
            Ok(r) => match r.content {
                Content::Text(t) => Some(t),
                Content::TooLarge { cap } => return Ok(json!({"path": path, "kind": "too_large", "cap": cap})),
                Content::Image { .. } | Content::Binary => return Ok(json!({"path": path, "kind": "binary"})),
            },
            Err(Refused::NotFound) => {
                // Deleted: `resolve` already refused it if its nearest existing folder may not be shown; a path in a
                // denied folder is refused by name too, even when that folder is gone.
                let joined = root.join(path);
                if [&self.data, &self.claude].iter().any(|d| joined.starts_with(d.canonicalize().unwrap_or_else(|_| d.to_path_buf()))) {
                    return Err(refused(Refused::Protected));
                }
                None
            }
            Err(e) => return Err(refused(e)),
        };
        let base = self.files_base(&root, &project).ok_or_else(|| fail("no_base", "the tree has no base to compare with"))?;
        // The base side, from git's object store only (`<commit>:<path>` takes the path literally).
        // `<commit>:./<path>` is relative to the folder git runs in, so a project below the repository's top reads its
        // own file, not a top-level one of the same name.
        let object = format!("{base}:./{path}");
        let old = match self.files_git(&root, &["cat-file", "-t", &object]).map(|b| String::from_utf8_lossy(&b).trim().to_string()).as_deref() {
            None => None,
            Some("blob") => {
                let o = self
                    .bridge
                    .run(&Request::Git { cwd: root.clone(), args: ["cat-file", "blob", &object].map(String::from).to_vec() })
                    .map_err(|e| fail("git_failed", e))?;
                if o.truncated || o.timed_out || o.stdout_raw.len() as u64 > files::TEXT_CAP {
                    return Ok(json!({"path": path, "kind": "too_large", "cap": files::TEXT_CAP}));
                }
                if o.code != Some(0) {
                    return Err(fail("git_failed", "git could not read this file at the base"));
                }
                let blob = files::Wiped::from(o.stdout_raw);
                if files::looks_binary(&blob) {
                    return Ok(json!({"path": path, "kind": "binary"}));
                }
                // Lossy is fine: only line boundaries and equality are used, and nothing of it is sent.
                Some(blob)
            }
            // A folder (tree) or a submodule (commit) at the base.
            Some(_) => return Err(refused(Refused::NotAFile)),
        };
        let notice = "Earlier content is hidden until the key check ships.";
        let Some(new) = new else {
            if old.is_none() {
                return Err(refused(Refused::NotFound));
            }
            // Deleted since the base: nothing current to show, and the old content is not sent.
            return Ok(json!({"path": path, "base": base, "kind": "deleted", "base_hidden": true, "notice": notice}));
        };
        let m = match &old {
            Some(blob) => files::diff::marks(&String::from_utf8_lossy(blob), &new),
            None => files::diff::Marks { added: (1..=new.split_inclusive('\n').count() as u32).collect(), removed: vec![] },
        };
        Ok(json!({
            "path": path, "base": base, "kind": "text", "text": new,
            "added": m.added, "removed": m.removed.iter().map(|(a, n)| json!([a, n])).collect::<Vec<_>>(),
            "untracked": old.is_none(), "base_hidden": true, "notice": notice,
        }))
    }
}

/// `git diff --name-status -z` output: `<status>\0<path>\0`, with renames and copies `<status>\0<old>\0<new>\0`.
pub fn parse_name_status(b: &[u8]) -> Vec<Change> {
    let mut parts = b.split(|&c| c == 0).map(|p| String::from_utf8_lossy(p).into_owned());
    let mut out = Vec::new();
    while let Some(st) = parts.next() {
        let Some(code) = st.chars().next() else { continue };
        match code {
            'R' | 'C' => {
                let (Some(from), Some(to)) = (parts.next(), parts.next()) else { break };
                out.push(Change { path: to, status: if code == 'R' { 'R' } else { 'A' }, from: Some(from) });
            }
            _ => {
                let Some(p) = parts.next() else { break };
                let status = match code {
                    'A' => 'A',
                    'D' => 'D',
                    _ => 'M',
                };
                out.push(Change { path: p, status, from: None });
            }
        }
    }
    out
}

/// The file kinds `files::Scope` caps, re-exported for the UI's contract test.
pub const TEXT_CAP: u64 = files::TEXT_CAP;
