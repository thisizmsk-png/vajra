#!/usr/bin/env bash
# Vajra full test runner — TypeScript (vitest) + portable shell suites.
# No bats dependency. Run: bash tests/run-all.sh
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

FAILED=0
section() { printf '\n=== %s ===\n' "$1"; }

section "TypeScript (vitest)"
if [ -x node_modules/.bin/vitest ]; then
  node_modules/.bin/vitest run 2>&1 | tail -6 || FAILED=1
else
  echo "SKIP: vitest not installed (run: npm install)"
fi

for t in \
  tests/hooks/run-pre-tool-use.sh \
  tests/hooks/run-session-start.sh \
  tests/hooks/run-post-tool-use.sh \
  tests/hooks/run-phase-mode.sh \
  tests/run-verify-work.sh \
  tests/run-event-log.sh \
  tests/fleet/run-relay-auth.sh \
; do
  section "$t"
  if bash "$t"; then :; else FAILED=1; fi
done

if [ -f tests/hmac-contract.sh ]; then
  section "tests/hmac-contract.sh"
  if bash tests/hmac-contract.sh >/dev/null 2>&1; then echo "hmac-contract: OK"; else echo "hmac-contract: FAIL"; FAILED=1; fi
fi

printf '\n========================================\n'
if [ "$FAILED" -eq 0 ]; then echo "ALL SUITES PASSED"; else echo "SOME SUITES FAILED"; fi
exit "$FAILED"
