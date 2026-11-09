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
//! **The project is the store's** (FG-02c): a run's folder is resolved to its `p_` project when it starts (a folder
//! that is not a project is refused, `not_a_project`), and that id is pinned in the guarded run record, so the app's
//! own record decides which project a run belongs to, not a later re-read of the repo. Every ask, every
//! project-bound tool call and every proposal start checks the project still exists and is not archived.
//!
//! Before the store, FG-02a/b keyed the orchestrator by a **stand-in** (`k_` and a digest of `caps::project_key`).
//! Owner decision Q3 carries those over: on a project's first use, a `k_` orchestrator whose recorded root is this
//! project's root lends it its conversation (`MAP_STANDIN_IDS`). Nothing is deleted.
//!
//! The project's `sessions.jsonl` (FG-01) gets a `kind: orchestrator` line per conversation change and turn, and a
//! `kind: run` line per run, parented to the orchestrator. Those lines are a record for the window only: what
//! `--resume` uses is the guarded `session` file here, never the log.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Value, json};

use super::{App, Res};
use crate::control::lead::LeadError;
use crate::control::service::ControlError;
use crate::control::{fsx, sha256, time};
use crate::ids;
use crate::lock;
use crate::projects::contract::{self as pc, Project, Session, SessionKind, SessionStatus};
use crate::projects::store::{Store, StoreError};

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
/// Owner decision Q3 (2026-11-08): a project's stand-in (`k_`) conversation carries over to its store id.
const MAP_STANDIN_IDS: bool = true;
/// In a `p_` orchestrator dir: the stand-in it took its conversation from. In the `k_` dir: the project it went to.
const MAPPED_FROM: &str = "mapped-from";
const MAPPED_TO: &str = "mapped-to";
/// The orchestrator's own session id (`s_…`) in its project's `sessions.jsonl`, created once.
const RECORD_FILE: &str = "record";

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

/// The stand-in id FG-02a/b gave a run folder, and the folder its Director-General ran in (the repo root for a folder
/// inside a repo or one of its worktrees, else the folder itself). Only used now to find a stand-in conversation to
/// carry over, and to check an old run's stand-in pin against its folder.
pub(super) fn project_ref(cwd: &Path) -> (String, PathBuf) {
    project_ref_of(&super::caps::project_key(cwd), cwd)
}

