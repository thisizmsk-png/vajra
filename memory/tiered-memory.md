# Tiered Memory Management (3-Tier)

Claude Code skill reference for Vajra's memory subsystem.

## Architecture

### L1 — Index (`memory/index.md`)
- **Always loaded** into context at session start
- Compact index, hard cap of **200 lines**
- Each line follows the format:
  ```
  - [Topic](topics/filename.md) — one-line description
  ```
- Acts as a table of contents for all persistent knowledge
- If a topic doesn't warrant its own file, the one-line description IS the knowledge

### L2 — Topics (`memory/topics/`)
- On-demand topic files, loaded only when L1 references are relevant to the current task
- Each file is a self-contained knowledge unit (decisions, preferences, architecture notes, etc.)
- Typical size: 20-100 lines
- Filename convention: `kebab-case.md`

### L3 — Archive (`memory/archive/`)
- Full session transcripts and pruned content
- Searchable but **never auto-loaded** into context
- Used for deep retrieval when L1+L2 are insufficient
- Filename convention: `YYYY-MM-DD-session-slug.md`

## Writing Protocol

When learning something worth persisting:

1. **Create or update L2 topic file** in `memory/topics/`
2. **Add L1 index entry** in `memory/index.md` with link and one-line description
3. If updating an existing topic, update the L1 description if it has changed

## Reading Protocol

1. **Always check L1 first** — scan the index for relevant entries
2. **Only load L2 files that are relevant** to the current task context
3. **Never bulk-load** — loading all L2 files defeats the purpose of tiering
4. If L2 is insufficient, search L3 archive for historical context

## Sanitization Requirement

**CRITICAL**: All memory content MUST pass through the sanitizer before loading into context. This applies to:
- L2 topic files loaded on demand
- L3 archive content retrieved via search
- Any externally-sourced content written to memory

Use `sanitize(content, source)` from `engine/sanitizer.ts` before injecting into prompt context.

## Overflow — Dream Consolidation

When L1 exceeds **200 lines**, trigger the dream consolidation process (`/vajra dream`):
- See `memory/dream-consolidation.md` for the full protocol
- This merges duplicates, resolves contradictions, prunes stale entries
- Goal: bring L1 back under the 200-line cap while preserving all valuable knowledge
