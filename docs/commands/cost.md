# /vajra cost [transcript]

Runs `bash ~/.claude/skills/vajra/scripts/vajra-cost.sh [--json] [transcript.jsonl]`.
With no argument it reads the most recently modified transcript under
`~/.claude/projects/`.

- It counts each assistant message once (deduplicated by message id; transcripts
  repeat a message for every content block).
- Per model it reports input, output, cache-write and cache-read tokens, the
  call count, the cache-hit rate, and USD.
- Rates come from `config/pricing.json`, most specific match first. A model
  with no rate shows `?` for cost; tokens are always exact.

Show the table to the user. For live spend, Claude Code's `/usage` and the
statusline are the source of truth; this ledger is per transcript.
