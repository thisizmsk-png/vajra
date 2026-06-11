#!/usr/bin/env bash
# Regression test for hooks/post-tool-use.sh practice-log hash chaining (VJR-08).
#   - each entry carries prevHmac + hmac
#   - the chain links (entry N prevHmac == entry N-1 hmac; first == "genesis")
#   - each entry's HMAC verifies over its signed payload
#   - deleting a middle entry breaks the chain (detectable)
#
# Run: bash tests/hooks/run-post-tool-use.sh

set -uo pipefail
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 0; }
command -v openssl >/dev/null || { echo "SKIP: openssl required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-ptu-XXXXXX")"
trap 'rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"

VDIR="$HOME/.claude/vajra"
LOG="$VDIR/practice-log.jsonl"
mkdir -p "$VDIR"
KEY="$(head -c 32 /dev/urandom | xxd -p | tr -d '\n')"
printf '%s' "$KEY" > "$VDIR/.hmac-key"; chmod 600 "$VDIR/.hmac-key"
HOOK="$REPO/hooks/post-tool-use.sh"

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

# Emit 3 skill-level entries (internal tools like Read/Bash are skipped by design).
for skill in testing code-review threat-modeling; do
  printf '%s' "{\"tool\":\"$skill\"}" | bash "$HOOK" >/dev/null 2>&1
done

n="$(wc -l < "$LOG" | tr -d ' ')"
[ "$n" -eq 3 ]; check "3 entries written" $?

h1="$(sed -n 1p "$LOG" | jq -r '.hmac')"
h2="$(sed -n 2p "$LOG" | jq -r '.hmac')"
p1="$(sed -n 1p "$LOG" | jq -r '.prevHmac')"
p2="$(sed -n 2p "$LOG" | jq -r '.prevHmac')"
p3="$(sed -n 3p "$LOG" | jq -r '.prevHmac')"

[ "$p1" = "genesis" ];  check "entry 1 prevHmac == genesis" $?
[ "$p2" = "$h1" ];      check "entry 2 chains to entry 1" $?
[ "$p3" = "$h2" ];      check "entry 3 chains to entry 2" $?

# Verify entry 1's HMAC over its signed payload (everything except .hmac).
verify_line() {
  local line="$1" expect payload actual
  expect="$(printf '%s' "$line" | jq -r '.hmac')"
  payload="$(printf '%s' "$line" | jq -c 'del(.hmac)')"
  actual="$(printf '%s' "$payload" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${KEY}" -hex 2>/dev/null | awk '{print $NF}')"
  [ "$expect" = "$actual" ]
}
verify_line "$(sed -n 1p "$LOG")"; check "entry 1 HMAC verifies" $?
verify_line "$(sed -n 3p "$LOG")"; check "entry 3 HMAC verifies" $?

# Tamper: delete the middle entry; the chain from 1 -> 3 must now be broken.
sed -i.bak '2d' "$LOG" && rm -f "$LOG.bak"
new_h1="$(sed -n 1p "$LOG" | jq -r '.hmac')"
new_p2="$(sed -n 2p "$LOG" | jq -r '.prevHmac')"   # was entry 3
[ "$new_p2" != "$new_h1" ]; check "deleting middle entry breaks the chain (detectable)" $?

echo "----------------------------------------"
echo "post-tool-use chaining: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