/// `project_ref` for a project key already found (the one a launch pinned before it spawned anything).
pub(super) fn project_ref_of(key: &Path, cwd: &Path) -> (String, PathBuf) {
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

/// A store failure that is not about the project itself. The text never carries a path.
fn store_io(e: &StoreError) -> LeadError {
    LeadError::Io(match e {
        StoreError::Full => "the project list is too large to search".into(),
        _ => "the project store could not be read".into(),
    })
}

/// The check every Director-General tool call passes first (`Service::set_project_gate`): the project exists in the
/// store and is not archived. A stand-in id is not a store project, so its calls are refused.
pub(super) fn project_gate(store: &Store, pid: &str) -> Result<(), ControlError> {
    if !pc::valid_project_id(pid) {
        return Err(ControlError::ProjectMissing);
    }
    match store.get(pid) {
        Ok(p) if p.archived.is_some() => Err(ControlError::ProjectArchived),
        Ok(_) => Ok(()),
        Err(StoreError::NotFound | StoreError::Invalid(_)) => Err(ControlError::ProjectMissing),
        Err(e) => Err(ControlError::Io(std::io::Error::other(e.to_string()))),
    }
}

/// One `vajra.session/1` line.
#[allow(clippy::too_many_arguments)]
pub(super) fn session_line(id: String, project: &str, kind: SessionKind, parent: Option<String>, native: Option<&str>, run: Option<(&str, &str)>, generation: u32, status: SessionStatus) -> Session {
    Session {
        schema: pc::SESSION_SCHEMA.into(),
        id,
        project_id: project.into(),
        kind,
        parent,
        provider: "claude".into(),
        native_session_id: native.map(str::to_string),
        run_id: run.map(|r| r.0.to_string()),
        campaign: run.map(|r| r.1.to_string()),
        task_id: None,
        attempt: None,
        generation,
        status,
        started: time::iso_secs(SystemTime::now()),
        ended: None,
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

impl App {
    /// `<data>/control/projects/<pid>/orchestrator/`.
    pub(super) fn orchestrator_dir(&self, pid: &str) -> PathBuf {
        self.data.join("control").join("projects").join(pid).join("orchestrator")
    }

    /// The project a folder belongs to, for starting something there (FG-02c): it must be a project, and active.
    pub(super) fn project_of_folder(&self, cwd: &Path) -> Result<Project, LeadError> {
        let p = self.projects.find_by_path(cwd).map_err(|e| store_io(&e))?.ok_or_else(|| LeadError::NotAProject(cwd.to_string_lossy().into_owned()))?;
        if p.archived.is_some() {
            return Err(LeadError::ProjectArchived);
        }
        Ok(p)
    }

    /// The scope gate (ADR-15 2.3): `pid` is in the store and not archived.
    pub(super) fn active_project(&self, pid: &str) -> Result<Project, LeadError> {
        match self.projects.get(pid) {
            Ok(p) if p.archived.is_some() => Err(LeadError::ProjectArchived),
            Ok(p) => Ok(p),
            Err(StoreError::NotFound | StoreError::Invalid(_)) => Err(LeadError::ProjectMissing),
            Err(e) => Err(store_io(&e)),
        }
    }

    /// The project id pinned in a run's guarded record, if any.
    pub(super) fn pinned_project(&self, run_id: &str) -> Option<String> {
        read_text(&self.data.join("control").join("lead").join(run_id).join(PROJECT_FILE))
    }

    /// The project of a run the app recorded, and the folder its Director-General's turns run in (the store's recorded
    /// root). The id pinned in the guarded record at launch decides; the folder is never re-read for it.
    /// - A store id must still be an active project (`project_archived`, `project_missing`).
    /// - A stand-in pin (a run from before the store) resolves to the store project of its guarded folder, and only
    ///   while that folder still gives the same stand-in id; otherwise the run is refused (`lead_guard_missing`).
    /// - A record with no pin gets one now from its guarded folder, which must be a project.
    pub(super) fn run_project(&self, run_id: &str, cwd: &Path) -> Result<(String, PathBuf), LeadError> {
        let dir = self.data.join("control").join("lead").join(run_id);
        let p = match read_text(&dir.join(PROJECT_FILE)) {
            Some(p) if pc::valid_project_id(&p) => self.active_project(&p)?,
            Some(k) if ids::valid_standin_ref(&k) => {
                if project_ref(cwd).0 != k {
                    return Err(LeadError::GuardMissing("the run's project record does not match its folder".into()));
                }
                let p = self.projects.find_by_path(cwd).map_err(|e| store_io(&e))?.ok_or_else(|| LeadError::GuardMissing("the run's folder is not a project".into()))?;
                if p.archived.is_some() {
                    return Err(LeadError::ProjectArchived);
                }
                p
            }
            Some(_) => return Err(LeadError::GuardMissing("the run's project record is not valid".into())),
            None => {
                let p = self.project_of_folder(cwd)?;
                if dir.is_dir() {
                    fsx::write_atomic(&dir, PROJECT_FILE, p.id.as_bytes()).map_err(io)?;
                }
                p
            }
        };
        Ok((p.id, PathBuf::from(p.root)))
    }

    /// Decision Q3: on a store project's first use, take the conversation of the stand-in orchestrator FG-02a/b kept
    /// for the same folder. Only when that `k_` dir's recorded root is this project's root (so a forged `.git` that once
    /// moved a run cannot hand its conversation over), and only once: `mapped-from` here, `mapped-to` there. The `k_`
    /// dir is left as it was.
    fn map_standin(&self, pid: &str, root: &Path, dir: &Path) -> Result<(), LeadError> {
        let (k, _) = project_ref(root);
        let kdir = self.orchestrator_dir(&k);
        let Some(kroot) = read_text(&kdir.join(ROOT_FILE)) else { return Ok(()) };
        if !same_folder(Path::new(&kroot), root) || read_text(&kdir.join(MAPPED_TO)).is_some_and(|to| to != pid) {
            return Ok(());
        }
        if let Some(g) = read_text(&kdir.join(GEN_FILE)).filter(|g| g.parse::<u32>().is_ok_and(|g| (1..=MAX_GENERATION).contains(&g))) {
            fsx::write_atomic(dir, GEN_FILE, g.as_bytes()).map_err(io)?;
        }
        if let Some(s) = read_text(&kdir.join(SESSION_FILE)).filter(|s| ids::valid_session_id(s)) {
            fsx::write_atomic(dir, SESSION_FILE, s.as_bytes()).map_err(io)?;
        }
        fsx::write_atomic(dir, MAPPED_FROM, k.as_bytes()).map_err(io)?;
        fsx::write_atomic(&kdir, MAPPED_TO, pid.as_bytes()).map_err(io)?;
        Ok(())
    }

    /// The orchestrator's `s_` id in its project's `sessions.jsonl`, created on first need. Store projects only.
    fn orchestrator_record(&self, orch: &Orchestrator) -> Option<String> {
        if !pc::valid_project_id(&orch.id) {
            return None;
        }
        if let Some(s) = read_text(&orch.dir.join(RECORD_FILE)).filter(|s| pc::valid_session_id(s)) {
            return Some(s);
        }
        let s = pc::fresh_id(pc::SESSION_PREFIX).ok()?;
        fsx::write_atomic(&orch.dir, RECORD_FILE, s.as_bytes()).ok()?;
        Some(s)
    }

    /// Append the orchestrator's current state to its project's `sessions.jsonl` (display only; a failure is logged).
    pub(super) fn note_orchestrator(&self, orch: &Orchestrator, native: Option<&str>, status: SessionStatus) -> Option<String> {
        let id = self.orchestrator_record(orch)?;
        let line = session_line(id.clone(), &orch.id, SessionKind::Orchestrator, None, native, None, orch.generation, status);
        if self.projects.append_session(&line).is_err() {
            self.note("session_record_failed", &[]);
        }
        Some(id)
    }

    /// A run of `project` started: one `kind: run` line, parented to the project's orchestrator.
    pub(super) fn note_run(&self, project: &Project, run_id: &str, campaign: &str) {
        let Ok(orch) = self.orchestrator(&project.id, Path::new(&project.root)) else { return };
        let parent = self.orchestrator_record(&orch);
        let Ok(id) = pc::fresh_id(pc::SESSION_PREFIX) else { return };
        let line = session_line(id, &project.id, SessionKind::Run, parent, None, Some((run_id, campaign)), 1, SessionStatus::Running);
        if self.projects.append_session(&line).is_err() {
            self.note("session_record_failed", &[]);
        }
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
        if MAP_STANDIN_IDS && pc::valid_project_id(pid) && !dir.join(ROOT_FILE).exists() {
            self.map_standin(pid, root, &dir)?;
        }
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

    /// The store project a stand-in orchestrator's conversation was carried over to (`mapped-to`), if any.
    pub(super) fn standin_mapped_to(&self, k: &str) -> Option<String> {
        if !ids::valid_standin_ref(k) {
            return None;
        }
        read_text(&self.orchestrator_dir(k).join(MAPPED_TO)).filter(|p| pc::valid_project_id(p))
    }

    /// The current generation of store project `pid`'s Director-General, without creating it: `None` when it has
    /// never been used or its record is not valid (`project_state`'s `generations.orchestrator`).
    pub(super) fn orchestrator_generation(&self, pid: &str) -> Option<u32> {
        if !pc::valid_project_id(pid) {
            return None;
        }
        let dir = self.orchestrator_dir(pid);
        if !dir.join(ROOT_FILE).exists() {
            return None;
        }
        match read_text(&dir.join(GEN_FILE)) {
            None => Some(1),
            Some(g) => g.parse::<u32>().ok().filter(|g| (1..=MAX_GENERATION).contains(g)),
        }
    }

    /// Store project `pid`'s Director-General as recorded, creating nothing and carrying nothing over: `Ok(None)` when
    /// it has never been used. A record that is there but not valid fails closed.
    pub(super) fn recorded_orchestrator(&self, pid: &str) -> Result<Option<Orchestrator>, LeadError> {
        let dir = self.orchestrator_dir(pid);
        if !pc::valid_project_id(pid) || !dir.join(ROOT_FILE).exists() {
            return Ok(None);
        }
        let root = self.orchestrator_root(pid).ok_or_else(|| LeadError::GuardMissing("the Director-General's project folder record is not valid".into()))?;
        let generation = self.orchestrator_generation(pid).ok_or_else(|| LeadError::GuardMissing("the Director-General's generation record is not valid".into()))?;
        let session = read_text(&dir.join(SESSION_FILE)).filter(|s| ids::valid_session_id(s));
        Ok(Some(Orchestrator { id: pid.to_string(), root, dir, generation, session }))
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
        self.new_conversation_expecting(run_id, None).map_err(|e| e.to_json())
    }

    /// `new_conversation` from conversation `expect` (a receipt's `expect_generation`), checked under the ask lock: two
    /// clicks aimed at the same conversation start one new conversation, not two.
    pub(super) fn new_conversation_expecting(&self, run_id: &str, expect: Option<u32>) -> Result<Value, LeadError> {
        if !ids::valid_run_id(run_id) {
            return Err(LeadError::Invalid("run_id: must be YYYYMMDD-HHMM".into()));
        }
        let t = self.lead_target(run_id)?;
        let found = self.orchestrator_for_run(&t.run_id, &t.cwd)?;
        // Under the ask lock: no turn can start between the busy check and the bump, and the generation is read here,
        // so two "New conversation" clicks bump it twice from what is current, never both from the same old value.
        let active = lock(&self.lead.active);
        if active.contains_key(&found.id) {
            return Err(LeadError::Busy);
        }
        let orch = self.orchestrator(&found.id, &found.root)?;
        if expect.is_some_and(|g| g != orch.generation) {
            return Err(LeadError::StaleGeneration(orch.generation));
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
        self.note_orchestrator(&Orchestrator { generation: next, session: None, ..orch.clone() }, None, SessionStatus::Stopped);
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
