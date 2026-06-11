# Vajra Repo Map — Just-in-Time Context Orientation

Dumping the whole `src/` tree into context dilutes attention and burns tokens
("context rot"). The repo map gives the agent a **budget-capped, relevance-ranked
index of the codebase's most important definitions** — load identifiers, then let
the agent `grep`/`Read` the specifics on demand (progressive disclosure).

## How it ranks (`scripts/vajra_repomap.py`)

Aider's algorithm, dependency-light (pure stdlib + git, runs anywhere):
1. **Extract** per file: the symbols it *defines* (regex per language) and the
   identifiers it *references*.
2. **Graph**: a file that references a symbol defined elsewhere → edge
   `referencer → definer`.
3. **PageRank** over that graph: heavily-referenced files (the load-bearing ones)
   rank highest. **Personalize** toward the current task by seeding the files /
   identifiers it touches (`--seed`), so the map is task-aware, not generic.
4. **Budget**: include top-ranked files' definitions until `--budget` tokens
   (default 1024) are used.

```
vajra_repomap.py [--root DIR] [--seed file_or_ident ...] [--budget N] [--json]
```

## When to call it (just-in-time, not always-on)

- **Session start / branch switch** — orient once; seed from the user's prompt
  identifiers.
- **On demand** — when the agent needs to find where something lives, before a
  broad `grep`. Cheaper and more targeted than scanning the tree.
- **Inside a Ralph iteration** — each fresh agent regenerates a small map for
  orientation (`engine/ralph-loop.md`).

Do NOT inject it every turn — it's pure overhead once the agent has oriented.

## Upgrade path (tree-sitter version)

The stdlib version uses regex symbol extraction, which is good but not perfect
for every language. For higher fidelity, swap the extractor for tree-sitter:
- deps: `tree_sitter_languages` (precompiled grammars, no per-grammar build),
  `grep_ast` (renders defs with structural parents), `networkx` (graph/PageRank),
  `tiktoken` (exact token counts).
- Replace `DEF_PATTERNS`/`extract()` with tree-sitter `.scm` tag queries
  (`name.definition.*` → defs, `name.reference.*` → refs); fall back to a Pygments
  lexer for ref-only files.
- Cache keyed on **git blob SHA** (`git ls-files -s`), not mtime — content-exact,
  survives `checkout`/`stash` without false misses. Recompute ranks only when the
  tree SHA changes.

The interface (`--root/--seed/--budget/--json`) stays the same, so the upgrade is
drop-in.
