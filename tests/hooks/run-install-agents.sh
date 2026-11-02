#!/usr/bin/env bash
# install-skills.sh agent handling in a throwaway HOME:
#   - default run never overwrites a differing agent, makes no .bak, reports the count
#   - --update-agents backs up (name.bak) then overwrites; identical agents untouched
#   - missing agents are installed; unknown args exit 2 and change nothing
#
# Run: bash tests/hooks/run-install-agents.sh

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-agents-XXXXXX")"
trap 'chmod -R u+w "$TEST_HOME" 2>/dev/null; rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"
SKILL="$HOME/.claude/skills/vajra"
AGENTS="$HOME/.claude/agents"

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); echo "FAIL :: $1"; fi; }

# Stage the script, one dummy skill, and four real agents as the bundle.
mkdir -p "$SKILL/scripts" "$SKILL/bundled-skills/dummy" "$SKILL/bundled-agents"
cp "$REPO/scripts/install-skills.sh" "$SKILL/scripts/"
echo "dummy" > "$SKILL/bundled-skills/dummy/SKILL.md"
for a in abhimanyu arjuna bhima vidura; do cp "$REPO/bundled-agents/$a.md" "$SKILL/bundled-agents/"; done
B="$SKILL/bundled-agents"
RUN="$SKILL/scripts/install-skills.sh"

# abhimanyu identical, arjuna + bhima modified, vidura absent.
seed() {
  mkdir -p "$AGENTS"
  rm -f "$AGENTS"/*.bak "$AGENTS/vidura.md"
  cp "$B/abhimanyu.md" "$AGENTS/abhimanyu.md"
  touch -t 202001010000 "$AGENTS/abhimanyu.md"
  printf 'my custom arjuna\n' > "$AGENTS/arjuna.md"
  printf 'my custom bhima\n' > "$AGENTS/bhima.md"
}

# --- default run ---
seed
out="$(bash "$RUN" 2>&1)"; rc=$?
[ "$rc" -eq 0 ]; check "default run exits 0" $?
[ "$(cat "$AGENTS/arjuna.md")" = "my custom arjuna" ]; check "modified arjuna untouched" $?
[ "$(cat "$AGENTS/bhima.md")" = "my custom bhima" ]; check "modified bhima untouched" $?
[ -z "$(ls "$AGENTS"/*.bak 2>/dev/null)" ]; check "default run makes no .bak" $?
echo "$out" | grep -q "SKIP: arjuna.md"; check "output says SKIP for arjuna" $?
echo "$out" | grep -q "SKIP: bhima.md"; check "output says SKIP for bhima" $?
echo "$out" | grep -Eq "Differs from bundled: 2 agents.*--update-agents"; check "reports count 2 and mentions --update-agents" $?
cmp -s "$B/vidura.md" "$AGENTS/vidura.md"; check "missing vidura installed" $?

# --- second default run: restore bundle copies, nothing differs ---
cp "$B/arjuna.md" "$AGENTS/arjuna.md"
cp "$B/bhima.md" "$AGENTS/bhima.md"
out="$(bash "$RUN" 2>&1)"
echo "$out" | grep -q "Differs from bundled: 0 agents"; check "zero differing reports 0" $?
if echo "$out" | grep -q -- "--update-agents"; then r=1; else r=0; fi
check "no hint when nothing differs" $r

# --- --update-agents ---
seed
before="$(ls -l "$AGENTS/abhimanyu.md")"
out="$(bash "$RUN" --update-agents 2>&1)"; rc=$?
[ "$rc" -eq 0 ]; check "--update-agents exits 0" $?
for a in arjuna bhima; do
  cmp -s "$B/$a.md" "$AGENTS/$a.md"; check "$a overwritten with bundle" $?
  [ "$(cat "$AGENTS/$a.md.bak")" = "my custom $a" ]; check "$a.md.bak holds user content" $?
done
[ ! -e "$AGENTS/abhimanyu.md.bak" ]; check "identical agent has no .bak" $?
[ "$(ls -l "$AGENTS/abhimanyu.md")" = "$before" ]; check "identical agent not rewritten (mtime)" $?
cmp -s "$B/vidura.md" "$AGENTS/vidura.md"; check "missing vidura installed" $?
echo "$out" | grep -q "UPDATE: arjuna.md (backup: arjuna.md.bak)"; check "UPDATE line printed" $?
echo "$out" | grep -q "Updated: 2 agents"; check "Updated: 2 agents summary" $?

# --- unknown arg ---
seed
snap="$(cd "$AGENTS" && ls -l && cat ./*.md | cksum)"
out="$(bash "$RUN" --bogus 2>&1)"; rc=$?
[ "$rc" -eq 2 ]; check "--bogus exits 2" $?
[ "$snap" = "$(cd "$AGENTS" && ls -l && cat ./*.md | cksum)" ]; check "--bogus changes nothing" $?

echo "install-agents: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
