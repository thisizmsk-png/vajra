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


# --- v0.5: dedupe by message id, current pricing, robustness ----------------
T2="$WORK/t2.jsonl"
cat > "$T2" <<'EOT'
{"type":"assistant","message":{"id":"msg_1","model":"claude-opus-5-5","usage":{"input_tokens":1000,"output_tokens":1000,"cache_creation_input_tokens":1000,"cache_read_input_tokens":10000}}}
{"type":"assistant","message":{"id":"msg_1","model":"claude-opus-5-5","usage":{"input_tokens":1000,"output_tokens":1000,"cache_creation_input_tokens":1000,"cache_read_input_tokens":10000}}}
{"type":"assistant","message":{"id":"msg_1","model":"claude-opus-5-5","usage":{"input_tokens":1000,"output_tokens":1000,"cache_creation_input_tokens":1000,"cache_read_input_tokens":10000}}}
this line is not json
{"type":"assistant","message":{"id":"msg_2","model":"claude-sonnet-5-5","usage":{"input_tokens":1000000,"output_tokens":1000000}}}
{"type":"assistant","message":{"id":"msg_3","model":"claude-haiku-5-5","usage":{"input_tokens":1000000,"output_tokens":1000000}}}
{"type":"assistant","message":{"id":"msg_4","model":"claude-fable-5-1","usage":{"input_tokens":1000000,"output_tokens":1000000}}}
{"type":"assistant","message":{"id":"msg_5","model":"claude-sonnet-4-6","usage":{"input_tokens":1000000,"output_tokens":0}}}
{"type":"assistant","message":{"model":"claude-opus-5-5","usage":{"input_tokens":5,"output_tokens":5}}}
{"type":"assistant","message":{"model":"claude-opus-5-5","usage":{"input_tokens":5,"output_tokens":5}}}
EOT
J2="$(bash "$SCRIPT" --json "$T2" 2>/dev/null)"
[ -n "$J2" ]; check "json output with a malformed line" $?
o5="$(echo "$J2" | jq '.byModel[] | select(.model=="claude-opus-5-5")')"
[ "$(echo "$o5" | jq '.calls')" = "3" ]; check "repeated message id counted once (+2 id-less lines)" $?
[ "$(echo "$o5" | jq '.input')" = "1010" ]; check "dedupe: opus-5-5 input 1000 + 2x5" $?
[ "$(echo "$o5" | jq -r '.rateId')" = "opus-5-5" ]; check "opus-5-5 matches its own rate, not legacy" $?
near "$(echo "$o5" | jq '.costUsd')" "0.03324"; check "opus-5-5 cost (4/20, cache multipliers)" $?
near "$(echo "$J2" | jq '.byModel[] | select(.model=="claude-sonnet-5-5") | .costUsd')" "12"; check "sonnet-5-5 \$2/\$10" $?
near "$(echo "$J2" | jq '.byModel[] | select(.model=="claude-haiku-5-5") | .costUsd')" "0.6"; check "haiku-5-5 \$0.10/\$0.50" $?
near "$(echo "$J2" | jq '.byModel[] | select(.model=="claude-fable-5-1") | .costUsd')" "60"; check "fable-5-1 \$10/\$50" $?
[ "$(echo "$J2" | jq -r '.byModel[] | select(.model=="claude-sonnet-4-6") | .rateId')" = "sonnet-legacy" ]; check "older sonnet uses legacy entry" $?
near "$(echo "$J2" | jq '.byModel[] | select(.model=="claude-sonnet-4-6") | .costUsd')" "3"; check "legacy sonnet \$3 input" $?
jq -e '.models | type == "array"' "$REPO/config/pricing.json" >/dev/null; check "pricing models is an ordered array" $?
# every specific entry precedes any legacy entry whose match it contains
jq -e '[.models | to_entries[] | select(.value.legacy != true) | .key] | max < ([$m[0].models | to_entries[] | select(.value.legacy == true) | .key] | min)' --slurpfile m "$REPO/config/pricing.json" "$REPO/config/pricing.json" >/dev/null; check "specific entries ordered before legacy" $?
EMPTY="$WORK/empty.jsonl"; : > "$EMPTY"
[ "$(bash "$SCRIPT" --json "$EMPTY" | jq '.totals.tokens')" = "0" ]; check "empty transcript -> 0 tokens" $?
bash "$SCRIPT" "$WORK/missing.jsonl" >/dev/null 2>&1; [ $? -eq 1 ]; check "missing transcript exits 1" $?

echo "----------------------------------------"
echo "cost-ledger: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
