#!/usr/bin/env bash
# Test for scripts/vajra-worktree-clean.sh (vajra-deliver's final cleanup step)
# in a throwaway repository with real worktrees under .claude/worktrees/:
#   - removes this run's worktrees whose only leftovers are allowed untracked
#     dirs (data/walkthroughs, .venv, setup dirs), then prunes
#   - never removes the main checkout, a dirty worktree (tracked or staged
#     change, stray untracked file), a locked one, another run's, or one
#     outside .claude/worktrees/
#   - branch-held worktrees of this run count; bad arguments exit 64
# Run: bash tests/run-worktree-clean.sh
set -uo pipefail
command -v git >/dev/null || { echo "SKIP: git required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO/scripts/vajra-worktree-clean.sh"
WORK="$(cd -P "$(mktemp -d "${TMPDIR:-/tmp}/vajra-wtc-XXXXXX")" && pwd)"
trap 'rm -rf "$WORK"' EXIT
export HOME="$WORK/home"; mkdir -p "$HOME"
export GIT_CONFIG_GLOBAL="$WORK/gitconfig" GIT_CONFIG_NOSYSTEM=1
git config --global user.email t@example.com; git config --global user.name t; git config --global init.defaultBranch main

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

R="$WORK/repo"; mkdir -p "$R"; cd "$R"
git init -q; echo base > a.txt; git add a.txt; git commit -qm base
BASE="$(git rev-parse HEAD)"
mk() { # name -> commit on a throwaway branch, returns sha
  git checkout -q -b "tmp-$1" "$BASE"; echo "$1" > "$1.txt"; git add "$1.txt"; git commit -qm "$1"
  git rev-parse HEAD; git checkout -q main
}
C1="$(mk t1)"; C2="$(mk t2)"; C3="$(mk t3)"; C4="$(mk t4)"; C5="$(mk t5)"; C6="$(mk t6)"; CO="$(mk other)"; C7="$(mk t7)"
WT="$R/.claude/worktrees"
add_wt() { git worktree add -q --detach "$WT/$1" "$2" >/dev/null 2>&1; }
add_wt clean "$C1";    mkdir -p "$WT/clean/data/walkthroughs/deliver-x" "$WT/clean/.venv/bin"; echo '{}' > "$WT/clean/data/walkthroughs/deliver-x/evidence.json"; echo x > "$WT/clean/.venv/bin/python"
add_wt nodemods "$C2"; mkdir -p "$WT/nodemods/node_modules/x"; echo x > "$WT/nodemods/node_modules/x/i.js"
add_wt dirty "$C3";    mkdir -p "$WT/dirty/data/walkthroughs"; echo changed > "$WT/dirty/a.txt"
add_wt staged "$C4";   echo new > "$WT/staged/new.txt"; git -C "$WT/staged" add new.txt
add_wt stray "$C5";    echo notes > "$WT/stray/notes.md"
add_wt locked "$C6";   git worktree lock "$WT/locked" >/dev/null 2>&1
add_wt other "$CO"
git worktree add -q -b "vajra/20261012-0900/t7" "$WT/onbranch" "$C7" >/dev/null 2>&1
mkdir -p "$WORK/elsewhere"; git worktree add -q --detach "$WORK/elsewhere/wt" "$C1" >/dev/null 2>&1
# The main checkout itself is detached at a run commit: it must never be touched.
git checkout -q --detach "$C1"
mkdir -p data/walkthroughs; echo '{}' > data/walkthroughs/e.json

ARGS=( --commits "${C1:0:12},$C2,$C3,$C4,$C5,$C6" --branch-prefix "vajra/20261012-0900/" --allow "data/walkthroughs,.venv,node_modules" )

out="$(bash "$SCRIPT" --dry-run "${ARGS[@]}" 2>&1)"; rc=$?
[ "$rc" -eq 0 ]; check "dry-run exits 0" $?
[ -d "$WT/clean" ]; check "dry-run removes nothing" $?
printf '%s\n' "$out" | grep -q "^would remove $WT/clean$"; check "dry-run names what it would remove" $?

out="$(bash "$SCRIPT" "${ARGS[@]}" 2>&1)"; rc=$?
[ "$rc" -eq 0 ]; check "cleanup exits 0 even when some are kept" $?
printf '%s\n' "$out" | grep -q "^removed $WT/clean$"; check "reports removed worktree" $?
[ ! -e "$WT/clean" ]; check "clean worktree (only evidence + .venv untracked) removed" $?
[ ! -e "$WT/nodemods" ]; check "worktree with only a setup dir (node_modules) removed" $?
[ ! -e "$WT/onbranch" ]; check "this run's branch-held worktree removed" $?
git rev-parse -q --verify "refs/heads/vajra/20261012-0900/t7" >/dev/null; check "removing a worktree keeps its branch" $?
[ -f "$WT/dirty/a.txt" ] && [ "$(cat "$WT/dirty/a.txt")" = changed ]; check "dirty worktree (tracked change) kept intact" $?
printf '%s\n' "$out" | grep -q "^kept $WT/dirty: uncommitted changes to tracked files$"; check "dirty worktree reported with reason" $?
[ -f "$WT/staged/new.txt" ]; check "staged change kept" $?
[ -f "$WT/stray/notes.md" ]; check "stray untracked file kept" $?
printf '%s\n' "$out" | grep -q "^kept $WT/stray: untracked files outside .*notes.md"; check "stray file named in the reason" $?
[ -d "$WT/locked" ]; check "locked worktree kept" $?
printf '%s\n' "$out" | grep -q "^kept $WT/locked: locked"; check "locked worktree reported" $?
[ -d "$WT/other" ]; check "another run's worktree untouched" $?
printf '%s\n' "$out" | grep -q "$WT/other"; [ $? -ne 0 ]; check "another run's worktree not even listed" $?
[ -d "$WORK/elsewhere/wt" ]; check "worktree outside .claude/worktrees/ kept" $?
[ -f "$R/a.txt" ] && [ -f "$R/data/walkthroughs/e.json" ]; check "main checkout untouched" $?
printf '%s\n' "$out" | grep -q "^removed $R$"; [ $? -ne 0 ]; check "main checkout never removed" $?
printf '%s\n' "$out" | tail -1 | grep -q "^worktrees: 3 removed, 5 kept$"; check "summary line counts" $?
git worktree list --porcelain | grep -q "$WT/clean"; [ $? -ne 0 ]; check "pruned from git worktree list" $?

# Without data/walkthroughs allowed (write-back not green) the evidence keeps the worktree.
add_wt evid "$C2"; mkdir -p "$WT/evid/data/walkthroughs/x"; echo '{}' > "$WT/evid/data/walkthroughs/x/evidence.json"
out="$(bash "$SCRIPT" --commits "$C2" --allow ".venv" 2>&1)"
[ -f "$WT/evid/data/walkthroughs/x/evidence.json" ]; check "evidence kept when data/walkthroughs is not allowed" $?

# Run from inside a linked worktree: that worktree is never removed either.
add_wt self "$C2"
out="$(cd "$WT/self" && bash "$SCRIPT" --commits "$C2" --allow ".venv" 2>&1)"
[ -d "$WT/self" ]; check "the worktree the script runs in is never removed" $?

# Bad arguments.
bash "$SCRIPT" --commits 'zz;rm' >/dev/null 2>&1; [ $? -eq 64 ]; check "bad commit -> 64" $?
bash "$SCRIPT" --commits "$C1" --allow '../x' >/dev/null 2>&1; [ $? -eq 64 ]; check "allow with .. -> 64" $?
bash "$SCRIPT" --commits "$C1" --allow '.git' >/dev/null 2>&1; [ $? -eq 64 ]; check "allow .git -> 64" $?
bash "$SCRIPT" --branch-prefix 'main/' >/dev/null 2>&1; [ $? -eq 64 ]; check "bad branch prefix -> 64" $?
bash "$SCRIPT" >/dev/null 2>&1; [ $? -eq 64 ]; check "nothing to match -> 64" $?
(cd "$WORK" && bash "$SCRIPT" --commits "$C1" >/dev/null 2>&1); [ $? -eq 1 ]; check "outside a repo -> 1" $?

# bash 3.2 (macOS system bash).
if [ -x /bin/bash ] && /bin/bash --version | head -1 | grep -q 'version 3\.'; then
  add_wt b32 "$C3"
  /bin/bash "$SCRIPT" --commits "$C3" --allow ".venv" >/dev/null 2>&1; rc=$?
  [ "$rc" -eq 0 ] && [ ! -e "$WT/b32" ] && [ -d "$WT/dirty" ]; check "runs under bash 3.2 (removes clean, keeps dirty)" $?
fi

echo "----------------------------------------"
echo "worktree-clean: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
