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
      "engine/plan-mode.md"
      "engine/verification.md"
      "scripts/vajra-phase.sh"
      "scripts/vajra-verify-work.sh"
      "config/default.json"
      "config/security-contracts.json"
      "fleet/fleet-coordinator.sh"
      "redteam/snapshot.sh"
      "redteam/orchestrator.md"
      "redteam/scorer.md"
      "scripts/install.sh"
      "scripts/generate-manifest.sh"
      "scripts/atman-observe.sh"
      "scripts/atman-dream.sh"
      "config/steering.json"
      "config/sandbox.json"
      "scripts/enable-sandbox.sh"
      "SECURITY.md"
      "steering/review-pipeline.md"
      "steering/rules/principles.md"
      "steering/rules/general.md"
      "steering/rules/security.md"
      "steering/rules/testing.md"
      "steering/rules/lambda.md"
      "steering/rules/aws-cdk.md"
      "steering/rules/typescript.md"
      "steering/rules/python.md"
      "steering/rules/java.md"
    )

    VIOLATIONS=""
    MISSING=""

    # Verify the manifest's own HMAC signature first (NEW-04). The key shares
    # the harness trust boundary, so this is defense-in-depth — but the manifest
    # is ALSO write-protected from inside the harness by the immutable-core hook,
    # so tampering requires breaking both. A bad signature is itself a violation.
    MANIFEST_SIG="${MANIFEST}.sig"
    HMAC_KEY_FILE="${VAJRA_DIR}/.hmac-key"
    if [ -f "$MANIFEST_SIG" ] && [ -f "$HMAC_KEY_FILE" ] && [ -s "$HMAC_KEY_FILE" ] && command -v openssl &>/dev/null; then
      m_hash="$(sha256_of "$MANIFEST")"
      key_hex="$(cat "$HMAC_KEY_FILE")"
      expect_sig="$(tr -d '[:space:]' < "$MANIFEST_SIG")"
      actual_sig="$(printf '%s' "$m_hash" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${key_hex}" -hex 2>/dev/null | awk '{print $NF}')"
      if [ -n "$expect_sig" ] && [ "$actual_sig" != "$expect_sig" ]; then
        VIOLATIONS="${VIOLATIONS}manifest.json(bad-signature) "
      fi
    fi

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

# --- Sandbox recommendation (read-only nudge) ---
# The native OS sandbox is Vajra's enforced security boundary. If it isn't on,
# point the operator at the enable helper once per session.
SETTINGS_FILE="${HOME}/.claude/settings.json"
if command -v jq &>/dev/null; then
  SANDBOX_ON="false"
  [ -f "$SETTINGS_FILE" ] && SANDBOX_ON="$(jq -r '.sandbox.enabled // false' "$SETTINGS_FILE" 2>/dev/null || echo false)"
  if [ "$SANDBOX_ON" != "true" ]; then
    echo "Vajra: native OS sandbox is off — it's the enforced security boundary. Enable it: bash ~/.claude/skills/vajra/scripts/enable-sandbox.sh (or /sandbox)." >&2
  fi
fi

exit 0
