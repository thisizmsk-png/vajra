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
echo "=== Done ==="
echo "  Installed: ${INSTALLED} skills"
echo "  Skipped: ${SKIPPED} (already existed)"
echo "  Total available: $(ls -d ${SKILLS_DIR}/*/ 2>/dev/null | wc -l) skills"
