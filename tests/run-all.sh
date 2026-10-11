#!/usr/bin/env bash
# Vajra test runner: vitest (if installed) plus every tests/**/run-*.sh suite.
# Each suite exits non-zero on failure. Bash 3.2 compatible (no globstar).
#   bash tests/run-all.sh            # everything
#   bash tests/run-all.sh router     # only suites whose path contains "router"
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
FILTER="${1:-}"

FAILED=0
FAILED_SUITES=""
RAN=0
section() { printf '\n=== %s ===\n' "$1"; }

if [ -z "$FILTER" ]; then
  section "TypeScript (vitest)"
  if [ -x node_modules/.bin/vitest ]; then
    VOUT="$(node_modules/.bin/vitest run 2>&1)"; VRC=$?
    printf '%s\n' "$VOUT" | tail -6
    [ "$VRC" -eq 0 ] || { FAILED=1; FAILED_SUITES="$FAILED_SUITES vitest"; }
  elif [ -n "${CI:-}" ]; then
    echo "FAIL: vitest not installed in CI (run: npm install)"; FAILED=1; FAILED_SUITES="$FAILED_SUITES vitest"
  else
    echo "SKIP: vitest not installed (run: npm install)"
  fi
fi

SUITES="$(find tests -type f -name 'run-*.sh' ! -name 'run-all.sh' ! -path '*/node_modules/*' | LC_ALL=C sort)"
[ -f tests/hmac-contract.sh ] && SUITES="$SUITES
tests/hmac-contract.sh"

for t in $SUITES; do
  case "$t" in *"$FILTER"*) ;; *) continue ;; esac
  section "$t"
  RAN=$((RAN+1))
  if bash "$t"; then :; else FAILED=1; FAILED_SUITES="$FAILED_SUITES $t"; fi
done

printf '\n========================================\n'
echo "suites run: $RAN"
if [ "$FAILED" -eq 0 ]; then echo "ALL SUITES PASSED"; else echo "FAILED:$FAILED_SUITES"; fi
exit "$FAILED"
