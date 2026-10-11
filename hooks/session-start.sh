#!/usr/bin/env bash
# Vajra SessionStart hook.
#
#   1. Integrity: verify the manifest signature (~/.claude/vajra/manifest.sig,
#      written on this machine by install.sh with the user's own HMAC key),
#      then every file hash in manifest.json, and look for unlisted files in
#      the directories code runs from. On a mismatch it writes the tamper flag
#      that pre-tool-use.sh reads to block mutations.
#   2. Prints the active campaign line (scripts/vajra-campaign.sh active --brief).
#   3. After a compaction (source=compact), points at the last checkpoint.
#
# Everything goes to stdout, which Claude Code adds to the session context.
# The hook never fails the session: it always exits 0.
#
#   session-start.sh --check   integrity only: print problems, write no flag,
#                              exit 1 if anything is wrong (used by install/doctor)

set -uo pipefail

SKILL_DIR="${HOME}/.claude/skills/vajra"
VAJRA_DIR="${HOME}/.claude/vajra"
MANIFEST="${SKILL_DIR}/manifest.json"
SIG_FILE="${VAJRA_DIR}/manifest.sig"
KEY_FILE="${VAJRA_DIR}/.hmac-key"
TAMPER_FLAG="${VAJRA_DIR}/.integrity-violation"
CHECK_ONLY=0
[ "${1:-}" = "--check" ] && CHECK_ONLY=1

PAYLOAD=""
[ "$CHECK_ONLY" = 1 ] || PAYLOAD="$(cat 2>/dev/null || true)"

sha256_check() { # stdin: "<hash>  <path>" lines; stdout: failing paths
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c --quiet - 2>/dev/null
  else
    shasum -a 256 -c --quiet - 2>/dev/null
  fi | sed -n 's/: FAILED$//p'
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# One line per problem; empty output = clean.
integrity_problems() {
  if ! command -v jq >/dev/null 2>&1 || ! command -v openssl >/dev/null 2>&1; then
    echo "cannot verify: jq and openssl are required"
    return
  fi
  if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
    echo "cannot verify: no sha256sum/shasum"
    return
  fi
  [ -f "$MANIFEST" ] || { echo "cannot verify: manifest.json is missing"; return; }
  if ! jq -e '.files | type == "object"' "$MANIFEST" >/dev/null 2>&1; then
    echo "manifest.json is not valid"
    return
  fi

  # Signature: HMAC-SHA256(key, sha256(manifest.json)), made on this machine.
  if [ ! -s "$KEY_FILE" ]; then
    echo "not set up: no HMAC key (run scripts/install.sh)"
  elif [ ! -s "$SIG_FILE" ]; then
    echo "not set up: manifest is not signed on this machine (run scripts/install.sh)"
  else
    local want got
    want="$(tr -d '[:space:]' < "$SIG_FILE")"
    got="$(sha256_of "$MANIFEST" | tr -d '\n' | openssl dgst -sha256 -mac HMAC -macopt "hexkey:$(tr -d '[:space:]' < "$KEY_FILE")" -hex 2>/dev/null | awk '{print $NF}')"
    if [ -z "$got" ] || [ "$got" != "$want" ]; then
      echo "manifest.json signature does not match (changed after install)"
    fi
  fi

  # File hashes: one sha256 process for the whole list.
  local bad
  bad="$(cd "$SKILL_DIR" && jq -r '.files | to_entries[] | "\(.value)  \(.key)"' "$MANIFEST" | sha256_check)"
  [ -n "$bad" ] && printf '%s\n' "$bad" | sed 's/^/modified or missing: /'
  # Some sha256sum builds report unreadable files only on stderr; check existence too.
  local f
  while IFS= read -r f; do
    [ -f "$SKILL_DIR/$f" ] || printf 'missing: %s\n' "$f"
  done < <(jq -r '.files | keys[]' "$MANIFEST")

  # Unlisted files in the directories code runs from.
  local listed present
  listed="$(jq -r '.files | keys[]' "$MANIFEST" | LC_ALL=C sort)"
  present="$(cd "$SKILL_DIR" && find hooks scripts config workflows atman -type f ! -name '*.pyc' ! -path '*/__pycache__/*' ! -name '.DS_Store' 2>/dev/null | LC_ALL=C sort)"
  LC_ALL=C comm -13 <(printf '%s\n' "$listed") <(printf '%s\n' "$present") | sed '/^$/d; s/^/not in manifest: /'
}

PROBLEMS="$(integrity_problems)"

if [ "$CHECK_ONLY" = 1 ]; then
  [ -z "$PROBLEMS" ] && exit 0
  printf '%s\n' "$PROBLEMS"
  exit 1
fi

mkdir -p "$VAJRA_DIR" 2>/dev/null || true

case "$PROBLEMS" in
  "")
    rm -f "$TAMPER_FLAG" 2>/dev/null || true
    ;;
  "cannot verify:"*|"not set up:"*)
    # Not installed yet or missing tools: warn, but do not lock the session.
    echo "Vajra integrity: $(printf '%s' "$PROBLEMS" | head -1)"
    ;;
  *)
    {
      printf '{"detected":"%s","problems":' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
      printf '%s\n' "$PROBLEMS" | jq -R . | jq -sc .
      printf '}\n'
    } > "$TAMPER_FLAG" 2>/dev/null
    chmod 600 "$TAMPER_FLAG" 2>/dev/null || true
    echo "Vajra INTEGRITY VIOLATION: security-relevant Vajra files changed since install:"
    printf '%s\n' "$PROBLEMS" | head -10 | sed 's/^/  - /'
    echo "Edits and mutating commands are blocked until this is resolved. Tell the user."
    echo "The user should review the change (git -C ~/.claude/skills/vajra status) and, if it is intended, rebuild and re-sign the manifest by running this in their own terminal: bash ~/.claude/skills/vajra/scripts/generate-manifest.sh"
    ;;
