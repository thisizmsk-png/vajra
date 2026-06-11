#!/usr/bin/env bash
# vajra-replay.sh — replay + integrity-verify a session's event log.
#
# The post-tool hook records every tool call to ~/.claude/vajra/events/<session>.jsonl
# as an HMAC-chained append-only stream. This reconstructs the causal trace and
# verifies the chain (each entry's prevHmac links to the prior entry, each HMAC
# is valid over its payload) so tampering/reordering is detectable.
#
#   vajra-replay.sh [session]     # default: the most-recently-modified log
#   vajra-replay.sh --list        # list available session logs

set -uo pipefail
command -v jq >/dev/null 2>&1 || { echo "jq required" >&2; exit 2; }

VAJRA_DIR="${HOME}/.claude/vajra"
EV_DIR="${VAJRA_DIR}/events"
KEY_FILE="${VAJRA_DIR}/.hmac-key"

if [ "${1:-}" = "--list" ]; then
  ls -1t "$EV_DIR"/*.jsonl 2>/dev/null | sed "s#.*/##; s#\.jsonl\$##" || echo "(no event logs)"
  exit 0
fi

RAW_SESSION="${1:-}"
if [ -z "$RAW_SESSION" ]; then
  LOG="$(ls -1t "$EV_DIR"/*.jsonl 2>/dev/null | head -1)"
else
  # Sanitize to a single safe path segment — no traversal into other dirs.
  SESSION="$(printf '%s' "$RAW_SESSION" | tr -cd '[:alnum:]._-' | sed 's/^[.-]*//' | cut -c1-64)"
  LOG="${EV_DIR}/${SESSION}.jsonl"
fi
[ -n "${LOG:-}" ] && [ -f "$LOG" ] || { echo "No event log found (${SESSION:-latest}). Try --list." >&2; exit 1; }

echo "Replay: ${LOG##*/}"
echo "---------------------------------------------------------------"

KEY=""; [ -f "$KEY_FILE" ] && KEY="$(cat "$KEY_FILE")"
prev="genesis"; n=0; bad=0; expected_seq=0
while IFS= read -r line; do
  [ -z "$line" ] && continue
  n=$((n+1))
  seq="$(printf '%s' "$line" | jq -r '.seq // "?"')"
  ts="$(printf '%s' "$line" | jq -r '.ts // "?"')"
  tool="$(printf '%s' "$line" | jq -r '.tool // "?"')"
  outcome="$(printf '%s' "$line" | jq -r '.outcome // "?"')"
  lp="$(printf '%s' "$line" | jq -r '.prevHmac // ""')"
  lh="$(printf '%s' "$line" | jq -r '.hmac // ""')"
  status="ok"
  # chain link
  [ "$lp" != "$prev" ] && { status="CHAIN-BREAK"; bad=$((bad+1)); }
  # sequence monotonic
  [ "$seq" != "$expected_seq" ] && status="${status}/SEQ-GAP"
  # hmac validity
  if [ -n "$KEY" ] && [ -n "$lh" ]; then
    payload="$(printf '%s' "$line" | jq -c 'del(.hmac)')"
    actual="$(printf '%s' "$payload" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${KEY}" -hex 2>/dev/null | awk '{print $NF}')"
    [ "$actual" != "$lh" ] && { status="${status}/HMAC-BAD"; bad=$((bad+1)); }
  fi
  marker=" "; [ "$outcome" = "failure" ] && marker="✗"
  printf '  %3s  %s  %-14s %s  [%s]\n' "$seq" "$ts" "$tool" "$marker" "$status"
  prev="$lh"; expected_seq=$((seq+1))
done < "$LOG"

echo "---------------------------------------------------------------"
if [ "$bad" -eq 0 ]; then
  echo "${n} events — chain INTACT, all HMACs valid."
  exit 0
else
  echo "${n} events — ${bad} integrity problem(s) detected (tampering/reorder)."
  exit 1
fi
