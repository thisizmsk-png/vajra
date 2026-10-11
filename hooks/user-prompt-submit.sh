#!/usr/bin/env bash
# Vajra UserPromptSubmit hook: 0-token router.
#
# Passes the hook JSON on stdin to scripts/vajra_route.py, which prints nothing
# or one hookSpecificOutput JSON object (additionalContext with a routing
# suggestion). This wrapper never blocks or fails the prompt: it always exits
# 0, drops router stderr, and only forwards stdout that is a single JSON object.

ROUTER="${HOME}/.claude/skills/vajra/scripts/vajra_route.py"
[ -f "$ROUTER" ] || exit 0
command -v python3 >/dev/null 2>&1 || exit 0

out="$(python3 -I "$ROUTER" 2>/dev/null)" || out=""
case "$out" in
  "{"*"}") printf '%s\n' "$out" ;;
esac
exit 0
