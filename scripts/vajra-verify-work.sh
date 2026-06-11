#!/usr/bin/env bash
# vajra-verify-work.sh — capture proof-of-work evidence before claiming "done".
#
# Runs each verification command, captures its exit code + output, records the
# git diff, and writes a walkthrough bundle. Exits NON-ZERO if any command
# failed — so "done" can be gated on real, captured evidence instead of the
# agent's say-so (the #1 trust failure in agent harnesses).
#
#   vajra-verify-work.sh <slug> "<cmd1>" "<cmd2>" ...
#
# Output: <base>/<slug>/  with evidence.json, cmd-N.log, diff.patch, WALKTHROUGH.md
# Base dir: $VAJRA_WALKTHROUGH_DIR (default ./data/walkthroughs).

set -uo pipefail

SLUG="${1:-}"
[ -z "$SLUG" ] && { echo "usage: vajra-verify-work.sh <slug> \"<cmd>\" [\"<cmd>\" ...]" >&2; exit 2; }
shift

BASE="${VAJRA_WALKTHROUGH_DIR:-data/walkthroughs}"
DIR="${BASE}/${SLUG}"
mkdir -p "$DIR"
TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

have_jq=0; command -v jq >/dev/null 2>&1 && have_jq=1

# Capture the diff (best-effort; empty if not a repo)
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  git diff > "${DIR}/diff.patch" 2>/dev/null || true
  git diff --stat > "${DIR}/diff.stat" 2>/dev/null || true
fi

n=0; failed=0
entries=""
for cmd in "$@"; do
  n=$((n+1))
  log="${DIR}/cmd-${n}.log"
  echo "\$ ${cmd}" > "$log"
  # Run the command, tee combined output, record exit code.
  bash -c "$cmd" >> "$log" 2>&1
  ec=$?
  [ "$ec" -ne 0 ] && failed=$((failed+1))
  printf '  [%s] cmd %d (exit %d): %s\n' "$([ "$ec" -eq 0 ] && echo PASS || echo FAIL)" "$n" "$ec" "$cmd"
  if [ "$have_jq" -eq 1 ]; then
    entry="$(jq -nc --arg c "$cmd" --argjson e "$ec" --arg l "cmd-${n}.log" '{cmd:$c, exit:$e, log:$l, passed:($e==0)}')"
    entries="${entries}${entries:+,}${entry}"
  fi
done

# evidence.json
if [ "$have_jq" -eq 1 ]; then
  jq -n --arg slug "$SLUG" --arg ts "$TS" --argjson total "$n" --argjson failed "$failed" \
        --argjson cmds "[${entries}]" \
        '{slug:$slug, generatedAt:$ts, commandsTotal:$total, commandsFailed:$failed, allPassed:($failed==0), commands:$cmds}' \
        > "${DIR}/evidence.json"
fi

# WALKTHROUGH.md
{
  echo "# Walkthrough: ${SLUG}"
  echo "generated: ${TS}"
  echo "result: $([ "$failed" -eq 0 ] && echo '✅ all checks passed' || echo "❌ ${failed}/${n} checks FAILED")"
  echo
  echo "## Verification commands"
  i=0
  for cmd in "$@"; do
    i=$((i+1))
    echo "- cmd ${i}: \`${cmd}\` → see cmd-${i}.log"
  done
  echo
  [ -f "${DIR}/diff.stat" ] && { echo "## Changes"; echo '```'; cat "${DIR}/diff.stat"; echo '```'; echo; }
  echo "## How to reproduce"
  echo "Re-run each command above from the repo root; outputs are captured in cmd-N.log."
} > "${DIR}/WALKTHROUGH.md"

echo "----------------------------------------"
if [ "$failed" -eq 0 ]; then
  echo "Verification PASSED (${n}/${n}). Evidence: ${DIR}"
  exit 0
else
  echo "Verification FAILED (${failed}/${n}). Do NOT claim done. Evidence: ${DIR}"
  exit 1
fi
