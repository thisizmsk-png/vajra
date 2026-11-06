#!/usr/bin/env bash
# generate-manifest.sh signs the real signature only for the installed skill and only for the
# operator (a TTY on stdin, or VAJRA_OPERATOR_SIGN=1, which the guard blocks in tool calls).
# Regression for 2026-11-05: a worktree copy re-signed ~/.claude/vajra/manifest.sig.
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
export HOME="$T/home"
VS="$HOME/.claude/vajra"; SKILL="$HOME/.claude/skills/vajra"; WT="$T/worktree"
mkdir -p "$VS" "$SKILL/scripts" "$WT/scripts"
( umask 077; openssl rand -hex 32 > "$VS/.hmac-key" )
for d in "$SKILL" "$WT"; do cp "$REPO/scripts/generate-manifest.sh" "$d/scripts/"; echo x > "$d/a.txt"; done
P=0; F=0; check() { if [ "$2" -eq 0 ]; then P=$((P+1)); else F=$((F+1)); echo "FAIL :: $1"; fi; }

touch "$VS/.integrity-violation"
out="$(bash "$WT/scripts/generate-manifest.sh" < /dev/null 2>&1)"; rc=$?
check "worktree copy exits 0" "$rc"
[ -f "$WT/manifest.json" ]; check "worktree copy still writes its manifest.json" $?
[ ! -e "$VS/manifest.sig" ]; check "worktree copy does not sign the real signature" $?
[ -e "$VS/.integrity-violation" ]; check "worktree copy does not clear the tamper flag" $?
printf '%s' "$out" | grep -q "not signed"; check "worktree copy says it did not sign" $?

VAJRA_OPERATOR_SIGN=1 bash "$WT/scripts/generate-manifest.sh" < /dev/null >/dev/null 2>&1
[ ! -e "$VS/manifest.sig" ]; check "operator flag does not let a worktree copy sign" $?

bash "$SKILL/scripts/generate-manifest.sh" < /dev/null >/dev/null 2>&1
[ ! -e "$VS/manifest.sig" ]; check "installed copy without a TTY or the flag does not sign" $?

echo '{"files":{}}' > "$SKILL/manifest.json"; before="$(shasum -a 256 < "$SKILL/manifest.json")"
bash "$SKILL/scripts/generate-manifest.sh" < /dev/null >/dev/null 2>&1
[ "$before" = "$(shasum -a 256 < "$SKILL/manifest.json")" ]; check "installed copy without TTY/flag leaves manifest.json untouched" $?

VAJRA_OPERATOR_SIGN=1 bash "$SKILL/scripts/generate-manifest.sh" < /dev/null >/dev/null 2>&1
[ -s "$VS/manifest.sig" ]; check "installed copy with the operator flag signs" $?
[ ! -e "$VS/.integrity-violation" ]; check "operator re-sign clears the tamper flag" $?

echo "generate-manifest: $P passed, $F failed"; [ "$F" -eq 0 ]
