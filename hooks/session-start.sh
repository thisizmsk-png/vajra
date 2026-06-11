#!/usr/bin/env bash
# Vajra SessionStart hook — resume campaigns + verify integrity of the
# security-critical surface against the SHA-256 manifest.
#
# Fixes red-team finding VJR-06: the previous check hashed only 3 doc files
# (SKILL.md, router.md, sanitizer.ts) and IGNORED every file that actually
# enforces security — the hooks, the engine, and config/default.json (which
# holds the sanitizer strip list and command policy). It also only warned.
#
# This version verifies the full enforcement set and, on a verified mismatch,
# writes a tamper flag that pre-tool-use.sh reads to block mutating operations
# until `/vajra verify` clears it. The manifest itself is write-protected from
# inside the harness by the pre-tool immutable-core rule.

set -uo pipefail

VAJRA_DIR="${HOME}/.claude/vajra"
VAJRA_DB="${VAJRA_DIR}/vajra.db"
SKILL_DIR="${HOME}/.claude/skills/vajra"
MANIFEST="${SKILL_DIR}/manifest.json"
TAMPER_FLAG="${VAJRA_DIR}/.integrity-violation"

mkdir -p "$VAJRA_DIR"

# --- Active campaign check (query SQLite directly) ---
if [ -f "$VAJRA_DB" ] && command -v sqlite3 &>/dev/null; then
  ACTIVE_CAMPAIGN="$(sqlite3 "$VAJRA_DB" "SELECT name FROM campaigns WHERE status='active' ORDER BY updated DESC LIMIT 1;" 2>/dev/null || echo "")"
  if [ -n "$ACTIVE_CAMPAIGN" ]; then
    echo "Active Vajra campaign: ${ACTIVE_CAMPAIGN}. Use /vajra continue to resume."
  fi
fi

# --- Portable SHA-256 helper (macOS: shasum; Linux: sha256sum) ---
sha256_of() {
  if command -v sha256sum &>/dev/null; then
    sha256sum "$1" 2>/dev/null | cut -d' ' -f1
  elif command -v shasum &>/dev/null; then
    shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1
  else
    return 1
  fi
}

# --- Manifest integrity check over the FULL enforcement set ---
if [ -f "$MANIFEST" ]; then
  # Validate manifest is parseable JSON
  if ! command -v jq &>/dev/null; then
    echo "Vajra integrity: jq not available — cannot verify manifest." >&2
  elif ! jq -e . "$MANIFEST" >/dev/null 2>&1; then
    echo "Vajra integrity WARNING: manifest.json is not valid JSON." >&2
  elif ! sha256_of "$MANIFEST" >/dev/null 2>&1; then
    echo "Vajra integrity: no SHA-256 tool (install coreutils/shasum) — cannot verify." >&2
  else
    # Every file whose modification changes security behavior. A mismatch on
    # ANY of these is treated as tampering (fail-closed via the tamper flag).
    CRITICAL_FILES=(
      "SKILL.md"
      "hooks/pre-tool-use.sh"
      "hooks/post-tool-use.sh"
      "hooks/session-start.sh"
      "engine/sanitizer.ts"
      "engine/engine.ts"
      "engine/router.md"
      "config/default.json"
      "config/security-contracts.json"
      "fleet/fleet-coordinator.sh"
      "redteam/snapshot.sh"
      "redteam/orchestrator.md"
      "redteam/scorer.md"
      "scripts/install.sh"
    )

    VIOLATIONS=""
    MISSING=""
    for rel_path in "${CRITICAL_FILES[@]}"; do
      full_path="${SKILL_DIR}/${rel_path}"
      expected_hash="$(jq -r ".files[\"${rel_path}\"] // empty" "$MANIFEST" 2>/dev/null)"
      if [ -z "$expected_hash" ]; then
        # Manifest is missing a hash for a file we consider critical.
        MISSING="${MISSING}${rel_path} "
        continue
      fi
      if [ ! -f "$full_path" ]; then
        VIOLATIONS="${VIOLATIONS}${rel_path}(deleted) "
        continue
      fi
      actual_hash="$(sha256_of "$full_path")"
      if [ "$actual_hash" != "$expected_hash" ]; then
        VIOLATIONS="${VIOLATIONS}${rel_path} "
      fi
    done

    if [ -n "$VIOLATIONS" ]; then
      {
        echo "{"
        echo "  \"detected\": \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\","
        echo "  \"files\": \"$(echo "$VIOLATIONS" | tr -d '\n')\""
        echo "}"
      } > "$TAMPER_FLAG"
      chmod 600 "$TAMPER_FLAG" 2>/dev/null || true
      echo "════════════════════════════════════════════════════════════" >&2
      echo "Vajra INTEGRITY VIOLATION — security-critical files modified:" >&2
      echo "  ${VIOLATIONS}" >&2
      echo "Mutating tool calls are now BLOCKED until you investigate." >&2
      echo "Run '/vajra verify' to review, then delete ${TAMPER_FLAG} to clear." >&2
      echo "════════════════════════════════════════════════════════════" >&2
    else
      # Clean — clear any stale tamper flag from a prior session.
      rm -f "$TAMPER_FLAG" 2>/dev/null || true
      if [ -n "$MISSING" ]; then
        echo "Vajra integrity: manifest missing hashes for: ${MISSING}(run scripts/generate-manifest.sh)" >&2
      fi
    fi
  fi
fi

exit 0
