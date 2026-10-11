#!/usr/bin/env bash
# install.sh + vajra-doctor.sh in a throwaway HOME:
#   - real 256-bit HMAC key, mode 600
#   - hooks merged into settings.json, user's own hooks and keys kept, backup made
#   - idempotent (second run adds nothing)
#   - --dry-run changes nothing, --no-hooks skips settings, --unregister-hooks removes only Vajra
#   - manifest verified then signed locally; tampering is refused at sign time
#   - workflows copied; doctor passes on a good install and fails on a broken one
#
# Run: bash tests/hooks/run-install.sh

set -uo pipefail
for t in jq openssl git; do command -v "$t" >/dev/null || { echo "SKIP: $t required"; exit 0; }; done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-install-XXXXXX")"
trap 'chmod -R u+w "$TEST_HOME" 2>/dev/null; rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"
SKILL="$HOME/.claude/skills/vajra"
VS="$HOME/.claude/vajra"
SETTINGS="$HOME/.claude/settings.json"

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); echo "FAIL :: $1"; fi; }

# A minimal copy of the skill (code-bearing dirs only), with a fresh manifest.
mkdir -p "$SKILL"
for d in hooks scripts config workflows atman; do [ -d "$REPO/$d" ] && cp -R "$REPO/$d" "$SKILL/"; done
cp "$REPO/SKILL.md" "$SKILL/"
find "$SKILL" -name '__pycache__' -type d -prune -exec rm -r {} + 2>/dev/null
bash "$SKILL/scripts/generate-manifest.sh" >/dev/null 2>&1
[ -f "$SKILL/manifest.json" ]; check "generate-manifest writes manifest.json" $?
[ ! -f "$SKILL/manifest.json.sig" ]; check "generate-manifest ships no manifest.json.sig" $?

# Pre-existing user settings with their own hook and an unrelated key.
mkdir -p "$HOME/.claude"
cat > "$SETTINGS" <<'JSON'
{
  "model": "opus",
  "hooks": {
    "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "/usr/local/bin/my-guard.sh" } ] } ],
    "Stop": [ { "hooks": [ { "type": "command", "command": "say done" } ] } ]
  }
}
JSON
ORIG="$(cat "$SETTINGS")"

# --- dry run changes nothing ---
out="$(bash "$SKILL/scripts/install.sh" --dry-run 2>&1)"; rc=$?
[ "$rc" -eq 0 ]; check "dry-run exits 0" $?
[ "$(cat "$SETTINGS")" = "$ORIG" ]; check "dry-run leaves settings.json untouched" $?
[ ! -e "$VS/.hmac-key" ]; check "dry-run creates no key" $?
printf '%s' "$out" | grep -q "would"; check "dry-run says what it would do" $?

# --- real install ---
out="$(bash "$SKILL/scripts/install.sh" 2>&1)"; rc=$?
[ "$rc" -eq 0 ]; check "install exits 0" $? || printf '%s\n' "$out"
printf '%s' "$out" | grep -q "WARNING: the workflows need agents that are not in ~/.claude/agents: .*vidura"; check "install warns about missing mahabharat agents" $?
printf '%s' "$out" | grep -q "install-skills.sh"; check "install warning gives the fix (install-skills.sh)" $?
tr -d '[:space:]' < "$VS/.hmac-key" | grep -Eq '^[0-9a-f]{64}$'; check "HMAC key is 64 hex chars" $?
mode="$(stat -c %a "$VS/.hmac-key" 2>/dev/null || stat -f %Lp "$VS/.hmac-key" 2>/dev/null)"
[ "$mode" = 600 ]; check "HMAC key mode 600" $?
ls "$HOME/.claude/"settings.json.vajra-backup-* >/dev/null 2>&1; check "settings.json backed up" $?
[ "$(jq -r .model "$SETTINGS")" = opus ]; check "unrelated settings kept" $?
jq -e '.hooks.PreToolUse[] | .hooks[] | select(.command == "/usr/local/bin/my-guard.sh")' "$SETTINGS" >/dev/null; check "user's own PreToolUse hook kept" $?
jq -e '.hooks.Stop[0].hooks[0].command == "say done"' "$SETTINGS" >/dev/null; check "user's Stop hook kept" $?
for ev in PreToolUse PostToolUse PostToolUseFailure SessionStart PreCompact UserPromptSubmit; do
  n="$(jq --arg ev "$ev" '[.hooks[$ev][]?.hooks[]? | select(.command | contains("/.claude/skills/vajra/hooks/"))] | length' "$SETTINGS")"
  [ "$n" = 1 ]; check "registered once: $ev" $?
