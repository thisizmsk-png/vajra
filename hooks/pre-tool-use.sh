#!/usr/bin/env bash
# Vajra PreToolUse hook — blocks dangerous tool invocations.
# Claude Code pipes JSON with { tool, input } (or { tool_name, tool_input }) on stdin.
# Exit 0 = allow, Exit 1 = block.
#
# Defense model (see redteam findings VJR-01, VJR-02):
#   1. IMMUTABLE CORE  — the harness may not modify its own policy surface
#                        (hooks, config, engine, .hmac-key, Claude settings).
#   2. DENYLIST+NORMALIZE — dangerous commands are matched against a copy of the
#                        command that has quotes/backslashes stripped and
#                        whitespace collapsed, defeating quote/space evasion.
#   3. EXFIL HEURISTICS — outbound network + credential material is blocked.
#
# A denylist on free-form shell is NOT a complete control. For real isolation,
# run Claude Code's native sandbox (Seatbelt/bubblewrap) with an egress
# allowlist. This hook is defense-in-depth, not the only line.

set -euo pipefail

AUDIT_LOG="${HOME}/.claude/vajra/audit.log"
mkdir -p "$(dirname "$AUDIT_LOG")"

TIMESTAMP="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

# Read the hook payload from stdin
PAYLOAD="$(cat)"

# Parse JSON — require jq, FAIL-CLOSED on parse failure
if ! command -v jq &>/dev/null; then
  echo "${TIMESTAMP} BLOCK unknown-tool reason=\"jq not installed — fail-closed\"" >> "$AUDIT_LOG"
  echo "Vajra hook: jq is required for safe tool validation. Install jq or allow in settings." >&2
  exit 1
fi

# Support both legacy {tool,input} and current {tool_name,tool_input} schemas.
TOOL_NAME="$(echo "$PAYLOAD" | jq -r '.tool // .tool_name // empty' 2>/dev/null || echo "")"

# FAIL-CLOSED: if we can't parse the tool name, block the operation
if [ -z "$TOOL_NAME" ]; then
  echo "${TIMESTAMP} BLOCK unknown-tool reason=\"parse failure — fail-closed\"" >> "$AUDIT_LOG"
  echo "Vajra hook: blocked unknown tool (could not parse payload)" >&2
  exit 1
fi

# Raw input can be a string (legacy) or an object. Extract:
#   INPUT_RAW  — the whole input serialized (string or compact JSON)
#   CMD        — the Bash command string, if any
#   TARGET     — the file path being written/edited, if any
INPUT_RAW="$(echo "$PAYLOAD" | jq -r 'if (.input // .tool_input) == null then "" elif ((.input // .tool_input)|type) == "string" then (.input // .tool_input) else ((.input // .tool_input)|tojson) end' 2>/dev/null || echo "")"
CMD="$(echo "$PAYLOAD" | jq -r '(.input // .tool_input) as $i | if ($i|type)=="object" then ($i.command // "") elif ($i|type)=="string" then $i else "" end' 2>/dev/null || echo "")"
TARGET="$(echo "$PAYLOAD" | jq -r '(.input // .tool_input) as $i | if ($i|type)=="object" then ($i.file_path // $i.path // $i.notebook_path // "") else "" end' 2>/dev/null || echo "")"

# ---------------------------------------------------------------------------
# Normalization — build a matching copy that defeats common evasion.
# Removing quotes/backslashes and collapsing whitespace cannot change what the
# real shell will execute (we never execute anything), but it removes the
# `rm -r''f`, `rm  -rf  /`, and `"rm" -rf /` style splits from the matcher.
# ---------------------------------------------------------------------------
normalize() {
  # shellcheck disable=SC2001
  printf '%s' "$1" \
    | tr -d '"'"'"'\\' \
    | tr '\t' ' ' \
    | tr -s ' '
}

HOME_ESC="$(printf '%s' "$HOME" | sed 's/[.[\*^$/]/\\&/g')"

