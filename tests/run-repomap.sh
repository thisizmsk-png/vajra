#!/usr/bin/env bash
# Test for scripts/vajra_repomap.py — relevance ranking via PageRank.
# Run: bash tests/run-repomap.sh
set -uo pipefail
command -v python3 >/dev/null || { echo "SKIP: python3 required"; exit 0; }
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO/scripts/vajra_repomap.py"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vajra-rmap-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
SRC="$WORK/src"; mkdir -p "$SRC"

# core.py is referenced by a, b, c → should rank highest.
cat > "$SRC/core.py" <<'EOF'
def helper(x):
    return x
class Core:
    pass
EOF
cat > "$SRC/a.py" <<'EOF'
from core import helper, Core
def run_a():
    return helper(Core())
EOF
cat > "$SRC/b.py" <<'EOF'
from core import helper, Core
def run_b():
    return helper(Core())
EOF
cat > "$SRC/c.py" <<'EOF'
from core import helper
def run_c():
    return helper(1)
EOF

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

J="$(python3 "$SCRIPT" --json --root "$SRC" 2>/dev/null)"
[ -n "$J" ]; check "json output produced" $?
[ "$(echo "$J" | jq '.filesScanned')" = "4" ]; check "4 files scanned" $?
top="$(echo "$J" | jq -r '.map[0].file')"
[ "$top" = "core.py" ]; check "most-referenced file (core.py) ranks #1" $?
echo "$J" | jq -r '.map[0].defs[]' | grep -qx "helper"; check "core.py defs include helper" $?
echo "$J" | jq -r '.map[0].defs[]' | grep -qx "Core"; check "core.py defs include Core" $?

# seed personalization boosts a.py into the map
Js="$(python3 "$SCRIPT" --json --root "$SRC" --seed a.py 2>/dev/null)"
echo "$Js" | jq -r '.map[].file' | grep -qx "a.py"; check "seed a.py appears in seeded map" $?

# tiny budget truncates
Jb="$(python3 "$SCRIPT" --json --root "$SRC" --budget 3 2>/dev/null)"
mapped="$(echo "$Jb" | jq '.map | length')"
[ "$mapped" -lt 4 ]; check "tiny budget truncates the map (<4)" $?
[ "$mapped" -ge 1 ]; check "tiny budget still returns at least 1" $?

# human-readable runs
python3 "$SCRIPT" --root "$SRC" >/dev/null 2>&1; check "human-readable output exits 0" $?

echo "----------------------------------------"
echo "repo-map: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
