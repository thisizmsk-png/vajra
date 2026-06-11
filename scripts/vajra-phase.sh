#!/usr/bin/env bash
# vajra-phase.sh — the actuator for explore-plan-act phase enforcement.
#
# The pre-tool-use hook reads ~/.claude/vajra/.phase and blocks Write/Edit and
# mutating Bash while the phase is `explore` or `plan` (read-only). This script
# is what sets that file, so Plan Mode is actually reachable:
#
#   vajra-phase.sh set explore|plan|act
#   vajra-phase.sh get
#   vajra-phase.sh clear        # remove the flag (no phase gating)
#
# The .phase file lives in ~/.claude/vajra/ (runtime state, not the immutable
# core), so writing it is permitted by the hook.

set -uo pipefail

VAJRA_DIR="${HOME}/.claude/vajra"
PHASE_FILE="${VAJRA_DIR}/.phase"
mkdir -p "$VAJRA_DIR"

cmd="${1:-get}"

case "$cmd" in
  set)
    phase="${2:-}"
    case "$phase" in
      explore|plan|act) ;;
      *) echo "usage: vajra-phase.sh set explore|plan|act" >&2; exit 2 ;;
    esac
    printf '%s' "$phase" > "$PHASE_FILE"
    chmod 600 "$PHASE_FILE" 2>/dev/null || true
    if [ "$phase" = "act" ]; then
      echo "Phase → act. Write/Edit and mutating Bash are now allowed (execute the approved plan)."
    else
      echo "Phase → ${phase}. Read-only: Write/Edit and mutating Bash are blocked until /vajra act."
    fi
    ;;
  get)
    if [ -f "$PHASE_FILE" ]; then
      cat "$PHASE_FILE"; echo
    else
      echo "act"   # no flag = unrestricted (default)
    fi
    ;;
  clear)
    rm -f "$PHASE_FILE" 2>/dev/null || true
    echo "Phase flag cleared (no phase gating)."
    ;;
  *)
    echo "usage: vajra-phase.sh set explore|plan|act | get | clear" >&2
    exit 2
    ;;
esac
