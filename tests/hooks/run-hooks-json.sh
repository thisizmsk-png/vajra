#!/usr/bin/env bash
# hooks/hooks.json wiring for the desktop control hook (E1) and guard matcher (E2):
#   - valid JSON; additive PreToolUse '*' and PostToolUse '*' entries for desktop-control.sh
#   - every pre-existing entry unchanged versus the base commit
#   - guard matcher matches mcp__vajra-desktop__list_workers and every previously matched tool
#     (matchers are tested as a full-string match)
# Run: bash tests/hooks/run-hooks-json.sh
set -uo pipefail
for t in jq python3 git; do command -v "$t" >/dev/null || { echo "SKIP: $t required"; exit 77; }; done
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO"
F=hooks/hooks.json
PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); echo "FAIL :: $1"; fi; }

jq -e . "$F" >/dev/null 2>&1; check "hooks.json is valid JSON" $?

# Base: main at the time the desktop work started.
BASE_REF="${HOOKS_JSON_BASE:-8a10d0522d286ac7e4eaa40c6ae7e982199bb72c}"
git cat-file -e "$BASE_REF^{commit}" 2>/dev/null || git fetch -q --depth=1 origin "$BASE_REF" 2>/dev/null
BASE="$(git show "$BASE_REF:$F" 2>/dev/null)"
[ -n "$BASE" ]; check "base hooks.json readable ($BASE_REF)" $?

DC='bash "$HOME/.claude/skills/vajra/hooks/desktop-control.sh"'
for ev in PreToolUse PostToolUse PostToolUseFailure; do
  n="$(jq --arg ev "$ev" --arg c "$DC" '[.hooks[$ev][] | select(.matcher=="*") | .hooks[] | select(.command==$c and .timeout==5 and .type=="command" and (.async|not))] | length' "$F")"
  [ "$n" = 1 ]; check "$ev has one '*' desktop-control.sh entry, timeout 5, not async" $?
done

# Remove our entries; what remains must equal the base, except the guard matcher may grow.
STRIPPED="$(jq --arg c "$DC" '.hooks |= with_entries(.value |= map(select(([.hooks[].command] | index($c)) | not)))' "$F")"
norm() { jq -S '.hooks.PreToolUse[0].matcher = "G"'; }
[ "$(printf '%s' "$STRIPPED" | norm)" = "$(printf '%s' "$BASE" | norm)" ]; check "existing entries unchanged (guard matcher aside)" $?

# Matcher regex semantics (full-string match).
OLD="$(printf '%s' "$BASE" | jq -r '.hooks.PreToolUse[0].matcher')"
NEW="$(jq -r '.hooks.PreToolUse[0].matcher' "$F")"
python3 - "$OLD" "$NEW" <<'PY'
import re, sys
old, new = sys.argv[1:3]
bad = []
for t in old.split("|"):
    if not re.fullmatch(new, t): bad.append("lost " + t)
for t in ("mcp__vajra-desktop__list_workers", "mcp__vajra-desktop__stop_worker"):
    if not re.fullmatch(new, t): bad.append("missing " + t)
for t in ("mcp__other__list_workers", "mcp__vajra-desktopx__a", "WebFetch"):
    if re.fullmatch(new, t): bad.append("over-matches " + t)
print("\n".join(bad)); sys.exit(1 if bad else 0)
PY
check "guard matcher keeps old tools and matches mcp__vajra-desktop__.*" $?

echo "run-hooks-json: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
