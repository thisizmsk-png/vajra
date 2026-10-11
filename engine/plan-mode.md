# Vajra Plan Mode — Native Plan Mode + Vajra's Plan Template

Plan Mode runs a cheap, **read-only** planning pass, emits a plan the user can read and
edit, and only executes once approved. In v0.5 Vajra does not implement its own phase
gate: it is a **thin wrapper over Claude Code's native plan mode**. Native plan mode
already makes the session read-only and ends with `ExitPlanMode`, which shows the plan
and asks for approval. The old `.phase` file and `vajra-phase.sh` are retired, and the
pre-tool-use hook no longer gates on a phase (it enforces the immutable core and
protected paths instead — see `SECURITY.md`).

> **Scope note (honest):** Plan Mode is a **workflow-discipline** feature. Native plan
> mode keeps a cooperative agent from editing before you approve; it is **not** a hard
> security boundary. The hard boundaries against a compromised agent are the immutable
> core, the PreToolUse guard and the OS sandbox.

## The flow

```
/vajra plan "<task>"
  → enter native plan mode            (session is read-only: Read/Grep/Glob + read-only Bash)
  → explore, then draft the plan in Vajra's template (below)
  → ExitPlanMode                      (Claude Code shows the plan and asks to approve)

[user edits or comments on the plan, or approves it]

approve → native plan mode exits; execute the approved steps IN ORDER
          (if the ask is substantial, hand it to /vajra deliver or a crew workflow,
           which plans, builds, verifies and delivers on its own)
```

For a large or multi-step ask, the plan's execution is usually a `vajra-deliver` run:
plan mode agrees *what* to build, then the workflow builds, verifies and delivers it
with evidence. For a small, direct change, execute the steps inline after approval.

## The plan template

Draft the plan in this shape so it is checkable and editable, and so its Verification
block can feed `/vajra verify-work` (or a workflow's acceptance criteria):

```markdown
# Plan: <task title>

## Findings
<2-5 bullets: key files, patterns, dependencies discovered while exploring>

## Steps
- [ ] 1. <imperative step>
      files: path/a.ts, path/b.ts
      commands: `npm test -- a.test.ts`
      risk: <what could go wrong; none if trivial>
- [ ] 2. <next step>

## Out of scope
<what this plan will NOT touch — prevents scope creep>

## Verification
<the exact commands that prove it worked, and their expected output —
 these become the `/vajra verify-work` checks or the acceptance criteria>
```

## Rules

1. **Plan is read-only to produce.** Native plan mode blocks writes while planning;
   don't edit code during planning. The plan itself is shown through `ExitPlanMode`.
2. **Execute the approved plan, not the original intent.** If the user removes a step,
   it does not happen. Re-read the approved plan before executing.
3. **Verify before "done".** Run the plan's Verification block and report real output
   (`/vajra verify-work`), or let the workflow's verify phase do it. No green evidence,
   not done.
4. **Scope creep → replan.** If execution reveals a change not in the plan, go back to
   plan mode rather than ad-hoc-ing it.
5. **Protected actions stay with the human.** A plan may *propose* a push, PR, deploy
   or migration, but Plan Mode (and any workflow it hands off to) never performs one;
   it returns it for the user to approve and run.

## Why native, not a custom gate

The v0.4 phase gate wrote `~/.claude/vajra/.phase` and asked the pre-tool hook to block
writes. That was a cooperative-only gate that duplicated a primitive Claude Code now
ships. Native plan mode is more robust (it is enforced by the harness, not a skill
file), needs no extra state file, and already supports review/approve. Vajra keeps only
what native mode lacks: the plan **template** above and the handoff to verification and
the delivery workflow.
