# /vajra repomap [--seed <file>...]

Runs `python3 -I ~/.claude/skills/vajra/scripts/vajra_repomap.py` in the repo.
It builds a relevance-ranked map (PageRank over definitions and references) for
quick orientation. Seed it with the files in scope to rank their neighbours
first. Details are in `engine/repo-map.md`.

Treat the output as a map, not ground truth: open the files before you change
them.
