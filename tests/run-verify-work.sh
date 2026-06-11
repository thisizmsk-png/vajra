#!/usr/bin/env bash
# Test for scripts/vajra-verify-work.sh — proof-of-work evidence capture.
# Run: bash tests/run-verify-work.sh
set -uo pipefail
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO/scripts/vajra-verify-work.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vajra-vw-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"
export VAJRA_WALKTHROUGH_DIR="$WORK/wt"

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

# --- all-pass case ---
bash "$SCRIPT" green "echo hello" "true" >/dev/null 2>&1; ec=$?
[ "$ec" -eq 0 ]; check "all-pass -> exit 0" $?
[ -f "$WORK/wt/green/evidence.json" ]; check "evidence.json written" $?
[ -f "$WORK/wt/green/WALKTHROUGH.md" ]; check "WALKTHROUGH.md written" $?
[ "$(jq -r '.allPassed' "$WORK/wt/green/evidence.json")" = "true" ]; check "evidence allPassed=true" $?
[ "$(jq -r '.commandsTotal' "$WORK/wt/green/evidence.json")" = "2" ]; check "2 commands recorded" $?
grep -q hello "$WORK/wt/green/cmd-1.log"; check "cmd-1 output captured" $?

# --- failing case ---
bash "$SCRIPT" red "true" "false" "echo after" >/dev/null 2>&1; ec=$?
[ "$ec" -eq 1 ]; check "any-fail -> exit 1" $?
[ "$(jq -r '.allPassed' "$WORK/wt/red/evidence.json")" = "false" ]; check "evidence allPassed=false" $?
[ "$(jq -r '.commandsFailed' "$WORK/wt/red/evidence.json")" = "1" ]; check "1 command failed" $?
grep -q 'checks FAILED' "$WORK/wt/red/WALKTHROUGH.md"; check "walkthrough marks failure" $?

# --- usage ---
bash "$SCRIPT" >/dev/null 2>&1; [ $? -eq 2 ]; check "no slug -> usage error" $?

echo "----------------------------------------"
echo "verify-work: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
