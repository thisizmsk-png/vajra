#!/usr/bin/env bash
set -euo pipefail

# Generate SHA-256 manifest for Vajra supply chain integrity
VAJRA_SKILL="${HOME}/.claude/skills/vajra"
MANIFEST="${VAJRA_SKILL}/manifest.json"

echo "Generating manifest..."

# Find all files excluding .git, node_modules, manifest itself
FILES=$(find "${VAJRA_SKILL}" \
  -type f \
  ! -path "*/node_modules/*" \
  ! -path "*/.git/*" \
  ! -name "manifest.json" \
  ! -name "*.db" \
  | sort)

echo "{" > "${MANIFEST}"
echo "  \"generated\": \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"," >> "${MANIFEST}"
echo "  \"version\": \"0.2.0\"," >> "${MANIFEST}"
echo "  \"files\": {" >> "${MANIFEST}"

FIRST=true
while IFS= read -r file; do
  HASH=$(sha256sum "$file" | cut -d' ' -f1)
  REL_PATH="${file#${VAJRA_SKILL}/}"
  if [ "$FIRST" = true ]; then
    FIRST=false
  else
    echo "," >> "${MANIFEST}"
  fi
  printf "    \"%s\": \"%s\"" "$REL_PATH" "$HASH" >> "${MANIFEST}"
done <<< "$FILES"

echo "" >> "${MANIFEST}"
echo "  }" >> "${MANIFEST}"
echo "}" >> "${MANIFEST}"

echo "Manifest written to ${MANIFEST}"
echo "Files checksummed: $(echo "$FILES" | wc -l)"
