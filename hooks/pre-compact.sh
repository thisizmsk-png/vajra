#!/usr/bin/env bash
# Vajra PreCompact hook — checkpoint + distillation before context compaction.
#
# Runs when Claude Code is about to compact the conversation. Compaction is
# lossy, so we (1) take a NON-DESTRUCTIVE git snapshot and (2) emit a checkpoint
# event, then (3) hand the model a distillation template so the summary keeps the
# load-bearing context (decisions, open threads, plan, key files) instead of
# whatever the generic summarizer would keep.
#
# Compaction + checkpoint must be atomic: a summary without a matching git_sha is
# dangerous (the agent "remembers" a plan but the tree moved). We snapshot first.

set -uo pipefail

VAJRA_DIR="${HOME}/.claude/vajra"
HMAC_KEY_FILE="${VAJRA_DIR}/.hmac-key"
COMPACT_DIR="${VAJRA_DIR}/compaction"
mkdir -p "$COMPACT_DIR"

PAYLOAD="$(cat 2>/dev/null || echo '{}')"
TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
SESSION="session"
if command -v jq &>/dev/null; then
  SESSION="$(echo "$PAYLOAD" | jq -r '.session_id // .sessionId // "session"' 2>/dev/null | tr -cd '[:alnum:]._-' | cut -c1-64)"
  [ -z "$SESSION" ] && SESSION="session"
fi

# 1. Non-destructive git snapshot (captures uncommitted changes as a dangling
#    commit without touching the working tree). Falls back to HEAD.
SNAP=""
if git rev-parse --is-inside-work-tree &>/dev/null; then
  SNAP="$(git stash create 2>/dev/null || true)"
  [ -z "$SNAP" ] && SNAP="$(git rev-parse --short HEAD 2>/dev/null || echo '')"
fi

# 2. Emit a checkpoint event into the session event log (HMAC-chained, like the
#    rest of the event stream).
if command -v jq &>/dev/null && command -v openssl &>/dev/null; then
  EV_LOG="${VAJRA_DIR}/events/${SESSION}.jsonl"
  mkdir -p "$(dirname "$EV_LOG")"
  prev="genesis"; seq=0
  if [ -f "$EV_LOG" ] && [ -s "$EV_LOG" ]; then
    prev="$(tail -n 1 "$EV_LOG" | jq -r '.hmac // "genesis"' 2>/dev/null || echo genesis)"
    seq="$(tail -n 1 "$EV_LOG" | jq -r '.seq // 0' 2>/dev/null || echo 0)"; seq=$((seq+1))
  fi
  entry="$(jq -nc --arg ts "$TS" --argjson seq "$seq" --arg gs "$SNAP" --arg prev "$prev" \
    '{seq:$seq, ts:$ts, tool:"__compaction__", kind:"checkpoint", outcome:"success", gitSha:$gs, prevHmac:$prev}')"
  if [ -f "$HMAC_KEY_FILE" ] && [ -s "$HMAC_KEY_FILE" ]; then
    key="$(cat "$HMAC_KEY_FILE")"
    h="$(printf '%s' "$entry" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${key}" -hex 2>/dev/null | awk '{print $NF}')"
    entry="$(echo "$entry" | jq -c --arg h "$h" '. + {hmac:$h}')"
  fi
  echo "$entry" >> "$EV_LOG"
fi

# 3. Record the checkpoint + leave the distillation template for the summary.
REC="${COMPACT_DIR}/${SESSION}-${TS//[:]/-}.md"
{
  echo "# Compaction checkpoint — ${SESSION}"
  echo "at: ${TS}"
  echo "git snapshot: ${SNAP:-<no repo>}   (restore: git stash apply ${SNAP})"
  echo
  echo "The summary that follows MUST preserve, in this structure:"
  echo "## Decisions — irreversible choices made + rationale"
  echo "## Open threads — unresolved bugs / TODO, with file:line"
  echo "## Plan — remaining steps, ordered"
  echo "## Key files — ≤5 most-recently-touched, one-line purpose each"
  echo "## Constraints / gotchas — learned this session"
} > "$REC"

# 4. Nudge the model (PreCompact hook stdout enters context).
echo "Vajra: checkpoint ${SNAP:-HEAD} saved before compaction. Distill the summary into Decisions / Open threads / Plan / Key files (≤5) / Constraints — see engine/compaction.md. Restore the tree with: git stash apply ${SNAP}"
exit 0
