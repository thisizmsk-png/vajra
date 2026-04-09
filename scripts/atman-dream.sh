#!/usr/bin/env bash
# atman-dream.sh — Dream consolidation + Atman self-improvement cycle
# Called by /vajra dream. Processes: memory consolidation, patch review, routing learning.
# Usage: bash scripts/atman-dream.sh
set -euo pipefail

VAJRA_DIR="${HOME}/.claude/vajra"
PRACTICE_LOG="${VAJRA_DIR}/practice-log.jsonl"
FLAGS_DIR="${VAJRA_DIR}/flags"
PATCHES_DIR="${VAJRA_DIR}/patches"
ROUTING_LOG="${VAJRA_DIR}/routing-log.jsonl"
ROUTING_PROPOSALS="${VAJRA_DIR}/routing-proposals.json"
EVOLVED_DIR="${VAJRA_DIR}/evolved-skills"
BACKUPS_DIR="${VAJRA_DIR}/backups"
CONFIG_FILE="${HOME}/.claude/skills/vajra/config/default.json"
SKILL_DIR="${HOME}/.claude/skills"

command -v jq &>/dev/null || { echo "ERROR: jq required for dream consolidation"; exit 1; }

mkdir -p "$PATCHES_DIR" "$EVOLVED_DIR" "$BACKUPS_DIR" "$FLAGS_DIR"

echo "============================================"
echo "  Vajra Dream Consolidation"
echo "  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "============================================"
echo ""

PATCH_COUNT=0
MAX_PATCHES=5

# ─── Phase 1: Scan for flagged skills ────────────────────────────────
echo "Phase 1: Scanning for flagged skills..."

