#!/usr/bin/env bash
# vajra-ralph.sh — hard-reset iteration loop ("Ralph"): each iteration runs a
# FRESH agent that carries forward only a durable progress file, instead of
# growing/compacting one context. Drift-resistant for long autonomous runs.
#
#   vajra-ralph.sh <progress-file> [max-iters]
#
# Each iteration: fresh `claude -p` reads progress.md (+ repo map if present),
# does ONE unit of work, updates progress.md, exits. The loop stops when the file
# has no unchecked `- [ ]` items or contains the marker RALPH-COMPLETE, or when
# max-iters is hit. The agent command is $CLAUDE_CMD (default: claude) so it can
# be mocked in tests.

set -uo pipefail

PROGRESS="${1:-}"
MAX="${2:-25}"
[ -z "$PROGRESS" ] && { echo "usage: vajra-ralph.sh <progress-file> [max-iters]" >&2; exit 2; }
[ -f "$PROGRESS" ] || { echo "progress file not found: $PROGRESS" >&2; exit 2; }

CLAUDE_CMD="${CLAUDE_CMD:-claude}"
ITER=0

is_done() {
  grep -q 'RALPH-COMPLETE' "$PROGRESS" 2>/dev/null && return 0
  # done if there are no unchecked todo boxes left
  ! grep -qE '^[[:space:]]*-[[:space:]]*\[ \]' "$PROGRESS" 2>/dev/null
}

echo "Ralph loop on ${PROGRESS} (max ${MAX} iterations, agent: ${CLAUDE_CMD})"
while : ; do
  if is_done; then
    echo "✅ Ralph: progress complete after ${ITER} iteration(s)."
    exit 0
  fi
  if [ "$ITER" -ge "$MAX" ]; then
    echo "⏹ Ralph: hit max ${MAX} iterations with work remaining. Inspect ${PROGRESS}." >&2
    exit 1
  fi
  ITER=$((ITER+1))
  echo "── iteration ${ITER} ──"

  # Compose the iteration prompt. Fresh context every time; durable state is the
  # progress file (+ an optional repo map the agent can regenerate).
  PROMPT="$(cat <<PROMPT_EOF
You are one iteration of a Ralph loop. Your entire memory is the progress file
below — there is no prior conversation. Do the SINGLE most valuable next unit of
work toward the goal, then update the progress file (check off what you finished,
add any newly-discovered steps), and stop. Do not try to finish everything.

When the goal is fully achieved, add the line: RALPH-COMPLETE

--- progress file (${PROGRESS}) ---
$(cat "$PROGRESS")
PROMPT_EOF
)"

  # Run a fresh agent. Non-interactive print mode; the agent edits the progress
  # file as part of its work. Failures don't abort the loop (next iteration
  # re-reads the durable state).
  "$CLAUDE_CMD" -p "$PROMPT" >/dev/null 2>&1 || echo "  (iteration ${ITER} agent exited non-zero; continuing)" >&2
done
