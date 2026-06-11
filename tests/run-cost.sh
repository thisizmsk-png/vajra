#!/usr/bin/env bash
# Test for scripts/vajra-cost.sh — transcript → per-model cost/token ledger.
# Run: bash tests/run-cost.sh
set -uo pipefail
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO/scripts/vajra-cost.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vajra-cost-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
T="$WORK/transcript.jsonl"

# Synthetic transcript: 2 opus calls + 1 unpriced model + a no-usage line.
cat > "$T" <<'EOF'
{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":1000,"output_tokens":500,"cache_creation_input_tokens":0,"cache_read_input_tokens":4000}}}
{"type":"user","message":{"role":"user"}}
{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":2000,"output_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":8000}}}
{"type":"assistant","message":{"model":"claude-fable-5","usage":{"input_tokens":100,"output_tokens":50}}}
EOF

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }
near() { awk -v a="$1" -v b="$2" 'BEGIN{d=a-b; if(d<0)d=-d; exit (d<0.0005)?0:1}'; }

J="$(bash "$SCRIPT" --json "$T" 2>/dev/null)"
[ -n "$J" ]; check "json output produced" $?

opus="$(echo "$J" | jq '.byModel[] | select(.model|contains("opus"))')"
[ "$(echo "$opus" | jq '.input')" = "3000" ]; check "opus input aggregated (3000)" $?
[ "$(echo "$opus" | jq '.output')" = "1500" ]; check "opus output aggregated (1500)" $?
[ "$(echo "$opus" | jq '.cr')" = "12000" ]; check "opus cache-read aggregated (12000)" $?
[ "$(echo "$opus" | jq '.calls')" = "2" ]; check "opus calls counted (2)" $?
near "$(echo "$opus" | jq '.costUsd')" "0.1755"; check "opus cost computed (~0.1755)" $?
near "$(echo "$opus" | jq '.cacheHitRate')" "0.8"; check "opus cache hit rate (0.8)" $?

fable="$(echo "$J" | jq '.byModel[] | select(.model|contains("fable"))')"
[ "$(echo "$fable" | jq '.costUsd')" = "null" ]; check "unpriced model cost = null (not fabricated)" $?
[ "$(echo "$fable" | jq '.input')" = "100" ]; check "unpriced model tokens still counted" $?

# no-usage line ignored
[ "$(echo "$J" | jq '.byModel | length')" = "2" ]; check "no-usage lines ignored (2 models)" $?

# human-readable report runs
bash "$SCRIPT" "$T" >/dev/null 2>&1; check "human-readable report exits 0" $?

echo "----------------------------------------"
echo "cost-ledger: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