for flag_file in "${FLAGS_DIR}"/*.json; do
  [ -f "$flag_file" ] || continue
  [ "$PATCH_COUNT" -ge "$MAX_PATCHES" ] && { echo "  Max patches (${MAX_PATCHES}) reached. Stopping."; break; }

  SKILL="$(jq -r '.skill // empty' "$flag_file" 2>/dev/null)"
  FAIL_COUNT="$(jq -r '.failCount // 0' "$flag_file" 2>/dev/null)"
  [ -z "$SKILL" ] && continue

  echo "  [FLAGGED] ${SKILL} — ${FAIL_COUNT} failures"

  # Check if a patch already exists for this skill
  if [ -f "${PATCHES_DIR}/${SKILL}.patch.md" ]; then
    echo "    Patch already pending. Skipping self-review."
    continue
  fi

  # Extract failure traces for this skill
  TRACES="$(grep "\"skill\":\"${SKILL}\"" "$PRACTICE_LOG" 2>/dev/null | grep '"outcome":"failure"' | tail -5)"
  TRACE_COUNT="$(echo "$TRACES" | grep -c '.' 2>/dev/null || echo 0)"

  if [ "$TRACE_COUNT" -lt 2 ]; then
    echo "    Not enough failure traces (${TRACE_COUNT} < 2). Skipping."
    continue
  fi

  # Find the skill's SKILL.md
  SKILL_FILE=""
  for candidate in "${SKILL_DIR}/${SKILL}/SKILL.md" "${SKILL_DIR}/vajra/bundled-skills/${SKILL}/SKILL.md"; do
    if [ -f "$candidate" ]; then
      SKILL_FILE="$candidate"
      break
    fi
  done

  if [ -z "$SKILL_FILE" ]; then
    echo "    Could not find SKILL.md for '${SKILL}'. Skipping."
    continue
  fi

  echo "    Generating self-review patch..."
  PATCH_COUNT=$((PATCH_COUNT + 1))

  # Write patch proposal
  cat > "${PATCHES_DIR}/${SKILL}.patch.md" <<PATCH
---
patch_id: p-$(date +%s)
target_skill: ${SKILL}
trigger_traces: ${TRACE_COUNT} failures in last 7 days
failure_count: ${FAIL_COUNT}
created: $(date -u +%Y-%m-%dT%H:%M:%SZ)
status: pending-peer-review
---

## Failure Traces
$(echo "$TRACES" | head -5)

## Analysis
This skill has failed ${FAIL_COUNT} times on similar tasks in the past 7 days.
A manual review of the skill's SKILL.md at ${SKILL_FILE} is needed to identify
the gap between instructions and actual behavior.

## Recommended Action
Run: /vajra atman review
This will show this patch for manual inspection and approval.
PATCH

  echo "    Patch written: ${PATCHES_DIR}/${SKILL}.patch.md"
done

echo ""

# ─── Phase 1b: Scan for flagged AGENTS ───────────────────────────────
echo "Phase 1b: Scanning for flagged agents..."

AGENT_PATCHES_DIR="${VAJRA_DIR}/patches/agents"
mkdir -p "$AGENT_PATCHES_DIR"

for flag_file in "${FLAGS_DIR}"/agent-*.json; do
  [ -f "$flag_file" ] || continue
  [ "$PATCH_COUNT" -ge "$MAX_PATCHES" ] && { echo "  Max patches (${MAX_PATCHES}) reached."; break; }

  AGENT_NAME="$(jq -r '.agent // empty' "$flag_file" 2>/dev/null)"
  FAIL_COUNT="$(jq -r '.failCount // 0' "$flag_file" 2>/dev/null)"
  [ -z "$AGENT_NAME" ] && continue

  echo "  [FLAGGED AGENT] ${AGENT_NAME} — ${FAIL_COUNT} failures"

  if [ -f "${AGENT_PATCHES_DIR}/${AGENT_NAME}.patch.md" ]; then
    echo "    Agent patch already pending. Skipping."
    continue
  fi

  # Extract failure traces for this agent
  TRACES="$(grep "\"agent\":\"${AGENT_NAME}\"" "$PRACTICE_LOG" 2>/dev/null | grep '"outcome":"failure"' | tail -5)"
  TRACE_COUNT="$(echo "$TRACES" | grep -c '.' 2>/dev/null || echo 0)"

  if [ "$TRACE_COUNT" -lt 2 ]; then
    echo "    Not enough failure traces (${TRACE_COUNT} < 2). Skipping."
    continue
  fi

  # Find the agent's persona file
  AGENT_FILE=""
  for candidate in "${HOME}/.claude/agents/${AGENT_NAME}.md" "${SKILL_DIR}/vajra/bundled-agents/${AGENT_NAME}.md"; do
    if [ -f "$candidate" ]; then
      AGENT_FILE="$candidate"
      break
    fi
  done

  if [ -z "$AGENT_FILE" ]; then
    echo "    Could not find persona for '${AGENT_NAME}'. Skipping."
    continue
  fi

  echo "    Generating agent self-review patch..."
  PATCH_COUNT=$((PATCH_COUNT + 1))

  cat > "${AGENT_PATCHES_DIR}/${AGENT_NAME}.patch.md" <<PATCH
---
patch_id: ap-$(date +%s)
target_agent: ${AGENT_NAME}
target_file: ${AGENT_FILE}
trigger_traces: ${TRACE_COUNT} failures in last 7 days
failure_count: ${FAIL_COUNT}
created: $(date -u +%Y-%m-%dT%H:%M:%SZ)
status: pending-peer-review
type: agent
---

## Failure Traces
$(echo "$TRACES" | head -5)

## Analysis
Agent ${AGENT_NAME} has failed ${FAIL_COUNT} times in the past 7 days.
Review the persona at ${AGENT_FILE} for:
- Missing escalation paths
- Incorrect tool restrictions
- Domain knowledge gaps
- Decision boundary issues

## Recommended Action
Run: /vajra atman review
PATCH

  echo "    Agent patch written: ${AGENT_PATCHES_DIR}/${AGENT_NAME}.patch.md"
done

echo ""

# ─── Phase 2: Peer review pending patches ─────────────────────────────
echo "Phase 2: Checking for patches pending peer review..."

PENDING_PATCHES=0
for patch_file in "${PATCHES_DIR}"/*.patch.md; do
  [ -f "$patch_file" ] || continue
  STATUS="$(grep '^status:' "$patch_file" | head -1 | sed 's/status: *//')"
  if [ "$STATUS" = "pending-peer-review" ] || [ "$STATUS" = "pending" ]; then
    SKILL_NAME="$(basename "$patch_file" .patch.md)"
    echo "  [PENDING] ${SKILL_NAME} — needs review via /vajra atman review"
    PENDING_PATCHES=$((PENDING_PATCHES + 1))
  fi
done

# Also check agent patches
for patch_file in "${AGENT_PATCHES_DIR}"/*.patch.md; do
  [ -f "$patch_file" ] || continue
  STATUS="$(grep '^status:' "$patch_file" | head -1 | sed 's/status: *//')"
  if [ "$STATUS" = "pending-peer-review" ] || [ "$STATUS" = "pending" ]; then
    AGENT_NAME="$(basename "$patch_file" .patch.md)"
    echo "  [PENDING AGENT] ${AGENT_NAME} — needs review via /vajra atman review"
    PENDING_PATCHES=$((PENDING_PATCHES + 1))
  fi
