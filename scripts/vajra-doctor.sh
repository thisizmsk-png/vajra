#!/usr/bin/env bash
# vajra-doctor.sh: check that Vajra is actually wired into Claude Code.
#
#   bash ~/.claude/skills/vajra/scripts/vajra-doctor.sh
#
# Prints one PASS / FAIL / WARN line per check and exits 1 if any check FAILs.
# Read-only except for the guard and router smoke tests, which append to
# Vajra's own audit and routing logs. Never prints the HMAC key.

set -uo pipefail

SKILL="${HOME}/.claude/skills/vajra"
VS="${HOME}/.claude/vajra"
SETTINGS="${HOME}/.claude/settings.json"
KEY="${VS}/.hmac-key"
WF_DIR="${HOME}/.claude/workflows"
MARK="/.claude/skills/vajra/hooks/"

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

FAILS=0 WARNS=0
pass() { printf 'PASS  %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; FAILS=$((FAILS + 1)); }
warn() { printf 'WARN  %s\n' "$1"; WARNS=$((WARNS + 1)); }

# --- Dependencies -------------------------------------------------------------
for dep in bash git jq openssl; do
  if command -v "$dep" >/dev/null 2>&1; then pass "dependency: $dep"; else fail "dependency: $dep not found"; fi
done
if command -v python3 >/dev/null 2>&1; then pass "dependency: python3"; else warn "dependency: python3 not found (router disabled)"; fi
HAVE_JQ=0; command -v jq >/dev/null 2>&1 && HAVE_JQ=1

# --- Skill files --------------------------------------------------------------
if [ -d "$SKILL" ]; then pass "skill dir: $SKILL"; else fail "skill dir missing: $SKILL"; fi
for h in pre-tool-use.sh post-tool-use.sh session-start.sh pre-compact.sh user-prompt-submit.sh; do
  f="$SKILL/hooks/$h"
  if [ -f "$f" ] && bash -n "$f" 2>/dev/null; then pass "hook script: hooks/$h"; else fail "hook script missing or has a syntax error: hooks/$h"; fi
done

# --- Registration in settings.json ---------------------------------------------
if [ "$HAVE_JQ" = 0 ]; then
  fail "settings.json: cannot inspect without jq"
elif [ ! -f "$SETTINGS" ]; then
  fail "settings.json: $SETTINGS does not exist (run scripts/install.sh)"
elif ! jq -e 'type == "object"' "$SETTINGS" >/dev/null 2>&1; then
  fail "settings.json: not valid JSON"
else
  pass "settings.json: valid JSON"
  for ev in PreToolUse PostToolUse PostToolUseFailure SessionStart PreCompact UserPromptSubmit; do
    cmds="$(jq -r --arg ev "$ev" --arg m "$MARK" '.hooks[$ev] // [] | .[] | .hooks // [] | .[] | select((.command // "") | contains($m)) | .command' "$SETTINGS" 2>/dev/null)"
    if [ -z "$cmds" ]; then
      fail "hook registered: $ev (run scripts/install.sh)"
      continue
    fi
    missing=""
    while IFS= read -r c; do
      rel="$(printf '%s' "$c" | sed -n 's#.*/\.claude/skills/vajra/\(hooks/[^"]*\.sh\).*#\1#p')"
      [ -n "$rel" ] && [ ! -f "$SKILL/$rel" ] && missing="$missing $rel"
    done <<< "$cmds"
    if [ -n "$missing" ]; then fail "hook registered: $ev points at missing file:$missing"; else pass "hook registered: $ev"; fi
  done
  ob="$(jq -r --arg m "$MARK" '[.hooks.PreToolUse // [] | .[] | .hooks // [] | .[] | select((.command // "") | contains($m)) | .onFailure // "continue"] | first // ""' "$SETTINGS")"
  if [ "$ob" = "block" ]; then pass "PreToolUse guard fails closed (onFailure: block)"; else fail "PreToolUse guard has onFailure '${ob:-unset}', want 'block'"; fi
  dups="$(jq -r --arg m "$MARK" '[.hooks // {} | to_entries[] | .key as $k | .value[] | .hooks // [] | .[] | select((.command // "") | contains($m)) | $k] | group_by(.) | map(select(length > 1) | .[0]) | join(" ")' "$SETTINGS")"
  if [ -n "$dups" ]; then warn "duplicate Vajra hook entries for: $dups (re-run scripts/install.sh)"; fi
  if [ "$(jq -r '.sandbox.enabled // false' "$SETTINGS")" = "true" ]; then pass "OS sandbox enabled"; else warn "OS sandbox is off (the enforced boundary; enable with /sandbox)"; fi
fi

# --- HMAC key and signature ---------------------------------------------------------
if [ -f "$KEY" ]; then
  mode="$(stat -c %a "$KEY" 2>/dev/null || stat -f %Lp "$KEY" 2>/dev/null)"
  if tr -d '[:space:]' < "$KEY" | grep -Eq '^[0-9a-f]{64}$'; then pass "HMAC key: present, 256-bit"; else fail "HMAC key: empty or malformed (run scripts/install.sh)"; fi
  if [ "$mode" = "600" ] || [ "$mode" = "400" ]; then pass "HMAC key: mode $mode"; else fail "HMAC key: mode $mode, want 600"; fi