# Expand ~ and $HOME / ${HOME} to the literal home so path checks are uniform.
expand_home() {
  printf '%s' "$1" \
    | sed "s#\\\${HOME}#${HOME}#g; s#\\\$HOME#${HOME}#g; s#~/#${HOME}/#g; s#\"~\"#${HOME}#g"
}

NORM_CMD="$(normalize "$CMD")"
NORM_INPUT="$(normalize "$INPUT_RAW")"
NORM_PAYLOAD="$(normalize "$PAYLOAD")"
# Combined haystack for command-style checks (normalized).
CHECK_STRING="$(expand_home "${TOOL_NAME} ${NORM_CMD} ${NORM_INPUT} ${NORM_PAYLOAD}")"
# Expanded, normalized target path for write checks.
TARGET_EXP="$(expand_home "$(normalize "$TARGET")")"
CMD_EXP="$(expand_home "$NORM_CMD")"

BLOCKED=false
REASON=""

block() { BLOCKED=true; REASON="$1"; }

# ---------------------------------------------------------------------------
# 0. IMMUTABLE CORE — the harness may not modify its own policy surface.
#    (VJR-01) Protect: vajra skill dir, vajra state hmac key, Claude settings,
#    and the Claude hooks dir. Resolved against $HOME.
# ---------------------------------------------------------------------------
# Match the protected path SUFFIX anywhere in the string (not HOME-anchored), so
# a relative path like `../../.claude/skills/vajra/hooks/...` is caught too (F-1
# follow-up: the old HOME-anchored regex missed relative targets).
# Core nodes are matched even when they TERMINATE the token (no trailing slash),
# so `mv ~/.claude/skills/vajra /tmp` and `ln -sfn x ~/.claude/skills/vajra`
# (relocating/repointing the whole core) are caught, not just `.../vajra/file`.
PROTECTED_RE='\.claude/skills/vajra($|[/"'"'"' ])|\.claude/vajra/\.hmac-key|\.claude/settings(\.local)?\.json|\.claude/hooks($|[/"'"'"' ])|(^|/)skills/vajra($|[/"'"'"' ])|(^|/)vajra/(hooks|config|engine|steering|redteam|fleet|atman|scripts)($|[/"'"'"' ])'
# The core's UNPROTECTED ancestors. A recursive/archive writer aimed at one of
# these (e.g. `rsync -a src ~/.claude/`, `tar -xf e -C ~/.claude/skills`) lands
# inside the core without the protected substring ever appearing in the command.
CORE_ANCESTOR_RE='\.claude(/skills)?/?(["'"'"' ]|$)'

# 0a. Write/Edit/NotebookEdit directly targeting a protected path (absolute or relative).
case "$TOOL_NAME" in
  Write|Edit|NotebookEdit|MultiEdit)
    if [ -n "$TARGET_EXP" ] && echo "$TARGET_EXP" | grep -qE "$PROTECTED_RE" 2>/dev/null; then
      block "write to Vajra immutable core (${TOOL_NAME})"
    fi
    ;;
esac

