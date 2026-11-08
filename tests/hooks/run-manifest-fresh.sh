#!/usr/bin/env bash
# The committed manifest.json must match the tree it ships with: install.sh verifies these
# hashes and refuses to sign when they differ. (The install suite regenerates its fixture
# manifest, so it cannot catch a stale committed one.) Read-only.
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO"
command -v jq >/dev/null || { echo "SKIP: jq missing"; exit 77; }
if command -v sha256sum >/dev/null 2>&1; then SHA=(sha256sum); else SHA=(shasum -a 256); fi
P=0; F=0
while IFS= read -r f; do
  case "$f" in app/*|manifest.json|manifest.json.sig|*/__pycache__/*|*.pyc|*.db|.DS_Store|*/.DS_Store) continue ;; esac
  [ -f "$f" ] && [ ! -L "$f" ] || continue
  want="$(jq -r --arg f "$f" '.files[$f] // "missing"' manifest.json)"
  have="$("${SHA[@]}" "$f" | cut -c1-64)"
  if [ "$want" = "$have" ]; then P=$((P+1)); else F=$((F+1)); echo "FAIL :: manifest.json stale for $f ($want)"; fi
done < <(git ls-files)
# Every manifest entry must be a shipped (tracked) file: session-start checks each exists.
while IFS= read -r f; do
  if git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then P=$((P+1)); else F=$((F+1)); echo "FAIL :: manifest.json lists $f, which is not shipped"; fi
done < <(jq -r '.files | keys[]' manifest.json)
echo "manifest fresh: $P passed, $F failed"
# Strict by default (local runs and pushes to main). CI sets VAJRA_MANIFEST_FRESH=warn for pull_request events only:
# feature PRs do not carry manifest.json, the operator regenerates main after merges, so a stale entry warns there.
if [ "$F" -gt 0 ] && [ "${VAJRA_MANIFEST_FRESH:-}" = "warn" ]; then
  echo "WARN: manifest.json is stale in $F place(s); not blocking this pull request (the operator regenerates it on main)"
  [ -z "${GITHUB_ACTIONS:-}" ] || echo "::warning::manifest.json is stale in $F place(s); the operator regenerates it on main after merge"
  exit 0
fi
[ "$F" -eq 0 ]
