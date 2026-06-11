#!/usr/bin/env bash
# Test for hooks/pre-compact.sh — checkpoint + distillation before compaction.
# Run: bash tests/run-compaction.sh
set -uo pipefail
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 0; }
command -v openssl >/dev/null || { echo "SKIP: openssl required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-compact-XXXXXX")"
trap 'rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"
VDIR="$HOME/.claude/vajra"; mkdir -p "$VDIR"
printf '%s' "$(head -c 32 /dev/urandom | xxd -p | tr -d '\n')" > "$VDIR/.hmac-key"; chmod 600 "$VDIR/.hmac-key"
HOOK="$REPO/hooks/pre-compact.sh"

# A throwaway git repo so the snapshot path is exercised.
WORK="$TEST_HOME/work"; mkdir -p "$WORK"; cd "$WORK"
git init -q 2>/dev/null; git config user.email t@t; git config user.name t
echo a > a.txt; git add a.txt; git commit -qm init 2>/dev/null
echo b >> a.txt   # uncommitted change → stash create yields a snapshot

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

out="$(printf '%s' '{"session_id":"c1"}' | bash "$HOOK" 2>&1)"; rc=$?
[ "$rc" -eq 0 ]; check "pre-compact exits 0" $?
echo "$out" | grep -q "checkpoint"; check "prints checkpoint nudge into context" $?
echo "$out" | grep -q "Distill"; check "prints distillation instruction" $?

EV="$VDIR/events/c1.jsonl"
[ -f "$EV" ]; check "checkpoint event written" $?
[ "$(jq -r '.kind' "$EV" 2>/dev/null)" = "checkpoint" ]; check "event kind=checkpoint" $?
[ "$(jq -r '.tool' "$EV" 2>/dev/null)" = "__compaction__" ]; check "event tool=__compaction__" $?
[ -n "$(jq -r '.gitSha // empty' "$EV" 2>/dev/null)" ]; check "git snapshot SHA captured" $?
[ -n "$(jq -r '.hmac // empty' "$EV" 2>/dev/null)" ]; check "checkpoint event HMAC-signed" $?

REC="$(ls "$VDIR"/compaction/c1-*.md 2>/dev/null | head -1)"
[ -n "$REC" ] && [ -f "$REC" ]; check "compaction record written" $?
grep -q "Decisions" "$REC" 2>/dev/null; check "record carries distillation template" $?
grep -q "git stash apply" "$REC" 2>/dev/null; check "record has restore command" $?

echo "----------------------------------------"
echo "compaction: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
