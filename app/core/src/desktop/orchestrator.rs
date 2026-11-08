//! One orchestrator per project (ADR-15 2, owner decisions D3 and D4 option A). The user-facing name is
//! "Director-General"; code ids stay `orchestrator` / `lead`.
//!
//! The orchestrator is a persistent identity, not a process: a chain of native Claude Code sessions, one per
//! **generation**, kept under the guarded control dir at `<data>/control/projects/<pid>/orchestrator/`:
//! - `root`: the project folder its turns run in, written once (first writer wins);
//! - `generation`: the current conversation number, from 1; "New conversation" bumps it;
//! - `session`: the current generation's native session id, written once the CLI reports `init`;
//! - `settings.json`, `mcp.json`: regenerated and checked byte for byte on every ask, as for the ADR-13 Lead.
//!
//! Each turn is one headless `claude -p --resume` (ADR-13's mechanism, scope widened from run to project): its MCP
//! calls are bound to `(project, generation)` and see only this project's live runs (`Service::call_tool_for_project`).
//!
//! **The project id is a stand-in** until the project store (FG-01) lands: `k_` and a digest of the project key
//! (`caps::project_key`, the git common dir). It is pinned in the guarded run record at launch, so the app's own
//! record decides which project a run belongs to, not a later re-read of the repo.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{App, Res, caps};
use crate::control::lead::LeadError;
use crate::control::{fsx, sha256};
use crate::ids;
use crate::lock;

/// The user-facing name of the per-project orchestrator.
pub(super) const NAME: &str = "Director-General";
/// Director-General turns in flight across the whole app (ADR-15 2.3). One per orchestrator on top of that.
pub(super) const MAX_TURNS: usize = 2;
/// The project id file in a run's guarded record (`<data>/control/lead/<run_id>/`).
pub(super) const PROJECT_FILE: &str = "project";
const ROOT_FILE: &str = "root";
const GEN_FILE: &str = "generation";
pub(super) const SESSION_FILE: &str = "session";
/// Most generations a project can go through; far beyond any real use, and keeps the counter well inside `u32`.
const MAX_GENERATION: u32 = 1_000_000;

/// A project's orchestrator as it is on disk now.
#[derive(Clone, Debug)]
pub(super) struct Orchestrator {
    pub id: String,
    pub root: PathBuf,
    pub dir: PathBuf,
    pub generation: u32,
    pub session: Option<String>,
}

impl Orchestrator {
    /// The bus stream of the current conversation. A new generation starts on an empty stream.
    pub fn stream(&self) -> String {
        stream(&self.id, self.generation)
    }
}

pub(super) fn stream(project: &str, generation: u32) -> String {
    format!("orchestrator-{project}-g{generation}")
}

/// The project a run folder belongs to: its stand-in id and the folder the Director-General's turns run in (the repo
/// root for a folder inside a repo or one of its worktrees, else the folder itself).
pub(super) fn project_ref(cwd: &Path) -> (String, PathBuf) {
    let key = caps::project_key(cwd);
    let digest = sha256::hex_digest(key.as_os_str().as_encoded_bytes());
    let root = match (key.file_name().and_then(|n| n.to_str()), key.parent()) {
        (Some(".git"), Some(p)) if p.is_dir() => p.to_path_buf(),
        _ => std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf()),
    };
    (format!("k_{}", &digest[..32]), root)
}

fn read_text(p: &Path) -> Option<String> {
    let body = fsx::read_nofollow(p).ok()?;
    Some(String::from_utf8(body).ok()?.trim().to_string())
}

fn io(e: impl std::fmt::Display) -> LeadError {
    LeadError::Io(e.to_string())
}

impl App {
    /// `<data>/control/projects/<pid>/orchestrator/`.
    fn orchestrator_dir(&self, pid: &str) -> PathBuf {
        self.data.join("control").join("projects").join(pid).join("orchestrator")
    }

    /// The project of a run the app recorded: the id pinned in its guarded record at launch. A record from before
    /// projects existed gets one now, computed from its (guarded) directory, and pinned the same way.
    pub(super) fn run_project(&self, run_id: &str, cwd: &Path) -> Result<(String, PathBuf), LeadError> {
        let dir = self.data.join("control").join("lead").join(run_id);
        let (pid, root) = match read_text(&dir.join(PROJECT_FILE)) {
            Some(p) if ids::valid_project_ref(&p) => (p, project_ref(cwd).1),
            Some(_) => return Err(LeadError::GuardMissing("the run's project record is not valid".into())),
            None => {
                let (p, root) = project_ref(cwd);
                if dir.is_dir() {
                    fsx::write_atomic(&dir, PROJECT_FILE, p.as_bytes()).map_err(io)?;
                }
                (p, root)
            }
        };
        Ok((pid, root))
    }

