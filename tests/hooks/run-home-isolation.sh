#!/usr/bin/env bash
# Proves the real-home tripwire used by tests/run-all.sh: a test that writes Vajra's
# state, Claude settings or agents under the real HOME is detected. Uses a fake home.
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
. "$REPO/tests/lib/home-guard.sh" || { echo "FAIL: tests/lib/home-guard.sh missing"; exit 1; }
P=0; F=0
ok() { P=$((P+1)); }; bad() { F=$((F+1)); echo "FAIL: $1"; }
H="$(mktemp -d)"; trap 'rm -rf "$H"' EXIT
mkdir -p "$H/.claude/vajra" "$H/.claude/agents" "$H/.claude/skills/vajra"
echo sig > "$H/.claude/vajra/manifest.sig"; echo '{}' > "$H/.claude/settings.json"
echo a > "$H/.claude/agents/a.md"; echo '{}' > "$H/.claude/skills/vajra/manifest.json"
echo k > "$H/.claude/vajra/.hmac-key"
base="$(home_fp "$H")"
[ "$base" = "$(home_fp "$H")" ] && ok || bad "fingerprint not stable"
case "$base" in *hmac*) bad "fingerprint mentions the HMAC key" ;; *) ok ;; esac
for t in "echo x > \"\$H/.claude/vajra/manifest.sig\"" \
         "echo '{\"a\":1}' > \"\$H/.claude/settings.json\"" \
         "echo b > \"\$H/.claude/agents/b.md\"" \
         "echo '{\"x\":1}' > \"\$H/.claude/skills/vajra/manifest.json\"" \
         "touch \"\$H/.claude/vajra/.integrity-violation\""; do
  before="$(home_fp "$H")"; sleep 1; eval "$t"
  [ "$before" != "$(home_fp "$H")" ] && ok || bad "rogue write not detected: $t"
done
grep -q 'home-guard.sh' "$REPO/tests/run-all.sh" && ok || bad "run-all.sh does not use the tripwire"
grep -q 'export HOME=' "$REPO/tests/run-all.sh" && ok || bad "run-all.sh does not give suites a decoy HOME"
echo "home isolation: $P passed, $F failed"; [ "$F" -eq 0 ]
