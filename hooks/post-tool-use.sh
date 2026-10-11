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
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
command -v python3 >/dev/null 2>&1 || exit 0
python3 -I "${HERE}/../atman/atman.py" hook >/dev/null 2>&1 || true
exit 0
