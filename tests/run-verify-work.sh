#!/usr/bin/env bash
# Test for scripts/vajra-verify-work.sh — proof-of-work evidence capture.
# Run: bash tests/run-verify-work.sh
set -uo pipefail
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO/scripts/vajra-verify-work.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vajra-vw-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"
export VAJRA_WALKTHROUGH_DIR="$WORK/wt"
export HOME="$WORK/home"; mkdir -p "$HOME"   # never the real ~/.claude (no key yet)

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

# --- all-pass case ---
bash "$SCRIPT" green "echo hello" "true" >/dev/null 2>&1; ec=$?
[ "$ec" -eq 0 ]; check "all-pass -> exit 0" $?
[ -f "$WORK/wt/green/evidence.json" ]; check "evidence.json written" $?
[ -f "$WORK/wt/green/WALKTHROUGH.md" ]; check "WALKTHROUGH.md written" $?
[ "$(jq -r '.allPassed' "$WORK/wt/green/evidence.json")" = "true" ]; check "evidence allPassed=true" $?
[ "$(jq -r '.commandsTotal' "$WORK/wt/green/evidence.json")" = "2" ]; check "2 commands recorded" $?
grep -q hello "$WORK/wt/green/cmd-1.log"; check "cmd-1 output captured" $?

# --- failing case ---
bash "$SCRIPT" red "true" "false" "echo after" >/dev/null 2>&1; ec=$?
[ "$ec" -eq 1 ]; check "any-fail -> exit 1" $?
[ "$(jq -r '.allPassed' "$WORK/wt/red/evidence.json")" = "false" ]; check "evidence allPassed=false" $?
[ "$(jq -r '.commandsFailed' "$WORK/wt/red/evidence.json")" = "1" ]; check "1 command failed" $?
grep -q 'checks FAILED' "$WORK/wt/red/WALKTHROUGH.md"; check "walkthrough marks failure" $?

# --- usage ---
bash "$SCRIPT" >/dev/null 2>&1; [ $? -eq 2 ]; check "no slug -> usage error" $?

# --- slug path-traversal is neutralized ---
ESC_PARENT="$(dirname "$WORK")"
rm -rf "${ESC_PARENT}/pwned" 2>/dev/null
bash "$SCRIPT" "../../pwned" "true" >/dev/null 2>&1
[ ! -d "${ESC_PARENT}/pwned" ]; check "traversal slug does NOT escape base dir" $?
[ -z "$(find "$WORK/wt" -maxdepth 1 -type d -name '*pwned*' 2>/dev/null)" ] || \
  [ -d "$WORK/wt/pwned" ]; check "traversal slug lands inside base (sanitized)" $?


# --- at least one command is required ---
bash "$SCRIPT" nocmds >/dev/null 2>&1; [ $? -eq 2 ]; check "slug with no commands -> usage error" $?
bash "$SCRIPT" blank "   " >/dev/null 2>&1; [ $? -eq 2 ]; check "blank command -> usage error" $?

# --- git evidence: diff vs HEAD plus untracked files, tied to the tree ---
REPO_T="$WORK/repo"; mkdir -p "$REPO_T"; cd "$REPO_T"
git init -q . && git config user.email t@t && git config user.name t
echo one > tracked.txt; git add tracked.txt; git commit -qm init
echo two >> tracked.txt            # unstaged change
echo staged > staged.txt; git add staged.txt
echo brand-new > untracked.txt     # untracked file
export VAJRA_WALKTHROUGH_DIR="$REPO_T/data/walkthroughs"   # inside the repo: must be excluded
bash "$SCRIPT" gitcase "true" >/dev/null 2>&1; ec=$?
EV="$REPO_T/data/walkthroughs/gitcase/evidence.json"
[ "$ec" -eq 0 ]; check "git case passes" $?
[ "$(jq -r '.git.head' "$EV")" = "$(git rev-parse HEAD)" ]; check "evidence records HEAD" $?
grep -q 'brand-new' "$REPO_T/data/walkthroughs/gitcase/diff.patch"; check "untracked file content in diff.patch" $?
grep -q '^+two' "$REPO_T/data/walkthroughs/gitcase/diff.patch"; check "unstaged change in diff.patch" $?
grep -q '^+staged' "$REPO_T/data/walkthroughs/gitcase/diff.patch"; check "staged change in diff.patch (diff vs HEAD)" $?
! grep -q 'walkthroughs' "$REPO_T/data/walkthroughs/gitcase/untracked.txt"; check "walkthrough dir excluded from untracked list" $?
[ "$(jq -r '.git.treeStable' "$EV")" = "true" ]; check "tree stable when checks do not edit files" $?
T1="$(jq -r '.git.worktreeTree' "$EV")"
[ -n "$T1" ] && [ "$T1" != "null" ]; check "worktree tree id recorded" $?
git diff --cached --name-only | grep -qx staged.txt && ! git diff --cached --name-only | grep -q untracked.txt
check "real index untouched" $?
bash "$SCRIPT" gitcase2 "true" >/dev/null 2>&1
[ "$(jq -r '.git.worktreeTree' "$REPO_T/data/walkthroughs/gitcase2/evidence.json")" = "$T1" ]; check "same tree -> same tree id (walkthrough output ignored)" $?
bash "$SCRIPT" mutate "echo changed > tracked.txt" >/dev/null 2>&1
[ "$(jq -r '.git.treeStable' "$REPO_T/data/walkthroughs/mutate/evidence.json")" = "false" ]; check "check that edits the tree -> treeStable=false" $?
cd "$WORK"

# --- signed evidence (hmac over canonical JSON without "hmac") ---
export VAJRA_WALKTHROUGH_DIR="$WORK/wt"
[ "$(jq -r 'has("hmac")' "$WORK/wt/green/evidence.json")" = "false" ]; check "no key -> evidence left unsigned" $?
mkdir -p "$HOME/.claude/vajra"; KEY="$(python3 -c 'import secrets;print(secrets.token_hex(32))')"
printf '%s' "$KEY" > "$HOME/.claude/vajra/.hmac-key"
bash "$SCRIPT" signed "test -d ." "echo 'héllo ✓'" >/dev/null 2>&1
SEV="$WORK/wt/signed/evidence.json"
[ "$(jq -r '.hmac | length' "$SEV")" = "64" ]; check "with a key -> evidence carries a 64-hex hmac" $?
verify_ev() { python3 -I -c '
import hashlib, hmac, json, sys
ev = json.load(open(sys.argv[1], encoding="utf-8")); mac = ev.pop("hmac")
msg = json.dumps(ev, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
sys.exit(0 if hmac.compare_digest(mac, hmac.new(bytes.fromhex(sys.argv[2]), msg, hashlib.sha256).hexdigest()) else 1)' "$1" "$KEY"; }
verify_ev "$SEV"; check "hmac verifies over sort_keys, (',', ':'), UTF-8 (non-ASCII command)" $?
jq '.commandsFailed=0 | .commands[0].cmd="make test"' "$SEV" > "$WORK/t.json"
! verify_ev "$WORK/t.json"; check "editing evidence breaks the hmac" $?
bash "$SCRIPT" signedred "false" >/dev/null 2>&1
verify_ev "$WORK/wt/signedred/evidence.json"; check "failing runs are signed too (green is decided by the reader)" $?

echo "----------------------------------------"
echo "verify-work: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
