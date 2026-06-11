#!/usr/bin/env bash
# Test for the event log (post-tool-use.sh capture) + scripts/vajra-replay.sh.
# Run: bash tests/run-event-log.sh
set -uo pipefail
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 0; }
command -v openssl >/dev/null || { echo "SKIP: openssl required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-ev-XXXXXX")"
trap 'rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"
VDIR="$HOME/.claude/vajra"; mkdir -p "$VDIR"
printf '%s' "$(head -c 32 /dev/urandom | xxd -p | tr -d '\n')" > "$VDIR/.hmac-key"; chmod 600 "$VDIR/.hmac-key"
POST="$REPO/hooks/post-tool-use.sh"
REPLAY="$REPO/scripts/vajra-replay.sh"
LOG="$VDIR/events/s1.jsonl"

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

# Feed a mix of internal + skill tools, all in session s1
for t in Read Bash testing code-review; do
  printf '%s' "{\"tool\":\"$t\",\"session_id\":\"s1\",\"input\":{\"x\":\"$t\"}}" | bash "$POST" >/dev/null 2>&1
done

[ -f "$LOG" ]; check "event log created" $?
[ "$(wc -l < "$LOG" | tr -d ' ')" = "4" ]; check "all 4 tool calls logged (incl internal)" $?
[ "$(sed -n 1p "$LOG" | jq -r '.seq')" = "0" ]; check "seq starts at 0" $?
[ "$(sed -n 4p "$LOG" | jq -r '.seq')" = "3" ]; check "seq increments to 3" $?
[ "$(sed -n 1p "$LOG" | jq -r '.tool')" = "Read" ]; check "internal Read captured in event log" $?
[ "$(sed -n 2p "$LOG" | jq -r '.prevHmac')" = "$(sed -n 1p "$LOG" | jq -r '.hmac')" ]; check "event 2 chains to event 1" $?
# args are hashed, not stored raw
[ "$(sed -n 1p "$LOG" | jq -r '.argsHash | length')" = "64" ]; check "args hashed (sha256), not raw" $?

# replay verifies clean
out="$(bash "$REPLAY" s1 2>&1)"; rc=$?
[ "$rc" -eq 0 ]; check "replay exit 0 on intact chain" $?
echo "$out" | grep -q "chain INTACT"; check "replay reports chain INTACT" $?
echo "$out" | grep -q "4 events"; check "replay shows 4 events" $?

# --list works
bash "$REPLAY" --list 2>/dev/null | grep -q "s1"; check "replay --list shows s1" $?

# tamper: delete middle event → chain break detected
sed -i.bak '2d' "$LOG" && rm -f "$LOG.bak"
bash "$REPLAY" s1 >/dev/null 2>&1; [ $? -eq 1 ]; check "replay exit 1 after tamper (chain break)" $?

echo "----------------------------------------"
echo "event-log: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