done

if [ "$PENDING_PATCHES" -eq 0 ]; then
  echo "  No patches pending review (skills or agents)."
fi

echo ""

# ─── Phase 3: Muscle memory — promote routing shortcuts ───────────────
echo "Phase 3: Routing muscle memory..."

if [ -f "$ROUTING_LOG" ]; then
  PROPOSALS="[]"
  PROPOSALS_FILE="$ROUTING_PROPOSALS"

  # Load existing proposals
  if [ -f "$PROPOSALS_FILE" ]; then
    PROPOSALS="$(jq '.proposals // []' "$PROPOSALS_FILE" 2>/dev/null || echo "[]")"
  fi

  # Find T4 routings that repeat ≥3 times for the same skill
  T4_CANDIDATES="$(grep '"tier":"T4"' "$ROUTING_LOG" 2>/dev/null | jq -r '.skill' 2>/dev/null | sort | uniq -c | sort -rn)"

  NEW_PROPOSALS=0
  echo "$T4_CANDIDATES" | while IFS= read -r line; do
    COUNT="$(echo "$line" | awk '{print $1}')"
    SKILL="$(echo "$line" | awk '{print $2}')"
    [ -z "$SKILL" ] && continue

    if [ "$COUNT" -ge 3 ]; then
      # Check if already proposed or already in T3 keywords
      ALREADY_EXISTS="$(echo "$PROPOSALS" | jq --arg s "$SKILL" '[.[] | select(.skill == $s)] | length' 2>/dev/null || echo 0)"
      IN_CONFIG="$(jq --arg s "$SKILL" '.routing.tier3Keywords | to_entries[] | select(.value == $s) | .key' "$CONFIG_FILE" 2>/dev/null || echo "")"

      if [ "$ALREADY_EXISTS" = "0" ] && [ -z "$IN_CONFIG" ]; then
        echo "  [AUTO-LEARN] '${SKILL}' → T3 keyword (routed via T4 ${COUNT} times)"
        # Auto-learn T3 keywords (low risk)
        jq --arg s "$SKILL" --arg k "$SKILL" \
          '.routing.tier3Keywords[$k] = $s' "$CONFIG_FILE" > "${CONFIG_FILE}.tmp" \
          && mv "${CONFIG_FILE}.tmp" "$CONFIG_FILE"
        NEW_PROPOSALS=$((NEW_PROPOSALS + 1))
      fi
    fi
  done

  if [ "$NEW_PROPOSALS" -eq 0 ]; then
    echo "  No new routing shortcuts to learn."
  fi
else
  echo "  No routing log found. Routing muscle memory inactive."
fi

echo ""

# ─── Phase 4: Practice log maintenance ────────────────────────────────
echo "Phase 4: Practice log maintenance..."

if [ -f "$PRACTICE_LOG" ]; then
  LINE_COUNT="$(wc -l < "$PRACTICE_LOG")"
  echo "  Practice log: ${LINE_COUNT} entries"

  if [ "$LINE_COUNT" -gt 10000 ]; then
    echo "  Pruning to 10,000 entries (keeping all failures)..."
    # Keep all failures + most recent 10000 entries
    FAILURES="$(grep '"outcome":"failure"' "$PRACTICE_LOG")"
    RECENT="$(tail -10000 "$PRACTICE_LOG")"
    {
      echo "$FAILURES"
      echo "$RECENT"
    } | sort -u > "${PRACTICE_LOG}.tmp"
    mv "${PRACTICE_LOG}.tmp" "$PRACTICE_LOG"
    NEW_COUNT="$(wc -l < "$PRACTICE_LOG")"
    echo "  Pruned: ${LINE_COUNT} → ${NEW_COUNT} entries"
  fi
fi

# Clean processed flags
for flag_file in "${FLAGS_DIR}"/*.json; do
  [ -f "$flag_file" ] || continue
  SKILL="$(jq -r '.skill // empty' "$flag_file" 2>/dev/null)"
  if [ -f "${PATCHES_DIR}/${SKILL}.patch.md" ]; then
    rm -f "$flag_file"
  fi
done

echo ""
echo "============================================"
echo "  Dream complete."
echo "  Patches pending: ${PENDING_PATCHES}"
echo "  Run '/vajra atman review' to approve/reject patches."
echo "  Run '/vajra atman status' for full stats."
echo "============================================"