# 0b. Bash touching a protected path — DEFAULT-DENY unless it is a pure read.
#     This INVERTS the old deny-known-mutators logic (F-1): enumerating write
#     tools is a losing race — rsync, shred, gawk -i inplace, sponge, csplit,
#     install, dd, interpreters and editors all mutate, and the next tool always
#     evades a denylist. Instead: if a protected path appears in a Bash command,
#     block it unless the command is a simple invocation of a known read-only
#     tool with no redirect, pipe, chaining, or command substitution. Legitimate
#     reads of harness files should use the Read/Grep tools, not Bash.
if [ "$BLOCKED" = false ] && [ "$TOOL_NAME" = "Bash" ] && [ -n "$CMD_EXP" ]; then
  if echo "$CMD_EXP" | grep -qE "$PROTECTED_RE" 2>/dev/null; then
    if echo "$CMD_EXP" | grep -qE '(>|>>|\||;|&&|`|\$\()' 2>/dev/null; then
      # Redirect / pipe / chaining / substitution near a protected path is never
      # a trivial read.
      block "Bash touching Vajra immutable core (non-trivial command)"
    else
      # Leading command word, skipping any VAR=value env-prefix tokens.
      FIRST_WORD="$(echo "$CMD_EXP" | awk '{for(i=1;i<=NF;i++){if($i ~ /=/ && $i !~ /^-/)continue; print $i; exit}}')"
      if ! echo "$FIRST_WORD" | grep -qE '^(cat|less|more|head|tail|grep|egrep|fgrep|rg|wc|diff|cmp|file|stat|ls|sha256sum|shasum|md5|md5sum|cut|sort|uniq|nl|od|hexdump|xxd|strings|column|bat|view|realpath|readlink|dirname|basename|echo|test|true)$' 2>/dev/null; then
        block "Bash mutation of Vajra immutable core (non-reader command)"
      fi
    fi
  fi
fi

# 0d. Any non-reader Bash command targeting a core ANCESTOR directory
#     (~/.claude or ~/.claude/skills, as a terminal directory) is blocked —
#     DEFAULT-DENY, the same inversion as 0b. A recursive/archive writer aimed
#     at an ancestor lands inside the core without naming a protected path
#     (`rsync -a /tmp/p/ ~/.claude/`), and enumerating writer binaries
#     (rsync/tar/7z/unar/ln/find -execdir/...) is a losing race. Pure reads of
#     the ancestor dir (ls/du/cat) still pass.
if [ "$BLOCKED" = false ] && [ "$TOOL_NAME" = "Bash" ] && [ -n "$CMD_EXP" ]; then
  if echo "$CMD_EXP" | grep -qE "$CORE_ANCESTOR_RE" 2>/dev/null; then
    if echo "$CMD_EXP" | grep -qE '(>|>>|\||;|&&|`|\$\()' 2>/dev/null; then
      block "Bash touching a Vajra core ancestor (non-trivial command)"
    else
      ANC_FW="$(echo "$CMD_EXP" | awk '{for(i=1;i<=NF;i++){if($i ~ /=/ && $i !~ /^-/)continue; print $i; exit}}')"
      if ! echo "$ANC_FW" | grep -qE '^(cat|less|more|head|tail|grep|egrep|fgrep|rg|wc|diff|cmp|file|stat|ls|du|tree|sha256sum|shasum|md5|md5sum|cut|sort|uniq|nl|od|hexdump|xxd|strings|column|bat|view|realpath|readlink|dirname|basename|echo|test|true)$' 2>/dev/null; then
        block "Bash mutation of a Vajra core ancestor (non-reader command)"
      fi
    fi
  fi
fi

# 0c. The HMAC signing key is never a legitimate read target for the agent.
#     Blocking reads keeps the campaign/relay/practice-log MAC meaningful and
#     stops a fleet agent from exfiltrating the key (VJR-04, VJR-08).
HMAC_KEY_RE='\.claude/vajra/\.hmac-key'
if [ "$BLOCKED" = false ]; then
  if [ -n "$TARGET_EXP" ] && echo "$TARGET_EXP" | grep -qE "$HMAC_KEY_RE" 2>/dev/null; then
    block "access to the Vajra HMAC signing key"
  elif [ "$TOOL_NAME" = "Bash" ] && [ -n "$CMD_EXP" ] && echo "$CMD_EXP" | grep -qE "$HMAC_KEY_RE" 2>/dev/null; then
    block "access to the Vajra HMAC signing key"
  fi
fi

