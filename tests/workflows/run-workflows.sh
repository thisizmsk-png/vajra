#!/usr/bin/env bash
# Runs the offline node stub-harness tests for every workflow script (workflows/vajra-*.js).
# Exits non-zero if any scenario fails, so tests/run-all.sh and CI pick it up.
# Run: bash tests/workflows/run-workflows.sh
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v node >/dev/null 2>&1; then echo "SKIP: node not installed"; exit 0; fi

FAIL=0
for t in "$DIR"/*.test.mjs; do
  [ -f "$t" ] || continue
  echo "--- $(basename "$t") ---"
  if node "$t"; then :; else FAIL=1; fi
done

echo "----------------------------------------"
if [ "$FAIL" -eq 0 ]; then echo "workflows: all scenarios passed"; else echo "workflows: SOME scenarios FAILED"; fi
exit "$FAIL"
