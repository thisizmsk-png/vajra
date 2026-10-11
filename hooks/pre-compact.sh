#!/usr/bin/env bash
# Vajra PreCompact hook: checkpoint before context compaction.
#
# Compaction is lossy, so before it runs we:
#   1. snapshot the working tree without touching it (`git stash create`, or
#      HEAD when the tree is clean) and PIN the snapshot with
#      `git update-ref refs/vajra/compact/<ts>`, so `git gc` cannot collect it;
#   2. append an HMAC-chained checkpoint event to the session event log;
#   3. write a compaction record with the restore command and the
#      distillation template the summary should keep.
#
# session-start.sh (source=compact) points the model at the record afterwards.
# Never blocks compaction: always exits 0.

set -uo pipefail

VAJRA_DIR="${HOME}/.claude/vajra"
HMAC_KEY_FILE="${VAJRA_DIR}/.hmac-key"
COMPACT_DIR="${VAJRA_DIR}/compaction"
mkdir -p "$COMPACT_DIR" 2>/dev/null || exit 0

PAYLOAD="$(cat 2>/dev/null || echo '{}')"
TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
REF_TS="$(date -u +%Y%m%dT%H%M%SZ)"
SESSION="session"
CWD="$PWD"
if command -v jq >/dev/null 2>&1; then
  SESSION="$(printf '%s' "$PAYLOAD" | jq -r '.session_id // "session"' 2>/dev/null | tr -cd '[:alnum:]._-' | cut -c1-64)"
  [ -n "$SESSION" ] || SESSION="session"
  c="$(printf '%s' "$PAYLOAD" | jq -r '.cwd // empty' 2>/dev/null)"
  [ -n "$c" ] && [ -d "$c" ] && CWD="$c"
fi

# 1. Snapshot + pin.
SNAP="" REF="" KIND=""
if git -C "$CWD" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  SNAP="$(git -C "$CWD" stash create 2>/dev/null || true)"
  KIND="stash"
  if [ -z "$SNAP" ]; then
    SNAP="$(git -C "$CWD" rev-parse HEAD 2>/dev/null || true)"
    KIND="head"
  fi
  if [ -n "$SNAP" ]; then
    REF="refs/vajra/compact/${REF_TS}-${SESSION}"
    git -C "$CWD" update-ref -m "vajra pre-compact ${TS}" "$REF" "$SNAP" 2>/dev/null || REF=""
  fi
fi

# 2. Checkpoint event (HMAC-chained like the rest of the event stream).
if command -v jq >/dev/null 2>&1 && command -v openssl >/dev/null 2>&1; then
  EV_LOG="${VAJRA_DIR}/events/${SESSION}.jsonl"
  mkdir -p "$(dirname "$EV_LOG")"
  prev="genesis"; seq=0
  if [ -s "$EV_LOG" ]; then
    prev="$(tail -n 1 "$EV_LOG" | jq -r '.hmac // "genesis"' 2>/dev/null || echo genesis)"
    seq="$(tail -n 1 "$EV_LOG" | jq -r '.seq // 0' 2>/dev/null || echo 0)"; seq=$((seq + 1))
  fi
  entry="$(jq -nc --arg ts "$TS" --argjson seq "$seq" --arg gs "$SNAP" --arg ref "$REF" --arg prev "$prev" \
    '{seq:$seq, ts:$ts, tool:"__compaction__", kind:"checkpoint", outcome:"success", gitSha:$gs, gitRef:$ref, prevHmac:$prev}')"
  if [ -s "$HMAC_KEY_FILE" ]; then
    key="$(tr -d '[:space:]' < "$HMAC_KEY_FILE")"
    h="$(printf '%s' "$entry" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${key}" -hex 2>/dev/null | awk '{print $NF}')"
    entry="$(printf '%s' "$entry" | jq -c --arg h "$h" '. + {hmac:$h}')"
  fi
  printf '%s\n' "$entry" >> "$EV_LOG"
fi

# 3. Compaction record.
if [ "$KIND" = "stash" ]; then
  RESTORE="git stash apply ${REF:-$SNAP}"
elif [ -n "$SNAP" ]; then
  RESTORE="tree was clean at ${SNAP}; nothing to restore (git checkout ${SNAP} to return)"
else
  RESTORE="no git repository"
fi
REC="${COMPACT_DIR}/${SESSION}-${TS//:/-}.md"
{
  echo "# Compaction checkpoint: ${SESSION}"
  echo "at: ${TS}"
  echo "cwd: ${CWD}"
  echo "git snapshot: ${SNAP:-<no repo>}"
  echo "git ref: ${REF:-<none>}"
  echo "restore: ${RESTORE}"
  echo
  echo "The summary that follows must keep, in this structure:"
  echo "## Decisions: irreversible choices made, with rationale"
  echo "## Open threads: unresolved bugs or TODOs, with file:line"
  echo "## Plan: remaining steps, in order"
  echo "## Key files: at most 5 recently touched, one-line purpose each"
  echo "## Constraints / gotchas learned this session"
} > "$REC"

echo "Vajra: checkpoint ${REF:-${SNAP:-none}} saved before compaction (${REC}). Distill the summary into Decisions / Open threads / Plan / Key files (max 5) / Constraints. Restore: ${RESTORE}"
exit 0
