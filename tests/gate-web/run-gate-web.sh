#!/usr/bin/env bash
# Test for scripts/gate-web.mjs: clean fixture passes, each bad-*.html fixture
# fails naming exactly its own check id. Exits 77 (skipped) if node, playwright,
# axe-core or Chromium cannot be resolved; run-all.sh reports that as skipped.
# Run: bash tests/gate-web/run-gate-web.sh
# GATE_WEB_DIR: directory whose node_modules holds playwright + axe-core
#               (default: current directory, as gate-web.mjs itself resolves).
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$REPO/scripts/gate-web.mjs"
FIX="$REPO/tests/fixtures/gate-web"
cd "${GATE_WEB_DIR:-$PWD}" || { echo "SKIP: GATE_WEB_DIR not found"; exit 77; }

command -v node >/dev/null || { echo "SKIP: gate-web needs node"; exit 77; }
node -e "
const r=require('module').createRequire(process.cwd()+'/x.cjs');
r('axe-core');const{chromium}=r('playwright');
(async()=>{const b=await chromium.launch();const p=await b.newPage();
await p.setContent('<title>ok</title>');if(await p.title()!=='ok')process.exit(1);await b.close()})().catch(()=>process.exit(1))
" >/dev/null 2>&1 || { echo "SKIP: gate-web needs playwright, axe-core and a Chromium that launches (npm i playwright axe-core)"; exit 77; }

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

for f in "$FIX"/clean*.html; do
  out="$(node "$GATE" "$f" 2>&1)"; rc=$?
  [ "$rc" -eq 0 ]; check "$(basename "$f") exits 0 (got $rc: $out)" $?
done

for f in "$FIX"/bad-*.html; do
  id="$(basename "$f" .html)"; id="${id#bad-}"
  out="$(node "$GATE" "$f" 2>&1)"; rc=$?
  [ "$rc" -eq 1 ]; check "bad-$id exits 1 (got $rc)" $?
  ids="$(printf '%s\n' "$out" | sed -n 's/^FAIL \([a-z0-9-]*\):.*/\1/p' | sort -u | tr '\n' ' ')"
  [ "$ids" = "$id " ]; check "bad-$id names exactly '$id' (got '$ids')" $?
done

echo "gate-web: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