# ---------------------------------------------------------------------------
# Integrity tamper flag (set by session-start.sh on manifest mismatch, VJR-06).
# While a violation is unresolved, allow read-only tools but block mutations so
# the operator can investigate without the (possibly compromised) harness acting.
# ---------------------------------------------------------------------------
TAMPER_FLAG="${HOME}/.claude/vajra/.integrity-violation"
if [ "$BLOCKED" = false ] && [ -f "$TAMPER_FLAG" ]; then
  case "$TOOL_NAME" in
    Write|Edit|NotebookEdit|MultiEdit)
      block "integrity violation unresolved — mutations blocked (see /vajra verify)"
      ;;
    Bash)
      if echo "$NORM_CMD" | grep -qE '(>|>>|\b(rm|mv|cp|tee|truncate|install|dd|git)\b|sed -i|chmod|chown|npm i|pip install|cargo add|curl|wget)' 2>/dev/null; then
        block "integrity violation unresolved — mutating/network Bash blocked (see /vajra verify)"
      fi
      ;;
  esac
fi

# ---------------------------------------------------------------------------
# Phase enforcement (explore-plan-act) — read-only during explore/plan.
# ---------------------------------------------------------------------------
PHASE_FILE="${HOME}/.claude/vajra/.phase"
if [ "$BLOCKED" = false ] && [ -f "$PHASE_FILE" ]; then
  CURRENT_PHASE="$(cat "$PHASE_FILE" 2>/dev/null | tr -d '[:space:]')"
  if [ "$CURRENT_PHASE" = "explore" ] || [ "$CURRENT_PHASE" = "plan" ]; then
    case "$TOOL_NAME" in
      Write|Edit|NotebookEdit|MultiEdit)
        block "blocked by ${CURRENT_PHASE} phase — read-only until /vajra act"
        ;;
      Bash)
        if echo "$NORM_CMD" | grep -qE '(mkdir|touch|cp |mv |rm |chmod|chown|tee |sed -i|git add|git commit|git push|git checkout|git reset|npm install|pip install|cargo add)' 2>/dev/null; then
          block "blocked by ${CURRENT_PHASE} phase — no mutations until /vajra act"
        fi
        if echo "$NORM_CMD" | grep -qE '(>>|>[^&])' 2>/dev/null; then
          block "blocked by ${CURRENT_PHASE} phase — no file writes until /vajra act"
        fi
        ;;
    esac
  fi
fi

# ---------------------------------------------------------------------------
# 1. Destructive filesystem operations
# ---------------------------------------------------------------------------

# 1a. recursive-force flags pointing at a dangerous target — independent of the
#     command name (catches `rm -rf /`, `$R -rf /`, `rm -fr ~`, alias indirection).
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE '(-[a-zA-Z]*r[a-zA-Z]*f|-[a-zA-Z]*f[a-zA-Z]*r|--recursive --force|--force --recursive)( |=)+('"${HOME_ESC}"'|/|~|\*|\.|/(etc|usr|var|bin|sbin|lib|boot|dev|sys|opt|root|home|System|Library|Applications|private)( |/|$))' 2>/dev/null; then
  block "recursive-force delete of a sensitive path"
fi

# 1b. explicit rm -rf of home / root / important roots even without the flag-order trick
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE '\brm\b[^|;&]*-[a-zA-Z]*[rf]'"" 2>/dev/null; then
  if echo "$CHECK_STRING" | grep -qE '\brm\b[^|;&]*( |=)('"${HOME_ESC}"'( |/|$)|/( |$)|~( |/|$)|\$HOME|/(etc|usr|var|bin|sbin|lib|boot|System|Library)( |/|$))' 2>/dev/null; then
    block "rm -rf of home/root/system path"
  fi
fi

# 2. git push --force (any form) including +refspec force. The `git\b[^|;&]*`
#    prefix tolerates global options like `git -C <path>` / `--git-dir=` (NEW-03).
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE 'git\b[^|;&]*\bpush\b[^|;&]*(--force|--force-with-lease|-f\b|[^a-zA-Z]\+[a-zA-Z._/*-]+:?)' 2>/dev/null; then
  block "git push --force / +refspec force-push"
fi

# 3. git reset --hard (unconditional — destroys uncommitted work)
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE 'git\b[^|;&]*\breset\b[^|;&]*--hard' 2>/dev/null; then
  block "git reset --hard"
