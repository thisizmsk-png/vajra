#!/usr/bin/env bash
# Vajra installer.
#
#   bash ~/.claude/skills/vajra/scripts/install.sh [--dry-run] [--no-hooks] [--unregister-hooks]
#
# What it does:
#   1. checks dependencies (bash, git, jq, openssl; python3 for the router)
#   2. creates ~/.claude/vajra (mode 700) and a real HMAC key (openssl rand -hex 32, mode 600)
#   3. merges hooks/hooks.json into ~/.claude/settings.json: backed up first,
#      idempotent, keeps every non-Vajra hook (skip with --no-hooks)
#   4. copies workflows/vajra-*.js to ~/.claude/workflows/
#   5. verifies the shipped files against manifest.json and signs the manifest
#      on this machine (~/.claude/vajra/manifest.sig) with your key
#
#   --unregister-hooks  remove Vajra's entries from settings.json and stop
#   --dry-run           print what would change, change nothing
#
# Re-running is safe. bash 3.2 compatible.

set -euo pipefail

DRY_RUN=0 NO_HOOKS=0 UNREGISTER=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --no-hooks) NO_HOOKS=1 ;;
    --unregister-hooks) UNREGISTER=1 ;;
    -h|--help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg (see --help)" >&2; exit 64 ;;
  esac
done

VAJRA_HOME="${HOME}/.claude/vajra"
VAJRA_SKILL="${HOME}/.claude/skills/vajra"
SETTINGS="${HOME}/.claude/settings.json"
HOOKS_JSON="${VAJRA_SKILL}/hooks/hooks.json"
KEY_FILE="${VAJRA_HOME}/.hmac-key"
SIG_FILE="${VAJRA_HOME}/manifest.sig"
WF_DIR="${HOME}/.claude/workflows"
VAJRA_MARK="/.claude/skills/vajra/hooks/"

