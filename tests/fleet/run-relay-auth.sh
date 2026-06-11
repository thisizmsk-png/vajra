#!/usr/bin/env bash
# Unit test for fleet-coordinator.sh discovery authenticity (VJR-03).
#
# Proves the coordinator is no longer a blind signing oracle:
#   - a provisioned agent's discovery is signed (with run nonce) and verifies
#   - a NON-provisioned (rogue) drop-in is rejected and deleted, never signed
#   - a replayed file carrying a foreign run nonce is rejected at collection
#   - a tampered HMAC is rejected
#
# Run: bash tests/fleet/run-relay-auth.sh

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-relay-XXXXXX")"
trap 'rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"

VDIR="$HOME/.claude/vajra"
RDIR="$VDIR/relay"
mkdir -p "$RDIR"
head -c 32 /dev/urandom | xxd -p | tr -d '\n' > "$VDIR/.hmac-key"
chmod 600 "$VDIR/.hmac-key"

# Source the coordinator's functions (guarded main won't run).
# shellcheck disable=SC1090
source "$REPO/fleet/fleet-coordinator.sh"
set +e +u
# Quiet, capturable logging; no-op cleanup so the EXIT trap can't nuke artifacts.
log_info() { echo "INFO $*"; }
log_warn() { echo "WARN $*"; }
cleanup() { :; }

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

# This run provisioned exactly one agent.
AGENT_IDS=("fleet-good")
RUN_NONCE="run-nonce-AAA"

# --- 1. Provisioned agent's unsigned discovery is signed + verifies ---
printf '%s' '{"agentId":"fleet-good","type":"finding","content":"legit"}' > "$RDIR/fleet-good.json"
sign_discoveries >/dev/null 2>&1
jq -e '.hmac' "$RDIR/fleet-good.json" >/dev/null 2>&1; check "provisioned discovery gets signed" $?
[ "$(jq -r '.runNonce' "$RDIR/fleet-good.json")" = "run-nonce-AAA" ]; check "signed payload carries run nonce" $?
out="$(collect_discoveries 2>&1)"
echo "$out" | grep -q "1 verified, 0 rejected"; check "provisioned discovery verifies at collection" $?

# --- 2. Rogue non-provisioned drop-in is rejected + deleted (signing oracle closed) ---
printf '%s' '{"agentId":"rogue-99","type":"finding","content":"pwned","capabilities":["bash","admin"]}' > "$RDIR/rogue.json"
sign_discoveries >/dev/null 2>&1
[ ! -f "$RDIR/rogue.json" ]; check "rogue drop-in deleted, never signed" $?

# --- 3. Replay across runs: foreign nonce rejected at collection ---
# Build a file properly signed under a DIFFERENT run nonce, then collect under
# the current nonce.
RUN_NONCE="run-nonce-OLD"
printf '%s' '{"agentId":"fleet-good","type":"finding","content":"replayed"}' > "$RDIR/replay.json"
sign_discoveries >/dev/null 2>&1     # signs replay.json with OLD nonce (+re-signs? good has hmac already)
RUN_NONCE="run-nonce-AAA"            # back to the real current run
out="$(collect_discoveries 2>&1)"
echo "$out" | grep -q "stale/foreign run nonce"; check "replayed (foreign-nonce) discovery rejected" $?

# --- 4. Tampered HMAC rejected ---
rm -f "$RDIR"/*.json
printf '%s' '{"agentId":"fleet-good","type":"finding","content":"legit"}' > "$RDIR/t.json"
RUN_NONCE="run-nonce-AAA"; sign_discoveries >/dev/null 2>&1
# flip the signature
jq '.hmac = "deadbeef"' "$RDIR/t.json" > "$RDIR/t.json.x" && mv "$RDIR/t.json.x" "$RDIR/t.json"
out="$(collect_discoveries 2>&1)"
echo "$out" | grep -q "0 verified, 1 rejected"; check "tampered HMAC rejected" $?

echo "----------------------------------------"
echo "fleet relay auth: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
