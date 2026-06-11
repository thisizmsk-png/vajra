#!/usr/bin/env bash
# Test for Plan Mode: scripts/vajra-phase.sh actuator + the pre-tool-use.sh
# phase gate it drives.
#
#   set plan  -> phase=plan; Write & mutating Bash blocked; Read allowed
#   set act   -> phase=act;  Write allowed
#   clear     -> default (act); Write allowed
#
# Run: bash tests/hooks/run-phase-mode.sh

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-phase-XXXXXX")"
trap 'chmod -R u+w "$TEST_HOME" 2>/dev/null; rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"

SKILL="$HOME/.claude/skills/vajra"
mkdir -p "$SKILL/hooks" "$SKILL/scripts" "$HOME/.claude/vajra"
cp "$REPO/hooks/pre-tool-use.sh" "$SKILL/hooks/"
cp "$REPO/scripts/vajra-phase.sh" "$SKILL/scripts/"
HOOK="$SKILL/hooks/pre-tool-use.sh"
PHASE="$SKILL/scripts/vajra-phase.sh"

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }
hook() { printf '%s' "$1" | bash "$HOOK" >/dev/null 2>&1; }

# --- set plan ---
bash "$PHASE" set plan >/dev/null
[ "$(bash "$PHASE" get)" = "plan" ]; check "set plan -> get plan" $?
hook '{"tool":"Write","input":{"file_path":"/tmp/x","content":"y"}}'; [ $? -eq 1 ]; check "plan: Write blocked" $?
hook '{"tool":"Bash","input":"mkdir /tmp/newdir"}'; [ $? -eq 1 ]; check "plan: mutating Bash blocked" $?
hook '{"tool":"Bash","input":"echo hi > /tmp/x"}'; [ $? -eq 1 ]; check "plan: redirect Bash blocked" $?
hook '{"tool":"Read","input":"/tmp/x"}'; [ $? -eq 0 ]; check "plan: Read allowed" $?
hook '{"tool":"Bash","input":"ls -la /tmp"}'; [ $? -eq 0 ]; check "plan: read-only Bash allowed" $?
# transition channel: writing the .phase control file is allowed even in plan
hook "{\"tool\":\"Write\",\"input\":{\"file_path\":\"$HOME/.claude/vajra/.phase\",\"content\":\"act\"}}"; [ $? -eq 0 ]; check "plan: Write to .phase allowed (transition channel)" $?
# but NOT to other files in ~/.claude/vajra
hook "{\"tool\":\"Write\",\"input\":{\"file_path\":\"$HOME/.claude/vajra/evil.txt\",\"content\":\"x\"}}"; [ $? -eq 1 ]; check "plan: Write to non-.phase still blocked" $?

# --- set act ---
bash "$PHASE" set act >/dev/null
[ "$(bash "$PHASE" get)" = "act" ]; check "set act -> get act" $?
hook '{"tool":"Write","input":{"file_path":"/tmp/x","content":"y"}}'; [ $? -eq 0 ]; check "act: Write allowed" $?
hook '{"tool":"Bash","input":"mkdir /tmp/newdir2"}'; [ $? -eq 0 ]; check "act: mutating Bash allowed" $?

# --- clear (default = act) ---
bash "$PHASE" clear >/dev/null
[ ! -f "$HOME/.claude/vajra/.phase" ]; check "clear removes .phase" $?
[ "$(bash "$PHASE" get)" = "act" ]; check "clear: get defaults to act" $?
hook '{"tool":"Write","input":{"file_path":"/tmp/x","content":"y"}}'; [ $? -eq 0 ]; check "no flag: Write allowed" $?

# --- explore behaves like plan (read-only) ---
bash "$PHASE" set explore >/dev/null
hook '{"tool":"Edit","input":{"file_path":"/tmp/x"}}'; [ $? -eq 1 ]; check "explore: Edit blocked" $?

# --- invalid input ---
bash "$PHASE" set bogus >/dev/null 2>&1; [ $? -eq 2 ]; check "set bogus -> usage error" $?

echo "----------------------------------------"
echo "plan-mode: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
