# /vajra verify-work <slug> "<cmd>" ["<cmd>" ...]

The proof-of-work gate. It runs `~/.claude/skills/vajra/scripts/vajra-verify-work.sh`.

```
bash ~/.claude/skills/vajra/scripts/vajra-verify-work.sh my-fix "npm test" "npm run build"
```

- At least one command is required.
- It runs each command, captures exit code and output, and records the diff
  against `HEAD` plus untracked files. It writes `evidence.json`, per-command
  logs, `diff.patch` and `WALKTHROUGH.md` under `$VAJRA_WALKTHROUGH_DIR`
  (default `./data/walkthroughs/<slug>/`).
- It exits non-zero if any command fails. Then the work is not done: say which
  command failed and show the tail of its log.

Pick commands that actually exercise the change (tests, build, lint). The gate
proves your checks passed, not that they are enough. Details are in
`engine/verification.md`.
