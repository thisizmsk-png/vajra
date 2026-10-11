#!/usr/bin/env bash
# vajra-cost.sh: per-session cost and token ledger from a Claude Code transcript.
#
# Folds the assistant-message `usage` blocks of a transcript into a per-model
# ledger: input, output, cache-write and cache-read tokens, USD where the
# model's rate is known (config/pricing.json, most specific match first), and
# the cache-hit rate.
#
# Claude Code writes one transcript line per content block, and every line of
# the same API message repeats the same usage. Lines are therefore
# deduplicated by message id (the last line for an id wins); lines without an
# id are counted as they are. Lines that are not JSON are skipped.
#
#   vajra-cost.sh [transcript.jsonl]   # default: most recent transcript on disk
#   vajra-cost.sh --json [file]        # machine-readable ledger
#
# Token totals are always exact; cost is shown only for priced models.

set -uo pipefail
command -v jq >/dev/null 2>&1 || { echo "jq required" >&2; exit 2; }

SKILL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRICING="${VAJRA_PRICING:-${SKILL_DIR}/config/pricing.json}"
[ -f "$PRICING" ] || { echo "pricing file not found: $PRICING" >&2; exit 2; }

JSON_OUT=false
[ "${1:-}" = "--json" ] && { JSON_OUT=true; shift; }

TRANSCRIPT="${1:-}"
if [ -z "$TRANSCRIPT" ]; then
  TRANSCRIPT="$(ls -1t "${HOME}/.claude/projects/"*/*.jsonl 2>/dev/null | head -1)"
fi
[ -n "${TRANSCRIPT:-}" ] && [ -f "$TRANSCRIPT" ] || { echo "No transcript found. Pass one as an argument." >&2; exit 1; }

LEDGER="$(jq -R -s --slurpfile p "$PRICING" '
  ($p[0]) as $pr
  | [ split("\n")[] | select(length > 0) | (fromjson? // empty) | select(type == "object") ]
  | [ to_entries[]
      | .key as $i
      | (.value.message // .value) as $m
      | select(($m | type) == "object")
      | ($m.usage // null) as $u
      | select(($u | type) == "object")
      | {
          key: (if ($m.id | type) == "string" then "id:" + $m.id else "line:" + ($i | tostring) end),
          model: ($m.model // "unknown"),
          input: ($u.input_tokens // 0),
          output: ($u.output_tokens // 0),
          cw: ($u.cache_creation_input_tokens // 0),
          cr: ($u.cache_read_input_tokens // 0)
        } ]
  | group_by(.key) | map(.[-1])
  | group_by(.model)
  | map({
      model: .[0].model,
      input: (map(.input) | add),
      output: (map(.output) | add),
      cw: (map(.cw) | add),
      cr: (map(.cr) | add),
      calls: length
    })
  | map(
      . as $row
      | ([ $pr.models[] | select(.match as $s | ($row.model | contains($s))) ] | .[0]) as $rate
      | . + {
          rateId: ($rate.id // null),
          rate: $rate,
          costUsd: (if $rate then
            (($row.input * $rate.input
              + $row.output * $rate.output
              + $row.cw * $rate.input * $pr.cacheWriteMult
              + $row.cr * $rate.input * $pr.cacheReadMult) / 1000000)
            else null end),
          cacheHitRate: (if ($row.input + $row.cr) > 0 then ($row.cr / ($row.input + $row.cr)) else 0 end)
        }
    )
' "$TRANSCRIPT")" || { echo "could not parse transcript: $TRANSCRIPT" >&2; exit 1; }

if [ "$JSON_OUT" = true ]; then
  echo "$LEDGER" | jq --arg t "$TRANSCRIPT" '{transcript:$t, byModel:., totals:{
    tokens: (map(.input+.output+.cw+.cr) | add // 0),
    costUsd: (map(.costUsd // 0) | add // 0),
    calls: (map(.calls) | add // 0)
  }}'
  exit 0
fi

echo "Cost ledger: ${TRANSCRIPT##*/}"
echo "-------------------------------------------------------------------------"
printf '%-22s %8s %8s %9s %9s %7s %8s\n' "model" "in" "out" "cacheW" "cacheR" "hit%" "USD"
echo "$LEDGER" | jq -r '.[] |
  [ (.model | .[0:22]),
    (.input|tostring), (.output|tostring), (.cw|tostring), (.cr|tostring),
    ((.cacheHitRate*100)|floor|tostring),
    (if .costUsd then (.costUsd*10000|floor/10000|tostring) else "?" end)
  ] | @tsv' | while IFS=$'\t' read -r m i o cw cr hr c; do
    printf '%-22s %8s %8s %9s %9s %6s%% %8s\n' "$m" "$i" "$o" "$cw" "$cr" "$hr" "$c"
  done
echo "-------------------------------------------------------------------------"
echo "$LEDGER" | jq -r '
  (map(.input+.output+.cw+.cr) | add // 0) as $tok
  | (map(.costUsd // 0) | add // 0) as $cost
  | (map(.cr)|add // 0) as $crd | (map(.input)|add // 0) as $inp
  | "total: \($tok) tokens, $\(($cost*10000|floor)/10000) (priced models), cache hit \(if ($inp+$crd)>0 then (($crd/($inp+$crd))*100|floor) else 0 end)%"'
echo "(models without a rate in config/pricing.json show cost as '?'; token totals are exact)"
