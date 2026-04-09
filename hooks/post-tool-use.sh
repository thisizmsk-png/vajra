#!/usr/bin/env bash
# Vajra PostToolUse hook — Atman practice observer
# Records every skill invocation to practice-log.jsonl for the self-improvement loop.
# Claude Code pipes JSON with { tool, input, output, error } on stdin.

set -euo pipefail

VAJRA_DIR="${HOME}/.claude/vajra"
PRACTICE_LOG="${VAJRA_DIR}/practice-log.jsonl"
HMAC_KEY_FILE="${VAJRA_DIR}/.hmac-key"
PHASE_FILE="${VAJRA_DIR}/.phase"
ROUTING_LOG="${VAJRA_DIR}/routing-log.jsonl"

mkdir -p "$VAJRA_DIR"

# Read the hook payload from stdin
PAYLOAD="$(cat)"

# Only process if jq is available (need structured JSON parsing)
command -v jq &>/dev/null || exit 0

TOOL_NAME="$(echo "$PAYLOAD" | jq -r '.tool // empty' 2>/dev/null || echo "")"
[ -z "$TOOL_NAME" ] && exit 0

# Skip internal tools — only log skill-level invocations
case "$TOOL_NAME" in
  Read|Write|Edit|Glob|Grep|Bash|Agent|NotebookEdit) exit 0 ;;
esac

TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
HAS_ERROR="$(echo "$PAYLOAD" | jq -r 'if .error then "true" else "false" end' 2>/dev/null || echo "false")"

# Determine outcome based on error presence
OUTCOME="success"
if [ "$HAS_ERROR" = "true" ]; then
  OUTCOME="failure"
fi

# Check if a Cortex agent is active
ACTIVE_AGENT=""
ACTIVE_AGENT_FILE="${VAJRA_DIR}/.active-agent"
if [ -f "$ACTIVE_AGENT_FILE" ]; then
  ACTIVE_AGENT="$(cat "$ACTIVE_AGENT_FILE" 2>/dev/null | tr -d '[:space:]')"
fi

# Build the practice log entry
ENTRY="$(jq -nc \
  --arg ts "$TIMESTAMP" \
  --arg skill "$TOOL_NAME" \
  --arg outcome "$OUTCOME" \
  --arg hasError "$HAS_ERROR" \
  --arg agent "$ACTIVE_AGENT" \
  '{ts: $ts, skill: $skill, outcome: $outcome, hasError: ($hasError == "true")} + (if $agent != "" then {agent: $agent} else {} end)')"

# HMAC sign the entry if key exists
if [ -f "$HMAC_KEY_FILE" ] && [ -s "$HMAC_KEY_FILE" ]; then
  KEY_HEX="$(cat "$HMAC_KEY_FILE")"
  HMAC="$(printf '%s' "$ENTRY" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${KEY_HEX}" -hex 2>/dev/null | awk '{print $NF}')"
  ENTRY="$(echo "$ENTRY" | jq -c --arg hmac "$HMAC" '. + {hmac: $hmac}')"
fi

# Append to practice log (append-only)
echo "$ENTRY" >> "$PRACTICE_LOG"

# --- Failure pattern detection ---
# Check if this skill has failed ≥2 times in the last 7 days
if [ "$OUTCOME" = "failure" ]; then
  WEEK_AGO="$(date -u -d '7 days ago' +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -v-7d +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo "")"
  if [ -n "$WEEK_AGO" ] && [ -f "$PRACTICE_LOG" ]; then
    FAIL_COUNT="$(grep "\"skill\":\"${TOOL_NAME}\"" "$PRACTICE_LOG" | grep '"outcome":"failure"' | while IFS= read -r line; do
      entry_ts="$(echo "$line" | jq -r '.ts // empty' 2>/dev/null)"
      if [[ "$entry_ts" > "$WEEK_AGO" ]]; then echo "1"; fi
    done | wc -l)"

    if [ "$FAIL_COUNT" -ge 2 ]; then
      # Flag skill for self-review
      mkdir -p "${VAJRA_DIR}/flags"
      echo "{\"skill\":\"${TOOL_NAME}\",\"failCount\":${FAIL_COUNT},\"flagged\":\"${TIMESTAMP}\",\"type\":\"skill\"}" > "${VAJRA_DIR}/flags/${TOOL_NAME}.json"
    fi

    # Also check agent-level failures if an agent was active
    if [ -n "$ACTIVE_AGENT" ]; then
      AGENT_FAIL_COUNT="$(grep "\"agent\":\"${ACTIVE_AGENT}\"" "$PRACTICE_LOG" | grep '"outcome":"failure"' | while IFS= read -r aline; do
        ats="$(echo "$aline" | jq -r '.ts // empty' 2>/dev/null)"
        if [[ "$ats" > "$WEEK_AGO" ]]; then echo "1"; fi
      done | wc -l)"

      if [ "$AGENT_FAIL_COUNT" -ge 2 ]; then
        mkdir -p "${VAJRA_DIR}/flags"
        echo "{\"agent\":\"${ACTIVE_AGENT}\",\"failCount\":${AGENT_FAIL_COUNT},\"flagged\":\"${TIMESTAMP}\",\"type\":\"agent\"}" > "${VAJRA_DIR}/flags/agent-${ACTIVE_AGENT}.json"
      fi
    fi
  fi
fi

# --- Routing log (persistent) ---
# Append routing decision for muscle memory learning
ROUTING_TIER="$(echo "$PAYLOAD" | jq -r '.routing_tier // empty' 2>/dev/null || echo "")"
if [ -n "$ROUTING_TIER" ]; then
  ROUTE_ENTRY="$(jq -nc \
    --arg ts "$TIMESTAMP" \
    --arg skill "$TOOL_NAME" \
    --arg tier "$ROUTING_TIER" \
    '{ts: $ts, skill: $skill, tier: $tier}')"
  echo "$ROUTE_ENTRY" >> "$ROUTING_LOG"
fi

exit 0
