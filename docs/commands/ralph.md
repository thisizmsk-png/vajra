# /vajra ralph <progress.md>

A hard-reset loop for long runs: every iteration is a fresh `claude -p` that
reads only the durable progress file, does one unit of work, updates the file
and exits. Runs `~/.claude/skills/vajra/scripts/vajra-ralph.sh`; see
`bash scripts/vajra-ralph.sh --help` for flags.

- It needs an explicit permission mode and allowed tools, and has a turn cap
  per iteration and a maximum number of iterations.
- It stops when no `- [ ]` items remain, when the file contains
  `RALPH-COMPLETE`, when the iteration cap is hit, or when the progress file
  hasn't changed for N iterations (stall detection).
- Each iteration writes a log.

Before you start it, confirm the permission mode and tool list with the user:
the loop runs unattended. Protocol details are in `engine/ralph-loop.md`.
