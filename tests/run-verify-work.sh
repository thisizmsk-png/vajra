#!/usr/bin/env bash
# Test for scripts/vajra-verify-work.sh — proof-of-work evidence capture.
# Run: bash tests/run-verify-work.sh
set -uo pipefail
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO/scripts/vajra-verify-work.sh"
# Canonical (no /var or /tmp link above it), as the core passes an absolute walkthrough dir to --sign.
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vajra-vw-XXXXXX")"; WORK="$(cd "$WORK" && pwd -P)"
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

# --- 6i-e: --run-only (never opens the key, evidence on fd 3 only) and --sign ---
sha_of() { python3 -I -c 'import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$1"; }
cd "$REPO_T"; git add -A >/dev/null 2>&1; git commit -qm two >/dev/null 2>&1
export VAJRA_WALKTHROUGH_DIR="$REPO_T/data/walkthroughs"
RO="$VAJRA_WALKTHROUGH_DIR/ro"; mkdir -p "$RO"; echo '{"stale":true}' > "$RO/evidence.json"
bash "$SCRIPT" --run-only ro "echo hi" "if { : >&3; } 2>/dev/null; then echo FD3-OPEN; fi" 3> "$WORK/ro.fd3" >/dev/null 2>&1; ec=$?
[ "$ec" -eq 0 ]; check "run-only: all-pass -> exit 0" $?
[ ! -e "$RO/evidence.json" ]; check "run-only: no evidence.json on disk (a stale one is removed)" $?
[ "$(wc -l < "$WORK/ro.fd3" | tr -d ' ')" = "1" ]; check "run-only: evidence is one line on fd 3" $?
[ "$(tail -c 1 "$WORK/ro.fd3" | od -An -c | tr -d ' ')" = '\n' ]; check "run-only: the line ends in a newline" $?
# A check command's leftover background child can't add a line: fd 3 is closed for it, and the script closes fd 3.
bash "$SCRIPT" --run-only robg "(sleep 1; echo LATE >&3) 2>/dev/null &" 3> "$WORK/robg.fd3" >/dev/null 2>&1
sleep 2; [ "$(wc -l < "$WORK/robg.fd3" | tr -d ' ')" = "1" ] && ! grep -qx LATE "$WORK/robg.fd3"
check "run-only: a leftover child never writes a second line" $?
jq -e '.slug == "ro" and .commandsTotal == 2 and .allPassed and (has("hmac") | not) and .git.head != null' "$WORK/ro.fd3" >/dev/null
check "run-only: fd 3 carries the unsigned bundle with its head" $?
! grep -qx FD3-OPEN "$RO/cmd-2.log"; check "run-only: fd 3 is closed for every command" $?
# Claude review #214 (MUST): a repo-configured clean filter, textconv driver or external diff (and fsmonitor) runs
# during the script's own git steps; none of them may reach the evidence fd, whichever number it tries.
HR="$WORK/hostile"; mkdir -p "$HR"; ( cd "$HR" && git init -q && git config user.email t@t && git config user.name t \
  && echo 'x' > a.txt && git add a.txt && git commit -qm one ) >/dev/null 2>&1
FORGE="$WORK/forge.sh"
printf '#!/bin/sh\nfor fd in 3 4 5 6 7 8 9; do eval "printf %%s\\\\\\\\n FORGED >&$fd" 2>/dev/null; done\ncat "${1:-/dev/stdin}" 2>/dev/null\nexit 0\n' > "$FORGE"; chmod +x "$FORGE"
( cd "$HR" && git config filter.f.clean "$FORGE" && git config filter.f.smudge cat && git config diff.t.textconv "$FORGE" \
  && git config diff.external "$FORGE" && git config core.fsmonitor "$FORGE" \
  && printf '*.txt filter=f diff=t\n' > .gitattributes && echo 'y' >> a.txt && echo 'z' > b.txt )
( cd "$HR" && VAJRA_WALKTHROUGH_DIR="$HR/data/walkthroughs" bash "$SCRIPT" --run-only hostile "true" 3> "$WORK/hostile.fd3" >/dev/null 2>&1 ); ec=$?
[ "$ec" -eq 0 ] && [ "$(wc -l < "$WORK/hostile.fd3" | tr -d ' ')" = "1" ] && ! grep -q FORGED "$WORK/hostile.fd3" \
  && jq -e '.slug == "hostile" and .commandsTotal == 1' "$WORK/hostile.fd3" >/dev/null
check "run-only: git's repo-configured filters and diff drivers can't write the evidence line" $?
# The forge script really runs under git (else the case above proves nothing): it writes to an fd git gave it.
( cd "$HR" && git add -A ) 3> "$WORK/hostile.probe" >/dev/null 2>&1
grep -q FORGED "$WORK/hostile.probe"; check "run-only: the hostile filter does run and write fd 3 when it is open" $?
( cd "$HR" && git reset -q ) >/dev/null 2>&1
grep -q hi "$RO/cmd-1.log" && [ -f "$RO/WALKTHROUGH.md" ]; check "run-only: logs and walkthrough still written" $?
bash "$SCRIPT" --run-only ro2 "true" >/dev/null 2>&1 3>&-; [ $? -eq 2 ] && [ ! -e "$VAJRA_WALKTHROUGH_DIR/ro2/cmd-1.log" ]
check "run-only: no fd 3 -> exit 2, nothing runs" $?
bash "$SCRIPT" --run-only rored "false" 3> "$WORK/rored.fd3" >/dev/null 2>&1; [ $? -eq 1 ] && jq -e '.allPassed == false' "$WORK/rored.fd3" >/dev/null
check "run-only: a failing command -> exit 1, still on fd 3" $?
# The key is never opened: a FIFO at the key path records any open (a writer unblocks only when a reader opens it).
KP="$HOME/.claude/vajra/.hmac-key"; mv "$KP" "$WORK/key.saved"; mkfifo "$KP"
( exec 4> "$KP"; echo opened > "$WORK/key.opened" ) & WPID=$!
bash "$SCRIPT" --run-only rokey "true" 3> "$WORK/rokey.fd3" >/dev/null 2>&1; ec=$?
sleep 1; kill "$WPID" 2>/dev/null; wait "$WPID" 2>/dev/null
[ "$ec" -eq 0 ] && [ ! -e "$WORK/key.opened" ] && [ -s "$WORK/rokey.fd3" ]; check "run-only: the key is never opened" $?
rm -f "$KP"; mv "$WORK/key.saved" "$KP"
chmod 000 "$KP"
bash "$SCRIPT" --run-only rounread "true" 3> "$WORK/rounread.fd3" >/dev/null 2>&1; [ $? -eq 0 ] && jq -e 'has("hmac") | not' "$WORK/rounread.fd3" >/dev/null
check "run-only: the key unreadable -> still an unsigned bundle" $?
chmod 600 "$KP"

S="$VAJRA_WALKTHROUGH_DIR/ro/evidence.json"; cp "$WORK/ro.fd3" "$S"; H="$(sha_of "$S")"
# sign_as <slug> <sha> <exit> <first stderr line>: the exit code and the fixed first line the core shows (vajra-63).
sign_as() {
  local err rc; err="$(bash "$SCRIPT" --sign "$1" --expect-sha256 "$2" 2>&1 >/dev/null)"; rc=$?
  [ "$rc" -eq "$3" ] && [ "$(printf '%s\n' "$err" | head -n 1)" = "$4" ]
}
sum_others() { (cd "$1" && cat diff.patch diff.stat untracked.txt cmd-*.log WALKTHROUGH.md | python3 -I -c 'import hashlib,sys;print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())'); }
OTHERS="$(sum_others "$VAJRA_WALKTHROUGH_DIR/ro")"
sign_as ro 0000000000000000000000000000000000000000000000000000000000000000 3 "sign refused: sha256 mismatch" && [ "$(sha_of "$S")" = "$H" ]
check "sign: a sha mismatch is refused (exit 3, 'sha256 mismatch'), file untouched" $?
bash "$SCRIPT" --sign ro --expect-sha256 "$H" >/dev/null 2>&1; [ $? -eq 0 ]; check "sign: the recorded sha -> exit 0" $?
verify_ev "$S" && [ "$(jq -r '.commandsTotal' "$S")" = "2" ]; check "sign: the signed file verifies, same content" $?
[ "$(sum_others "$VAJRA_WALKTHROUGH_DIR/ro")" = "$OTHERS" ]; check "sign: the bundle's other files are untouched" $?
[ -z "$(find "$VAJRA_WALKTHROUGH_DIR/ro" -name '*.tmp')" ]; check "sign: no temp file is left" $?
H2="$(sha_of "$S")"; sign_as ro "$H2" 3 "sign refused: already signed"
check "sign: an already signed file is refused ('already signed')" $?
jq -c '.commandsFailed=0 | .allPassed=true | del(.hmac)' "$WORK/rored.fd3" > "$WORK/forged.json"
mkdir -p "$VAJRA_WALKTHROUGH_DIR/rored"; cp "$WORK/forged.json" "$VAJRA_WALKTHROUGH_DIR/rored/evidence.json"
sign_as rored "$(sha_of "$VAJRA_WALKTHROUGH_DIR/rored/evidence.json")" 3 "sign refused: not a bundle for rored"
check "sign: counts that don't match its commands are refused ('not a bundle for <slug>')" $?
jq -c '.slug="other"' "$WORK/ro.fd3" > "$VAJRA_WALKTHROUGH_DIR/rokey/evidence.json"
sign_as rokey "$(sha_of "$VAJRA_WALKTHROUGH_DIR/rokey/evidence.json")" 3 "sign refused: not a bundle for rokey"
check "sign: a bundle for another slug is refused" $?
jq -c '.slug="rounread" | .git.head="'"$(git rev-parse HEAD~1)"'"' "$WORK/ro.fd3" > "$VAJRA_WALKTHROUGH_DIR/rounread/evidence.json"
sign_as rounread "$(sha_of "$VAJRA_WALKTHROUGH_DIR/rounread/evidence.json")" 3 "sign refused: head mismatch"
check "sign: a bundle from another head is refused ('head mismatch')" $?
mkdir -p "$VAJRA_WALKTHROUGH_DIR/nojson"; printf 'not json' > "$VAJRA_WALKTHROUGH_DIR/nojson/evidence.json"
sign_as nojson "$(sha_of "$VAJRA_WALKTHROUGH_DIR/nojson/evidence.json")" 3 "sign refused: not JSON"; check "sign: not JSON" $?
mkdir -p "$VAJRA_WALKTHROUGH_DIR/noev"; sign_as noev "$H" 3 "sign refused: no evidence.json"; check "sign: no evidence.json" $?
ln -s "$S" "$VAJRA_WALKTHROUGH_DIR/noev/evidence.json"
sign_as noev "$H2" 3 "sign refused: no evidence.json"; check "sign: a symlinked evidence.json counts as none" $?
mkdir -p "$VAJRA_WALKTHROUGH_DIR/nokey"; jq -c '.slug="nokey"' "$WORK/ro.fd3" > "$VAJRA_WALKTHROUGH_DIR/nokey/evidence.json"
mv "$KP" "$WORK/key.saved"
sign_as nokey "$(sha_of "$VAJRA_WALKTHROUGH_DIR/nokey/evidence.json")" 4 "sign refused: no usable key"
check "sign: no key -> exit 4 ('no usable key')" $?
mv "$WORK/key.saved" "$KP"
# No refusal line names a path or the key.
for s in ro rored rokey rounread nojson noev; do bash "$SCRIPT" --sign "$s" --expect-sha256 "$H" 2>&1 >/dev/null | head -n 1; done > "$WORK/reasons.txt"
! grep -qE '/|hmac|key-' "$WORK/reasons.txt"; check "sign: no refusal line holds a path or key material" $?
bash "$SCRIPT" --sign "../x" --expect-sha256 "$H" >/dev/null 2>&1; [ $? -eq 2 ]; check "sign: a traversal slug is a usage error" $?
bash "$SCRIPT" --sign ro >/dev/null 2>&1; [ $? -eq 2 ]; check "sign: no --expect-sha256 is a usage error" $?
# bhishma #214 H1: the core signs outside the sandbox, in a folder the agent can write. A planted link at the old fixed
# temp name must not redirect the write.
echo victim > "$WORK/victim.txt"
PL="$VAJRA_WALKTHROUGH_DIR/plant"; mkdir -p "$PL"; jq -c '.slug="plant"' "$WORK/ro.fd3" > "$PL/evidence.json"
ln -s "$WORK/victim.txt" "$PL/evidence.json.tmp"
bash "$SCRIPT" --sign plant --expect-sha256 "$(sha_of "$PL/evidence.json")" >/dev/null 2>&1; ec=$?
[ "$ec" -eq 0 ] && [ "$(cat "$WORK/victim.txt")" = victim ] && [ -f "$PL/evidence.json" ] && [ ! -L "$PL/evidence.json" ] && verify_ev "$PL/evidence.json"
check "sign: a planted link at a temp name is never written through" $?
# M1: a symlinked bundle folder, or a symlinked part above it, is refused; the folder it points at is untouched.
mkdir -p "$WORK/elsewhere"; jq -c '.slug="linked"' "$WORK/ro.fd3" > "$WORK/elsewhere/evidence.json"; ESH="$(sha_of "$WORK/elsewhere/evidence.json")"
ln -s "$WORK/elsewhere" "$VAJRA_WALKTHROUGH_DIR/linked"
sign_as linked "$ESH" 3 "sign refused: a symlinked path" && [ "$(sha_of "$WORK/elsewhere/evidence.json")" = "$ESH" ]
check "sign: a symlinked bundle folder is refused, its target untouched" $?
mkdir -p "$VAJRA_WALKTHROUGH_DIR/up"; jq -c '.slug="up"' "$WORK/ro.fd3" > "$VAJRA_WALKTHROUGH_DIR/up/evidence.json"
ln -s "$VAJRA_WALKTHROUGH_DIR" "$REPO_T/wlink"
# A link in the walkthrough dir is refused, relative or absolute: nothing is resolved.
( cd "$REPO_T" && export VAJRA_WALKTHROUGH_DIR="wlink"; sign_as up "$(sha_of "$REPO_T/wlink/up/evidence.json")" 3 "sign refused: a symlinked path" )
check "sign: a symlinked folder above the bundle is refused" $?
( export VAJRA_WALKTHROUGH_DIR="$REPO_T/wlink"; sign_as up "$(sha_of "$REPO_T/wlink/up/evidence.json")" 3 "sign refused: a symlinked path" )
check "sign: an absolute walkthrough dir through a link is refused" $?
# bhishma #214 (40ee44a): an absolute base inside the agent's checkout whose data folder the agent swapped for a link to
# another folder must not sign in that other folder. Stand-in folders only.
SW="$WORK/swap"; mkdir -p "$SW/repo" "$WORK/swapped/walkthroughs/sw"
jq -c '.slug="sw"' "$WORK/ro.fd3" > "$WORK/swapped/walkthroughs/sw/evidence.json"; SWH="$(sha_of "$WORK/swapped/walkthroughs/sw/evidence.json")"
ln -s "$WORK/swapped" "$SW/repo/data"
( cd "$REPO_T" && export VAJRA_WALKTHROUGH_DIR="$SW/repo/data/walkthroughs"; sign_as sw "$SWH" 3 "sign refused: a symlinked path" ) \
  && [ "$(sha_of "$WORK/swapped/walkthroughs/sw/evidence.json")" = "$SWH" ]
check "sign: an absolute base whose data folder is a planted link is refused, the other folder untouched" $?
# bhishma #214 M1-b: a hard link of a real root-owned relative link (a framework's Versions/Current) as the slug
# folder, its target name an agent symlink to another folder's evidence: refused, nothing signed. Stand-in folders.
RL="$(find /Library/Frameworks /Applications -maxdepth 6 -type l -user root -name Current 2>/dev/null | head -n 1)"
if [ -n "$RL" ] && ln -P "$RL" "$VAJRA_WALKTHROUGH_DIR/hlink" 2>/dev/null; then
  mkdir -p "$WORK/other"; jq -c '.slug="hlink"' "$WORK/ro.fd3" > "$WORK/other/evidence.json"; OH="$(sha_of "$WORK/other/evidence.json")"
  ln -s "$WORK/other" "$VAJRA_WALKTHROUGH_DIR/$(readlink "$RL")"
  sign_as hlink "$OH" 3 "sign refused: a symlinked path" && [ "$(sha_of "$WORK/other/evidence.json")" = "$OH" ]
  check "sign: a hard-linked root-owned link as the slug folder is refused" $?
else
  echo "SKIP: no root-owned relative link on this volume to hard-link"
fi
# A hard-linked evidence.json is refused too.
HL="$VAJRA_WALKTHROUGH_DIR/hardev"; mkdir -p "$HL"; jq -c '.slug="hardev"' "$WORK/ro.fd3" > "$WORK/hardev.json"; ln "$WORK/hardev.json" "$HL/evidence.json"
sign_as hardev "$(sha_of "$HL/evidence.json")" 3 "sign refused: no evidence.json"; check "sign: a hard-linked evidence.json is refused" $?
# L1: a write error is a refusal (exit 3, fixed line), the file untouched, no temp file left.
UW="$VAJRA_WALKTHROUGH_DIR/unwritable"; mkdir -p "$UW"; jq -c '.slug="unwritable"' "$WORK/ro.fd3" > "$UW/evidence.json"; UH="$(sha_of "$UW/evidence.json")"
chmod 555 "$UW"
sign_as unwritable "$UH" 3 "sign refused: cannot write" && [ "$(sha_of "$UW/evidence.json")" = "$UH" ] && [ -z "$(find "$UW" -name '*.tmp')" ]
check "sign: an unwritable folder -> 'cannot write', exit 3, nothing changed" $?
chmod 755 "$UW"
cd "$WORK"
# The default mode signs as the agent itself, but a planted fixed-name link still mustn't redirect its write.
export VAJRA_WALKTHROUGH_DIR="$WORK/wt"; mkdir -p "$WORK/wt/dplant"; ln -s "$WORK/victim.txt" "$WORK/wt/dplant/evidence.json.tmp"
bash "$SCRIPT" dplant "true" >/dev/null 2>&1
[ "$(cat "$WORK/victim.txt")" = victim ] && [ ! -L "$WORK/wt/dplant/evidence.json" ] && verify_ev "$WORK/wt/dplant/evidence.json"
check "default mode: a planted temp-name link is never written through" $?

echo "----------------------------------------"
echo "verify-work: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
