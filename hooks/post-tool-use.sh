#!/usr/bin/env bash
# Vajra PostToolUse / PostToolUseFailure hook: Atman practice observer.
#
# Claude Code pipes the hook payload (session_id, hook_event_name, tool_name,
# tool_input, tool_response, error, agent_type, cwd) as JSON on stdin.
# atman/atman.py appends one HMAC-chained line to ~/.claude/vajra/practice-log.jsonl
# (names, outcome, a hash of the normalized error; never raw text) and one line to
# the per-session event log used by scripts/vajra-replay.sh.
#
# This hook never blocks and never fails the tool call: it always exits 0.
# NFR-SEC-2: a hook runs outside the guard, and the programs it starts can read the HMAC key file. A same-uid worker
# can write user dirs (for example ~/.local/bin, or the Homebrew prefix), so every tool comes only from the root-owned system dirs
# and python3 only from its absolute path, whatever PATH the caller (a Claude Code session, the app) had.
PATH="/usr/bin:/bin:/usr/sbin:/sbin"; export PATH
PYTHON3="/usr/bin/python3"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
[ -x "$PYTHON3" ] || exit 0
"$PYTHON3" -I "${HERE}/../atman/atman.py" hook >/dev/null 2>&1 || true
exit 0
