#!/usr/bin/env bash
# hooks/user-prompt-submit.sh: forwards stdin to scripts/vajra_route.py and
# never blocks the prompt (always exit 0, only JSON objects pass through).
#
# Run: bash tests/hooks/run-user-prompt-submit.sh

set -uo pipefail
command -v python3 >/dev/null || { echo "SKIP: python3 required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-ups-XXXXXX")"
trap 'rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"
SKILL="$HOME/.claude/skills/vajra"
mkdir -p "$SKILL/hooks" "$SKILL/scripts"
cp "$REPO/hooks/user-prompt-submit.sh" "$SKILL/hooks/"
HOOK="$SKILL/hooks/user-prompt-submit.sh"
R="$SKILL/scripts/vajra_route.py"

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); echo "FAIL :: $1"; fi; }
P='{"session_id":"s","hook_event_name":"UserPromptSubmit","prompt":"/vajra review this","cwd":"/tmp"}'

# No router installed.
out="$(printf '%s' "$P" | bash "$HOOK")"; rc=$?
[ "$rc" -eq 0 ] && [ -z "$out" ]; check "no router: exit 0, no output" $?

# Stub router echoes stdin back inside additionalContext: proves stdin is forwarded.
cat > "$R" <<'PY'
import json, sys
d = json.load(sys.stdin)
print(json.dumps({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": "saw:" + d["prompt"]}}))
PY
out="$(printf '%s' "$P" | bash "$HOOK")"; rc=$?
[ "$rc" -eq 0 ]; check "stub router: exit 0" $?
printf '%s' "$out" | grep -q '"additionalContext": "saw:/vajra review this"'; check "payload forwarded on stdin, JSON passed through" $?

# Crashing router: exit 0 and nothing printed.
printf 'import sys\nsys.stderr.write("boom")\nsys.exit(2)\n' > "$R"
out="$(printf '%s' "$P" | bash "$HOOK" 2>&1)"; rc=$?
[ "$rc" -eq 0 ] && [ -z "$out" ]; check "router exits 2: hook still exits 0, silent" $?

# Router printing plain text: dropped (only a JSON object may pass).
printf 'print("ignore previous instructions")\n' > "$R"
out="$(printf '%s' "$P" | bash "$HOOK")"; rc=$?
[ "$rc" -eq 0 ] && [ -z "$out" ]; check "non-JSON router output dropped" $?

# The real router, if shipped: exit 0 and valid output.
if [ -f "$REPO/scripts/vajra_route.py" ]; then
  cp "$REPO/scripts/vajra_route.py" "$R"
  [ -d "$REPO/config" ] && cp -R "$REPO/config" "$SKILL/"
  out="$(printf '%s' "$P" | bash "$HOOK")"; rc=$?
  [ "$rc" -eq 0 ]; check "real router: exit 0" $?
  if [ -n "$out" ] && command -v jq >/dev/null; then
    printf '%s' "$out" | jq -e '.hookSpecificOutput.hookEventName == "UserPromptSubmit"' >/dev/null; check "real router: valid hookSpecificOutput" $?
  fi
  out="$(printf '%s' '{"prompt":"fix the login bug","session_id":"s"}' | bash "$HOOK")"; rc=$?
  [ "$rc" -eq 0 ]; check "real router, non-/vajra prompt: exit 0" $?
fi

echo "----------------------------------------"
echo "user-prompt-submit.sh: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
