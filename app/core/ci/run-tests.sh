#!/usr/bin/env bash
# The core Rust tests, as CI runs them: every test, in two passes.
#   1. Everything except the tests in ci/serial-tests.txt, in parallel (cargo test's default).
#   2. The tests in ci/serial-tests.txt, one at a time (--test-threads=1), with their assertions and limits unchanged.
# Nothing is retried. A listed name that matches no test fails the run.
#
#   bash app/core/ci/run-tests.sh
set -euo pipefail
cd "$(dirname "$0")/.."

all=$(cargo test --lib -- --list 2>/dev/null | sed -n 's/: test$//p')
[ -n "$all" ] || { echo "run-tests: cargo test --list found no tests"; exit 1; }

serial=()
while IFS= read -r entry; do
    case "$entry" in ''|'#'*) continue ;; esac
    if [[ "$entry" == *:: ]]; then
        matched=$(grep -F -- "$entry" <<<"$all" | grep -E "^${entry//./\\.}" || true)
    else
        matched=$(grep -xF -- "$entry" <<<"$all" || true)
    fi
    [ -n "$matched" ] || { echo "run-tests: ci/serial-tests.txt names no test: $entry"; exit 1; }
    while IFS= read -r t; do serial+=("$t"); done <<<"$matched"
done < ci/serial-tests.txt

skips=()
for t in "${serial[@]}"; do skips+=(--skip "$t"); done

echo "run-tests: pass 1, every test but ${#serial[@]} serial ones, in parallel"
cargo test -- --exact "${skips[@]}"
echo "run-tests: pass 2, ${#serial[@]} tests one at a time"
cargo test --lib -- --exact --test-threads=1 "${serial[@]}"
