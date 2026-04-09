# Dream Consolidation Protocol

Instructions for `/vajra dream` — the memory compaction process that keeps L1 under its 200-line cap.

## When to Trigger

- Automatically when L1 (`memory/index.md`) exceeds 200 lines
- Manually via `/vajra dream` when memory feels cluttered or contradictory

## Process

### Step 1: Read All L1 Index Entries

Load the full `memory/index.md` and parse every entry into structured records:
- Topic name
- Link to L2 file
- One-line description

### Step 2: Identify Duplicates

Find entries that refer to the same topic or overlapping knowledge:
- Same L2 file linked from multiple entries
- Different entries describing the same concept with different wording
- Topics that have been superseded by more comprehensive entries

### Step 3: Identify Contradictions

Find entries with conflicting information:
- Two entries making opposing claims about the same subject
- Preferences or decisions that have been revised but old entries remain
- Architecture notes that conflict with newer implementation choices

### Step 4: Identify Stale Entries

Find entries that no longer match reality:
- Topics referencing deleted files or deprecated systems
- Decisions that have been reversed
- Temporary notes that have outlived their usefulness
- Entries about completed one-off tasks

### Step 5: Consolidate

For each issue found:

| Issue | Action |
|-------|--------|
| **Duplicate entries** | Merge into a single entry with the most complete description |
| **Contradictions** | Prefer the newer information; update or remove the older entry |
| **Stale entries** | Remove from L1 index |
| **Overly verbose descriptions** | Tighten to a single concise line |
| **Related small topics** | Consider merging L2 files into a single topic file |

### Step 6: Rewrite L1 Index

Write the consolidated `memory/index.md`:
- Must be **<= 200 lines**
- Maintain alphabetical or categorical grouping (match existing style)
- Every entry must link to a valid L2 file

### Step 7: Archive Pruned Content

All removed or merged content goes to L3:
- Create `memory/archive/YYYY-MM-DD-dream-consolidation.md`
- Include: what was removed, what was merged, what contradictions were resolved
- This preserves the audit trail and allows recovery if needed

## Post-Consolidation Verification

After consolidation, verify:
- [ ] L1 is <= 200 lines
- [ ] All L1 links point to existing L2 files
- [ ] No orphaned L2 files (every topic file has an L1 entry)
- [ ] Archive file was created with pruned content
