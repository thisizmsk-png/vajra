#!/usr/bin/env bash
# hooks/pre-compact.sh: the snapshot is pinned under refs/vajra/compact/ so
# git gc cannot collect it, the record carries the restore command, and the
# hook never blocks compaction.
#
# Run: bash tests/hooks/run-pre-compact.sh

set -uo pipefail
for t in jq git; do command -v "$t" >/dev/null || { echo "SKIP: $t required"; exit 0; }; done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-pc-XXXXXX")"
trap 'rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"
HOOK="$REPO/hooks/pre-compact.sh"
VS="$HOME/.claude/vajra"

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); echo "FAIL :: $1"; fi; }

W="$TEST_HOME/work"; mkdir -p "$W"
git -C "$W" init -q; git -C "$W" config user.email t@t; git -C "$W" config user.name t
echo a > "$W/a.txt"; git -C "$W" add a.txt; git -C "$W" commit -qm init
echo dirty >> "$W/a.txt"

# Hook runs from an unrelated cwd; it must use the payload cwd.
out="$(cd / && printf '%s' "{\"session_id\":\"s1\",\"cwd\":\"$W\",\"hook_event_name\":\"PreCompact\",\"trigger\":\"auto\"}" | bash "$HOOK")"; rc=$?
[ "$rc" -eq 0 ]; check "exit 0" $?
refs="$(git -C "$W" for-each-ref --format='%(refname)' refs/vajra/compact/)"
[ "$(printf '%s\n' "$refs" | grep -c .)" -eq 1 ]; check "one pinned ref under refs/vajra/compact/" $?
ref="$(printf '%s' "$refs" | head -1)"
git -C "$W" show "$ref:a.txt" 2>/dev/null | grep -q dirty; check "pinned snapshot holds the uncommitted change" $?
[ "$(cat "$W/a.txt" | tail -1)" = dirty ]; check "working tree untouched" $?
git -C "$W" gc --prune=now -q 2>/dev/null; git -C "$W" cat-file -e "$ref" 2>/dev/null; check "snapshot survives git gc --prune=now" $?
rec="$(ls "$VS"/compaction/s1-*.md | head -1)"
grep -q "git ref: $ref" "$rec"; check "record names the ref" $?
grep -q "git stash apply $ref" "$rec"; check "record has the restore command" $?
printf '%s' "$out" | grep -q "Distill"; check "prints distillation nudge" $?

# Clean tree -> pins HEAD.
git -C "$W" checkout -q -- a.txt
sleep 1
printf '%s' "{\"session_id\":\"s2\",\"cwd\":\"$W\"}" | bash "$HOOK" >/dev/null; rc=$?
[ "$rc" -eq 0 ]; check "clean tree: exit 0" $?
r2="$(git -C "$W" for-each-ref --format='%(refname) %(objectname)' 'refs/vajra/compact/*s2')"
[ "${r2##* }" = "$(git -C "$W" rev-parse HEAD)" ]; check "clean tree: HEAD pinned" $?

# Not a repo, garbage payload: still exit 0.
printf 'garbage' | (cd "$TEST_HOME" && bash "$HOOK") >/dev/null 2>&1; check "no repo + garbage payload: exit 0" $?

echo "----------------------------------------"
echo "pre-compact.sh: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
