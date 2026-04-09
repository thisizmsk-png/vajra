#!/usr/bin/env bash
# Vajra PreToolUse hook — blocks dangerous tool invocations.
# Claude Code pipes JSON with { tool, input } on stdin.
# Exit 0 = allow, Exit 1 = block.

set -euo pipefail

AUDIT_LOG="${HOME}/.claude/vajra/audit.log"
mkdir -p "$(dirname "$AUDIT_LOG")"

TIMESTAMP="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

# Read the hook payload from stdin
PAYLOAD="$(cat)"

# Parse JSON — prefer jq, fall back to grep, FAIL-CLOSED on parse failure
if command -v jq &>/dev/null; then
  TOOL_NAME="$(echo "$PAYLOAD" | jq -r '.tool // empty' 2>/dev/null || echo "")"
  INPUT_RAW="$(echo "$PAYLOAD" | jq -r '.input // empty' 2>/dev/null || echo "")"
else
  # No jq — fail-closed: block and warn
  echo "${TIMESTAMP} BLOCK unknown-tool reason=\"jq not installed — fail-closed\"" >> "$AUDIT_LOG"
  echo "Vajra hook: jq is required for safe tool validation. Install jq or allow in settings." >&2
  exit 1
fi

# FAIL-CLOSED: if we can't parse the tool name, block the operation
if [ -z "$TOOL_NAME" ]; then
  echo "${TIMESTAMP} BLOCK unknown-tool reason=\"parse failure — fail-closed\"" >> "$AUDIT_LOG"
  echo "Vajra hook: blocked unknown tool (could not parse payload)" >&2
  exit 1
fi

# Combine tool + input for pattern matching
CHECK_STRING="${TOOL_NAME} ${INPUT_RAW} ${PAYLOAD}"

BLOCKED=false
REASON=""

# --- Phase enforcement (explore-plan-act) ---
PHASE_FILE="${HOME}/.claude/vajra/.phase"
if [ -f "$PHASE_FILE" ]; then
  CURRENT_PHASE="$(cat "$PHASE_FILE" 2>/dev/null | tr -d '[:space:]')"
  if [ "$CURRENT_PHASE" = "explore" ] || [ "$CURRENT_PHASE" = "plan" ]; then
    # Block write operations during EXPLORE and PLAN phases
    case "$TOOL_NAME" in
      Write|Edit|NotebookEdit)
        BLOCKED=true
        REASON="blocked by ${CURRENT_PHASE} phase — read-only until /vajra act"
        ;;
      Bash)
        # Block destructive bash commands during explore/plan
        if echo "$INPUT_RAW" | grep -qE '(mkdir|touch|cp |mv |rm |chmod|chown|tee |sed -i|git add|git commit|git push|git checkout|git reset|npm install|pip install|cargo add)' 2>/dev/null; then
          BLOCKED=true
          REASON="blocked by ${CURRENT_PHASE} phase — no mutations until /vajra act"
        fi
        # Block output redirects
        if echo "$INPUT_RAW" | grep -qE '(>>|>[^&])' 2>/dev/null; then
          BLOCKED=true
          REASON="blocked by ${CURRENT_PHASE} phase — no file writes until /vajra act"
        fi
        ;;
    esac
  fi
fi

# --- Dangerous pattern checks ---

# 1. rm -rf / (root filesystem wipe)
if echo "$CHECK_STRING" | grep -qE 'rm\s+(-[a-zA-Z]*r[a-zA-Z]*f|--recursive\s+--force|-[a-zA-Z]*f[a-zA-Z]*r)\s+/[^a-zA-Z]' 2>/dev/null; then
  BLOCKED=true
  REASON="rm -rf on root filesystem"
fi

if echo "$CHECK_STRING" | grep -qE 'rm\s+(-[a-zA-Z]*r[a-zA-Z]*f|--recursive\s+--force|-[a-zA-Z]*f[a-zA-Z]*r)\s+/$' 2>/dev/null; then
  BLOCKED=true
  REASON="rm -rf on root filesystem"
fi

# 2. git push --force (to any branch)
if echo "$CHECK_STRING" | grep -qE 'git\s+push\s+.*--force' 2>/dev/null; then
  BLOCKED=true
  REASON="git push --force"
fi

if echo "$CHECK_STRING" | grep -qE 'git\s+push\s+-f\b' 2>/dev/null; then
  BLOCKED=true
  REASON="git push --force (-f shorthand)"
fi

# 3. git reset --hard on main/master
if echo "$CHECK_STRING" | grep -qE 'git\s+reset\s+--hard.*(main|master)' 2>/dev/null; then
  BLOCKED=true
  REASON="git reset --hard on main/master"
fi

# 4. chmod 777 on sensitive paths
if echo "$CHECK_STRING" | grep -qE 'chmod\s+777\s+/' 2>/dev/null; then
  BLOCKED=true
  REASON="chmod 777 on root path"
fi

# 5. dd writing to block devices
if echo "$CHECK_STRING" | grep -qE 'dd\s+.*of=/dev/(sd|hd|nvme|vd)' 2>/dev/null; then
  BLOCKED=true
  REASON="dd write to block device"
fi

# 6. mkfs on devices (only block when Bash tool is invoking it)
if [ "$TOOL_NAME" = "Bash" ] && echo "$CHECK_STRING" | grep -qE '\bmkfs\b' 2>/dev/null; then
  BLOCKED=true
  REASON="filesystem format command via Bash"
fi

# 7. --no-preserve-root (strong signal of destructive intent)
if echo "$CHECK_STRING" | grep -qE '\b--no-preserve-root\b' 2>/dev/null; then
  BLOCKED=true
  REASON="--no-preserve-root flag detected"
fi

# 8. Pipe to shell (command injection vector)
if echo "$CHECK_STRING" | grep -qE '\|\s*(ba)?sh\b' 2>/dev/null; then
  BLOCKED=true
  REASON="pipe to shell detected"
fi

# 9. eval execution
if [ "$TOOL_NAME" = "Bash" ] && echo "$CHECK_STRING" | grep -qE '\beval\s+' 2>/dev/null; then
  BLOCKED=true
  REASON="eval execution detected"
fi

# 10. find -delete (recursive file deletion)
if echo "$CHECK_STRING" | grep -qE 'find\s.*-delete\b' 2>/dev/null; then
  BLOCKED=true
  REASON="find -delete detected"
fi

# 11. chmod on sensitive dotfiles
if echo "$CHECK_STRING" | grep -qE 'chmod\s+777\s+~/\.' 2>/dev/null; then
  BLOCKED=true
  REASON="chmod 777 on sensitive dotfiles"
fi

# --- Log and decide ---

if [ "$BLOCKED" = true ]; then
  # Sanitize reason for log injection (strip newlines, control chars)
  SAFE_REASON="$(echo "$REASON" | tr -d '\n\r' | tr -cd '[:print:]')"
  echo "${TIMESTAMP} BLOCK ${TOOL_NAME} reason=\"${SAFE_REASON}\"" >> "$AUDIT_LOG"
  # Print message for user visibility
  echo "Vajra hook blocked dangerous operation: ${REASON}" >&2
  exit 1
else
  echo "${TIMESTAMP} ALLOW ${TOOL_NAME}" >> "$AUDIT_LOG"
  exit 0
fi
