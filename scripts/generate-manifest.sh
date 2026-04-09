#!/usr/bin/env bash
set -euo pipefail

# Generate SHA-256 manifest for Vajra supply chain integrity
VAJRA_SKILL="${HOME}/.claude/skills/vajra"
VAJRA_HOME="${HOME}/.claude/vajra"
MANIFEST="${VAJRA_SKILL}/manifest.json"
HMAC_KEY_FILE="${VAJRA_HOME}/.hmac-key"

echo "Generating manifest..."

# Find all files excluding .git, node_modules, manifest itself, graphify temp
FILES=$(find "${VAJRA_SKILL}" \
  -type f \
  ! -path "*/node_modules/*" \
  ! -path "*/.git/*" \
  ! -path "*/graphify-out/*" \
  ! -name "manifest.json" \
  ! -name "manifest.json.sig" \
  ! -name "*.db" \
  | sort)

# Use jq if available for safe JSON escaping, otherwise basic printf
if command -v jq &>/dev/null; then
  # Build manifest with jq for proper escaping
  ENTRIES="{"
  FIRST=true
  while IFS= read -r file; do
    HASH=$(sha256sum "$file" | cut -d' ' -f1)
    REL_PATH="${file#${VAJRA_SKILL}/}"
    if [ "$FIRST" = true ]; then
      FIRST=false
    else
      ENTRIES="${ENTRIES},"
    fi
    # jq -Rs safely escapes the key
    SAFE_KEY=$(printf '%s' "$REL_PATH" | jq -Rs '.')
    ENTRIES="${ENTRIES} ${SAFE_KEY}: \"${HASH}\""
  done <<< "$FILES"
  ENTRIES="${ENTRIES} }"

  jq -n \
    --arg gen "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg ver "0.2.0" \
    --argjson files "$ENTRIES" \
    '{generated: $gen, version: $ver, files: $files}' > "${MANIFEST}"
else
  # Fallback: basic printf (no special char support in filenames)
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
fi

echo "Manifest written to ${MANIFEST}"
echo "Files checksummed: $(echo "$FILES" | wc -l)"

# Sign the manifest with HMAC if key exists
if [ -f "$HMAC_KEY_FILE" ] && [ -s "$HMAC_KEY_FILE" ]; then
  MANIFEST_HASH=$(sha256sum "${MANIFEST}" | cut -d' ' -f1)
  KEY_HEX="$(cat "$HMAC_KEY_FILE")"
  MANIFEST_SIG=$(printf '%s' "$MANIFEST_HASH" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${KEY_HEX}" -hex 2>/dev/null | awk '{print $NF}')
  echo "$MANIFEST_SIG" > "${MANIFEST}.sig"
  echo "Manifest signed: ${MANIFEST}.sig"
fi
