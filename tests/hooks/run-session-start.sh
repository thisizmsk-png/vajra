#!/usr/bin/env bash
# Integration test for hooks/session-start.sh integrity verification (VJR-06)
# and the pre-tool-use.sh tamper-flag enforcement.
#
#   clean tree           -> no tamper flag, mutations allowed
#   tampered hook        -> tamper flag written naming the file
#   flag present         -> mutating tool blocked, read-only tool allowed
#
# Run: bash tests/hooks/run-session-start.sh

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-ss-XXXXXX")"
trap 'chmod -R u+w "$TEST_HOME" 2>/dev/null; rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"

SKILL="$HOME/.claude/skills/vajra"
VDIR="$HOME/.claude/vajra"
mkdir -p "$SKILL/hooks" "$SKILL/engine" "$SKILL/config" "$SKILL/fleet" "$SKILL/redteam" "$SKILL/scripts" "$VDIR"

# Copy the real critical files into the temp skill dir.
cp "$REPO/hooks/pre-tool-use.sh"   "$SKILL/hooks/"
cp "$REPO/hooks/post-tool-use.sh"  "$SKILL/hooks/"
cp "$REPO/hooks/session-start.sh"  "$SKILL/hooks/"
cp "$REPO/engine/sanitizer.ts"     "$SKILL/engine/"
cp "$REPO/engine/engine.ts"        "$SKILL/engine/"
cp "$REPO/engine/router.md"        "$SKILL/engine/"
cp "$REPO/config/default.json"     "$SKILL/config/"
cp "$REPO/config/security-contracts.json" "$SKILL/config/"
cp "$REPO/fleet/fleet-coordinator.sh"     "$SKILL/fleet/"
cp "$REPO/redteam/snapshot.sh"     "$SKILL/redteam/"
cp "$REPO/redteam/orchestrator.md" "$SKILL/redteam/"
cp "$REPO/redteam/scorer.md"       "$SKILL/redteam/"
cp "$REPO/scripts/generate-manifest.sh" "$SKILL/scripts/"
cp "$REPO/scripts/install.sh"      "$SKILL/scripts/" 2>/dev/null || echo "#noop" > "$SKILL/scripts/install.sh"
cp "$REPO/SKILL.md"                "$SKILL/"

# Generate a manifest for the temp tree.
bash "$SKILL/scripts/generate-manifest.sh" >/dev/null 2>&1

PASS=0; FAIL=0
check() { # desc cond(0=ok)
  if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi
}

# 1. Clean tree: session-start should NOT create the tamper flag.
bash "$SKILL/hooks/session-start.sh" >/dev/null 2>&1
[ ! -f "$VDIR/.integrity-violation" ]; check "clean tree -> no tamper flag" $?

# 2. Tamper a critical enforcement file.
echo "# malicious change" >> "$SKILL/hooks/pre-tool-use.sh"
bash "$SKILL/hooks/session-start.sh" >/dev/null 2>&1
[ -f "$VDIR/.integrity-violation" ]; check "tampered hook -> tamper flag written" $?
grep -q "pre-tool-use.sh" "$VDIR/.integrity-violation" 2>/dev/null; check "flag names the tampered file" $?

# 3. With the flag present, pre-tool-use blocks mutations but allows reads.
ec=0; printf '%s' '{"tool":"Write","input":{"file_path":"/tmp/x","content":"y"}}' \
  | bash "$SKILL/hooks/pre-tool-use.sh" >/dev/null 2>&1 || ec=1
[ "$ec" -eq 1 ]; check "flag present -> Write blocked" $?

ec=0; printf '%s' '{"tool":"Read","input":"/tmp/x"}' \
  | bash "$SKILL/hooks/pre-tool-use.sh" >/dev/null 2>&1 || ec=1
[ "$ec" -eq 0 ]; check "flag present -> Read allowed (investigation)" $?

ec=0; printf '%s' '{"tool":"Bash","input":"rm /tmp/scratch"}' \
  | bash "$SKILL/hooks/pre-tool-use.sh" >/dev/null 2>&1 || ec=1
[ "$ec" -eq 1 ]; check "flag present -> mutating Bash blocked" $?

# 4. Clearing the flag + clean re-verify restores normal operation.
# (restore the file so the tree is clean again)
cp "$REPO/hooks/pre-tool-use.sh" "$SKILL/hooks/"
bash "$SKILL/hooks/session-start.sh" >/dev/null 2>&1
[ ! -f "$VDIR/.integrity-violation" ]; check "restored tree -> flag auto-cleared" $?

echo "----------------------------------------"
echo "session-start integrity: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
