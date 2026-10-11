#!/usr/bin/env bash
# Rebuild manifest.json (SHA-256 of every shipped file) and sign it locally.
#
#   bash scripts/generate-manifest.sh
#
# manifest.json is committed and shipped. The signature is NOT shipped: it is
# written to ~/.claude/vajra/manifest.sig with this machine's HMAC key, so each
# install trusts its own key (install.sh signs after verifying the hashes).
#
# File list: `git ls-files` (tracked + untracked, minus .gitignore) when the
# skill is a git checkout, else find. Run by the operator, never by the agent
# (pre-tool-use.sh blocks it).

set -euo pipefail

VAJRA_SKILL="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VAJRA_HOME="${HOME}/.claude/vajra"
MANIFEST="${VAJRA_SKILL}/manifest.json"
KEY_FILE="${VAJRA_HOME}/.hmac-key"
SIG_FILE="${VAJRA_HOME}/manifest.sig"

command -v jq >/dev/null 2>&1 || { echo "ERROR: jq required" >&2; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then SHA=(sha256sum); else SHA=(shasum -a 256); fi

cd "$VAJRA_SKILL"

list_files() {
  if git -C "$VAJRA_SKILL" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git ls-files -co --exclude-standard
  else
    find . -type f ! -path './.git/*' ! -path './node_modules/*' | sed 's#^\./##'
  fi
}

keep_files() { # stdin paths -> shipped regular files
  local f
  while IFS= read -r f; do
    case "$f" in
      manifest.json|manifest.json.sig|.git/*|node_modules/*|graphify-out/*|*/__pycache__/*|__pycache__/*|*.pyc|*.db|.DS_Store|*/.DS_Store) continue ;;
    esac
    if [ -f "$f" ] && [ ! -L "$f" ]; then printf '%s\n' "$f"; fi
  done
}

FILES="$(list_files | LC_ALL=C sort -u | keep_files)"

[ -n "$FILES" ] || { echo "ERROR: no files found" >&2; exit 1; }

printf '%s\n' "$FILES" | tr '\n' '\0' | xargs -0 "${SHA[@]}" \
  | jq -R -n --arg gen "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg ver "0.5.0" '
      [inputs | capture("^(?<h>[0-9a-f]{64}) [ *](?<p>.+)$")]
      | {generated: $gen, version: $ver, files: (map({(.p): .h}) | add)}' > "${MANIFEST}.tmp"
mv "${MANIFEST}.tmp" "$MANIFEST"
echo "Manifest written: ${MANIFEST} ($(jq '.files | length' "$MANIFEST") files)"

if [ -s "$KEY_FILE" ] && command -v openssl >/dev/null 2>&1; then
  mkdir -p "$VAJRA_HOME"
  ( umask 077
    "${SHA[@]}" "$MANIFEST" | cut -d' ' -f1 | tr -d '\n' \
      | openssl dgst -sha256 -mac HMAC -macopt "hexkey:$(tr -d '[:space:]' < "$KEY_FILE")" -hex \
      | awk '{print $NF}' > "$SIG_FILE" )
  rm -f "${VAJRA_HOME}/.integrity-violation"
  echo "Signed locally: ${SIG_FILE}"
else
  echo "No HMAC key yet; run scripts/install.sh to sign the manifest on this machine."
fi
