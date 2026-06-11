# Vajra Plan Mode — Editable Plan Before Execution

Plan Mode is the highest-leverage trust + token-saving feature in modern agent
harnesses (Cline Plan/Act, Cursor Plan Mode, Devin's planning step): run a cheap,
**read-only** planning pass, emit a plan the user can **edit**, and only execute
once approved. It builds directly on Vajra's explore-plan-act enforcement
(`engine/explore-plan-act.md`) — Plan Mode is what makes that enforcement
*reachable* and *editable*.

## The flow

```
/vajra plan "<task>"
  → vajra-phase.sh set plan        (pre-tool hook now blocks all writes)
  → read-only exploration          (Read/Grep/Glob + read-only Bash only)
  → emit data/plans/{slug}.plan.md  (the editable artifact, see format below)
  → PAUSE — show the plan, invite edits

[user edits data/plans/{slug}.plan.md — reorder, delete, or adjust steps]

/vajra act      (or "approved" / "go ahead")
  → vajra-phase.sh set act         (writes unblocked)
  → execute the plan file's steps IN ORDER, checkpoint after each
  → on completion: vajra-phase.sh set explore (back to read-only)
```

## The plan artifact — `data/plans/{slug}.plan.md`

A plan is a checklist the user can edit. Each step is independently checkable and
carries its target files, the exact commands, and its risk. The agent executes
**only** what survives the user's edit.

```markdown
# Plan: <task title>
slug: <kebab-slug>
created: <ISO8601>
status: draft        # draft → approved → executing → done

## Findings
<2-5 bullets: key files, patterns, dependencies discovered while exploring>

## Steps
- [ ] 1. <imperative step>
      files: path/a.ts, path/b.ts
      commands: `npm test -- a.test.ts`
      risk: <what could go wrong; none if trivial>
- [ ] 2. <next step>
      ...

## Out of scope
<explicitly list what this plan will NOT touch — prevents scope creep>

## Verification
<how we'll know it worked: tests/commands to run after, expected output>
```

## Rules

1. **Plan is read-only to produce.** While in `plan` phase the hook blocks
   Write/Edit/mutating-Bash — the plan file itself is written when transitioning
   (or the agent presents it inline and writes it on `act`). Never edit code
   during planning.
2. **Execute the edited plan, not the original intent.** If the user deletes
   step 3, step 3 does not happen. Re-read the plan file at `act` time.
3. **Checkpoint per step.** After each step, save campaign state
   (`engine/checkpoint.md`) so a failure resumes mid-plan, not from zero.
4. **Scope creep → replan.** If execution reveals a needed change not in the
   plan, return to `plan` (don't ad-hoc it). Update the plan file, re-confirm.
5. **Emergency override.** "urgent"/"hotfix" may skip to `act` with a logged
   warning (per explore-plan-act edge cases).
6. **Verification gate.** Before marking done, run the plan's Verification block
   and report real output — pairs with `/vajra review` and the red-team.

## Why editable (not just a printed plan)

A printed plan the user can only accept/reject forces a round-trip for every
tweak. An **editable artifact** lets the user reorder, prune, and constrain in
place — the agent then executes exactly that, which is the single biggest
reducer of "the agent confidently did the wrong thing."
