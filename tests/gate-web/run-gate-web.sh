#!/usr/bin/env bash
# Test for scripts/gate-web.mjs: clean fixture passes, each bad-*.html fixture
# fails naming exactly its own check id. Exits 77 (skipped) if node, playwright,
# axe-core or Chromium cannot be resolved; run-all.sh reports that as skipped.
# With GATE_WEB_DIR or CI set the deps are expected, so missing ones fail instead.
# Run: bash tests/gate-web/run-gate-web.sh
# GATE_WEB_DIR: directory whose node_modules holds playwright + axe-core
#               (default: current directory, as gate-web.mjs itself resolves).
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$REPO/scripts/gate-web.mjs"
FIX="$REPO/tests/fixtures/gate-web"
skip() {
  if [ -n "${GATE_WEB_DIR:-}${CI:-}" ]; then echo "FAIL :: $1"; exit 1; fi
  echo "SKIP: $1"; exit 77
}
cd "${GATE_WEB_DIR:-$PWD}" || skip "GATE_WEB_DIR not found"

command -v node >/dev/null || skip "gate-web needs node"
node -e "
const r=require('module').createRequire(process.cwd()+'/x.cjs');
r('axe-core');const{chromium}=r('playwright');
(async()=>{const b=await chromium.launch();const p=await b.newPage();
await p.setContent('<title>ok</title>');if(await p.title()!=='ok')process.exit(1);await b.close()})().catch(()=>process.exit(1))
" >/dev/null 2>&1 || skip "gate-web needs playwright, axe-core and a Chromium that launches (npm i -D -E playwright@1.64.1 axe-core@4.14.0)"

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

# A local page must not reach files outside the repo and its own folder, or dot files
# inside it. Each page turns itself into an em-dash failure if the load succeeds.
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/page" "$TMP/outside"
echo 'window.__leak = 1;' > "$TMP/outside/leak.js"
echo 'window.__leak = 1;' > "$TMP/page/.leak.js"
ln -s "$TMP/outside/leak.js" "$TMP/page/link.js"
for src in "file://$TMP/outside/leak.js" ".leak.js" "link.js"; do
  sed "s|SRC|$src|" > "$TMP/page/index.html" <<'HTML'
<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>gate-web probe</title><style>html{background:#f7f5f0;font-family:system-ui,sans-serif}</style></head>
<body><main><h1>probe</h1><p id="out">waiting</p></main>
<script src="SRC"></script>
<script>document.getElementById('out').textContent = window.__leak ? 'loaded \u2014 not contained' : 'blocked';</script>
</body></html>
HTML
  out="$(node "$GATE" "$TMP/page/index.html" 2>&1)"; rc=$?
  [ "$rc" -eq 0 ]; check "local file stays out of reach: $src (got $rc: $out)" $?
done

out="$(node "$GATE" "FILE://$TMP/page/index.html" 2>&1)"; rc=$?
[ "$rc" -eq 0 ]; check "containment holds for a FILE:// target (got $rc: $out)" $?
out="$(node "$GATE" "view-source:file://$TMP/page/index.html" 2>&1)"; rc=$?
[ "$rc" -eq 2 ]; check "other schemes are refused (got $rc: $out)" $?

echo "gate-web: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
