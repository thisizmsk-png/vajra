#!/usr/bin/env bash
# enable-sandbox.sh — print platform-checked instructions to enable Claude Code's
# native OS sandbox (the durable security boundary for Vajra).
#
# This script is PRINT-ONLY by design. It never writes ~/.claude/settings.json:
# that file is both Vajra's immutable core and a system-settings change, so the
# operator applies it themselves. Run, read, then apply.

set -uo pipefail

SKILL_DIR="${HOME}/.claude/skills/vajra"
REC="${SKILL_DIR}/config/sandbox.json"
SETTINGS="${HOME}/.claude/settings.json"

bold() { printf '\033[1m%s\033[0m\n' "$1"; }
ok() { printf '  \033[32m✓\033[0m %s\n' "$1"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$1"; }

bold "Vajra — enable the native OS sandbox (the enforced security boundary)"
echo

# --- Platform + dependency check ---
OS="$(uname -s)"
case "$OS" in
  Darwin)
    ok "macOS detected — Seatbelt is built in, no dependencies to install."
    ;;
  Linux)
    if grep -qi microsoft /proc/version 2>/dev/null; then
      ok "WSL2 detected."
    else
      ok "Linux detected."
    fi
    for dep in bubblewrap socat; do
      if command -v "${dep%bubblewrap}" >/dev/null 2>&1 || command -v bwrap >/dev/null 2>&1 && [ "$dep" = bubblewrap ] || command -v "$dep" >/dev/null 2>&1; then
        ok "$dep present"
      else
        warn "$dep MISSING — install: sudo apt-get install bubblewrap socat   (or dnf)"
      fi
    done
    if ! npm ls -g @anthropic-ai/sandbox-runtime >/dev/null 2>&1; then
      warn "seccomp Unix-socket filter not found — install: npm i -g @anthropic-ai/sandbox-runtime"
    else
      ok "seccomp filter present"
    fi
    ;;
  *)
    warn "Native Windows is not supported. Run Claude Code inside WSL2."
    ;;
esac
echo

# --- Current state ---
if [ -f "$SETTINGS" ] && command -v jq >/dev/null 2>&1; then
  if [ "$(jq -r '.sandbox.enabled // false' "$SETTINGS" 2>/dev/null)" = "true" ]; then
    ok "sandbox.enabled is already true in ${SETTINGS}"
  else
    warn "sandbox is NOT enabled in ${SETTINGS}"
  fi
fi
echo

# --- The recommended policy ---
bold "Recommended policy (from ${REC#$HOME/}):"
if command -v jq >/dev/null 2>&1 && [ -f "$REC" ]; then
  jq '.settings' "$REC"
  echo
  bold "Also set this environment variable (strips credentials from subprocesses):"
  jq -r '.env | to_entries[] | "  export \(.key)=\(.value)"' "$REC"
else
  warn "jq or config/sandbox.json not found — see ${REC}"
fi
echo

# --- How to apply ---
bold "Apply it one of two ways:"
cat <<'EOF'
  1. Interactive (recommended first time):
       Run  /sandbox  inside a Claude Code session → Mode tab → auto-allow.
       This writes .claude/settings.local.json for the current project.

  2. All projects (user scope):
       Merge the "settings" block above into  ~/.claude/settings.json  yourself,
       then restart Claude Code. (Vajra will not edit settings.json for you.)

  Verify after enabling:
       Run  /sandbox  → Config tab shows the resolved policy.
       Strict mode: set  "allowUnsandboxedCommands": false  (Overrides tab).
EOF
echo
warn "The default read policy still allows ~/.aws and ~/.ssh — the denyRead list above is required, not optional."
exit 0