esac

# Active campaign (vajra-campaign.sh prints one line or nothing). This hook adds
# the "Vajra campaign:" prefix exactly once, even if the brief already has it.
CAMPAIGN_SH="${SKILL_DIR}/scripts/vajra-campaign.sh"
if [ -f "$CAMPAIGN_SH" ]; then
  line="$(bash "$CAMPAIGN_SH" active --brief 2>/dev/null | head -1)"
  while :; do
    case "$line" in
      [[:space:]]*) line="${line#?}" ;;
      "Vajra campaign:"*) line="${line#Vajra campaign:}" ;;
      "Vajra campaign "*) line="${line#Vajra campaign }" ;;
      *) break ;;
    esac
  done
  [ -n "$line" ] && echo "Vajra campaign: ${line}"
fi

# After compaction, point at the checkpoint pre-compact.sh pinned.
SOURCE=""
if command -v jq >/dev/null 2>&1 && [ -n "$PAYLOAD" ]; then
  SOURCE="$(printf '%s' "$PAYLOAD" | jq -r '.source // empty' 2>/dev/null || true)"
fi
if [ "$SOURCE" = "compact" ] && [ -d "${VAJRA_DIR}/compaction" ]; then
  last="$(ls -t "${VAJRA_DIR}/compaction/"*.md 2>/dev/null | head -1)"
  if [ -n "$last" ]; then
    ref="$(sed -n 's/^git ref: \([^ ]*\).*/\1/p' "$last" | head -1)"
    echo "Vajra: context was compacted. Pre-compaction checkpoint: ${last}${ref:+ (pinned at ${ref})}."
  fi
fi

# Sandbox nudge: the OS sandbox is the enforced boundary.
SETTINGS_FILE="${HOME}/.claude/settings.json"
if command -v jq >/dev/null 2>&1 && [ -f "$SETTINGS_FILE" ]; then
  if [ "$(jq -r '.sandbox.enabled // false' "$SETTINGS_FILE" 2>/dev/null)" != "true" ]; then
    echo "Vajra: the Claude Code OS sandbox is off. It is the enforced boundary; the Vajra guard is defense in depth. Enable with /sandbox (see scripts/enable-sandbox.sh)."
  fi
fi

exit 0
