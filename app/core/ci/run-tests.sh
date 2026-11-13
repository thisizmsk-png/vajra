#!/usr/bin/env bash
# The core Rust tests, as CI runs them: every test, in two passes.
#   1. Everything except the tests in ci/serial-tests.txt, in parallel (cargo test's default).
#   2. The tests in ci/serial-tests.txt, one at a time (--test-threads=1), with their assertions and limits unchanged.
# Nothing is retried. A listed name that matches no test fails the run. Both passes always run; the run fails if
# either one did.
#
#   bash app/core/ci/run-tests.sh
set -euo pipefail
cd "$(dirname "$0")/.."

# Build first, with cargo's output shown, so a compile error (or a missing app/ui/dist) is in the log (#182 review).
cargo test --no-run || { echo "run-tests: the test build failed (see cargo's output above)"; exit 1; }
listing=$(cargo test --lib -- --list) || { echo "run-tests: listing the tests failed (cargo test --lib -- --list)"; exit 1; }
all=$(sed -n 's/: test$//p' <<<"$listing")
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

# Empty-array safe on macOS's bash 3.2 under set -u: "${a[@]+"${a[@]}"}" expands to nothing for an empty array.
skips=()
for t in ${serial[@]+"${serial[@]}"}; do skips+=(--skip "$t"); done

# Both passes always run, so a failure in pass 1 doesn't hide pass 2's results; the run fails if either failed.
echo "run-tests: pass 1, every test but ${#serial[@]} serial ones, in parallel"
p1=0
cargo test -- --exact ${skips[@]+"${skips[@]}"} || p1=$?
# With no serial tests there is no pass 2: an unfiltered --test-threads=1 run would run the whole suite again.
p2=0
if [ "${#serial[@]}" -eq 0 ]; then
    echo "run-tests: pass 2 skipped, ci/serial-tests.txt lists no tests"
else
    echo "run-tests: pass 2, ${#serial[@]} tests one at a time"
    cargo test --lib -- --exact --test-threads=1 "${serial[@]}" || p2=$?
fi
echo "run-tests: pass 1 $([ "$p1" -eq 0 ] && echo passed || echo "FAILED ($p1)"), pass 2 $([ "$p2" -eq 0 ] && echo passed || echo "FAILED ($p2)")"
[ "$p1" -eq 0 ] && [ "$p2" -eq 0 ]
