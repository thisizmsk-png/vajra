# Self-Review — A Skill Examines Its Own Failures

Like watching your own game tape. Not to judge — to find the specific moment the play broke down.

## When to Self-Review

Triggered when `practice-observer.md` detects ≥2 failures of the same skill on similar tasks.

## Self-Review Process

1. **Load the failing skill's instructions** from its SKILL.md
2. **Load the failure traces** from practice-log.jsonl (the specific entries)
3. **Analyze the gap**: What did the instructions tell the agent to do? What actually happened? Where's the disconnect?

### Ask These Questions
- Is there a step missing from the instructions?
- Is there an instruction that's ambiguous or misleading?
- Is the skill trying to handle too many cases at once?
- Did the skill rely on a tool that wasn't available or behaved unexpectedly?
- Would a human expert do something different here?

4. **Generate a Patch** — a targeted, minimal change to the skill's instructions

### Patch Format
```markdown
---
patch_id: p-{timestamp}
target_skill: code-review
trigger_traces: [trace-id-1, trace-id-2]
failure_pattern: "fails to check for async error handling in Express routes"
change_type: add-instruction | modify-instruction | remove-instruction
---

## Before
[exact text being changed]

## After  
[proposed replacement text]

## Evidence
- Trace 1: code-review missed unhandled promise rejection in routes/api.ts
- Trace 2: code-review missed async middleware without try/catch in auth.ts
- Pattern: Both failures involve Express async route handlers without error boundaries

## Expected Impact
Code review will now explicitly check for async error handling patterns in Express routes.
```

5. **Write patch** to `~/.claude/vajra/patches/{skill-name}/{patch-id}.md`
6. **Queue for peer review** — do NOT self-approve

## Anti-Bloat Rules

- Patch must be ≤ 10 lines of net new instruction text
- Patch must link to ≥2 failure traces (no one-off fixes)
- If the skill already has a patch pending, don't create another (queue)
- Never add "IMPORTANT" or "CRITICAL" or "ALWAYS" — these are instruction bloat signals
