---
name: atman
description: Self-improvement loop for Vajra — skills get better through practice, self-review, peer review, and muscle memory.
---

# Vajra Atman — The Practice Loop

You have a continuous improvement system. Like a basketball player who practices daily:
- Every shot is tracked (hit or miss)
- You review your own game tape
- Teammates review each other
- Good patterns become muscle memory
- Bad patterns get corrected

## The Loop

Every time a skill executes, this cycle runs:

### 1. OBSERVE (automatic — after every skill use)

After any skill completes, record:
```json
{
  "skill": "which skill ran",
  "task": "what was asked",
  "outcome": "success | failure | partial",
  "toolCalls": 5,
  "duration": "30s",
  "userReaction": "accepted | rejected | corrected",
  "errors": ["any tool errors"],
  "timestamp": "ISO8601"
}
```

Store in `~/.claude/vajra/practice-log.jsonl` (append-only, one JSON per line).

### 2. SELF-REVIEW (triggered when a skill fails ≥2 times)

When the practice log shows a skill failing on similar tasks ≥2 times:

1. Read the skill's SKILL.md
2. Read the failed traces
3. Ask: "What instruction in this skill caused the failure? What's missing?"
4. Generate a **patch** — a small, targeted fix to the skill's instructions
5. Write the patch to `~/.claude/vajra/patches/{skill-name}.patch.md`

Patches are NOT applied automatically. They are proposals.

### 3. PEER REVIEW (during `/vajra dream`)

During dream consolidation, skills review each other:

**Review pairs (domain expertise):**
- `code-review` reviews `testing` patches (does the test strategy make sense?)
- `sentry-security-review` reviews `api-design` patches (are security implications covered?)
- `threat-modeling` reviews `sentry-find-bugs` patches (are threat vectors complete?)
- `hld` reviews `lld` patches (does the low-level match the high-level?)
- `systematic-debugging` reviews `code-review` patches (does the review catch real bugs?)

**Review process:**
1. Load the proposed patch
2. Load the skill being patched
3. Ask: "Does this patch improve the skill? Does it introduce contradictions? Does it bloat the instructions?"
4. Verdict: APPROVE / REJECT / REVISE
5. If APPROVED: move to promotion queue
6. If REJECTED: log reason, discard patch
7. If REVISE: generate revised patch, re-queue

### 4. PROMOTE (user confirms or auto-promote after 3 approvals)

Approved patches can be:
- **Auto-promoted**: If `atman.autoPromote = true` in config AND patch has ≥2 peer approvals
- **User-confirmed**: `/vajra atman review` shows pending patches for user to accept/reject

When promoted:
1. Backup original skill to `~/.claude/vajra/backups/`
2. Apply patch to the skill's instructions
3. Log the evolution with before/after hash
4. If performance degrades in next 3 sessions → auto-rollback

### 5. MUSCLE MEMORY (routing learns from practice)

After every successful task completion:
1. Check if the routing decision could be a T1 regex or T3 keyword shortcut
2. If the same task pattern was routed via T4 (LLM) ≥3 times with the same skill → add it as a T3 keyword
3. If a very specific pattern (like `/deploy staging`) always goes to the same skill → propose a T1 regex rule

Write proposed routing rules to `~/.claude/vajra/routing-proposals.json`. Apply during dream.

## Commands

| Command | What it does |
|---------|-------------|
| `/vajra atman status` | Show: practice log size, pending patches, approved patches, routing proposals |
| `/vajra atman review` | Show pending patches for user approval |
| `/vajra atman log` | Show evolution history (what changed, when, why) |
| `/vajra atman rollback <id>` | Revert a specific skill patch |
| `/vajra atman pause` | Stop observing/proposing (keeps practice log) |
| `/vajra atman resume` | Resume the loop |

## Safety Rules

1. **Never modify bundled-skills/ directly** — patches create new files in evolved-skills/ or modify copies
2. **Never apply without evidence** — every patch links to ≥2 failure traces
3. **Max 5 patches per dream cycle** — prevents runaway self-editing
4. **Peer review is mandatory** — no self-approved patches
5. **Auto-rollback on regression** — if patched skill performs worse in 3 sessions, revert
6. **All patches go through sanitizer** — no prompt injection in evolved instructions
7. **User can always override** — `/vajra atman rollback` works instantly
8. **Practice log is append-only** — can't be tampered retroactively (HMAC on each entry)

## What Gets Better Over Time

| Week 1 | Week 4 | Week 12 |
|--------|--------|---------|
| All routing via T4 (LLM) | 40% via T1/T3 (free) | 70%+ via T1/T3 |
| Generic skill instructions | 5-10 patched skills | 20+ refined skills |
| No memory of past work | Heuristics from 100+ tasks | Pattern library of 500+ |
| Asks "what framework?" | Knows your stack | Anticipates your patterns |