    /// The project's orchestrator, created on first use with generation 1 and no session yet (ADR-15 2.5). Its root is
    /// the one written when it was created, never a later run's guess.
    pub(super) fn orchestrator(&self, pid: &str, root: &Path) -> Result<Orchestrator, LeadError> {
        if !ids::valid_project_ref(pid) {
            return Err(LeadError::Invalid("project: not a project id".into()));
        }
        let control = self.data.join("control");
        for d in [control.clone(), control.join("projects"), control.join("projects").join(pid)] {
            fsx::private_dir(&d).map_err(io)?;
        }
        let dir = self.orchestrator_dir(pid);
        fsx::private_dir(&dir).map_err(io)?;
        let root = match read_text(&dir.join(ROOT_FILE)) {
            Some(r) if Path::new(&r).is_absolute() => PathBuf::from(r),
            Some(_) => return Err(LeadError::GuardMissing("the Director-General's project folder record is not valid".into())),
            None => {
                fsx::write_atomic(&dir, ROOT_FILE, root.to_string_lossy().as_bytes()).map_err(io)?;
                root.to_path_buf()
            }
        };
        let generation = match read_text(&dir.join(GEN_FILE)) {
            None => 1,
            Some(g) => g.parse::<u32>().ok().filter(|g| (1..=MAX_GENERATION).contains(g)).ok_or_else(|| LeadError::GuardMissing("the Director-General's generation record is not valid".into()))?,
        };
        let session = read_text(&dir.join(SESSION_FILE)).filter(|s| ids::valid_session_id(s));
        Ok(Orchestrator { id: pid.to_string(), root, dir, generation, session })
    }

    /// The project folder recorded for `pid`'s Director-General, if it has one.
    pub(super) fn orchestrator_root(&self, pid: &str) -> Option<PathBuf> {
        if !ids::valid_project_ref(pid) {
            return None;
        }
        read_text(&self.orchestrator_dir(pid).join(ROOT_FILE)).map(PathBuf::from).filter(|p| p.is_absolute())
    }

    /// The Director-General of the project a run belongs to.
    pub(super) fn orchestrator_for_run(&self, run_id: &str, cwd: &Path) -> Result<Orchestrator, LeadError> {
        let (pid, root) = self.run_project(run_id, cwd)?;
        self.orchestrator(&pid, &root)
    }

    /// "New conversation" (ADR-15 2.5): the project's Director-General starts generation n+1 with no memory of the
    /// last one, on an empty stream. Refused while a turn is in flight. A process of the old generation that is still
    /// around can no longer see or steer anything (its MCP calls are `stale_generation`).
    pub fn new_conversation(&self, run_id: &str) -> Res {
        self.new_conversation_inner(run_id).map_err(|e| e.to_json())
    }

    fn new_conversation_inner(&self, run_id: &str) -> Result<Value, LeadError> {
        if !ids::valid_run_id(run_id) {
            return Err(LeadError::Invalid("run_id: must be YYYYMMDD-HHMM".into()));
        }
        let t = self.lead_target(run_id)?;
        let orch = self.orchestrator_for_run(&t.run_id, &t.cwd)?;
        // Under the ask lock: no turn can start between the busy check and the bump.
        let active = lock(&self.lead.active);
        if active.contains_key(&orch.id) {
            return Err(LeadError::Busy);
        }
        let next = orch.generation.checked_add(1).filter(|g| *g <= MAX_GENERATION).ok_or_else(|| LeadError::Invalid("generation: too many conversations".into()))?;
        // The session goes first: a crash between the two leaves the old generation with no session (a fresh start),
        // never the new generation resuming the old conversation.
        match std::fs::remove_file(orch.dir.join(SESSION_FILE)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
        fsx::write_atomic(&orch.dir, GEN_FILE, next.to_string().as_bytes()).map_err(io)?;
        lock(&self.control).set_generation(&orch.id, next).map_err(|e| LeadError::Io(e.code().to_string()))?;
        drop(active);
        Ok(json!({"project_id": orch.id, "generation": next, "stream": stream(&orch.id, next)}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repo_its_subfolder_and_its_worktree_share_one_project_ref() {
        let d = std::env::temp_dir().join(format!("vajra-orch-ref-{}-{}", std::process::id(), crate::control::nonce::random_hex(4).unwrap()));
        let repo = d.join("repo");
        std::fs::create_dir_all(repo.join(".git/worktrees/w1")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join(".git/worktrees/w1/commondir"), "../..\n").unwrap();
        let wt = d.join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", repo.join(".git/worktrees/w1").display())).unwrap();
        let repo = repo.canonicalize().unwrap();
        let (id, root) = project_ref(&repo);
        assert!(ids::valid_project_ref(&id), "{id}");
        assert_eq!(root, repo, "the Director-General runs in the repo root");
        assert_eq!(project_ref(&repo.join("src")), (id.clone(), repo.clone()));
        assert_eq!(project_ref(&wt), (id.clone(), repo.clone()), "a worktree is its repo's project");
        let plain = d.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let (pid, proot) = project_ref(&plain);
        assert_ne!(pid, id);
        assert_eq!(proot, plain.canonicalize().unwrap());
        assert_eq!(stream(&id, 2), format!("orchestrator-{id}-g2"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
