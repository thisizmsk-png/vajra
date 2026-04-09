#!/usr/bin/env bash
# Vajra SessionStart hook — checks for active campaigns and verifies manifest integrity.
# Runs at the start of each Claude Code session.

set -euo pipefail

VAJRA_DIR="${HOME}/.claude/vajra"
VAJRA_DB="${VAJRA_DIR}/vajra.db"
SKILL_DIR="${HOME}/.claude/skills/vajra"
MANIFEST="${SKILL_DIR}/manifest.json"

# --- Active campaign check (query SQLite directly) ---

if [ -f "$VAJRA_DB" ] && command -v sqlite3 &>/dev/null; then
  ACTIVE_CAMPAIGN="$(sqlite3 "$VAJRA_DB" "SELECT name FROM campaigns WHERE status='active' ORDER BY updated DESC LIMIT 1;" 2>/dev/null || echo "")"
  if [ -n "$ACTIVE_CAMPAIGN" ]; then
    echo "Active Vajra campaign: ${ACTIVE_CAMPAIGN}. Use /vajra continue to resume."
  fi
fi

# --- Manifest integrity check (actually verify SHA-256 hashes) ---

if [ -f "$MANIFEST" ]; then
  INTEGRITY_OK=true

  # Verify manifest is valid JSON
  if ! python3 -c "import json,sys; json.load(open(sys.argv[1]))" "$MANIFEST" 2>/dev/null; then
    if ! node -e "JSON.parse(require('fs').readFileSync(process.argv[1],'utf8'))" "$MANIFEST" 2>/dev/null; then
      INTEGRITY_OK=false
      echo "Vajra integrity warning: manifest.json is not valid JSON." >&2
    fi
  fi

  # Spot-check 3 critical files against manifest hashes
  if [ "$INTEGRITY_OK" = true ] && command -v jq &>/dev/null; then
    KEY_FILES=("SKILL.md" "engine/router.md" "engine/sanitizer.ts")
    for rel_path in "${KEY_FILES[@]}"; do
      full_path="${SKILL_DIR}/${rel_path}"
      if [ -f "$full_path" ]; then
        expected_hash="$(jq -r ".files[\"${rel_path}\"] // empty" "$MANIFEST" 2>/dev/null)"
        if [ -n "$expected_hash" ]; then
          actual_hash="$(sha256sum "$full_path" | cut -d' ' -f1)"
          if [ "$actual_hash" != "$expected_hash" ]; then
            INTEGRITY_OK=false
            echo "Vajra integrity warning: ${rel_path} has been modified." >&2
          fi
        fi
      fi
    done
  fi

  if [ "$INTEGRITY_OK" = false ]; then
    echo "Vajra: Some integrity checks failed. Run /vajra verify to diagnose." >&2
  fi
fi

exit 0