done
jq -e '[.hooks.PreToolUse[].hooks[] | select(.command | contains("pre-tool-use.sh"))][0].onFailure == "block"' "$SETTINGS" >/dev/null; check "guard registered with onFailure: block" $?
jq -e '[.hooks.PostToolUse[].hooks[]][0].async == true' "$SETTINGS" >/dev/null; check "PostToolUse registered async" $?
[ -s "$VS/manifest.sig" ]; check "manifest signed locally" $?
for wf in "$SKILL"/workflows/vajra-*.js; do
  [ -f "$wf" ] || continue
  cmp -s "$wf" "$HOME/.claude/workflows/$(basename "$wf")"; check "workflow copied: $(basename "$wf")" $?
done

# The registered guard command really runs and blocks (shell form, $HOME expanded).
GUARD_CMD="$(jq -r '[.hooks.PreToolUse[].hooks[] | select(.command | contains("pre-tool-use.sh"))][0].command' "$SETTINGS")"
printf '%s' '{"tool_name":"Bash","tool_input":{"command":"rm -rf /"},"cwd":"/tmp"}' | bash -c "$GUARD_CMD" >/dev/null 2>&1; rc=$?
[ "$rc" -eq 2 ]; check "registered guard command exits 2 on rm -rf /" $?

# --- idempotent ---
before="$(jq -S . "$SETTINGS")"
bash "$SKILL/scripts/install.sh" >/dev/null 2>&1; rc=$?
[ "$rc" -eq 0 ]; check "second install exits 0" $?
[ "$(jq -S . "$SETTINGS")" = "$before" ]; check "second install changes nothing" $?
KEY1="$(cat "$VS/.hmac-key")"

