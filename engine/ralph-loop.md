# Vajra Ralph Loop — Hard-Reset Iteration

For very long autonomous runs (fleet/Atman jobs, overnight backlogs), the failure
mode isn't running out of context — it's **drift**: each compaction is lossy, and
fidelity to the original goal decays over many turns. The Ralph loop is the
opposite strategy to compaction: **discard the whole in-context history each
iteration and re-derive from durable artifacts.**

> Compaction *summarizes* history to keep going. Ralph *throws history away* and
> re-derives from `progress.md` + the repo map each iteration. No accumulated
> context means no accumulated drift.

## The loop (`scripts/vajra-ralph.sh`)

```
vajra-ralph.sh <progress-file> [max-iters]

  while not done and iter < max:
    fresh agent (claude -p) gets ONLY:
      - the progress file (goal + checklist + decisions)
      - (optionally) the repo map, which it can regenerate
    it does the SINGLE most valuable next unit of work,
    updates progress.md (check off done, add discovered steps),
    and exits — context gone.
  done when: no unchecked `- [ ]` items, or the marker RALPH-COMPLETE appears.
```

## The progress file is the entire memory

```markdown
# Goal: <one sentence>

## Decisions (carry forward — never re-litigate)
- <decision + why>

## Steps
- [x] done step
- [ ] next step
- [ ] later step

## Constraints / gotchas
- <"don't do X" learned the hard way>
```

Because each iteration starts clean, the progress file must be **self-sufficient**:
anything not written down is forgotten. That discipline is the point — it forces
durable state and prevents the "remembered a plan that no longer matches the tree"
class of bug.

## When to use Ralph vs compaction

| | Compaction | Ralph loop |
|--|------------|-----------|
| Strategy | summarize history, keep going | reset context, re-derive |
| Best for | interactive work, one coherent thread | long unattended runs, backlogs |
| Drift | accumulates slowly across summaries | ~none (fresh each iter) |
| Cost | one big context, periodic summary | many small contexts |
| State | in-context + checkpoint | durable `progress.md` only |

## Composition

- Pairs with the **repo map** (each iteration regenerates a budget-capped map for
  orientation) and **verification** (an iteration that claims a step done runs
  `/vajra verify-work`).
- Pairs with **fleet/scheduled-tasks** for overnight "backlog → detached run →
  reviewed PR" workflows — the proof-of-work walkthrough + 5-pass review gate
  what the loop produces.
- Bounded by `max-iters` (default 25) so a stuck loop halts instead of burning
  tokens forever.