fi

# 3b. git clean -fdx (wipes untracked)
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE 'git\b[^|;&]*\bclean\b[^|;&]*-[a-zA-Z]*f' 2>/dev/null; then
  block "git clean -f (deletes untracked files)"
fi

# 4. chmod / chown dangerous: 777, or recursive on root/home/system
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE 'chmod( |.*)(-[a-zA-Z]* )*(777|a\+rwx|=rwx)' 2>/dev/null; then
  block "world-writable chmod (777)"
fi
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE 'ch(mod|own)( |.*)-[a-zA-Z]*R[a-zA-Z]*( |=)+('"${HOME_ESC}"'|/( |$)|~|/(etc|usr|var|System|Library))' 2>/dev/null; then
  block "recursive chmod/chown on sensitive path"
fi

# 5. Raw writes to block/disk devices (dd OR shell redirect)
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE '(of=|>|>>)( )*/dev/(sd|hd|nvme|vd|disk|rdisk|mmcblk|loop)' 2>/dev/null; then
  block "write to block/disk device"
fi

# 6. mkfs on devices (block when Bash is invoking it)
if [ "$BLOCKED" = false ] && [ "$TOOL_NAME" = "Bash" ] && echo "$CHECK_STRING" | grep -qE '\bmkfs(\.[a-z0-9]+)?\b' 2>/dev/null; then
  block "filesystem format command via Bash"
fi

# 7. --no-preserve-root
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE '\-\-no-preserve-root\b' 2>/dev/null; then
  block "--no-preserve-root flag"
fi

# 8. Pipe to a shell OR interpreter (curl|sh, wget|python, etc.), with optional sudo.
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE '\|( )*(sudo )?(ba|z|k|da|c)?sh\b' 2>/dev/null; then
  block "pipe to shell"
fi
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE '\|( )*(sudo )?(python[23]?|perl|ruby|node|php|lua|Rscript)\b' 2>/dev/null; then
  block "pipe to interpreter"
fi

# 9. eval / exec of arbitrary or decoded content
if [ "$BLOCKED" = false ] && [ "$TOOL_NAME" = "Bash" ] && echo "$CHECK_STRING" | grep -qE '\b(eval|exec)( )' 2>/dev/null; then
  block "eval/exec execution"
fi

# 9b. decode-then-execute: base64/xxd/openssl piped or substituted into a shell
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE '(base64( |.*)(-d|--decode|-D)|xxd -r|openssl( |.*)(enc|base64).*-d)' 2>/dev/null; then
  if echo "$CHECK_STRING" | grep -qE '(\|( )*(ba|z)?sh|bash -c|\beval\b|\$\(|`)' 2>/dev/null; then
    block "decode-and-execute (obfuscated payload)"
  fi
fi

# 9c. Interpreter inline-code that shells out or decodes-and-executes (NEW-02,
#     F-2). Covers python/ruby/perl/node -c/-e AND php -r. The sink list is a
#     denylist and therefore not exhaustive — a free-form interpreter can always
#     express RCE; the durable boundary is the OS sandbox (see header). This
#     closes the demonstrated evasions (os.exec*, __import__, passthru/shell_exec,
#     Open3, getattr-built names) on top of the obvious sinks.
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE '\b(python[23]?|ruby|perl|node|php)\b[^|;]*[[:space:]]-(c|e|r)\b' 2>/dev/null; then
  if echo "$CHECK_STRING" | grep -qE '(os\.(system|exec|popen|spawn)|subprocess|system\(|[^a-z]exec[a-z]*\(|execSync|child_process|popen|spawn|passthru|shell_exec|proc_open|Open3|Runtime\.getRuntime|__import__|getattr|b64decode|base64|fromhex|unhexlify|codecs\.decode)' 2>/dev/null; then
    block "interpreter inline-code shelling out or decoding-and-executing"
  fi
fi

