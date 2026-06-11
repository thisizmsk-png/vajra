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

TOOL_NAME="$(echo "$PAYLOAD" | jq -r '.tool // .tool_name // empty' 2>/dev/null || echo "")"
[ -z "$TOOL_NAME" ] && exit 0

# --- Event log: causal trace of EVERY tool call, HMAC-chained ---------------
# Append-only event stream for replay/debugging (OpenHands-style). Captures ALL
# tools, including the internal ones the practice log below skips. We store a
# hash of the args (what happened) rather than the raw args (size/secrets), and
# chain each entry to the previous so deletion/reorder is detectable.
sha256_hex() {
  if command -v sha256sum &>/dev/null; then printf '%s' "$1" | sha256sum | cut -d' ' -f1
  elif command -v shasum &>/dev/null; then printf '%s' "$1" | shasum -a 256 | cut -d' ' -f1
  else printf '%s' "$1" | openssl dgst -sha256 2>/dev/null | awk '{print $NF}'; fi
}
EV_TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
SESSION="$(echo "$PAYLOAD" | jq -r '.session_id // .sessionId // "session"' 2>/dev/null | tr -cd '[:alnum:]._-' | cut -c1-64)"
[ -z "$SESSION" ] && SESSION="session"
EV_DIR="${VAJRA_DIR}/events"
mkdir -p "$EV_DIR"
EV_LOG="${EV_DIR}/${SESSION}.jsonl"
EV_OUTCOME="success"
[ "$(echo "$PAYLOAD" | jq -r 'if .error then "true" else "false" end' 2>/dev/null)" = "true" ] && EV_OUTCOME="failure"
EV_INPUT="$(echo "$PAYLOAD" | jq -c '.input // .tool_input // {}' 2>/dev/null || echo '{}')"
EV_ARGS_HASH="$(sha256_hex "$EV_INPUT")"
# Repo HEAD at emit time — turns the log into a CAUSAL trace: "which tool call,
# against which tree state, produced this regression" becomes a join.
EV_GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo '')"
EV_SEQ=0; EV_PREV="genesis"
if [ -f "$EV_LOG" ] && [ -s "$EV_LOG" ]; then
  EV_PREV="$(tail -n 1 "$EV_LOG" | jq -r '.hmac // "genesis"' 2>/dev/null || echo genesis)"
  EV_SEQ="$(tail -n 1 "$EV_LOG" | jq -r '.seq // 0' 2>/dev/null || echo 0)"; EV_SEQ=$((EV_SEQ+1))
fi
EV_ENTRY="$(jq -nc --arg ts "$EV_TS" --arg tool "$TOOL_NAME" --arg outcome "$EV_OUTCOME" \
  --argjson seq "$EV_SEQ" --arg ah "$EV_ARGS_HASH" --arg prev "$EV_PREV" --arg gs "$EV_GIT_SHA" \
  '{seq:$seq, ts:$ts, tool:$tool, outcome:$outcome, argsHash:$ah, gitSha:$gs, prevHmac:$prev}')"
if [ -f "$HMAC_KEY_FILE" ] && [ -s "$HMAC_KEY_FILE" ]; then
  EV_KEY="$(cat "$HMAC_KEY_FILE")"
  EV_HMAC="$(printf '%s' "$EV_ENTRY" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${EV_KEY}" -hex 2>/dev/null | awk '{print $NF}')"
  EV_ENTRY="$(echo "$EV_ENTRY" | jq -c --arg h "$EV_HMAC" '. + {hmac:$h}')"
fi
echo "$EV_ENTRY" >> "$EV_LOG"

# Skip internal tools for the practice log — only skill-level invocations
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

# Hash-chain the practice log (VJR-08): each entry's HMAC covers the previous
# entry's HMAC, so selective deletion, truncation, or reordering breaks the
# chain and is detectable at verification time. NOTE: the signing key lives in
# the same trust boundary as the logger, so this is tamper-EVIDENCE against
# accidental/external modification and selective edits — not a guarantee
# against an attacker who can read the key and re-chain the whole file. The
# pre-tool hook blocks agent reads of .hmac-key to keep that bar as high as the
# single-key model allows.
PREV_HMAC="genesis"
if [ -f "$PRACTICE_LOG" ] && [ -s "$PRACTICE_LOG" ]; then
  PREV_HMAC="$(tail -n 1 "$PRACTICE_LOG" | jq -r '.hmac // "genesis"' 2>/dev/null || echo "genesis")"
fi

# Build the practice log entry (prevHmac is part of the signed payload)
ENTRY="$(jq -nc \
  --arg ts "$TIMESTAMP" \
  --arg skill "$TOOL_NAME" \
  --arg outcome "$OUTCOME" \
  --arg hasError "$HAS_ERROR" \
  --arg agent "$ACTIVE_AGENT" \
  --arg prev "$PREV_HMAC" \
  '{ts: $ts, skill: $skill, outcome: $outcome, hasError: ($hasError == "true"), prevHmac: $prev} + (if $agent != "" then {agent: $agent} else {} end)')"

# HMAC sign the entry (covering prevHmac) if key exists
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
