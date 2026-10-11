# Explore → Plan → Act (Working Discipline)

This is the working discipline Vajra follows: understand before you change, agree a plan
before you act, and verify before you call it done. In v0.5 it is **guidance, not a hook
gate** — the old `.phase` file and `vajra-phase.sh` actuator are retired, and the
PreToolUse hook no longer blocks tools by phase. The read-only part of the discipline is
provided by Claude Code's **native plan mode** (see `engine/plan-mode.md`); the hard
boundaries against a compromised agent are the immutable core, the guard and the OS
sandbox (see `SECURITY.md`), not this document.

## The three phases

### EXPLORE (read-only)
Understand the codebase before proposing changes. Read files, search, and run read-only
commands (`ls`, `cat`, `git log`, `git diff`, `git status`, `find`, `rg`, dependency
listings). Make no edits. Exit when you can articulate what needs to change, why, and the
dependencies, risks and edge cases.

### PLAN (read-only)
Produce findings, an enumerated list of changes with rationale, a risk assessment, and an
execution order. Still no code changes. Use native plan mode and Vajra's plan template,
and end with `ExitPlanMode` so the user can edit and approve. Exit when the user approves
(partial approval → only the approved items proceed).

### ACT (full access)
Execute the approved plan, in order, and nothing outside it. Checkpoint meaningful steps,
stop and report on an unexpected error rather than improvising, and if you discover a
needed change that is not in the plan, return to PLAN instead of ad-hoc-ing it. For a
substantial ask, ACT is usually a `vajra-deliver` run or a crew workflow, which builds in
worktrees, verifies against acceptance criteria and delivers a branch — never performing
a protected action.

## Transitions

```
explore → plan   (understanding is sufficient)
plan    → act    (user approves)
act     → explore (new task, or scope change needs re-exploration)
act     → plan    (unexpected findings require replanning)
```

## Defaults and edge cases

- New work starts in EXPLORE unless the ask is a trivial, well-understood one-liner.
- A tiny codebase can shorten EXPLORE but not skip it.
- "urgent" / "hotfix" may go straight to ACT, but still verify before claiming done.
- Verification is not optional: a change is done only when its checks have been run and
  seen to pass (`/vajra verify-work`, or a workflow's verify phase).
