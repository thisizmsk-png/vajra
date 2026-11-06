#!/usr/bin/env bash
# Desktop UI kit (K1, K2): typecheck, design guard, contrast tables, component accessibility tests.
# Run: bash tests/ui/run-ui.sh
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
command -v npm >/dev/null || { echo "SKIP: npm not installed"; exit 0; }
cd "$REPO/app/ui" || exit 1
[ -d node_modules/vitest ] || npm install --no-audit --no-fund >/dev/null 2>&1 || { echo "FAIL :: npm install"; exit 1; }
npx tsc --noEmit || { echo "ui: typecheck FAILED"; exit 1; }
npx vitest run 2>&1 | tail -8
exit "${PIPESTATUS[0]}"
