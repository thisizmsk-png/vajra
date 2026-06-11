# Vajra Compaction — Survive Long Runs Without Losing the Plot

Long-horizon runs die at the context limit, and the generic summarizer keeps the
wrong things (recent chatter) and drops the load-bearing ones (decisions, the
plan). Vajra compacts on a discipline: **snapshot first, distill the right
fields, re-read after.**

## Triggers (compact proactively, not at the cliff)

- **~75% of the context window** is the practitioner threshold (Anthropic's
  compaction API defaults to 150k on a 200k window). Compact there, not at 95%.
- Before that, **clear redundant tool outputs first** — keep a one-line "ran X →
  Y", drop the raw blob. This is the cheapest lever and buys 30–50% headroom
  before you ever pay for a full compaction.

## The PreCompact hook (`hooks/pre-compact.sh`)

When Claude Code compacts, the hook fires and:
1. Takes a **non-destructive git snapshot** (`git stash create` → a dangling
   commit; working tree untouched). Compaction + checkpoint are atomic — a
   summary without a matching `git_sha` is dangerous.
2. Emits a `checkpoint` event into the HMAC-chained event log (so compaction is
   replayable like every other event — "condensation as a first-class event").
3. Writes a checkpoint record under `~/.claude/vajra/compaction/` with the
   snapshot SHA and the distillation template.
4. Prints the distillation instruction into context.

## What the summary MUST keep (distillation template)

```markdown
## Decisions        — irreversible choices made + the rationale
## Open threads      — unresolved bugs / TODO, each with file:line
## Plan              — remaining steps, ordered (mirror data/plans/{slug}.plan.md)
## Key files         — ≤5 most-recently-touched, one-line purpose each
## Constraints       — gotchas / "don't do X" learned this session
```

Maximize **recall** first (capture everything relevant), then tighten precision.
Drop raw tool outputs; keep decisions, the plan, and the active file diffs
verbatim — those are exactly what lossy summaries mangle.

## After compaction

- Re-read the checkpoint record + any `data/plans/{slug}.plan.md` and
  `data/progress/*` — durable artifacts survive compaction; in-context memory
  does not.
- The tree is unchanged (the snapshot was non-destructive). If a later step went
  wrong, `git stash apply <snap>` restores the pre-compaction state; the snapshot
  SHA is in the checkpoint record and the `checkpoint` event.

## Relationship to the Ralph loop

Compaction *summarizes* in-context history. The opposite strategy — **fully
reset context each iteration and re-derive from a durable `progress.md` + the
repo map** — is more drift-resistant for very long autonomous runs (fleet/Atman
jobs). Both rely on the same durable artifacts; pick compaction for interactive
work, hard-reset for unattended loops.
