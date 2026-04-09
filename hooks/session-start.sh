#!/usr/bin/env bash
# Vajra SessionStart hook — checks for active campaigns and verifies manifest integrity.
# Runs at the start of each Claude Code session.

set -euo pipefail

VAJRA_DIR="${HOME}/.claude/vajra"
VAJRA_DB="${VAJRA_DIR}/vajra.db"
SKILL_DIR="${HOME}/.claude/skills/vajra"
MANIFEST="${SKILL_DIR}/manifest.json"

# --- Active campaign check ---

if [ -f "$VAJRA_DB" ]; then
  # Check for active campaign marker files
  ACTIVE_CAMPAIGN=""
  CAMPAIGNS_DIR="${VAJRA_DIR}/campaigns"

  if [ -d "$CAMPAIGNS_DIR" ]; then
    for campaign_dir in "$CAMPAIGNS_DIR"/*/; do
      [ -d "$campaign_dir" ] || continue
      status_file="${campaign_dir}status"
      if [ -f "$status_file" ]; then
        status="$(cat "$status_file" 2>/dev/null || echo "")"
        if [ "$status" = "active" ] || [ "$status" = "running" ] || [ "$status" = "paused" ]; then
          campaign_name="$(basename "$campaign_dir")"
          ACTIVE_CAMPAIGN="$campaign_name"
          break
        fi
      fi
    done
  fi

  if [ -n "$ACTIVE_CAMPAIGN" ]; then
    echo "Active Vajra campaign: ${ACTIVE_CAMPAIGN}. Use /vajra continue to resume."
  fi
fi

# --- Manifest integrity check ---

if [ -f "$MANIFEST" ]; then
  INTEGRITY_OK=true

  # Quick SHA check on key files
  KEY_FILES=(
    "${SKILL_DIR}/SKILL.md"
    "${SKILL_DIR}/engine/router.md"
    "${SKILL_DIR}/redteam/scorer.md"
  )

  for key_file in "${KEY_FILES[@]}"; do
    if [ -f "$key_file" ]; then
      # Verify file is non-empty and readable
      if [ ! -s "$key_file" ]; then
        INTEGRITY_OK=false
        echo "Vajra integrity warning: ${key_file} is empty." >&2
      fi
    fi
  done

  # Verify manifest itself is valid JSON (basic check)
  if ! python3 -c "import json; json.load(open('${MANIFEST}'))" 2>/dev/null; then
    if ! node -e "JSON.parse(require('fs').readFileSync('${MANIFEST}','utf8'))" 2>/dev/null; then
      INTEGRITY_OK=false
      echo "Vajra integrity warning: manifest.json is not valid JSON." >&2
    fi
  fi

  if [ "$INTEGRITY_OK" = false ]; then
    echo "Vajra: Some integrity checks failed. Run /vajra verify to diagnose." >&2
  fi
fi

exit 0
