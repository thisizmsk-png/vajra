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
SKIPPED=""
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

# Tripwire: shell suites get a decoy HOME, and the real home's protected files are
# fingerprinted before and after. Any change fails the run (tests/lib/home-guard.sh).
. "$REPO/tests/lib/home-guard.sh"
REAL_HOME="$HOME"
if [ -z "${PLAYWRIGHT_BROWSERS_PATH:-}" ]; then
  case "$(uname -s)" in
    Darwin) export PLAYWRIGHT_BROWSERS_PATH="$REAL_HOME/Library/Caches/ms-playwright" ;;
    *) export PLAYWRIGHT_BROWSERS_PATH="${XDG_CACHE_HOME:-$REAL_HOME/.cache}/ms-playwright" ;;
  esac
fi
HOME_BEFORE="$(home_fp "$REAL_HOME")"
DECOY_HOME="$(mktemp -d)"
trap 'rm -rf "$DECOY_HOME"' EXIT
export HOME="$DECOY_HOME"

for t in $SUITES; do
  case "$t" in *"$FILTER"*) ;; *) continue ;; esac
  section "$t"
  RAN=$((RAN+1))
  bash "$t"; rc=$?
  case "$rc" in
    0) ;;
    77) SKIPPED="$SKIPPED $t" ;;
    *) FAILED=1; FAILED_SUITES="$FAILED_SUITES $t" ;;
  esac
done

export HOME="$REAL_HOME"
if [ "$HOME_BEFORE" != "$(home_fp "$REAL_HOME")" ]; then
  echo "FAIL: a test changed the real home (Vajra state, Claude settings or agents):"
  diff <(printf '%s\n' "$HOME_BEFORE") <(home_fp "$REAL_HOME")
  FAILED=1; FAILED_SUITES="$FAILED_SUITES real-home-tripwire"
fi

printf '\n========================================\n'
echo "suites run: $RAN"
[ -z "$SKIPPED" ] || echo "SKIPPED, not proven:$SKIPPED"
if [ "$FAILED" -eq 0 ] && [ -n "$SKIPPED" ]; then echo "PASSED WITH SKIPS"; elif [ "$FAILED" -eq 0 ]; then echo "ALL SUITES PASSED"; else echo "FAILED:$FAILED_SUITES"; fi
exit "$FAILED"
