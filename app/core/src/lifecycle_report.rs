//! What keeps running after the window closes, after the app quits, and after a worker is archived (ADR-9, FG-24).
//!
//! A pure function over a snapshot, so the claim the UI makes is the one this module computes and tests. It asks for
//! one snapshot on demand; nothing polls. User terminals are described only by `TermInfo`, and agent runs only by a
//! count: this module does not reach into `terminal`'s internals or the agent supervisor.
//!
//! * Close the last window (ADR-9 item 2): the app stays alive in the menu bar; runs and live user shells keep going.
//! * Quit (ADR-9 items 3, 4): live shells get SIGHUP and are reaped; runs have their process groups stopped. Ended
//!   shells need nothing, so they are not counted.
//! * Archive a worker: its worktree's live user shells are not ended by it. They are named so the human can close them.

use std::path::PathBuf;

use crate::terminal::TermInfo;

/// One user terminal as the diagnostics list shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TermDiag {
    pub id: u32,
    pub pid: u32,
    pub cwd: PathBuf,
    pub root: PathBuf,
    pub alive: bool,
    /// Opened in a worker worktree rather than the project folder itself.
    pub in_worktree: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Count {
    pub terminals: usize,
    pub runs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Report {
    pub terminals: Vec<TermDiag>,
    /// After the last window closes: these keep going.
    pub close_keeps: Count,
    /// After Cmd-Q: these are ended. Nothing resumes by itself (ADR-9 item 4).
    pub quit_ends: Count,
    /// After a worker is archived: live user shells in worker worktrees that stay open until closed.
    pub archive_leaves: Vec<u32>,
}

pub fn report(terms: &[TermInfo], active_runs: usize) -> Report {
    let diag: Vec<TermDiag> = terms
        .iter()
        .map(|t| TermDiag { id: t.id, pid: t.pid, cwd: t.cwd.clone(), root: t.root.clone(), alive: t.alive, in_worktree: t.cwd != t.root })
        .collect();
    let live = diag.iter().filter(|t| t.alive).count();
    Report {
        close_keeps: Count { terminals: live, runs: active_runs },
        quit_ends: Count { terminals: live, runs: active_runs },
        archive_leaves: diag.iter().filter(|t| t.alive && t.in_worktree).map(|t| t.id).collect(),
        terminals: diag,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(id: u32, cwd: &str, alive: bool) -> TermInfo {
        TermInfo { id, pid: 100 + id, cwd: cwd.into(), root: "/p".into(), rows: 24, cols: 80, alive }
    }

    #[test]
    fn given_nothing_open_when_asked_then_nothing_keeps_running_or_ends() {
        let r = report(&[], 0);
        assert_eq!(r.close_keeps, Count { terminals: 0, runs: 0 });
        assert_eq!(r.quit_ends, Count { terminals: 0, runs: 0 });
        assert!(r.archive_leaves.is_empty() && r.terminals.is_empty());
    }

    #[test]
    fn given_live_shells_and_runs_when_the_window_closes_then_all_of_them_keep_going() {
        let r = report(&[term(1, "/p", true), term(2, "/p/.claude/worktrees/w", true)], 3);
        assert_eq!(r.close_keeps, Count { terminals: 2, runs: 3 });
    }

    #[test]
    fn given_live_shells_and_runs_when_the_app_quits_then_all_of_them_end() {
        let r = report(&[term(1, "/p", true), term(2, "/p/w", true)], 2);
        assert_eq!(r.quit_ends, Count { terminals: 2, runs: 2 });
    }

    #[test]
    fn given_an_ended_shell_when_asked_then_it_is_listed_but_nothing_keeps_running_or_ends_for_it() {
        let r = report(&[term(1, "/p", false), term(2, "/p", true)], 0);
        assert_eq!(r.close_keeps.terminals, 1);
        assert_eq!(r.quit_ends.terminals, 1);
        assert_eq!(r.terminals.len(), 2);
        assert!(!r.terminals[0].alive);
    }

    #[test]
    fn given_shells_in_a_project_and_a_worktree_when_a_worker_is_archived_then_only_the_worktree_live_shells_are_named() {
        let r = report(&[term(1, "/p", true), term(2, "/p/w1", true), term(3, "/p/w2", false)], 0);
        assert_eq!(r.archive_leaves, vec![2]);
        assert!(!r.terminals[0].in_worktree && r.terminals[1].in_worktree);
    }

    #[test]
    fn diagnostics_carry_the_pid_and_folders_and_no_output_or_environment() {
        let v = serde_json::to_value(report(&[term(1, "/p", true)], 0)).unwrap();
        let t = &v["terminals"][0];
        let mut keys: Vec<_> = t.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["alive", "cwd", "id", "in_worktree", "pid", "root"]);
    }
}