# 10. find -delete / find -exec rm|sh
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE 'find\b[^|]*-delete\b' 2>/dev/null; then
  block "find -delete"
fi
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE 'find\b[^|]*-exec( )+(rm|sh|bash|chmod|chown)\b' 2>/dev/null; then
  block "find -exec destructive command"
fi

# 11. Fork bomb signature
if [ "$BLOCKED" = false ] && echo "$NORM_CMD" | grep -qE ':\(\)( )*\{( )*:( )*\|( )*:( )*&( )*\}( )*;( )*:' 2>/dev/null; then
  block "fork bomb"
fi
if [ "$BLOCKED" = false ] && echo "$NORM_PAYLOAD" | grep -qE ':\(\)\{:\|:&\};:' 2>/dev/null; then
  block "fork bomb"
fi

# ---------------------------------------------------------------------------
# 12. Secret / credential exfiltration (lethal-trifecta egress control)
# ---------------------------------------------------------------------------
NET_RE='\b(curl|wget|nc|ncat|netcat|telnet|scp|sftp|rsync|ftp|socat)\b'
CRED_RE='(\.aws/|\.ssh/|id_rsa|id_ed25519|id_ecdsa|\.netrc|\.npmrc|\.pgpass|\.gnupg|\.kube/|\.docker/config|credentials|secrets?|\.env\b|\benv\b|/etc/shadow|\.config/gh|access[_-]?token|api[_-]?key|private[_-]?key)'

# 12a. network command + credential path in same command
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qiE "$NET_RE" 2>/dev/null \
   && echo "$CHECK_STRING" | grep -qiE "$CRED_RE" 2>/dev/null; then
  block "network egress referencing credential material"
fi

# 12b. curl/wget posting a local file (--data @file, -d @file, --upload-file, -T)
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE '\b(curl|wget)\b[^|;]*((--data(-binary|-raw|-urlencode)?|-d|-F|--form)( |=)*@|--upload-file|(-T )( )*[^ ])' 2>/dev/null; then
  block "curl/wget uploading a local file (exfil vector)"
fi

# 12c. DNS-tunnel exfil: dig/nslookup/host with command substitution or base64
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE '\b(dig|nslookup|host|drill)\b' 2>/dev/null; then
  if echo "$CHECK_STRING" | grep -qE '(\$\(|`|base64|xxd|\bcat\b)' 2>/dev/null; then
    block "DNS exfiltration (lookup of command-substituted data)"
  fi
fi

# 12d. piping a secret file into a network sink (cat ~/.ssh/id_rsa | nc ...)
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qiE "$CRED_RE" 2>/dev/null \
   && echo "$CHECK_STRING" | grep -qE '\|( )*'"$NET_RE" 2>/dev/null; then
  block "credential file piped to a network command"
fi

# 13. Read of /etc/shadow (auth hashes)
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE '/etc/shadow\b' 2>/dev/null; then
  block "access to /etc/shadow"
fi

# 14. Disabling the hook via chmod -x / attribute changes on the policy surface
if [ "$BLOCKED" = false ] && echo "$CHECK_STRING" | grep -qE 'chmod( |.*)-x' 2>/dev/null \
   && echo "$CHECK_STRING" | grep -qE "$PROTECTED_RE" 2>/dev/null; then
  block "attempt to disable Vajra hook (chmod -x on policy surface)"
fi

# --- Log and decide ---
if [ "$BLOCKED" = true ]; then
  SAFE_REASON="$(echo "$REASON" | tr -d '\n\r' | tr -cd '[:print:]')"
  echo "${TIMESTAMP} BLOCK ${TOOL_NAME} reason=\"${SAFE_REASON}\"" >> "$AUDIT_LOG"
  echo "Vajra hook blocked dangerous operation: ${REASON}" >&2
  exit 1
else
  echo "${TIMESTAMP} ALLOW ${TOOL_NAME}" >> "$AUDIT_LOG"
  exit 0
fi