# Agents the workflows call: every literal agentType value, plus any quoted
# name from the mahabharat roster (rosters are arrays passed as agentType: v).
VAJRA_ROSTER=" abhimanyu arjuna ashwatthama bhima bhishma dhrishtadyumna draupadi drona duryodhana hanuman karna krishna nakula sahadeva sepoy-backend-1 sepoy-frontend-1 sepoy-test-1 shakuni vidura yudhishthira "
required_agents() { # workflows-dir -> one name per line, sorted
  local f lits w
  for f in "$1"/*.js; do [ -f "$f" ] && cat "$f"; done 2>/dev/null \
    | grep -oE "agentType: *['\"][A-Za-z0-9_-]+['\"]|['\"][a-z0-9-]+['\"]" 2>/dev/null \
    | while IFS= read -r w; do
        case "$w" in agentType*) w="${w#agentType}"; w="${w#:}"; w="${w# }"; lits=1 ;; *) lits=0 ;; esac
        w="${w//\'/}"; w="${w//\"/}"
        if [ "$lits" = 1 ] || [[ "$VAJRA_ROSTER" == *" $w "* ]]; then printf '%s\n' "$w"; fi
      done | LC_ALL=C sort -u
  return 0
}
missing_agents() { # workflows-dir agents-dir -> space-separated missing names
  local a out=""
  for a in $(required_agents "$1"); do [ -f "$2/$a.md" ] || out="$out $a"; done
  printf '%s' "${out# }"
}
AGENTS_FIX="run: bash ~/.claude/skills/vajra/scripts/install-skills.sh (installs the 20 mahabharat agents), or install them from https://github.com/thisizmsk-png/mahabharat-agents"

say() { echo "$@"; }
plan() { if [ "$DRY_RUN" = 1 ]; then echo "  [dry-run] would $*"; fi; }
die() { echo "ERROR: $*" >&2; exit 1; }

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

say "=== Vajra install$([ "$DRY_RUN" = 1 ] && echo ' (dry run)') ==="

# --- 1. Dependencies --------------------------------------------------------
say "[1/5] Dependencies"
for dep in git jq openssl; do
  command -v "$dep" >/dev/null 2>&1 || die "$dep not found (install it and re-run)"
done
command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 || die "need sha256sum or shasum"
command -v python3 >/dev/null 2>&1 || say "  WARNING: python3 not found; the /vajra prompt router will do nothing"
[ -d "$VAJRA_SKILL" ] || die "Vajra must live at ${VAJRA_SKILL} (git clone it there, or symlink it)"
[ -f "$HOOKS_JSON" ] && jq -e '.hooks | type == "object"' "$HOOKS_JSON" >/dev/null 2>&1 || die "missing or invalid ${HOOKS_JSON}"
say "  ok: git jq openssl$(command -v python3 >/dev/null 2>&1 && echo ' python3')"

# --- hooks merge helpers ----------------------------------------------------
# Remove every hook handler whose command points into the Vajra hooks dir,
# then drop groups and events left empty. Everything else is untouched.
JQ_STRIP='
  def strip: map(.hooks = ((.hooks // []) | map(select(((.command // "") | contains($mark)) | not))))
             | map(select((.hooks | length) > 0));
  if (.hooks | type) == "object" then
    .hooks |= (with_entries(.value |= strip) | with_entries(select((.value | length) > 0)))
    | if (.hooks | length) == 0 then del(.hooks) else . end
  else . end'
JQ_ADD='
  reduce ($vajra.hooks | to_entries[]) as $e (.;
    .hooks[$e.key] = ((.hooks[$e.key] // []) + $e.value))'

current_settings() {
  if [ -f "$SETTINGS" ]; then
    jq -e 'type == "object"' "$SETTINGS" >/dev/null 2>&1 || die "${SETTINGS} is not a JSON object; fix it by hand first (nothing was changed)"
    cat "$SETTINGS"
  else
    echo '{}'
  fi
}

write_settings() { # new-json
  local new="$1" backup
  if [ -f "$SETTINGS" ] && [ "$(jq -S . "$SETTINGS")" = "$(printf '%s' "$new" | jq -S .)" ]; then
    say "  settings.json already up to date"
    return 0
  fi
  if [ "$DRY_RUN" = 1 ]; then
    plan "update ${SETTINGS}; resulting hooks block:"
    printf '%s' "$new" | jq '.hooks // {}' | sed 's/^/    /'
    return 0
  fi
  mkdir -p "$(dirname "$SETTINGS")"
  if [ -f "$SETTINGS" ]; then
    backup="${SETTINGS}.vajra-backup-$(date -u +%Y%m%dT%H%M%SZ)"
    cp -p "$SETTINGS" "$backup"
    say "  backup: ${backup}"
  fi
  # Write through (keeps a symlinked settings.json and its permissions).
  printf '%s\n' "$new" | jq . > "${SETTINGS}.vajra-tmp"
  cat "${SETTINGS}.vajra-tmp" > "$SETTINGS"
  rm -f "${SETTINGS}.vajra-tmp"
  say "  updated: ${SETTINGS}"
}

if [ "$UNREGISTER" = 1 ]; then
  say "[hooks] Removing Vajra hooks from ${SETTINGS}"
  if [ ! -f "$SETTINGS" ]; then say "  no settings.json; nothing to do"; exit 0; fi
  NEW="$(current_settings | jq --arg mark "$VAJRA_MARK" "$JQ_STRIP")"
  write_settings "$NEW"
  say "Done. Vajra state in ${VAJRA_HOME} and workflows in ${WF_DIR} were left in place."
  exit 0
fi

# --- 2. State dir and HMAC key ---------------------------------------------
say "[2/5] State directory and HMAC key"
if [ "$DRY_RUN" = 1 ]; then
  [ -d "$VAJRA_HOME" ] || plan "create ${VAJRA_HOME} (mode 700) with campaigns/ and proposals/"
else
  mkdir -p "$VAJRA_HOME/campaigns" "$VAJRA_HOME/proposals"
  chmod 700 "$VAJRA_HOME" "$VAJRA_HOME/campaigns" "$VAJRA_HOME/proposals"
fi
key_ok() { [ -f "$KEY_FILE" ] && tr -d '[:space:]' < "$KEY_FILE" | grep -Eq '^[0-9a-f]{64}$'; }
if key_ok; then
  say "  HMAC key present"
  [ "$DRY_RUN" = 1 ] || chmod 600 "$KEY_FILE"
else
  if [ -e "$KEY_FILE" ]; then
    say "  WARNING: existing key is empty or malformed; replacing it (old signatures will no longer verify)"
  fi
  if [ "$DRY_RUN" = 1 ]; then
    plan "generate ${KEY_FILE} with openssl rand -hex 32 (mode 600)"
  else
    ( umask 077; openssl rand -hex 32 > "${KEY_FILE}.tmp" )
    mv "${KEY_FILE}.tmp" "$KEY_FILE"
    chmod 600 "$KEY_FILE"
    key_ok || die "could not create a valid HMAC key"
    say "  generated new HMAC key (mode 600)"
  fi
fi

# --- 3. Hooks ----------------------------------------------------------------
say "[3/5] Hooks"
if [ "$NO_HOOKS" = 1 ]; then
  say "  skipped (--no-hooks). Vajra's guard, router and learning hooks are NOT active."
else
  for f in $(jq -r '.hooks[][].hooks[].command' "$HOOKS_JSON" | sed -n 's#.*/\.claude/skills/vajra/\(hooks/[^"]*\.sh\).*#\1#p' | sort -u); do
    [ -f "${VAJRA_SKILL}/$f" ] || say "  WARNING: ${f} is registered but missing"
  done
  NEW="$(current_settings | jq --arg mark "$VAJRA_MARK" "$JQ_STRIP" | jq --slurpfile v "$HOOKS_JSON" '$v[0] as $vajra | '"$JQ_ADD")"
  write_settings "$NEW"
fi

# --- 4. Workflows ------------------------------------------------------------
say "[4/5] Workflows"
WF_COUNT=0
for wf in "${VAJRA_SKILL}"/workflows/vajra-*.js; do
  [ -f "$wf" ] || continue
  WF_COUNT=$((WF_COUNT + 1))
  dest="${WF_DIR}/$(basename "$wf")"
  if [ -f "$dest" ] && cmp -s "$wf" "$dest"; then continue; fi
  if [ "$DRY_RUN" = 1 ]; then plan "copy $(basename "$wf") to ${WF_DIR}/"; continue; fi
  mkdir -p "$WF_DIR"
  cp "$wf" "$dest"
  say "  installed $(basename "$wf")"
done
[ "$WF_COUNT" -gt 0 ] && say "  ${WF_COUNT} workflow(s) in ${WF_DIR}" || say "  no workflows shipped in this checkout"
MISSING_AGENTS="$(missing_agents "${VAJRA_SKILL}/workflows" "${HOME}/.claude/agents")"
if [ -n "$MISSING_AGENTS" ]; then
  say "  WARNING: the workflows need agents that are not in ~/.claude/agents: ${MISSING_AGENTS}"
  say "  Fix: ${AGENTS_FIX}"
fi

# --- 5. Verify and sign the manifest ------------------------------------------
say "[5/5] Integrity"
MANIFEST="${VAJRA_SKILL}/manifest.json"
if [ ! -f "$MANIFEST" ]; then
  say "  WARNING: no manifest.json; integrity checking is off (maintainers: scripts/generate-manifest.sh)"
elif [ "$DRY_RUN" = 1 ]; then
  plan "verify manifest.json and write ${SIG_FILE}"
else
  rm -f "$SIG_FILE"
  if PROBLEMS="$(bash "${VAJRA_SKILL}/hooks/session-start.sh" --check 2>&1)"; then :; fi
  # Without a signature the check only reports "not set up"; any other line is a real mismatch.
  REAL="$(printf '%s\n' "$PROBLEMS" | grep -v '^not set up:' | sed '/^$/d' || true)"
  if [ -n "$REAL" ]; then
    say "  Files differ from manifest.json, so the manifest was NOT signed:"
    printf '%s\n' "$REAL" | head -20 | sed 's/^/    /'
    say "  If these are your own changes, run: bash ${VAJRA_SKILL}/scripts/generate-manifest.sh"
    say "  (it rebuilds and signs the manifest), then start a new session."
  else
    ( umask 077
      sha256_of "$MANIFEST" | tr -d '\n' \
        | openssl dgst -sha256 -mac HMAC -macopt "hexkey:$(tr -d '[:space:]' < "$KEY_FILE")" -hex \
        | awk '{print $NF}' > "$SIG_FILE" )
    chmod 600 "$SIG_FILE"
    rm -f "${VAJRA_HOME}/.integrity-violation"
    say "  verified $(jq '.files | length' "$MANIFEST") files and signed the manifest: ${SIG_FILE}"
  fi
fi

say ""
say "=== Vajra installed$([ "$DRY_RUN" = 1 ] && echo ' (dry run: nothing changed)') ==="
say "  Skill: ${VAJRA_SKILL}"
say "  State: ${VAJRA_HOME}"
say "Check the wiring: bash ${VAJRA_SKILL}/scripts/vajra-doctor.sh"
say "Restart Claude Code so it loads the hooks. Turn on the OS sandbox too: /sandbox"
