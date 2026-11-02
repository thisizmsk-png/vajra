#!/usr/bin/env bash
set -euo pipefail

# Vajra — Install Bundled Skills
# Copies the skills in bundled-skills/ to ~/.claude/skills/. Third-party skills are
# not here; scripts/vajra_skills.py installs them from config/skills.json on demand.

UPDATE_AGENTS=0
for arg in "$@"; do
  case "$arg" in
    --update-agents) UPDATE_AGENTS=1 ;;
    *)
      echo "Usage: install-skills.sh [--update-agents]" >&2
      echo "  --update-agents  overwrite installed agents that differ from the bundle (old copy saved as .bak)" >&2
      exit 2
      ;;
  esac
done

VAJRA_SKILL="${HOME}/.claude/skills/vajra"
SKILLS_DIR="${HOME}/.claude/skills"
BUNDLED="${VAJRA_SKILL}/bundled-skills"

if [ ! -d "$BUNDLED" ]; then
  echo "ERROR: bundled-skills/ not found. Run from the vajra skill directory."
  exit 1
fi

echo "=== Vajra — Installing Bundled Skills ==="

INSTALLED=0
SKIPPED=0

for skill_dir in "${BUNDLED}"/*/; do
  skill_name=$(basename "$skill_dir")
  target="${SKILLS_DIR}/${skill_name}"

  if [ -d "$target" ]; then
    echo "  SKIP: ${skill_name} (already exists)"
    SKIPPED=$((SKIPPED + 1))
  else
    cp -r "$skill_dir" "$target"
    echo "  INSTALL: ${skill_name}"
    INSTALLED=$((INSTALLED + 1))
  fi
done

echo ""
echo "=== Skills Done ==="
echo "  Installed: ${INSTALLED} skills"
echo "  Skipped: ${SKIPPED} (already existed)"
echo "  Total available: $(ls -d ${SKILLS_DIR}/*/ 2>/dev/null | wc -l) skills"

# --- Install Bundled Agents (Cortex) ---
AGENTS_DIR="${HOME}/.claude/agents"
BUNDLED_AGENTS="${VAJRA_SKILL}/bundled-agents"

if [ -d "$BUNDLED_AGENTS" ]; then
  echo ""
  echo "=== Vajra — Installing Cortex Agents ==="
  mkdir -p "$AGENTS_DIR"

  AGENT_INSTALLED=0
  AGENT_SKIPPED=0
  AGENT_UPDATED=0
  AGENT_DIFFERS=0

  for agent_file in "${BUNDLED_AGENTS}"/*; do
    agent_name=$(basename "$agent_file")
    target="${AGENTS_DIR}/${agent_name}"

    if [ -e "$target" ]; then
      if [ -d "$agent_file" ]; then
        same=0; diff -rq "$agent_file" "$target" >/dev/null 2>&1 && same=1
      else
        same=0; cmp -s "$agent_file" "$target" && same=1
      fi
      if [ "$same" -eq 1 ]; then
        echo "  SKIP: ${agent_name} (already exists)"
        AGENT_SKIPPED=$((AGENT_SKIPPED + 1))
      elif [ "$UPDATE_AGENTS" -eq 1 ]; then
        if [ -d "$agent_file" ]; then
          rm -rf "${target}.bak"
          cp -r "$target" "${target}.bak"
          rm -rf "$target"
          cp -r "$agent_file" "$target"
        else
          cp -p "$target" "${target}.bak"
          cp "$agent_file" "$target"
        fi
        echo "  UPDATE: ${agent_name} (backup: ${agent_name}.bak)"
        AGENT_UPDATED=$((AGENT_UPDATED + 1))
      else
        echo "  SKIP: ${agent_name} (already exists)"
        AGENT_SKIPPED=$((AGENT_SKIPPED + 1))
        AGENT_DIFFERS=$((AGENT_DIFFERS + 1))
      fi
    else
      cp -r "$agent_file" "$target"
      echo "  INSTALL: ${agent_name}"
      AGENT_INSTALLED=$((AGENT_INSTALLED + 1))
    fi
  done

  echo ""
  echo "=== Agents Done ==="
  echo "  Installed: ${AGENT_INSTALLED} agents"
  echo "  Skipped: ${AGENT_SKIPPED} (already existed)"
  if [ "$UPDATE_AGENTS" -eq 1 ]; then
    echo "  Updated: ${AGENT_UPDATED} agents"
  elif [ "$AGENT_DIFFERS" -gt 0 ]; then
    echo "  Differs from bundled: ${AGENT_DIFFERS} agents (run with --update-agents to replace them; originals are saved as .bak)"
  else
    echo "  Differs from bundled: 0 agents"
  fi
  echo "  Total available: $(ls ${AGENTS_DIR}/*.md 2>/dev/null | wc -l) agents"
fi
