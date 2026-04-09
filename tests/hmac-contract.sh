#!/usr/bin/env bash
# hmac-contract.sh — Cross-component HMAC contract test
#
# Verifies that the shell-based HMAC (openssl, as used by fleet-coordinator.sh)
# and the Node.js-based HMAC (crypto module, as used by engine.ts) produce
# identical output for the same key + payload.
#
# Exit 0 = pass, Exit 1 = fail.
set -euo pipefail

PASS=0
FAIL=0

report() {
  local label="$1" status="$2"
  if [ "$status" = "PASS" ]; then
    printf '  \033[32mPASS\033[0m  %s\n' "$label"
    ((PASS++))
  else
    printf '  \033[31mFAIL\033[0m  %s\n' "$label"
    ((FAIL++))
  fi
}

echo "=== HMAC Cross-Component Contract Test ==="
echo ""

# --- Prerequisites ---
if ! command -v openssl &>/dev/null; then
  echo "SKIP: openssl not installed"
  exit 0
fi

if ! command -v node &>/dev/null; then
  echo "SKIP: node not installed"
  exit 0
fi

# --- Generate a deterministic test key (64 hex chars = 32 bytes) ---
TEST_KEY_HEX="a]$(openssl rand -hex 32)"
# Use a clean 64-char hex key
TEST_KEY_HEX="$(openssl rand -hex 32)"

# --- Test payloads ---
PAYLOADS=(
  'hello world'
  '{"id":"abc123","name":"test-campaign","status":"active","currentNode":null}'
  ''
  'line1\nline2\ttab'
  '{"nested":{"key":"value with spaces and \"quotes\""}}'
)

TMPDIR="$(mktemp -d)"
KEY_FILE="${TMPDIR}/.hmac-key"
printf '%s' "$TEST_KEY_HEX" > "$KEY_FILE"

for i in "${!PAYLOADS[@]}"; do
  PAYLOAD="${PAYLOADS[$i]}"
  LABEL="payload[$i]: '${PAYLOAD:0:40}...'"

  # --- Shell HMAC (same as fleet-coordinator.sh compute_hmac) ---
  SHELL_HMAC="$(printf '%s' "$PAYLOAD" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${TEST_KEY_HEX}" -hex 2>/dev/null | awk '{print $NF}')"

  # --- Node.js HMAC (same as engine.ts computeHmac) ---
  NODE_HMAC="$(node -e "
    const crypto = require('crypto');
    const key = Buffer.from('${TEST_KEY_HEX}', 'hex');
    const hmac = crypto.createHmac('sha256', key).update(process.argv[1], 'utf8').digest('hex');
    process.stdout.write(hmac);
  " "$PAYLOAD")"

  if [ "$SHELL_HMAC" = "$NODE_HMAC" ]; then
    report "$LABEL" "PASS"
  else
    report "$LABEL" "FAIL"
    echo "    shell: $SHELL_HMAC"
    echo "    node:  $NODE_HMAC"
  fi
done

# --- Additional: verify that different payloads produce different HMACs ---
HMAC_A="$(printf '%s' "payload-a" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${TEST_KEY_HEX}" -hex 2>/dev/null | awk '{print $NF}')"
HMAC_B="$(printf '%s' "payload-b" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${TEST_KEY_HEX}" -hex 2>/dev/null | awk '{print $NF}')"

if [ "$HMAC_A" != "$HMAC_B" ]; then
  report "different payloads produce different HMACs" "PASS"
else
  report "different payloads produce different HMACs" "FAIL"
fi

# --- Additional: verify key-sensitivity (same payload, different key) ---
OTHER_KEY="$(openssl rand -hex 32)"
HMAC_KEY1="$(printf '%s' "test" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${TEST_KEY_HEX}" -hex 2>/dev/null | awk '{print $NF}')"
HMAC_KEY2="$(printf '%s' "test" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${OTHER_KEY}" -hex 2>/dev/null | awk '{print $NF}')"

if [ "$HMAC_KEY1" != "$HMAC_KEY2" ]; then
  report "different keys produce different HMACs" "PASS"
else
  report "different keys produce different HMACs" "FAIL"
fi

# --- Cleanup ---
rm -rf "$TMPDIR"

# --- Summary ---
echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"

if [ "$FAIL" -gt 0 ]; then
  echo "HMAC CONTRACT TEST FAILED — shell and Node.js implementations diverge!"
  exit 1
fi

echo "HMAC contract verified: shell and Node.js produce identical output."
exit 0
