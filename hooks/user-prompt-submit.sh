#!/usr/bin/env bash
# Vajra UserPromptSubmit hook: 0-token router.
#
# Passes the hook JSON on stdin to scripts/vajra_route.py, which prints nothing
# or one hookSpecificOutput JSON object (additionalContext with a routing
# suggestion). This wrapper never blocks or fails the prompt: it always exits
# 0, drops router stderr, and only forwards stdout that is a single JSON object.

# NFR-SEC-2: a hook runs outside the guard, and the programs it starts can read the HMAC key file. A same-uid worker
# can write user dirs (for example ~/.local/bin, or the Homebrew prefix), so every tool comes from the system dirs first
# and python3 only from its absolute path, whatever PATH the caller (a Claude Code session, the app) had.
PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin"; export PATH
PYTHON3="/usr/bin/python3"
ROUTER="${HOME}/.claude/skills/vajra/scripts/vajra_route.py"
[ -f "$ROUTER" ] || exit 0
[ -x "$PYTHON3" ] || exit 0

out="$("$PYTHON3" -I "$ROUTER" 2>/dev/null)" || out=""
case "$out" in
  "{"*"}") printf '%s\n' "$out" ;;
esac
exit 0