else
  fail "HMAC key: missing (run scripts/install.sh)"
fi
if [ -f "$SKILL/manifest.json.sig" ]; then warn "legacy shipped manifest.json.sig present; it is ignored (signatures live in ~/.claude/vajra/manifest.sig)"; fi
if [ -f "$SKILL/hooks/session-start.sh" ]; then
  problems="$(bash "$SKILL/hooks/session-start.sh" --check 2>&1)"
  if [ -z "$problems" ]; then
    pass "integrity: manifest signed on this machine and all files match"
  else
    fail "integrity: $(printf '%s\n' "$problems" | head -3 | tr '\n' ';')"
  fi
fi
if [ -f "$VS/.integrity-violation" ]; then fail "tamper flag is set: $VS/.integrity-violation (mutations are blocked)"; fi

# --- Guard smoke test ---------------------------------------------------------
if [ -f "$SKILL/hooks/pre-tool-use.sh" ] && [ "$HAVE_JQ" = 1 ]; then
  bad='{"session_id":"doctor","cwd":"/tmp","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"rm -rf /"}}'
  good='{"session_id":"doctor","cwd":"/tmp","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"}}'
  printf '%s' "$bad" | bash "$SKILL/hooks/pre-tool-use.sh" >/dev/null 2>&1; rc_bad=$?
  printf '%s' "$good" | bash "$SKILL/hooks/pre-tool-use.sh" >/dev/null 2>&1; rc_good=$?
  if [ "$rc_bad" = 2 ]; then pass "guard blocks a dangerous command (exit 2)"; else fail "guard returned $rc_bad for 'rm -rf /', want 2"; fi
  if [ "$rc_good" = 0 ]; then pass "guard allows a safe command (exit 0)"
  elif [ -f "$VS/.integrity-violation" ]; then warn "guard blocks 'ls' only because the tamper flag is set"
  else fail "guard returned $rc_good for 'ls', want 0"; fi
fi

# --- Router smoke test -------------------------------------------------------
if [ -f "$SKILL/hooks/user-prompt-submit.sh" ]; then
  out="$(printf '%s' '{"session_id":"doctor","hook_event_name":"UserPromptSubmit","prompt":"/vajra help","cwd":"/tmp"}' | bash "$SKILL/hooks/user-prompt-submit.sh" 2>/dev/null)"; rc=$?
  if [ "$rc" != 0 ]; then
    fail "router hook exited $rc (it must always exit 0)"
  elif [ -z "$out" ] || { [ "$HAVE_JQ" = 1 ] && printf '%s' "$out" | jq -e '.hookSpecificOutput.hookEventName == "UserPromptSubmit"' >/dev/null 2>&1; }; then
    pass "router hook: exit 0, output ${out:+is valid hookSpecificOutput}${out:-empty}"
  else
    fail "router hook printed something that is not hookSpecificOutput JSON"
  fi
  [ -f "$SKILL/scripts/vajra_route.py" ] || warn "scripts/vajra_route.py missing (router does nothing)"
fi

# --- Workflows ----------------------------------------------------------------
found=0
for wf in "$SKILL"/workflows/vajra-*.js; do
  [ -f "$wf" ] || continue
  found=1
  name="$(basename "$wf")"
  if [ ! -f "$WF_DIR/$name" ]; then fail "workflow installed: $name (run scripts/install.sh)"
  elif cmp -s "$wf" "$WF_DIR/$name"; then pass "workflow installed: $name"
  else warn "workflow $name differs from the shipped copy (re-run scripts/install.sh)"; fi
done
[ "$found" = 1 ] || warn "no workflows/vajra-*.js in this checkout"
[ -f "$SKILL/workflows/vajra-deliver.js" ] || warn "workflows/vajra-deliver.js not shipped in this checkout"

# --- Agents the workflows need ---------------------------------------------------
AGENTS_DIR="${HOME}/.claude/agents"
REQ="$(required_agents "$SKILL/workflows" | tr '\n' ' ')"; REQ="${REQ% }"
if [ -z "$REQ" ]; then
  warn "agents: no agentType found in workflows/*.js"
else
  MISS="$(missing_agents "$SKILL/workflows" "$AGENTS_DIR")"
  if [ -z "$MISS" ]; then pass "agents: all $(printf '%s\n' $REQ | wc -l | tr -d ' ') agents the workflows use are in ~/.claude/agents"
  else fail "agents missing from ~/.claude/agents (the workflows need them): $MISS; $AGENTS_FIX"; fi
fi

# --- Campaign script ------------------------------------------------------------
if [ -f "$SKILL/scripts/vajra-campaign.sh" ]; then
  if bash "$SKILL/scripts/vajra-campaign.sh" active --brief >/dev/null 2>&1; then pass "campaign script runs"; else warn "scripts/vajra-campaign.sh active --brief exited non-zero"; fi
else
  warn "scripts/vajra-campaign.sh missing"
fi

echo "----------------------------------------"
echo "vajra doctor: ${FAILS} failed, ${WARNS} warnings"
[ "$FAILS" -eq 0 ]
