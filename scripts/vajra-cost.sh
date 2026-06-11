#!/usr/bin/env bash
# vajra-cost.sh — per-session cost + token ledger from the Claude Code transcript.
#
# Folds the assistant-message `usage` blocks in a session transcript into a
# per-model ledger: tokens (input/output/cache-write/cache-read), USD cost where
# the model's rate is known (config/pricing.json), and the cache HIT RATE — the
# most actionable efficiency KPI (cache reads are 0.1× input; a high dollar total
# at a low hit rate means the prompt prefix is thrashing).
#
#   vajra-cost.sh [transcript.jsonl]   # default: most-recent transcript on disk
#   vajra-cost.sh --json [file]        # machine-readable ledger
#
# Token totals are always accurate; cost is shown only for priced models.

set -uo pipefail
command -v jq >/dev/null 2>&1 || { echo "jq required" >&2; exit 2; }

SKILL_DIR="${HOME}/.claude/skills/vajra"
PRICING="${SKILL_DIR}/config/pricing.json"

JSON_OUT=false
[ "${1:-}" = "--json" ] && { JSON_OUT=true; shift; }

TRANSCRIPT="${1:-}"
if [ -z "$TRANSCRIPT" ]; then
  TRANSCRIPT="$(ls -1t "${HOME}/.claude/projects/"*/*.jsonl 2>/dev/null | head -1)"
fi
[ -n "${TRANSCRIPT:-}" ] && [ -f "$TRANSCRIPT" ] || { echo "No transcript found. Pass one as an argument." >&2; exit 1; }

# Fold usage blocks → per-model totals. Handles both {message:{usage,model}} and
# top-level {usage,model} shapes; ignores lines without usage.
LEDGER="$(jq -s --slurpfile p "$PRICING" '
  ($p[0]) as $pr
  | (map(
      (.message // .) as $m
      | ($m.usage // empty) as $u
      | select($u != null)
      | {
          model: ($m.model // "unknown"),
          input: ($u.input_tokens // 0),
          output: ($u.output_tokens // 0),
          cw: ($u.cache_creation_input_tokens // 0),
          cr: ($u.cache_read_input_tokens // 0)
        }
    )) as $rows
  | ($rows | group_by(.model) | map({
      model: .[0].model,
      input: (map(.input) | add),
      output: (map(.output) | add),
      cw: (map(.cw) | add),
      cr: (map(.cr) | add),
      calls: length
    })) as $byModel
  | $byModel
  | map(
      . as $row
      | ($pr.models | to_entries | map(select(.value.match as $m | ($row.model | contains($m)))) | .[0].value) as $rate
      | . + {
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
' "$TRANSCRIPT")"

if [ "$JSON_OUT" = true ]; then
  echo "$LEDGER" | jq --arg t "$TRANSCRIPT" '{transcript:$t, byModel:., totals:{
    tokens: (map(.input+.output+.cw+.cr) | add // 0),
    costUsd: (map(.costUsd // 0) | add),
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
  | (map(.costUsd // 0) | add) as $cost
  | (map(.cr)|add // 0) as $crd | (map(.input)|add // 0) as $inp
  | "total: \($tok) tokens · $\(($cost*10000|floor)/10000) (priced models) · cache hit \(if ($inp+$crd)>0 then (($crd/($inp+$crd))*100|floor) else 0 end)%"'
echo "(models without a rate in config/pricing.json show cost as '?' — token totals are exact)"
