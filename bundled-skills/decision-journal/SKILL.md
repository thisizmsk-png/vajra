---
name: decision-journal
description: Lightweight decision logging for solo founders — capture context, alternatives, rationale. Activate for decision, why did we, trade-off, chose, decided, picked tasks.
user-invocable: true
---

# Decision Journal

## When to Activate
- User mentions: decision, why did we choose, trade-off, considered, decided, picked X over Y
- Making a significant technical or business choice
- Revisiting a past decision ("why did we do it this way?")

## What Qualifies as a "Decision"
Not everything is worth logging. Log when:
- **Irreversible or expensive to reverse** (database choice, pricing model, API contract)
- **Multiple viable alternatives existed** (you chose one, need to remember why)
- **Future you will ask "why?"** in 3 months

Don't log:
- Variable naming, code style choices (that's CLAUDE.md territory)
- Obvious choices with one good option
- Temporary decisions ("use this for now")

## Decision Format
```markdown
## [Title] — [Date]

**Context**: What situation led to this decision?
**Decision**: What did we choose?
**Alternatives considered**:
1. [Option A] — rejected because [reason]
2. [Option B] — rejected because [reason]
**Consequences**: What trade-offs did we accept?
**Revisit if**: [Under what conditions should we reconsider?]
```

## Where to Store
- `~/.claude/memory/decisions/YYYY-MM-DD.md` — one file per day, multiple decisions per file
- Referenced from `~/.claude/MEMORY.md` under "Active Decisions"
- CLAUDE.md "Key Design Decisions" section for architecture-level decisions

## Process
1. When a decision point arises, pause and name it
2. List at least 2 alternatives (zero-bias: resist anchoring on the first option)
3. State the criteria that matter (cost, speed, simplicity, scale, reversibility)
4. Choose and document
5. State the "revisit if" trigger — when should we reconsider?

## Guardrails
- Log the decision WHEN you make it, not after (context fades fast)
- Include "alternatives considered" — this prevents revisiting dead ends
- Keep it lightweight — 5 lines per decision, not a design doc
- "Revisit if" is the most important field — it tells future you when the decision expires