# --- doctor requires the agents the workflows use ---
out="$(bash "$SKILL/scripts/vajra-doctor.sh" 2>&1)"; rc=$?
[ "$rc" -ne 0 ]; check "doctor fails when the mahabharat agents are missing" $?
printf '%s' "$out" | grep -q "^FAIL  agents missing from ~/.claude/agents.*arjuna.*install-skills.sh"; check "doctor names the missing agents and the fix" $?
mkdir -p "$HOME/.claude/agents"
cp "$REPO/bundled-agents/"*.md "$HOME/.claude/agents/"
mv "$HOME/.claude/agents/bhishma.md" "$TEST_HOME/bhishma.md"
out="$(bash "$SKILL/scripts/vajra-doctor.sh" 2>&1)"; rc=$?
[ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "^FAIL  agents missing.*: bhishma;"; check "doctor fails when one workflow agent (bhishma) is missing" $?
mv "$TEST_HOME/bhishma.md" "$HOME/.claude/agents/bhishma.md"
out="$(bash "$SKILL/scripts/install.sh" 2>&1)"
printf '%s' "$out" | grep -q "need agents"; [ $? -ne 0 ]; check "install does not warn once the agents are installed" $?

# --- doctor on a good install ---
out="$(bash "$SKILL/scripts/vajra-doctor.sh" 2>&1)"; rc=$?
[ "$rc" -eq 0 ]; check "doctor passes on a good install" $? || printf '%s\n' "$out" | grep -E '^FAIL'
printf '%s' "$out" | grep -q "PASS  hook registered: PreToolUse"; check "doctor reports PASS lines" $?
printf '%s' "$out" | grep -q "PASS  agents: all [0-9][0-9]* agents the workflows use"; check "doctor passes the agents check" $?
printf '%s' "$out" | grep -q "$KEY1"; [ $? -ne 0 ]; check "doctor never prints the key" $?

# --- session-start on a clean signed install ---
out="$(printf '%s' '{"session_id":"s","source":"startup"}' | bash "$SKILL/hooks/session-start.sh")"; rc=$?
[ "$rc" -eq 0 ] && [ ! -f "$VS/.integrity-violation" ]; check "session-start clean after install" $?

# --- tamper: session-start flags it, doctor fails, install refuses to re-sign ---
echo "# tampered" >> "$SKILL/hooks/pre-tool-use.sh"
out="$(printf '%s' '{"session_id":"s","source":"startup"}' | bash "$SKILL/hooks/session-start.sh")"
[ -f "$VS/.integrity-violation" ]; check "tamper flag written" $?
printf '%s' "$out" | grep -q "INTEGRITY VIOLATION"; check "violation reported on stdout" $?
bash "$SKILL/scripts/vajra-doctor.sh" >/dev/null 2>&1; rc=$?
[ "$rc" -ne 0 ]; check "doctor fails on a tampered install" $?
out="$(bash "$SKILL/scripts/install.sh" 2>&1)"
printf '%s' "$out" | grep -q "NOT signed"; check "install refuses to sign a mismatching tree" $?
[ ! -s "$VS/manifest.sig" ]; check "no signature after refused sign" $?
[ "$(cat "$VS/.hmac-key")" = "$KEY1" ]; check "re-install keeps the existing key" $?
# Operator accepts the change: regenerate (signs) and the flag clears.
bash "$SKILL/scripts/generate-manifest.sh" >/dev/null 2>&1
bash "$SKILL/hooks/session-start.sh" --check >/dev/null 2>&1; check "regenerated manifest verifies" $?

# --- empty key is replaced ---
: > "$VS/.hmac-key"
bash "$SKILL/scripts/install.sh" >/dev/null 2>&1
tr -d '[:space:]' < "$VS/.hmac-key" | grep -Eq '^[0-9a-f]{64}$'; check "empty key replaced by a real one" $?

# --- unregister removes only Vajra ---
bash "$SKILL/scripts/install.sh" --unregister-hooks >/dev/null 2>&1; rc=$?
[ "$rc" -eq 0 ]; check "unregister exits 0" $?
n="$(jq '[.hooks // {} | .[][] | .hooks[] | select(.command | contains("/.claude/skills/vajra/hooks/"))] | length' "$SETTINGS")"
[ "$n" = 0 ]; check "unregister removes all Vajra hooks" $?
jq -e '.hooks.PreToolUse[0].hooks[0].command == "/usr/local/bin/my-guard.sh" and .hooks.Stop[0].hooks[0].command == "say done" and .model == "opus"' "$SETTINGS" >/dev/null; check "unregister keeps user hooks and settings" $?
jq -e '.hooks.PostToolUse == null' "$SETTINGS" >/dev/null; check "unregister drops emptied events" $?
bash "$SKILL/scripts/vajra-doctor.sh" >/dev/null 2>&1; rc=$?
[ "$rc" -ne 0 ]; check "doctor fails when hooks are not registered" $?

# --- --no-hooks leaves settings alone ---
before="$(jq -S . "$SETTINGS")"
bash "$SKILL/scripts/install.sh" --no-hooks >/dev/null 2>&1
[ "$(jq -S . "$SETTINGS")" = "$before" ]; check "--no-hooks leaves settings.json alone" $?

# --- fresh HOME with no settings.json ---
rm -f "$SETTINGS"
bash "$SKILL/scripts/install.sh" >/dev/null 2>&1; rc=$?
[ "$rc" -eq 0 ] && jq -e '.hooks.SessionStart | length == 1' "$SETTINGS" >/dev/null; check "install creates settings.json when absent" $?

# --- invalid settings.json is never clobbered ---
echo '{ not json' > "$SETTINGS"
bash "$SKILL/scripts/install.sh" >/dev/null 2>&1; rc=$?
[ "$rc" -ne 0 ] && [ "$(cat "$SETTINGS")" = '{ not json' ]; check "invalid settings.json: abort, untouched" $?

# --- bash 3.2 (macOS system bash) ---
if [ -x /bin/bash ] && /bin/bash --version | head -1 | grep -q 'version 3\.'; then
  echo '{}' > "$SETTINGS"
  /bin/bash "$SKILL/scripts/install.sh" >/dev/null 2>&1; check "install runs under bash 3.2" $?
  /bin/bash "$SKILL/scripts/vajra-doctor.sh" >/dev/null 2>&1; check "doctor passes under bash 3.2" $?
fi

echo "----------------------------------------"
echo "install/doctor: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
