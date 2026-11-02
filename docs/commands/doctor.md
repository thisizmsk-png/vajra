# /vajra doctor

Runs `bash ~/.claude/skills/vajra/scripts/vajra-doctor.sh` and prints its
PASS/FAIL lines: hooks registered in `~/.claude/settings.json`, the HMAC key
present with mode 600, the manifest signature valid, the workflows installed in
`~/.claude/workflows/`, and the required tools on PATH.

For each FAIL, show its fix. Most are fixed by re-running
`bash ~/.claude/skills/vajra/scripts/install.sh`. Legacy `/vajra verify`
lands here.

A `WARN  stale install: workflow ...` line means a file in
`~/.claude/workflows/vajra-*.js` differs from the skill's `workflows/` (or is
no longer shipped). Re-run the installer, then restart Claude Code: it loads
hooks and saved workflows only at session start, so a running session keeps
executing the old workflow until it is restarted.
