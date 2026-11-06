#!/usr/bin/env bash
# E9 (ADR-12 step 4, AC-7.1.8): the Vajra Desktop approval path for vajra_kb.py and atman.sh.
# Runs in a temp HOME with its own HMAC key; never touches ~/.claude or the real app data dir.
# Run: bash tests/kb/run-desktop-approval.sh
set -uo pipefail
for c in python3 git openssl; do command -v "$c" >/dev/null || { echo "SKIP: $c required"; exit 0; }; done
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-e9-XXXXXX")"
trap 'rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"
unset VAJRA_ATMAN_ASSUME_HUMAN CLAUDECODE
mkdir -p "$HOME/.claude/vajra"
( umask 077; openssl rand -hex 32 | tr -d '\n' > "$HOME/.claude/vajra/.hmac-key" )
git config --global user.email t@t 2>/dev/null; git config --global user.name t 2>/dev/null
python3 "$REPO/tests/kb/desktop_approval_test.py" < /dev/null
