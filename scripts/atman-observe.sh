#!/usr/bin/env bash
# atman-observe.sh — Scan practice log for failure patterns and flag skills for self-review
# Usage: bash scripts/atman-observe.sh
set -euo pipefail

VAJRA_DIR="${HOME}/.claude/vajra"
PRACTICE_LOG="${VAJRA_DIR}/practice-log.jsonl"
FLAGS_DIR="${VAJRA_DIR}/flags"

command -v jq &>/dev/null || { echo "ERROR: jq required"; exit 1; }
[ -f "$PRACTICE_LOG" ] || { echo "No practice log found. Nothing to observe."; exit 0; }

mkdir -p "$FLAGS_DIR"

WEEK_AGO="$(date -u -d '7 days ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -v-7d +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo "1970-01-01T00:00:00Z")"

echo "=== Atman Observer ==="
echo "Scanning practice log for failure patterns (last 7 days)..."
echo ""

# Get unique skills with failures in last 7 days
FAILED_SKILLS="$(grep '"outcome":"failure"' "$PRACTICE_LOG" 2>/dev/null | while IFS= read -r line; do
  ts="$(echo "$line" | jq -r '.ts // empty' 2>/dev/null)"
  if [[ "$ts" > "$WEEK_AGO" ]]; then
    echo "$line" | jq -r '.skill // empty' 2>/dev/null
  fi
done | sort | uniq -c | sort -rn)"

if [ -z "$FAILED_SKILLS" ]; then
  echo "No failure patterns detected. All skills performing well."
  exit 0
fi

FLAGGED=0
echo "Failure patterns detected:"
echo "$FAILED_SKILLS" | while IFS= read -r line; do
  COUNT="$(echo "$line" | awk '{print $1}')"
  SKILL="$(echo "$line" | awk '{print $2}')"
  [ -z "$SKILL" ] && continue

  if [ "$COUNT" -ge 2 ]; then
    echo "  [FLAG] ${SKILL}: ${COUNT} failures in 7 days — flagged for self-review"
    jq -nc \
      --arg skill "$SKILL" \
      --arg count "$COUNT" \
      --arg flagged "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      '{skill: $skill, failCount: ($count | tonumber), flagged: $flagged, status: "pending-review"}' \
      > "${FLAGS_DIR}/${SKILL}.json"
    FLAGGED=$((FLAGGED + 1))
  else
    echo "  [OK]   ${SKILL}: ${COUNT} failure (below threshold)"
  fi
done

echo ""

# --- Routing efficiency report ---
ROUTING_LOG="${VAJRA_DIR}/routing-log.jsonl"
if [ -f "$ROUTING_LOG" ]; then
  TOTAL="$(wc -l < "$ROUTING_LOG")"
  T4_COUNT="$(grep -c '"tier":"T4"' "$ROUTING_LOG" 2>/dev/null || echo 0)"
  FREE_PCT=0
  if [ "$TOTAL" -gt 0 ]; then
    FREE_PCT=$(( (TOTAL - T4_COUNT) * 100 / TOTAL ))
  fi
  echo "Routing efficiency: ${FREE_PCT}% free (T1-T3), ${T4_COUNT}/${TOTAL} via T4 (LLM)"

  # Check for muscle memory candidates (same skill routed via T4 ≥3 times)
  echo ""
  echo "Muscle memory candidates (T4 → T3 promotion):"
  grep '"tier":"T4"' "$ROUTING_LOG" 2>/dev/null | jq -r '.skill' 2>/dev/null | sort | uniq -c | sort -rn | while IFS= read -r line; do
    COUNT="$(echo "$line" | awk '{print $1}')"
    SKILL="$(echo "$line" | awk '{print $2}')"
    [ -z "$SKILL" ] && continue
    if [ "$COUNT" -ge 3 ]; then
      echo "  [PROPOSE] '${SKILL}' routed via T4 ${COUNT} times — propose T3 keyword"
    fi
  done
fi

echo ""
echo "Run '/vajra atman review' to act on flagged skills."
echo "Run '/vajra dream' to apply approved patches and routing proposals."
