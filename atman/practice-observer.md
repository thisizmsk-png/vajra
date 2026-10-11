# Practice observer

The observer is code, not a prompt: `hooks/post-tool-use.sh` calls
`atman/atman.py hook` on every `PostToolUse` and `PostToolUseFailure` event.
The model does not need to record anything.

Each line in `~/.claude/vajra/practice-log.jsonl` has these fields:
`ts, session_id, event, tool, skill, agent, outcome, error_sha256, seq, prevHmac, hmac`,
plus `signal: "verify-work"` on Bash calls of `scripts/vajra-verify-work.sh`.

- **outcome:** `failure` for `PostToolUseFailure`, or when `tool_response` reports
  `is_error`, `success: false` or a non-zero exit code. Otherwise `success`.
- **error_sha256:** sha256 of the normalized error (lowercased, paths, hex and
  numbers collapsed). Raw text is never stored.
- **Integrity:** HMAC-chained (see `ATMAN.md` section 2).
- **Size:** `atman.sh dream` keeps the newest 10,000 lines and archives the rest.

Read it with `atman.sh status` and `atman.sh observe`.
