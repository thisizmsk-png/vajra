#!/usr/bin/env bash
set -euo pipefail

# Vajra — Install Bundled Skills
# Copies 87 skills from bundled-skills/ to ~/.claude/skills/

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

  for agent_file in "${BUNDLED_AGENTS}"/*; do
    agent_name=$(basename "$agent_file")
    target="${AGENTS_DIR}/${agent_name}"

    if [ -e "$target" ]; then
      echo "  SKIP: ${agent_name} (already exists)"
      AGENT_SKIPPED=$((AGENT_SKIPPED + 1))
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
  echo "  Total available: $(ls ${AGENTS_DIR}/*.md 2>/dev/null | wc -l) agents"
fi
