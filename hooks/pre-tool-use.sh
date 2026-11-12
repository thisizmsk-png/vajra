#!/usr/bin/env bash
# Vajra PreToolUse guard.
#
# Contract (Claude Code hooks): JSON on stdin with tool_name, tool_input, cwd.
#   exit 0 = no decision (normal permission flow continues)
#   exit 2 = BLOCK; the reason on stderr is shown to Claude.
# Any other exit is a non-blocking error in Claude Code, so this script never
# exits with anything else: an EXIT trap turns every unexpected exit (parse
# failure, missing jq, unbound variable, crash) into exit 2. hooks.json also
# registers it with "onFailure": "block" for timeouts and launch failures.
#
# What it inspects: tool_input.command (Bash) and the path fields
# (file_path, path, notebook_path). Never file contents, never the description.
#
# Path checks are canonical: each path-like token is resolved against the
# session cwd (or a preceding `cd`), `..` is collapsed, symlinks are resolved,
# and comparison is case-insensitive (the default macOS filesystem is).
#
# This is defense in depth over free-form shell, not a sandbox. The durable
# boundary is Claude Code's OS sandbox (see SECURITY.md).
#
# bash 3.2 compatible (macOS /bin/bash): no ${x,,}, no associative arrays, and
# empty arrays are expanded with ${a[@]+"${a[@]}"} because of set -u.

set -u
unset CDPATH
# NFR-SEC-2: a hook runs outside the guard, and the programs it starts can read the HMAC key file. A same-uid worker
# can write user dirs (for example ~/.local/bin, or the Homebrew prefix), so every tool comes only from the root-owned system dirs
# and python3 only from its absolute path, whatever PATH the caller (a Claude Code session, the app) had.
PATH="/usr/bin:/bin:/usr/sbin:/sbin"; export PATH
PYTHON3="/usr/bin/python3"
# Path comparisons are case-insensitive (nocasematch is switched on only inside
# classify/inside/icase, so `case` on flags like -c vs -C stays exact).

DECIDED=0
on_exit() {
  local rc=$?
  if [ "$DECIDED" != 1 ]; then
    echo "Vajra guard: internal error (rc=${rc}); blocking the tool call (fail-closed)." >&2
    exit 2
  fi
}
trap on_exit EXIT

VS="${HOME}/.claude/vajra"
AUDIT_LOG="${VS}/audit.log"

audit() { # verdict tool reason
  [ -d "$VS" ] || mkdir -p "$VS" 2>/dev/null || return 0
  local r="${3//[^[:print:]]/}"
  printf '%s %s %s%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$1" "$2" "${r:+ reason=\"$r\"}" >> "$AUDIT_LOG" 2>/dev/null || true
}

block() {
  audit BLOCK "${TOOL:-unknown}" "$1"
  echo "Vajra guard blocked this ${TOOL:-tool} call: $1" >&2
  DECIDED=1
  exit 2
}

allow() {
  audit ALLOW "${TOOL:-unknown}" ""
  DECIDED=1
  exit 0
}

TOOL=""
NOJQ="jq was not found in /usr/bin, /bin, /usr/sbin or /sbin and /usr/bin/python3 could not parse the payload, so every tool call is blocked (fail-closed). Vajra's hooks use only those root-owned system dirs: a jq in /opt/homebrew/bin, /usr/local/bin, ~/.local/bin, ~/.nix-profile/bin or an asdf/mise shim is never run, because a same-uid worker can replace it. macOS 15+ ships /usr/bin/jq; otherwise xcode-select --install provides /usr/bin/python3, which the guard then uses. Retry after."

PAYLOAD="$(cat)"
CWD_IN="" CMD="" P1="" P2="" P3="" PAT="" AID=""
if command -v jq >/dev/null 2>&1; then
PARSED="$(jq -r '
  def s: if type == "string" then . elif . == null then "" else tostring end;
  (.tool_input | if type == "object" then . else {} end) as $i
  | @sh "TOOL=\(.tool_name | s) CWD_IN=\(.cwd | s) CMD=\($i.command | s) P1=\($i.file_path | s) P2=\($i.path | s) P3=\($i.notebook_path | s) PAT=\($i.pattern | s) AID=\(.agent_id | s)"
' <<<"$PAYLOAD" 2>/dev/null)" || block "unparseable hook payload (fail-closed)"
elif [ -x "$PYTHON3" ]; then
  # No jq in the system dirs (macOS 14): the same fields, quoted as jq's @sh does, by the pinned python3's json module.
  # A non-object payload, bad JSON or a python3 that cannot run (the Command Line Tools stub) all block.
  PARSED="$("$PYTHON3" -I -c '
import json, sys
d = json.loads(sys.stdin.read())
if not isinstance(d, dict):
    sys.exit(1)
def s(v):
    if isinstance(v, str):
        return v
    if v is None:
        return ""
    return json.dumps(v, separators=(",", ":"), ensure_ascii=False)
def q(v):
    t = chr(39)
    return t + s(v).replace(t, t + "\\" + t + t) + t
i = d.get("tool_input")
if not isinstance(i, dict):
    i = {}
sys.stdout.write("TOOL=%s CWD_IN=%s CMD=%s P1=%s P2=%s P3=%s PAT=%s AID=%s" % (q(d.get("tool_name")), q(d.get("cwd")),
    q(i.get("command")), q(i.get("file_path")), q(i.get("path")), q(i.get("notebook_path")), q(i.get("pattern")),
    q(d.get("agent_id"))))
' <<<"$PAYLOAD" 2>/dev/null)" || block "$NOJQ"
else
  block "$NOJQ"
fi
eval "$PARSED"
[ -n "$TOOL" ] || block "payload has no tool_name (fail-closed)"

# ---------------------------------------------------------------------------
# Canonicalization
# ---------------------------------------------------------------------------
CANON=""
canon() { # path base -> CANON (absolute, physical, .. collapsed)
  local p="$1" base="$2" tail="" n=0 t c out
  case "$p" in
    "~") p="$HOME" ;;
    "~/"*) p="$HOME/${p#??}" ;;
    "~+"|"~+/"*) p="${base%/}${p#??}" ;;  # ~+ is the cwd
    # ~- (OLDPWD), a dir-stack entry (~N, ~+N, ~-N, once the command uses pushd) and ~name for an
    # existing account expand to a directory this script cannot know: fail closed, as Glob does. Any
    # other ~word stays literal in bash (~2s in a sentence), so it is a plain relative name (#148 review).
    "~"?*)
      local tn="${p#\~}" tnd
      tn="${tn%%/*}"; tnd="${tn#[+-]}"
      if [ "$tn" = - ]; then block "a ~- path cannot be resolved; write the path in full: $p"; fi
      case "$tnd" in
        ""|*[!0-9]*) ;;
        *[!0]*) [[ "$CMD" == *pushd* ]] && block "a ~N dir-stack path cannot be resolved; write the path in full: $p" ;;
        # ~0 and ~+0 are always the cwd. ~-0 is the bottom stack entry: the cwd only while the stack has one
        # entry, so once the command uses pushd it is refused like any other stack entry (#148 review).
        *)
          case "$tn" in -*) [[ "$CMD" == *pushd* ]] && block "a ~-N dir-stack path cannot be resolved; write the path in full: $p" ;; esac
          p="${base%/}${p#"~$tn"}" ;;
      esac
      case "$tn" in
        [A-Za-z_]*) case "$tn" in *[!A-Za-z0-9._-]*) ;; *)
          /usr/bin/id -u "$tn" >/dev/null 2>&1 && block "a ~name path cannot be resolved; write the path in full: $p" ;;
        esac ;;
      esac
      ;;
  esac
  case "$p" in /*) ;; *) p="${base%/}/$p" ;; esac
  while :; do
    while [ "${#p}" -gt 1 ] && [ "${p%/}" != "$p" ]; do p="${p%/}"; done
    if [ -d "$p" ] && cd -P -- "$p" 2>/dev/null; then p="$PWD"; break; fi
    if [ -L "$p" ] && [ "$n" -lt 16 ]; then
      n=$((n + 1))
      t="$(readlink -- "$p" 2>/dev/null)" || t=""
      if [ -n "$t" ]; then
        case "$t" in /*) p="$t$tail" ;; *) p="${p%/*}/$t$tail" ;; esac
        tail=""
        continue
      fi
    fi
    [ "$p" = "/" ] && break
    tail="/${p##*/}$tail"
    p="${p%/*}"
    [ -n "$p" ] || p="/"
  done
  out="$p"
  local IFS=/
  set -f
  for c in $tail; do
    case "$c" in
      ""|.) ;;
      ..) out="${out%/*}"; [ -n "$out" ] || out="/" ;;
      *) out="${out%/}/$c" ;;
    esac
  done
  set +f
  CANON="$out"
}

canon "$HOME" /; CHOME="$CANON"
canon "$HOME/.claude" /; CL="$CANON"
canon "$HOME/.claude/vajra" /; VS_C="$CANON"
canon "$HOME/.claude/skills/vajra" /; SKILL="$CANON"
SKILL_LINK="$CL/skills/vajra"
KEY="$VS_C/.hmac-key"

if [ -n "$CWD_IN" ]; then canon "$CWD_IN" /; else canon "$PWD" /; fi
CWD="$CANON"

# Write-protected roots (the path itself and everything below it).
ROOTS=(
  "$SKILL" "$SKILL_LINK" "$CL/agents" "$CL/hooks"
  "$KEY" "$VS_C/audit.log" "$VS_C/practice-log.jsonl" "$VS_C/routing-log.jsonl"
  "$VS_C/learned-routes.json" "$VS_C/manifest.sig" "$VS_C/.integrity-violation"
  "$VS_C/proposals" "$VS_C/events" "$VS_C/atman"
  "$VS_C/practice-log.lock" "$VS_C/practice-archive" "$VS_C/practice-log.anchor.json"
  "$VS_C/atman-history.jsonl" "$VS_C/patches" "$VS_C/atman.paused"
  "$VS_C/kb"
)
# Vajra Desktop control directory ("$HOME/Library/Application Support/VajraDesktop/control"):
# only the desktop app writes it. The Bash analysis splits words on spaces, so
# a second Bash pass (end of file) joins "Application Support" into the single
# token Application__SP__Support; the alias root below is what that token
# canonicalizes to.
canon "$HOME/Library/Application Support/VajraDesktop/control" /; DCTL="$CANON"
canon "$HOME/Library/Application__SP__Support/VajraDesktop/control" /; DCTL2="$CANON"
ROOTS+=( "$DCTL" "$DCTL2" )
# ADR-16 (design.md 16.6/16.8): the whole Vajra Desktop app data dir <data>
# ("$HOME/Library/Application Support/VajraDesktop") is core-only state (seats.json,
# run records, profiles, ticket dirs, the git baseline, the control queue): agents
# never write anywhere under it. Reads stay allowed (the control dir keeps its own
# worker read block below). The worktree root <work> (VajraWorktrees, a sibling,
# never a prefix match) stays writable.
canon "$HOME/Library/Application Support/VajraDesktop" /; DDATA="$CANON"
canon "$HOME/Library/Application__SP__Support/VajraDesktop" /; DDATA2="$CANON"
ROOTS+=( "$DDATA" "$DDATA2" )
# Read-protected for workers (payload carries agent_id): the control dir holds the
# steering nonce, and transcripts hold each worker's brief (which carries it). The
# main session and the Lead chat session have no agent_id and keep reading.
canon "$HOME/.claude/projects" /; NOREAD=( "$DCTL" "$DCTL2" "$CANON" )
noread() { # canonical path -> 0 if a worker may not read it
  local r
  [ -n "$AID" ] || return 1
  for r in "${NOREAD[@]}"; do inside "$1" "$r" && return 0; done
  return 1
}
reach_check() { # canonical dir, label -> block if a recursive read there reaches the key, or (for a worker) a NOREAD root
  local r
  key_anc "$1" && block "$2 would reach the Vajra HMAC key; give a path below it (for example your project directory)"
  [ -n "$AID" ] || return 0
  for r in "${NOREAD[@]}"; do
    inside "$r" "$1" && block "$2 would reach Vajra Desktop control files or Claude Code transcripts; give a path below it"
  done
  return 0
}
# Knowledge ledger (kb/) and task graphs (campaigns/<slug>/tasks.json*) change
# only through scripts/vajra_kb.py and scripts/vajra_tasks.py; reading is fine.

PKIND=""
# The shortcut above is valid only while no protected root was moved by a symlink under HOME.
CLASSIFY_FAST=""
if [ "$CL" = "$CHOME/.claude" ] && [ "$VS_C" = "$CL/vajra" ] && [ "$SKILL" = "$CL/skills/vajra" ] \
   && [[ "$DCTL" == "$CHOME/Library/"* ]] && [[ "$DDATA" == "$CHOME/Library/"* ]]; then CLASSIFY_FAST=1; fi
classify() { local was=0; shopt -q nocasematch && was=1; shopt -s nocasematch; classify_ "$1"; [ "$was" = 1 ] || shopt -u nocasematch; return 0; }
classify_() { # canonical path -> PKIND = key|core|ancestor|""
  local p="$1" r rest
  PKIND=""
  # Every protected path is under the Claude dir or ~/Library, or is a settings file in a Claude dir:
  # anything else is decided here, once, instead of by the loops below (a 4 KB command has ~4,000 words).
  if [ -n "$CLASSIFY_FAST" ]; then
    case "$p" in *.claude*|"$CHOME"/Library*) ;; *) return 0 ;; esac
  fi
  if [[ "$p" == "$KEY" ]]; then PKIND=key; return 0; fi
  for r in "${ROOTS[@]}"; do
    if [[ "$p" == "$r" || "$p" == "$r"/* ]]; then PKIND=core; return 0; fi
  done
  # Any Claude settings file, user or project scope.
  if [[ "$p" =~ /\.claude/settings[^/]*\.json$ ]]; then PKIND=core; return 0; fi
  if [[ "$p" == "$CL/workflows/vajra-"* ]]; then PKIND=core; return 0; fi
  if [[ "$p" == "$VS_C/"* ]]; then
    rest="${p:${#VS_C}+1}"
    case "$rest" in
      */*) ;;
      *.jsonl|*.log) PKIND=core; return 0 ;;
    esac
    if [[ "$rest" == campaigns/*/checkpoints || "$rest" == campaigns/*/checkpoints/* ]]; then PKIND=core; return 0; fi
    if [[ "$rest" == campaigns/*/tasks.json* && "${rest#campaigns/*/}" != */* ]]; then PKIND=core; return 0; fi
    # vajra_tasks.py's lock directory (mkdir-based): only the script takes it.
    if [[ "$rest" == campaigns/?*/.tasks.lock* ]]; then
      local slug="${rest#campaigns/}"; slug="${slug%%/*}"
      if [[ "$rest" == "campaigns/$slug/.tasks.lock"* ]]; then PKIND=core; return 0; fi
    fi
    if [[ "$rest" == campaigns ]]; then PKIND=ancestor; return 0; fi
    if [[ "$rest" == campaigns/* && "${rest#campaigns/}" != */* ]]; then PKIND=ancestor; return 0; fi
  fi
  # Ancestors of protected roots strictly below HOME (~/.claude, ~/.claude/skills, ...).
  if [[ "$p" == "$CHOME"/?* ]]; then
    for r in "${ROOTS[@]}" "$CL/workflows/vajra-x" "$CL/settings.json"; do
      if [[ "$r" == "$p"/* ]]; then PKIND=ancestor; return 0; fi
    done
  fi
  return 0
}

inside() { # path dir -> 0 if path is dir or below it (case-insensitive)
  local rc=1 was=0
  shopt -q nocasematch && was=1 # the caller's setting survives: a Glob walk is case-insensitive end to end
  shopt -s nocasematch
  if [[ "$1" == "$2" || "$1" == "$2"/* ]]; then rc=0; fi
  [ "$was" = 1 ] || shopt -u nocasematch
  return $rc
}

# 0 if the canonical path is the HMAC key, its directory, or any ancestor of
# it (~/.claude, HOME, /Users, /): a recursive read or copy there reaches the key.
key_anc() { [ "$1" = / ] || inside "$KEY" "$1"; }

same_dir() { # path dir -> 0 if they are the same directory (case-insensitive)
  inside "$1" "$2" && inside "$2" "$1"
}
# ADR-16 (#148 review N1-N3): one rule for every word the analysis cannot resolve to
# one path. A word is ambiguous when it holds a glob or brace character (* ? [ {), an
# unresolved expansion ($, backticks) or a non-ASCII byte (APFS folds U+212A KELVIN
# SIGN to k and U+017F LONG S to s, and is normalization-insensitive). Its literal
# directory is the text before the first such character, up to the last "/" (none:
# the cwd). If that directory is HOME or above it, or is or holds a protected path
# (classify: core or ancestor), the word could resolve into <data> or another
# protected root, so a write through it is refused. Mode "uni" checks only non-ASCII
# (the file tools never expand globs or variables).
AMB_D="" AMB_R=0
amb_reach() { # canonical dir -> 0 if an ambiguous name there could reach a protected path
  [ "$1" = "$AMB_D" ] && return $AMB_R
  AMB_D="$1"; AMB_R=1
  # "/" holds everything (inside would compare against "//*").
  if [ "$1" = / ] || inside "$CHOME" "$1"; then AMB_R=0; else classify "$1"; [ -n "$PKIND" ] && AMB_R=0; fi
  return $AMB_R
}
amb_check() { # word base mode(all|uni|cd|read) -> block if ambiguous and it could reach a protected path
  # cd: mark the cwd unknown instead of blocking; read: return 0 (reach) / 1, never block (pipe taint, #148 F3).
  local w="$1" lit l2 d rest
  if [ "$3" = uni ]; then
    case "$w" in *[![:ascii:]]*) ;; *) return 1 ;; esac
    lit="${w%%[![:ascii:]]*}"
  else
    case "$w" in *[\*\?\[\{\$\`]*|*[![:ascii:]]*) ;; *) return 1 ;; esac
    case "$w" in
      */*|"~"*) lit="${w%%[\*\?\[\{\$\`]*}"; l2="${w%%[![:ascii:]]*}"; [ "${#l2}" -lt "${#lit}" ] && lit="$l2" ;;
      *) lit="" ;; # no "/": the literal dir is the cwd (the common case in a long interpreter string; amb_reach caches it)
    esac
  fi
  case "$lit" in
    */*) d="${lit%/*}"; [ -n "$d" ] || d=/ ;;
    "~"*) d="$lit" ;;
    *) d="" ;;
  esac
  # #148 F1: what follows the first ambiguous character can climb out of the literal dir: a ".." after it, a second
  # ambiguous segment, or (when this command assigns a value that could climb or point near a protected root,
  # AMB_DYN) any expansion. The literal dir then bounds nothing, so the word is judged from "/".
  rest="${w:${#lit}}"
  case "$rest" in
    *..*) d=/ ;;
    */*[\*\?\[\{\$\`]*) [ "$3" = uni ] || d=/ ;;
    *[\$\`]*) [ "$3" != uni ] && [ "$AMB_DYN" = 1 ] && d=/ ;;
  esac
  if [ -z "$d" ]; then d="$2"; elif [ "$d" != / ]; then canon "$d" "$2"; d="$CANON"; fi
  amb_reach "$d" || return 1
  case "$3" in
    cd) CWD_UNK=1; return 0 ;;
    read) return 0 ;;
  esac
  block "a glob, brace, variable or non-ASCII name under $d could resolve into a protected Vajra path (such as the Vajra Desktop app data dir); write the full path literally"
}
# Set once per command by amb_dyn_scan: 1 if it assigns a variable a value that could climb (..), is computed ($(...),
# backticks) or whose literal dir could reach a protected path, or fills one by for/select/read/mapfile/printf -v.
AMB_DYN=0
# 1 after a cd/pushd whose target is ambiguous near a protected path: the real cwd is unknown, so a later relative
# write target, or any non-reader, in the same command is refused (#148 F1/F5).
CWD_UNK=0
amb_val_dyn() { # value -> AMB_DYN=1 if it could climb, is computed, or its literal dir could reach a protected path
  local v="$1" lit d
  case "$v" in *..*|*'$('*|*\`*) AMB_DYN=1; return 0 ;; esac
  case "$v" in /*|"~"*) ;; *) return 0 ;; esac
  lit="${v%%[\*\?\[\{\$\`]*}"
  case "$lit" in */*) d="${lit%/*}"; [ -n "$d" ] || d=/ ;; *) d="$lit" ;; esac
  canon "$d" "$CWD"
  amb_reach "$CANON" && AMB_DYN=1
  return 0
}
amb_dyn_scan() { # tokens...
  # read/mapfile/readarray/getopts count only as a segment's command word, and -v only as an option of
  # printf/declare/typeset/local/read (#148 review: grep -v or cp -v is not an assignment).
  local t f=0 first=1 cw=""
  AMB_DYN=0
  for t in "$@"; do
    case "$t" in ";"|PIPE) f=0; first=1; cw=""; continue ;; esac
    if [ "$first" = 1 ]; then
      case "$t" in
        do|then|else|elif|if|while|until|"!"|"{"|time) continue ;; # keywords: the command word comes next
        ?*=*) amb_val_dyn "${t#*=}"; [ "$AMB_DYN" = 1 ] && return 0; continue ;; # VAR=value prefix
      esac
      first=0; cw="${t##*/}"
      case "$cw" in
        read|mapfile|readarray|getopts) AMB_DYN=1; return 0 ;;
        for|select) f=1 ;;
      esac
      continue
    fi
    if [ "$f" = 1 ]; then [ "$t" = in ] && f=2; continue; fi
    if [ "$f" = 2 ]; then amb_val_dyn "$t"; [ "$AMB_DYN" = 1 ] && return 0; continue; fi
    case "$t" in
      -v|-v?*) case "$cw" in printf|declare|typeset|local|read) AMB_DYN=1; return 0 ;; esac ;;
      ?*=*) amb_val_dyn "${t#*=}"; [ "$AMB_DYN" = 1 ] && return 0 ;;
    esac
  done
  return 0
}
# A Glob pattern can reach the state directory's file names (the HMAC key, the audit log) however it is written. Every
# pattern goes through ONE normalizer, and a pattern whose meaning cannot be decided is denied:
#   1. Backslash is read both ways: as an escape, and (second pass) as a separator, so "..\.claude\vajra\*" is judged too.
#   2. Braces are expanded over the WHOLE pattern before it is split (an alternative may hold "/", "..", "~" or an
#      absolute root). More than BX_MAX results is undecidable and denied.
#   3. "~" is HOME; any other "~name" form is another account's home and is denied.
#   4. Each result is split into components. The leading components without a glob character are resolved physically
#      (symlinks, "..") against the search root; a pattern that starts with a glob is rooted at "/" (absolute) or the
#      search root. If that directory is the state directory or an ancestor, the glob components after it are matched
#      against the state directory's own components, then the next one against the names that must stay hidden.
#   5. After the first glob character a component is a climb (".." step) if the glob can match "..", e.g. "..", "[.][.]",
#      ".[.]", ".?", "\.\.", ".*"; it may stay put if it can match "." (or nothing). "**" may stand for any number of
#      directories. A glob component that can match a symlink in the literal directory is followed to its target.
# Residual: a symlink more than one level below the literal directory, reached only through the glob, is not followed;
# the OS sandbox is the enforced boundary, this hook is defense in depth.
GLOB_R=() GLOB_S=() GLOB_K=0 GLOB_N=0 PNAMES=()
BX=() BX_OVER=0 BX_MAX=256
brace_expand() { # string -> appends every brace expansion to BX (a group without a comma is one alternative); sets BX_OVER
  local s="$1" n=${#1} i=0 ch depth=0 start=-1 end=-1 pre body post alt k
  [ "${#BX[@]}" -le "$BX_MAX" ] || { BX_OVER=1; return 0; }
  while [ "$i" -lt "$n" ]; do
    ch="${s:$i:1}"
    if [ "$ch" = "{" ]; then
      [ "$depth" -eq 0 ] && start=$i
      depth=$((depth + 1))
    elif [ "$ch" = "}" ] && [ "$depth" -gt 0 ]; then
      depth=$((depth - 1))
      if [ "$depth" -eq 0 ]; then end=$i; break; fi
    fi
    i=$((i + 1))
  done
  if [ "$end" -lt 0 ]; then BX[${#BX[@]}]="$s"; return 0; fi # no complete group: literal text
  pre="${s:0:$start}"; body="${s:$((start + 1)):$((end - start - 1))}"; post="${s:$((end + 1))}"
  depth=0 alt="" k=0
  while [ "$k" -le "${#body}" ]; do
    ch="${body:$k:1}"
    if [ "$k" -eq "${#body}" ] || { [ "$ch" = "," ] && [ "$depth" -eq 0 ]; }; then
      brace_expand "$pre$alt$post"; alt=""
    else
      [ "$ch" = "{" ] && depth=$((depth + 1))
      [ "$ch" = "}" ] && depth=$((depth - 1))
      alt="$alt$ch"
    fi
    k=$((k + 1))
  done
}
glob_pat_has() { case "$1" in *[*?[\\]*) return 0 ;; *) return 1 ;; esac; }
glob_comp_match() { # pattern-component state-component -> 0 if the component can match the name
  # shellcheck disable=SC2053
  [[ "$2" == $1 ]]
}
glob_leaf() { # pattern-component (glob or literal) -> 0 if it can match a protected name directly inside the state dir
  local nm
  # A literal here follows a glob component, so it is reached through a glob: an exact match on a protected name counts.
  for nm in ${PNAMES[@]+"${PNAMES[@]}"}; do
    # shellcheck disable=SC2053
    [[ "$nm" == $1 ]] && return 0
  done
  return 1
}
glob_lead_dot() { # pattern-component -> 0 if its first character is a "." (plain, escaped, or inside a [...] class)
  case "$1" in
    .*|\\.*) return 0 ;;
    \[*) [[ "${1%%]*}" == *.* ]]; return $? ;;
  esac
  return 1
}
glob_climbs() { # pattern-component -> 0 if it can match ".." (a leading "." is matched only by a "." in the pattern)
  # shellcheck disable=SC2053
  glob_lead_dot "$1" && [[ .. == $1 ]]
}
glob_stays() { # pattern-component -> 0 if it can match "." (the same directory)
  # shellcheck disable=SC2053
  glob_lead_dot "$1" && [[ . == $1 ]]
}
glob_star() { case "$1" in *\*\**) return 0 ;; *) return 1 ;; esac; }
glob_chain() { # index into GLOB_R, index into GLOB_S -> 0 if the pattern can list the state dir's protected names
  local ri="$1" si="$2" c
  if [ "$si" -ge "$GLOB_K" ]; then
    [ "$ri" -lt "$GLOB_N" ] || return 1
    c="${GLOB_R[$ri]}"
    if glob_star "$c"; then
      [ $((ri + 1)) -lt "$GLOB_N" ] || return 0
      glob_chain $((ri + 1)) "$si"; return $?
    fi
    glob_leaf "$c"; return $?
  fi
  [ "$ri" -lt "$GLOB_N" ] || return 1
  c="${GLOB_R[$ri]}"
  if glob_star "$c"; then
    glob_chain $((ri + 1)) "$si" && return 0
    glob_chain "$ri" $((si + 1)); return $?
  fi
  glob_comp_match "$c" "${GLOB_S[$si]}" || return 1
  glob_chain $((ri + 1)) $((si + 1))
}
glob_dotdot_reaches() { # canonical literal prefix -> 0 if a ".." in the glob part can climb to a directory at or above the
  # deepest one the prefix shares with the state dir (so the rest of the pattern could name the state dir from there).
  # Each other component descends one level, "**" and "." none (they may match zero directories).
  # $2 = index of the first glob component to count (a followed symlink already used the first one).
  local IFS=/ a b common=0 depth min c i start="${2:-0}"
  set -f; a=($1); b=($VS_C); set +f
  while [ "$common" -lt "${#a[@]}" ] && [ "$common" -lt "${#b[@]}" ] && [ "${a[$common]}" = "${b[$common]}" ]; do common=$((common + 1)); done
  depth=${#a[@]}; min=$depth
  for ((i = start; i < GLOB_N; i++)); do
    c="${GLOB_R[$i]}"
    if glob_climbs "$c"; then depth=$((depth - 1)); elif glob_stays "$c" || glob_star "$c"; then :; else depth=$((depth + 1)); fi
    [ "$depth" -lt "$min" ] && min=$depth
  done
  [ "$min" -le "$common" ]
}
glob_walk() { # directory first-component-index -> 0 if the pattern, taken from that directory, can list the state dir's names
  local dir="$1" start="$2" rel r
  inside "$VS_C" "$dir" || [ "$dir" = / ] || return 1 # the directory is not at or above the state dir
  rel="${VS_C:${#dir}}"; rel="${rel#/}"
  GLOB_S=() GLOB_K=0
  if [ -n "$rel" ]; then
    set -f; local IFS=/
    for r in $rel; do [ -n "$r" ] && { GLOB_S[$GLOB_K]="$r"; GLOB_K=$((GLOB_K + 1)); }; done
    set +f
  fi
  PNAMES=()
  for r in "${ROOTS[@]}"; do [ "${r%/*}" = "$VS_C" ] && PNAMES[${#PNAMES[@]}]="${r##*/}"; done
  glob_chain "$start" 0
}
glob_symlinks() { # literal directory, 1 if the pattern has a ".." after a glob character -> blocks if the first glob component can match a symlink that leads to the state dir or above
  local base="$1" dotdot="${2:-0}" first="${GLOB_R[0]}" ent name start
  [ -d "$base" ] || return 0
  for ent in "$base"/* "$base"/.[!.]* "$base"/..?*; do # plain, dot-led, and "..x"-led names (a symlink can be called ..x)
    [ -L "$ent" ] || continue
    name="${ent##*/}"
    glob_star "$first" || glob_comp_match "$first" "$name" || continue
    canon "$ent" "$base"
    start=1; glob_star "$first" && start=0
    # ".." resolves physically, so from the symlink's target it climbs out of the target's own parent chain.
    if [ "$dotdot" = 1 ] && glob_dotdot_reaches "$CANON" "$start"; then shopt -u nocasematch; block "Glob pattern steps back with .. after the symlink $ent far enough to reach a directory that holds the Vajra state directory"; fi
    if glob_walk "$CANON" "$start"; then shopt -u nocasematch; block "Glob pattern can follow the symlink $ent into the Vajra state directory, including the HMAC key and audit log file names"; fi
  done
}
glob_check_one() { # one brace-free pattern, canonical search root
  local pat="$1" root="$2" lit="" c rest_started=0 dotdot=0
  case "$pat" in "~") pat="$HOME" ;; "~/"*) pat="$HOME/${pat#??}" ;; "~"*) shopt -u nocasematch; block "Glob pattern starts with a ~user form that cannot be resolved" ;; esac
  # Split into components: the leading literal ones, then the rest from the first with a glob character.
  GLOB_R=() GLOB_N=0
  local IFS=/
  set -f
  for c in $pat; do
    [ -n "$c" ] || continue
    if [ "$rest_started" = 0 ] && ! glob_pat_has "$c"; then lit="$lit/$c"; continue; fi
    rest_started=1
    # After the first glob character the filesystem walks the pattern: "." is a no-op, ".." steps back out of whatever
    # the glob matched, which cannot be followed statically.
    [ "$c" = . ] && continue
    glob_climbs "$c" && dotdot=1
    GLOB_R[$GLOB_N]="$c"; GLOB_N=$((GLOB_N + 1))
  done
  set +f
  unset IFS
  case "$pat" in /*) ;; *) lit="${lit#/}" ;; esac
  if [ "$GLOB_N" -eq 0 ]; then
    # No glob character left (a brace alternative can leave a plain path): it may only not name a protected file.
    [ -n "$lit" ] || return 0
    canon "$lit" "$root"
    for c in "${ROOTS[@]}"; do
      if [ "${c%/*}" = "$VS_C" ] && same_dir "$CANON" "$c"; then shopt -u nocasematch; block "Glob pattern names a protected Vajra file ($CANON)"; fi
    done
    return 0
  fi
  # A pattern that starts with a glob has no literal prefix: an absolute one is rooted at "/", a relative one at the search root.
  [ -n "$lit" ] || case "$pat" in /*) lit="/" ;; *) lit="." ;; esac
  canon "$lit" "$root"
  local base="$CANON"
  if [ "$dotdot" = 1 ] && glob_dotdot_reaches "$base"; then shopt -u nocasematch; block "Glob pattern steps back with .. after a glob character far enough to reach a directory that holds the Vajra state directory"; fi
  glob_symlinks "$base" "$dotdot"
  if glob_walk "$base" 0; then shopt -u nocasematch; block "Glob pattern would list the Vajra state directory ($VS_C), including the HMAC key and audit log file names"; fi
  return 0
}
glob_pattern_check() { # canonical search root
  local root="$1" v e
  local -a list
  [ -n "$PAT" ] || return 0
  shopt -s nocasematch
  case "$PAT" in *.hmac-key*) shopt -u nocasematch; block "Glob pattern names the Vajra HMAC key" ;; esac
  for v in "$PAT" "${PAT//\\//}"; do
    BX=() BX_OVER=0
    brace_expand "$v"
    if [ "$BX_OVER" = 1 ]; then shopt -u nocasematch; block "Glob pattern has too many brace alternatives to check (more than $BX_MAX); narrow it"; fi
    list=("${BX[@]}")
    for e in "${list[@]}"; do glob_check_one "$e" "$root"; done
  done
  shopt -u nocasematch
  return 0
}

TAMPER=0
[ -f "$VS/.integrity-violation" ] && TAMPER=1

# ---------------------------------------------------------------------------
# File tools
# ---------------------------------------------------------------------------
case "$TOOL" in
  Write|Edit|MultiEdit|NotebookEdit)
    for f in "$P1" "$P2" "$P3"; do
      [ -n "$f" ] || continue
      amb_check "$f" "$CWD" uni
      canon "$f" "$CWD"
      classify "$CANON"
      case "$PKIND" in
        key) block "the Vajra HMAC key is never a tool target" ;;
        core|ancestor) block "write to a protected Vajra/Claude path: $CANON" ;;
      esac
    done
    [ "$TAMPER" = 1 ] && block "integrity violation is unresolved; edits are blocked (run: bash ~/.claude/skills/vajra/scripts/vajra-doctor.sh)"
    allow
    ;;
  Read|Grep|Glob|LS)
    # Grep, Glob and LS search the session cwd when no path is given.
    FPATHS=( "$P1" "$P2" "$P3" )
    if [ -z "$P1$P2$P3" ] && [ "$TOOL" != Read ]; then FPATHS=( "$CWD" ); fi
    for f in "${FPATHS[@]}"; do
      [ -n "$f" ] || continue
      canon "$f" "$CWD"
      classify "$CANON"
      [ "$PKIND" = key ] && block "the Vajra HMAC key is never readable by tools"
      noread "$CANON" && block "workers may not read Vajra Desktop control files or Claude Code transcripts: $CANON"
      # Grep reads file contents recursively: an ancestor of the key (~/.claude,
      # HOME, /) or of a worker-unreadable root would include it.
      [ "$TOOL" = Grep ] && reach_check "$CANON" "Grep over $CANON"
      # Glob lists names, not contents, but the state directory's names include the signing key's file and the audit log.
      if [ "$TOOL" = Glob ]; then
        same_dir "$CANON" "$VS_C" && block "Glob of the Vajra state directory would list the HMAC key and audit log file names; give a path below it (for example $VS_C/campaigns)"
        glob_pattern_check "$CANON"
      fi
    done
    allow
    ;;
  Bash) ;;
  *) allow ;;
esac

# ---------------------------------------------------------------------------
# Bash
# ---------------------------------------------------------------------------
[ -n "$CMD" ] || allow
# Raw size cap (the linear pre-pass below stays well under the hook timeout).
[ "${#CMD}" -le 524288 ] || block "command too large to analyze ($(( ${#CMD} / 1024 )) KB; limit 512 KB). Write the script or data with the Write tool and run the file instead."

# --- Quote- and heredoc-aware pre-pass ---------------------------------------
# The analysis below drops quotes, so text inside a heredoc body or a quoted
# argument would otherwise be judged as commands and paths. This POSIX awk pass
# (run only when the command has quotes or a heredoc) rewrites the command first:
#   * A heredoc body is removed when its command is a plain reader or tee (cat,
#     tee, grep, ...), nothing on that line pipes it on or opens a subshell, and
#     the body cannot run code (quoted delimiter, or no $( or backticks in it).
#     `cat > notes.md <<'EOF'` whose text names a protected path is therefore
#     judged only by its real target, notes.md.
#   * In quoted arguments of plain readers (echo, printf, cat, grep, jq, ...),
#     ; | & < > ( ) and newlines become spaces, so quoted text is not split
#     into fake commands or redirects. In echo/printf text (they never open
#     files) the key's file name is also neutralized, unless the command has a
#     pipe, an input redirect, xargs or parallel that could feed it on.
#     Nothing is masked in a redirect target or anywhere after > >> < in the
#     segment, so `echo x > "$HOME/.claude/vajra/<key>"` is still seen.
#     VAJRA_ATMAN_ASSUME_HUMAN is still matched on the raw command.
#   * The same separator masking applies to the quoted arguments of
#     `python3|bash <...>/vajra_kb.py|vajra_tasks.py|vajra-campaign.sh`: they
#     are titles, bodies and summaries, not code.
#   * A git commit -m/--message value (also -qm, -am, ...; including the
#     "$(cat <<'EOF' ... EOF)" form) becomes __MSG__: never a path.
# Shells, interpreters, eval, xargs and the like keep their quoted text intact,
# so code passed to them is still analyzed. The pass exits 3 on syntax it does
# not model exactly ($( ), backticks, ${...} other than ${HOME}, $'...',
# unterminated quotes, unusual heredoc delimiters); the raw command is then
# analyzed as before. Every fallback is the old, stricter behaviour.
PRE_AWK='
function flush() { printf "%s", buf; buf = "" }
function emit(s) { if (length(s) > 512) { flush(); printf "%s", s; return }; buf = buf s; if (length(buf) > 512) flush() }
function bad() { unsafe = 1; exit 3 }
function startword() { if (!inword) { inword = 1; cw = ""; cwq = 0 } }
function endword() {
  if (!inword) return
  inword = 0
  if (skipw) { skipw = 0 }
  else {
    if (nw == 0) { segfirst = (cwq ? "?" : cw); segbase = segfirst; sub(/.*\//, "", segbase); if (index(DSCR, " " segbase " ")) dataseg = 1 }
    else if (nw == 1 && !cwq && index(DINT, " " segbase " ")) { b2 = cw; sub(/.*\//, "", b2); if (index(DSCR, " " b2 " ")) dataseg = 1 }
    else if (segbase == "git" && !gitdone) {
      if (gitskip) gitskip = 0
      else if (cw ~ /^-/) { if (cw == "-C" || cw == "-c" || cw == "--git-dir" || cw == "--work-tree" || cw == "--namespace") gitskip = 1 }
      else { gitdone = 1; if (!cwq && cw == "commit") commit = 1 }
    }
    nw++
  }
  prevw = (cwq ? "?" : cw)
}
function newseg() { endword(); segfirst = ""; segbase = ""; nw = 0; commit = 0; gitdone = 0; gitskip = 0; skipw = 0; prevw = ""; segredir = 0; dataseg = 0 }
function closehd(  h) { for (h = 1; h <= nh; h++) HOPEN[h] = 0 }
function killhd(  h) { for (h = 1; h <= nh; h++) if (HOPEN[h]) HOK[h] = 0 }
# Never mask a redirect target, or anything after a redirect in the segment.
function masking() { return (nw >= 1 && !skipw && !segredir && (dataseg || index(MASK, " " segbase " ") > 0)) }
function msgarg() { return (commit && (prevw ~ /^-[A-Za-z]*m$/ || prevw == "--message")) }
function ishomebrace(j) { return (C[j] == "{" && C[j+1] == "H" && C[j+2] == "O" && C[j+3] == "M" && C[j+4] == "E" && C[j+5] == "}") }
function emitrange(a, b, mask,   t) {
  if (b < a) return
  t = substr(src, a, b - a + 1)
  if (mask) {
    gsub(/[;|&<>()\n]/, " ", t)
    if (KEYOK && (segbase == "echo" || segbase == "printf")) gsub(/\.[Hh][Mm][Aa][Cc]-[Kk][Ee][Yy]/, "__KEYNAME__", t)
  }
  emit(t)
}
# git commit -m "$(cat <<\047EOF\047 ... EOF)": index after the closing quote, or 0.
function commitheredoc(i,   j, d, dash, s, k, ok, m) {
  if (C[i+1] != "$" || C[i+2] != "(" || C[i+3] != "c" || C[i+4] != "a" || C[i+5] != "t" || C[i+6] != " ") return 0
  j = i + 7
  while (C[j] == " ") j++
  if (C[j] != "<" || C[j+1] != "<") return 0
  j += 2; dash = 0
  if (C[j] == "-") { dash = 1; j++ }
  while (C[j] == " ") j++
  if (C[j] != "\047") return 0
  j++; d = ""
  while (j <= n && C[j] ~ /[A-Za-z0-9_]/) { d = d C[j]; j++ }
  if (d == "" || C[j] != "\047") return 0
  j++
  while (C[j] == " " || C[j] == "\t") j++
  if (C[j] != "\n") return 0
  j++
  m = length(d)
  while (j <= n) {
    s = j
    if (dash) while (C[s] == "\t") s++
    k = s; while (k <= n && C[k] != "\n") k++
    ok = (k - s == m)
    if (ok) for (x = 0; x < m; x++) if (C[s+x] != substr(d, x+1, 1)) { ok = 0; break }
    j = k + 1
    if (ok) {
      while (C[j] == " " || C[j] == "\t" || C[j] == "\n") j++
      if (C[j] == ")" && C[j+1] == "\"") return j + 2
      return 0
    }
  }
  return 0
}
{ src = (NR == 1) ? $0 : src "\n" $0 }
END {
  if (unsafe) exit 3
  MASK = " echo printf cat head tail grep egrep fgrep wc jq test [ ls stat diff cut tr nl basename dirname realpath true false "
  # Vajra scripts whose quoted arguments are data (titles, bodies, queries):
  # `python3|bash <...>/vajra_kb.py ...` and friends.
  DSCR = " vajra_kb.py vajra_tasks.py vajra-campaign.sh "
  DINT = " python python3 bash sh "
  SINK = MASK "tee "
  SEPS = ";|&<>()\n"
  # The key file name inside quoted reader text is just text, unless something
  # in the command could feed that text on as a file name.
  KEYOK = (src !~ /[|<]|xargs|parallel/)
  n = split(src, C, "")
  buf = ""; inword = 0; cw = ""; cwq = 0; nw = 0; segfirst = ""; segbase = ""; commit = 0; skipw = 0; prevw = ""; segredir = 0; dataseg = 0
  paren = 0; nh = 0; lastop = ""
  i = 1
  while (i <= n) {
    c = C[i]
    if (c == "\\") {
      startword(); cwq = 1
      emit(c); if (i < n) emit(C[i+1])
      i += 2; lastop = ""; continue
    }
    if (c == "\047") {
      at = !inword; startword(); cwq = 1
      j = i + 1; while (j <= n && C[j] != "\047") j++
      if (j > n) bad()
      if (at && msgarg()) emit("__MSG__")
      else { emit(c); emitrange(i + 1, j - 1, masking()); emit(c) }
      i = j + 1; lastop = ""; continue
    }
    if (c == "\"") {
      at = !inword; startword(); cwq = 1
      if (at && msgarg()) { e = commitheredoc(i); if (e) { emit("__MSG__"); i = e; lastop = ""; continue } }
      j = i + 1
      while (j <= n && C[j] != "\"") {
        if (C[j] == "\\") { j += 2; continue }
        if (C[j] == "`") bad()
        if (C[j] == "$" && (C[j+1] == "(" || (C[j+1] == "{" && !ishomebrace(j+1)))) bad()
        j++
      }
      if (j > n) bad()
      if (at && msgarg()) emit("__MSG__")
      else { emit(c); emitrange(i + 1, j - 1, masking()); emit(c) }
      i = j + 1; lastop = ""; continue
    }
    if (c == "`") bad()
    if (c == "$") {
      if (C[i+1] == "(" || C[i+1] == "\047" || (C[i+1] == "{" && !ishomebrace(i+1))) bad()
      startword(); if (length(cw) < 64) cw = cw c
      emit(c); i++; lastop = ""; continue
    }
    if (c == "#" && !inword) {
      while (i <= n && C[i] != "\n") { emit(C[i]); i++ }
      continue
    }
    if (c == " " || c == "\t") { endword(); emit(c); i++; continue }
    if (c == "\n") {
      newseg(); emit(c); i++; lastop = ""
      for (h = 1; h <= nh; h++) {
        bs = i; m = length(HD[h]); found = 0; hasexp = 0
        while (i <= n) {
          s = i
          if (HDASH[h]) while (C[s] == "\t") s++
          k = s; while (k <= n && C[k] != "\n") k++
          ok = (k - s == m)
          if (ok) for (x = 0; x < m; x++) if (C[s+x] != substr(HD[h], x+1, 1)) { ok = 0; break }
          if (!ok && !HQ[h]) for (x = i; x < k; x++) if (C[x] == "`" || (C[x] == "$" && C[x+1] == "(")) hasexp = 1
          i = k + 1
          if (ok) { found = 1; break }
        }
        if (i > n + 1) i = n + 1
        if (HOK[h] && (HQ[h] || !hasexp)) emit("\n")
        else emitrange(bs, i - 1, 0)
      }
      nh = 0; paren = paren
      continue
    }
    if (c == ";") { newseg(); closehd(); emit(c); i++; lastop = ""; continue }
    if (c == "&") {
      if (lastop == ">" || lastop == "<") { emit(c); i++; lastop = ""; continue }
      endword()
      if (C[i+1] == ">") { emit("&>"); i += 2; if (C[i] == ">") { emit(">"); i++ }; skipw = 1; segredir = 1; lastop = ""; continue }
      if (C[i+1] == "&") { newseg(); closehd(); emit("&&"); i += 2; lastop = ""; continue }
      newseg(); closehd(); emit(c); i++; lastop = ""; continue
    }
    if (c == "|") {
      if (lastop == ">") { emit(c); i++; lastop = ""; continue }
      if (C[i+1] == "|") { newseg(); closehd(); emit("||"); i += 2; lastop = ""; continue }
      killhd(); newseg(); emit(c); i++
      if (C[i] == "&") { emit("&"); i++ }
      lastop = ""; continue
    }
    if (c == "(") { paren = 1; killhd(); newseg(); emit(c); i++; lastop = ""; continue }
    if (c == ")") { newseg(); emit(c); i++; lastop = ""; continue }
    if (c == ">") { endword(); emit(c); i++; skipw = 1; segredir = 1; lastop = ">"; if (C[i] == ">") { emit(">"); i++ }; continue }
    if (c == "<") {
      endword(); segredir = 1
      if (C[i+1] == "<" && C[i+2] == "<") { emit("<<<"); i += 3; skipw = 1; lastop = ""; continue }
      if (C[i+1] == "<") {
        j = i + 2; dash = 0
        if (C[j] == "-") { dash = 1; j++ }
        while (C[j] == " " || C[j] == "\t") j++
        q = 0; d = ""
        if (C[j] == "\047" || C[j] == "\"") {
          qc = C[j]; q = 1; k = j + 1
          while (k <= n && C[k] != qc) { d = d C[k]; k++ }
          if (k > n) bad()
          j = k + 1
        } else {
          if (C[j] == "\\") { q = 1; j++ }
          while (j <= n && C[j] ~ /[A-Za-z0-9_.-]/) { d = d C[j]; j++ }
        }
        if (d == "" || d !~ /^[A-Za-z0-9_.-]+$/) bad()
        if (j <= n && index(" \t\n;&|<>)", C[j]) == 0) bad()
        nh++; HD[nh] = d; HQ[nh] = q; HDASH[nh] = dash; HOPEN[nh] = 1
        HOK[nh] = (nw >= 1 && !paren && index(SINK, " " segbase " ") > 0)
        emitrange(i, j - 1, 0)
        i = j; lastop = ""; continue
      }
      emit(c); i++; skipw = 1; lastop = "<"; continue
    }
    startword(); if (length(cw) < 64) cw = cw c
    emit(c); i++; lastop = ""
  }
  flush()
  exit 0
}
'
SCAN="$CMD"; RAW_FALLBACK=0
case "$CMD" in
  *"<<"*|*"'"*|*'"'*)
    if PRE_OUT="$(printf '%s' "$CMD" | LC_ALL=C awk "$PRE_AWK" 2>/dev/null)"; then SCAN="$PRE_OUT"; else RAW_FALLBACK=1; fi
    ;;
esac

# Normalized copy: quotes and backslashes removed (they change how the shell
# splits words, not which programs run), whitespace collapsed, newlines become
# command separators, $HOME expanded.
# A standalone empty argument ("" or '') becomes the token __E__, so an empty
# grep pattern still counts as the pattern operand.
#
# Bounded cost: bash 3.2 ${var//x/y} is O(length x matches), so a long command
# (a big heredoc for an interpreter, say) could run past the hook timeout,
# which blocks the call. From 1 KB the same normalization (N, N2 and T below)
# runs as one linear awk pass, and above GUARD_MAX_SCAN the command is blocked
# at once with a clear reason instead of timing out.
GUARD_MAX_SCAN=32768
NORM_AWK='
BEGIN { RS = "\001" }
{ src = (NR == 1) ? $0 : src RS $0 }
END {
  q = sprintf("%c", 39)
  H = ENVIRON["HOME"]; gsub(/[\\&]/, "\\\\&", H)
  N = " " src " "
  gsub(/\t/, " ", N)
  re = " (\"\"|" q q ")+ "
  while (match(N, re)) N = substr(N, 1, RSTART - 1) " __E__ " substr(N, RSTART + RLENGTH)
  N = substr(N, 2, length(N) - 2)
  gsub(/["\\]/, "", N); gsub(q, "", N)
  gsub(/\n/, " ; ", N); gsub(/\r/, " ; ", N)
  gsub(/\$\{HOME\}/, H, N); gsub(/\$HOME/, H, N)
  gsub(/  +/, " ", N)
  N2 = " " N " "; gsub(/[,\[\]()]/, " ", N2); gsub(/  +/, " ", N2)
  T = " " N " "
  gsub(/\$\(/, " ; ", T); gsub(/`/, " ; ", T)
  gsub(/\(/, " ; ", T); gsub(/\)/, " ; ", T); gsub(/,/, " ", T)
  gsub(/>\|/, " > ", T)
  gsub(/>&/, " __DUP__ ", T); gsub(/&>/, " > ", T)
  gsub(/&/, " ; ", T); gsub(/;/, " ; ", T); gsub(/\|/, " PIPE ", T)
  gsub(/>/, " > ", T); gsub(/</, " < ", T)
  printf "%s\n%s\n%s\n", N, N2, T
}
'
BIGNORM=0
if [ "${#SCAN}" -gt "$GUARD_MAX_SCAN" ]; then
  block "command too large to analyze ($(( ${#SCAN} / 1024 )) KB after skipping inert heredoc text; limit $(( GUARD_MAX_SCAN / 1024 )) KB). Write the script or data with the Write tool and run the file instead."
fi
if [ "${#SCAN}" -ge 1024 ]; then
  # N, N2 and T hold no newlines (they become " ; "), so one line each. (Not
  # ${x#*sep}: that is quadratic in bash 3.2.)
  NORM_OUT="$(printf '%s' "$SCAN" | LC_ALL=C awk "$NORM_AWK" 2>/dev/null)" || block "normalization failed (fail-closed)"
  N="" N2="" T=""
  { IFS= read -r N; IFS= read -r N2; IFS= read -r T; } <<<"$NORM_OUT"
  [ -n "$T" ] || block "normalization failed (fail-closed)"
  BIGNORM=1
else
N=" ${SCAN//$'\t'/ } "
RE_EMPTY=" ((\"\"|'')+) "
while [[ "$N" =~ $RE_EMPTY ]]; do N="${N/"${BASH_REMATCH[0]}"/ __E__ }"; done
N="${N# }"; N="${N% }"
N="${N//\"/}"; N="${N//\'/}"; N="${N//\\/}"
N="${N//$'\n'/ ; }"; N="${N//$'\r'/ ; }"; N="${N//$'\t'/ }"
N="${N//'${HOME}'/$HOME}"; N="${N//'$HOME'/$HOME}"
while [[ "$N" == *"  "* ]]; do N="${N//  / }"; done
fi

RE_OUT=""
re_escape() { # ERE-escape -> RE_OUT
  local s="$1" o="" ch i
  for ((i = 0; i < ${#s}; i++)); do
    ch="${s:i:1}"
    case "$ch" in
      [].[\\*^\$+?\(\)\{\}\|/]) o="$o\\$ch" ;;
      *) o="$o$ch" ;;
    esac
  done
  RE_OUT="$o"
}
re_escape "$HOME"; HOME_RE="$RE_OUT"

# --- Whole-command signatures --------------------------------------------
# "Application Support" is spelled plainly when the raw command has the two words joined by a space, or by a
# backslash-escaped space (the form tab completion writes). Either way N holds a real space, and the second pass
# joins it and judges the real path. (The old pattern was a space followed by anything, so it also accepted text
# between the words; that is kept, and the path analysis still runs on every form.)
appsup_plain() {
  [[ "$CMD" == *[Aa]pplication[\ ]*[Ss]upport* || "$CMD" == *[Aa]pplication\\\ [Ss]upport* ]]
}
shopt -s nocasematch
[[ "$N" == *".hmac-key"* ]] && block "the Vajra HMAC key is never a command target"
# Checked on the raw command too: even a heredoc body may not set it.
[[ "$N" == *VAJRA_ATMAN_ASSUME_HUMAN* || "$CMD" == *VAJRA_ATMAN_ASSUME_HUMAN* ]] && block "VAJRA_ATMAN_ASSUME_HUMAN is for a human at a terminal, not the agent"
[[ "$N" == *--desktop-approval* || "$CMD" == *--desktop-approval* ]] && block "--desktop-approval is for the Vajra Desktop app, not the agent"
[[ "$N" == *VAJRA_OPERATOR_SIGN* || "$CMD" == *VAJRA_OPERATOR_SIGN* ]] && block "VAJRA_OPERATOR_SIGN is for the operator re-signing the manifest, not the agent"
if [[ "$CMD" == *vajradesktop/control* ]]; then
  [ -n "$AID" ] && block "workers may not touch the Vajra Desktop control dir"
  appsup_plain || block "the Vajra Desktop control dir is named by an unusual spelling; write the path plainly"
fi
# ADR-16: the same for the app data dir <data> and every path below it, so a
# spelling the path analysis cannot resolve never hides a write there. A command
# names <data> path-wise when, in the raw text or with quotes and backslashes
# removed (Vajra''Desktop, "Vajra"Desktop, Vajra\Desktop), it has /VajraDesktop at
# a word end, VajraDesktop/ or VajraDesktop\. Such a command:
#   * must spell "Application Support" plainly (not ${IFS}, $'\040', ...);
#   * may use no shell expansion but $HOME/${HOME}: a variable, $(...), backticks,
#     $'...' or ${...} could hold or build the path (D=<data>; rm -rf "$D").
# A command with an "Application ... Support/Vajra..." path (other than VajraWorktrees,
# the writable <work>) gets the expansion rule too, so Vajra$V is refused as well.
# bash 3.2 pattern substitution is quadratic (a 4 KB command took ~600 ms): tr strips linearly.
DRAW="$CMD"
case "$CMD" in *[\"\'\\]*) DRAW="$(printf '%s' "$CMD" | LC_ALL=C tr -d "\"'\\\\")" ;; esac
DNAMED=0 DAPP=0
for v in "$CMD" "$DRAW"; do
  # Every form below holds "vajradesktop" or "support/vajra"; skip the costlier patterns otherwise.
  [[ "$v" == *vajradesktop* || "$v" == *support/vajra* ]] || continue
  case "$v" in
    */vajradesktop|*/vajradesktop[!a-z0-9_.-]*|*vajradesktop/*|*vajradesktop\\*) DNAMED=1 ;;
  esac
  case "$v" in *application*support/vajra|*application*support/vajra[!w]*) DAPP=1 ;; esac
done
if [ "$DNAMED" = 1 ]; then
  appsup_plain || block "the Vajra Desktop app data dir is named by an unusual spelling; write the path plainly"
fi
if [ "$DNAMED" = 1 ] || [ "$DAPP" = 1 ]; then
  DX="${CMD//'${HOME}'/}"
  [[ "$DX" == *'$HOME'[A-Za-z0-9_]* ]] && block "a command that names the Vajra Desktop app data dir may not use shell variables; write the path literally (\$HOME is fine)"
  DX="${DX//'$HOME'/}"
  [[ "$DX" == *'$'* || "$DX" == *'`'* ]] && block "a command that names the Vajra Desktop app data dir may not use shell variables or substitutions; write the path literally (\$HOME is fine)"
fi
shopt -u nocasematch

RE_RF="(^| )-[[:alnum:]]*([rR][[:alnum:]]*f|f[[:alnum:]]*[rR])[[:alnum:]]*( [^;|&]*)? (/|/\*|~|~/|~/\*|${HOME_RE}|${HOME_RE}/|${HOME_RE}/\*)( |;|\||&|$)"
# N2 also splits on the punctuation interpreters use around argv lists,
# so python -c "subprocess.run(['rm','-rf','/'])" is seen as rm -rf /.
if [ "$BIGNORM" = 0 ]; then
N2=" $N "; N2="${N2//,/ }"; N2="${N2//[/ }"; N2="${N2//]/ }"; N2="${N2//(/ }"; N2="${N2//)/ }"
while [[ "$N2" == *"  "* ]]; do N2="${N2//  / }"; done
fi
if [[ "$N" =~ $RE_RF ]] || [[ "$N2" =~ $RE_RF ]]; then block "recursive force delete of / or HOME"; fi
[[ "$N" =~ --no-preserve-root ]] && block "--no-preserve-root"
RE_PUSH='(^|[;&| ])git( [^;&|]*)? push( [^;&|]*)? (--force|--force-with-lease|--force-if-includes|-f|-[a-z]*f[a-z]*|\+[^ ;&|]+)( |=|;|&|\||$)'
[[ "$N" =~ $RE_PUSH ]] && block "git force push"
RE_RESET='(^|[;&| ])git( [^;&|]*)? reset( [^;&|]*)? --hard( |;|&|\||$)'
[[ "$N" =~ $RE_RESET ]] && block "git reset --hard (destroys uncommitted work)"
RE_CLEAN='(^|[;&| ])git( [^;&|]*)? clean( [^;&|]*)? (-[a-z]*f[a-z]*|--force)( |;|&|\||$)'
[[ "$N" =~ $RE_CLEAN ]] && block "git clean -f (deletes untracked files)"
RE_CHMOD='(^|[;&| ])chmod( [^;&|]*)? (0?777|a\+rwx|ugo\+rwx|=rwx)( |;|&|\||$)'
[[ "$N" =~ $RE_CHMOD ]] && block "world-writable chmod"
RE_CHR="(^|[;&| ])ch(mod|own|grp)( [^;&|]*)? -[a-zA-Z]*R[a-zA-Z]*( [^;&|]*)? (/|~|${HOME_RE}|/(etc|usr|var|bin|sbin|lib|System|Library|Applications|private|Users|home))/?( |;|&|\||$)"
[[ "$N" =~ $RE_CHR ]] && block "recursive chmod/chown of a system root or HOME"
RE_DEV='(of=|> ?)/dev/(r?disk|sd|hd|nvme|vd|xvd|mmcblk|loop)'
[[ "$N" =~ $RE_DEV ]] && block "raw write to a disk device"
RE_MKFS='(^|[;&| ])(sudo )?(mkfs(\.[a-z0-9]+)?|newfs(_[a-z]+)?|diskutil (erase[a-zA-Z]*|zeroDisk|secureErase|partitionDisk))( |;|$)'
[[ "$N" =~ $RE_MKFS ]] && block "filesystem format / disk erase"
RE_FORK=':\(\) ?\{ ?: ?\| ?: ?& ?\} ?; ?:'
[[ "$N" =~ $RE_FORK ]] && block "fork bomb"
RE_DECODE='(base64( [^;|&]*)? (-d|--decode|-D)|xxd( [^;|&]*)? -r|openssl( [^;|&]*)? (enc|base64)( [^;|&]*)? -d)'
RE_EXEC_SINK='(\| ?(sudo )?(ba|z|k|da|fi)?sh( |;|$)|(ba|z|k|da)?sh -c|(^|[;&| ])eval |\$\(|`|(^|[;&| ])(source|\.) )'
if [[ "$N" =~ $RE_DECODE ]] && [[ "$N" =~ $RE_EXEC_SINK ]]; then block "decode-and-execute"; fi
RE_DLEXEC='((ba|z|k|da)?sh -c|(^|[;&| ])eval|(^|[;&| ])(source|\.)) [^;]*(curl|wget) '
[[ "$N" =~ $RE_DLEXEC ]] && block "download-and-execute"
RE_INLINE='(^|[;&| /])(python[0-9.]*|ruby|perl|node|php|deno|bun)( [^;|&]*)? -(c|e|r|-eval) '
if [[ "$N" =~ $RE_INLINE ]]; then
  RE_DEC2='(b64decode|base64|fromhex|unhexlify|codecs\.decode|atob\(|Buffer\.from)'
  RE_NET2='(urlopen|urllib|requests\.(get|post)|http\.get|fetch\(|socket\.)'
  RE_RUN2='(exec\(|eval\(|system\(|popen|spawn|subprocess|child_process|Function\()'
  if [[ "$N" =~ $RE_RUN2 ]] && { [[ "$N" =~ $RE_DEC2 ]] || [[ "$N" =~ $RE_NET2 ]]; }; then
    block "interpreter inline code that decodes or downloads and then executes"
  fi
fi

# Credential exfiltration: a network tool plus credential material in one command.
RE_NET='(^|[;&| /])(curl|wget|nc|ncat|netcat|telnet|scp|sftp|rsync|ftp|socat|http|https|xh)( |;|$)'
if [[ "$N" =~ $RE_NET ]]; then
  NC="$N"
  RE_IKEY='(^| )-i ?[^ ]+'
  while [[ "$NC" =~ $RE_IKEY ]]; do NC="${NC/"${BASH_REMATCH[0]}"/ }"; done  # ssh -i keyfile is not exfil
  RE_CRED="(^|[ =@<>:/])(~/|${HOME_RE}/)?\.(aws|ssh|gnupg|kube|netrc|npmrc|pypirc|pgpass|git-credentials)(/| |;|$)|id_(rsa|dsa|ecdsa|ed25519)( |;|$|\.)|/etc/shadow|\.config/gh/|\.docker/config\.json|(^|[ =@<>/])\.env(\.[a-z]+)?( |;|$)"
  [[ "$NC" =~ $RE_CRED ]] && block "network command combined with credential material"
  RE_ENVDUMP='(^|[;&| ])(env|printenv|set|export -p)( [^;|&]*)? ?\|'
  [[ "$N" =~ $RE_ENVDUMP ]] && block "environment dump piped toward the network"
fi
[[ "$N" =~ /etc/(shadow|master\.passwd) ]] && block "access to system password hashes"
RE_DNS='(^|[;&|(] ?|\$\( ?)(dig|nslookup|host|drill) '
RE_DNS2='(\$\(|`|base64|xxd)'
if [[ "$N" =~ $RE_DNS ]] && [[ "$N" =~ $RE_DNS2 ]]; then block "DNS exfiltration (lookup of computed data)"; fi

# --- Per-segment analysis --------------------------------------------------
if [ "$BIGNORM" = 0 ]; then
T=" $N "
T="${T//'$('/ ; }"; T="${T//\`/ ; }"
T="${T//(/ ; }"; T="${T//)/ ; }"; T="${T//,/ }"
T="${T//>|/ > }"  # >| (noclobber override) is a plain redirect, never a pipe
T="${T//>&/ __DUP__ }"; T="${T//&>/ > }"
T="${T//&/ ; }"; T="${T//;/ ; }"; T="${T//|/ PIPE }"
T="${T//>/ > }"; T="${T//</ < }"
fi
set -f
TOKS=( $T )
set +f
# Bounded cost: per-token analysis is the slow part, so a command that splits
# into more words than this is refused at once instead of running toward the
# hook timeout (which also blocks, but slowly and without a reason).
GUARD_MAX_TOKENS=2500
[ "${#TOKS[@]}" -le "$GUARD_MAX_TOKENS" ] || block "command too complex to analyze (${#TOKS[@]} words after splitting on shell operators; limit ${GUARD_MAX_TOKENS}). Write the script or data with the Write tool and run the file instead."

# Readers: commands with no file-writing or command-running mode. (sort -o,
# uniq OUT, tree -o, xxd -r IN OUT, yq -i write files, so they are not here.)
READERS=" cat less more head tail grep egrep fgrep rg ag ack wc diff cmp file stat ls du sha1sum sha256sum sha512sum shasum md5 md5sum cksum cut nl od hexdump strings column bat realpath readlink dirname basename echo printf test [ true false jq tr pwd which type whoami id date env printenv "
SHELLS=" sh bash zsh dash ksh fish "
INTERPS=" python python2 python3 perl ruby node php lua Rscript deno bun "
WRAPPERS=" sudo command nohup time nice ionice env "
NOT_SCRIPTS=" install.sh install-skills.sh generate-manifest.sh "
# Vajra scripts whose arguments are data (titles, bodies, queries), not paths
# they write: their text may name protected paths. Output flags stay checked.
DATA_SCRIPTS=" vajra_kb.py vajra_tasks.py vajra-campaign.sh "

# #190 H1 (4): a campaign that Vajra Desktop's core runs is written only by the core, which runs the scripts as its own
# children, never in an agent session. Its slug is the one a core seats file records: campaign_slug in
# <data>/control/lead/<run>/seats.json. Matched by those recorded slugs, never by the deliver- prefix, because the CLI
# workflow uses that prefix too. A seats file that exists but can't be read stands for both slugs its run could have.
CORE_SLUGS=" "; CORE_ANY=0; CORE_UNKNOWN=0; CORE_PTR=0; CORE_DATA=( "$DDATA" )
SLUG_LIT='^[a-z0-9][a-z0-9-]*$'
# The core honours VAJRA_DESKTOP_DATA (app/core/src/paths.rs), so its seats may live there too (bhishma #194 F5).
if [ -n "${VAJRA_DESKTOP_DATA:-}" ]; then canon "$VAJRA_DESKTOP_DATA" /; CORE_DATA+=( "$CANON" ); fi
core_slugs_load() {
  local ng=0 d f run s base lead
  shopt -q nullglob && ng=1
  shopt -s nullglob
  for base in "${CORE_DATA[@]}"; do
  lead="$base/control/lead"
  # A lead dir (or a run dir) that exists but can't be listed: core runs may exist with slugs we can't read.
  if [ -e "$lead" ] && { [ ! -d "$lead" ] || [ ! -r "$lead" ] || [ ! -x "$lead" ]; }; then CORE_UNKNOWN=1; continue; fi
  for d in "$lead"/*/; do
    d="${d%/}"; run="${d##*/}"; f="$d/seats.json"
    if [ ! -r "$d" ] || [ ! -x "$d" ]; then CORE_ANY=1; CORE_SLUGS+="deliver-$run deliver-$run-og "; continue; fi
    [ -e "$f" ] || [ -L "$f" ] || continue   # no seats file: not a core run (the CLI workflow writes its campaign)
    s=""
    if [ -f "$f" ] && [ ! -L "$f" ] && [ -r "$f" ]; then
      if command -v jq >/dev/null 2>&1; then
        s="$(jq -r '.campaign_slug | if type == "string" then . else empty end' "$f" 2>/dev/null)" || s=""
      elif [ -x "$PYTHON3" ]; then
        s="$("$PYTHON3" -I -c 'import json,sys
v = json.load(open(sys.argv[1], encoding="utf-8")).get("campaign_slug")
print(v if isinstance(v, str) else "")' "$f" 2>/dev/null)" || s=""
      fi
    fi
    CORE_ANY=1
    if [[ "$s" =~ $SLUG_LIT ]]; then
      CORE_SLUGS+="$s "
    else
      CORE_SLUGS+="deliver-$run deliver-$run-og "
    fi
  done
  done
  # An active pointer that names a core slug: only the core's record repo has one (vajra-62, #190).
  for f in "$HOME/.claude/vajra/campaigns"/.active-*; do
    s="$(tr -d ' \r\n' < "$f" 2>/dev/null)"
    [ -n "$s" ] && [[ "$CORE_SLUGS" == *" $s "* ]] && CORE_PTR=1
  done
  [ "$ng" = 1 ] || shopt -u nullglob
  return 0
}
core_slug_block() { block "campaign $1 is run by Vajra Desktop: only the app writes it, so agents may read it (show, list, verify, status) but never change it"; }
# 0 when the command has a shell expansion outside single quotes: $ (but $PWD and $HOME), a backtick, or (unquoted)
# a brace or glob character. Single-quoted text is literal, which is how the CLI workflow quotes every value.
# With $1 = strict (#194 H2: a nested shell or eval runs its quoted body as code), single quotes protect nothing.
core_expands() {
  printf '%s' "$SCAN" | LC_ALL=C awk -v strict="${1:-}" '
  function okvar(s, i) { return substr(s, i + 1, 7) ~ /^(PWD|HOME)([^A-Za-z0-9_]|$)/ || substr(s, i + 1, 7) ~ /^\{(PWD|HOME)\}/ }
  BEGIN { RS = "\001"; sq = sprintf("%c", 39) }
  {
    s = $0; n = length(s); q = 0
    for (i = 1; i <= n; i++) {
      c = substr(s, i, 1)
      if (strict != "" && (c == sq || c == "\"")) continue
      if (q == 1) { if (c == sq) q = 0; continue }
      if (c == "\\") { i++; continue }
      if (c == "`") exit 0
      if (c == "$") { if (okvar(s, i)) continue; exit 0 }
      if (q == 2) { if (c == "\"") q = 0; continue }
      if (c == sq) { q = 1; continue }
      if (c == "\"") { q = 2; continue }
      if (index("{}*?[]", c)) exit 0
    }
    exit 1
  }'
}
# #194 (bhishma F1-F4, claude thread): one check over the whole command, before it is split into segments.
# Either script is found by file name wherever its copy lives, nested shells and xargs included. A use counts as a
# read only when the word after it (redirects skipped) is a literal read subcommand; anything else is a write.
# While a core run exists, a write is refused when the pre-pass could not model the command, when it has a shell
# expansion, or when a core slug appears anywhere in it (also as vajra-campaign.sh create would sanitize it).
core_campaign_guard() {
  local X lc hit=0 was=0
  # Fork-free first test (most calls stop here); any case, as APFS is case-insensitive.
  shopt -q nocasematch && was=1
  shopt -s nocasematch
  # Wide on purpose (#194 H1): a name built by a glob or variable may not spell the script out.
  [[ "$N" == *vajra* || "$N" == *campaign* || "$N" == *task* || "$N" == *scripts* || "$N" == *.claude* ]] && hit=1
  [ "$was" = 1 ] || shopt -u nocasematch
  [ "$hit" = 1 ] || return 0
  lc="$(printf '%s' "$N" | tr '[:upper:]' '[:lower:]')"
  core_slugs_load
  [ "$CORE_ANY" = 1 ] || [ "$CORE_UNKNOWN" = 1 ] || return 0
  X="${lc//&>/ > }"; X="${X//>&/ > }"
  X="${X//;/ ; }"; X="${X//|/ ; }"; X="${X//&/ ; }"; X="${X//(/ ; }"; X="${X//)/ ; }"; X="${X//\`/ ; }"
  X="${X//>/ > }"; X="${X//</ < }"
  local W=() i j nw t sub write=0 cwrite=0 name nested=0 feeder=0 lit
  local why="a write through vajra-campaign.sh or vajra_tasks.py while Vajra Desktop runs a campaign"
  set -f; W=( $X ); set +f
  nw=${#W[@]}
  for ((i = 0; i < nw; i++)); do
    name="${W[$i]##*/}"
    # H1: a word that may expand to either script's name is refused before any write is looked for.
    case "$name" in
      *[\$\{\}\*\?\[\]]*)
        lit="${name//[\$\{\}\*\?\[\]]/}"
        # As a pattern it matches a script's name; or its fixed letters hint at one and it looks like a script path or
        # file name (a slash, or ending in py or sh, as $'..' leaves .py once its escape is dropped).
        if [[ vajra-campaign.sh == $name || vajra_tasks.py == $name ]] \
           || { [[ "$lit" == *vajra* || "$lit" == *campaign* || "$lit" == *task* || "${W[$i]}" == *scripts/* || "${W[$i]}" == *.claude/* ]] \
                && [[ "${W[$i]}" == */* || "$lit" == *py || "$lit" == *sh ]]; }; then
          block "while Vajra Desktop runs a campaign, name vajra-campaign.sh and vajra_tasks.py literally (no globs, variables or \$'...'): ${W[$i]}"
        fi ;;
    esac
    # H2: a nested shell or eval runs its quoted body as code. H3: xargs, parallel and find -exec add arguments the
    # command text never shows.
    case "$name" in
      sh|bash|zsh|dash|ksh) case "${W[$((i + 1))]:-}" in -*c*|"<") nested=1 ;; esac ;;
      eval|source) nested=1 ;;
      xargs|parallel) feeder=1 ;;
      -exec|-execdir|-ok|-okdir) feeder=1 ;;
    esac
    [ "$name" = vajra-campaign.sh ] || [ "$name" = vajra_tasks.py ] || continue
    sub=""
    for ((j = i + 1; j < nw; j++)); do
      t="${W[$j]}"
      case "$t" in
        ">"|"<") j=$((j + 1)); while [ "$j" -lt "$nw" ] && { [ "${W[$j]}" = ">" ] || [ "${W[$j]}" = "<" ]; }; do j=$((j + 1)); done; continue ;;
        [0-9]|[0-9][0-9]) case "${W[$((j + 1))]:-}" in ">"|"<") continue ;; esac ;;
      esac
      sub="$t"; break
    done
    if [ "$name" = vajra-campaign.sh ]; then
      case "$sub" in list|show|active|verify|verify-digest) continue ;; esac
      cwrite=1
    else
      case "$sub" in validate|ready|status|list|waves|graph|check-evidence) continue ;; esac
    fi
    write=1
  done
  [ "$write" = 1 ] || return 0
  [ "$RAW_FALLBACK" = 0 ] || block "$why must use plain quoting the guard can read (no \$( ), backticks, \$'...' or unclosed quotes)"
  [ "$feeder" = 0 ] || block "$why must name its arguments in the command itself, not through xargs, parallel or find -exec"
  if [ "$nested" = 1 ]; then
    core_expands strict && block "$why inside bash -c, sh -c or eval must have no variables (only \$PWD and \$HOME), substitutions, braces or globs, even in quotes"
  fi
  core_expands && block "$why must name everything literally: no variables (only \$PWD and \$HOME), substitutions, braces or globs outside single quotes"
  local NS s
  NS="$(printf '%s' "$lc" | tr -c 'a-z0-9-' '-' | tr -s '-')"
  for s in $CORE_SLUGS; do [[ "$NS" == *"$s"* ]] && core_slug_block "$s"; done
  [ "$CORE_UNKNOWN" = 1 ] && [[ "$NS" == *deliver-* ]] && block "$why can't name a deliver- campaign: the app's run folders can't be read, so any of them may be the app's"
  # No slug named: vajra-campaign.sh writes the active campaign of the cwd's repository. Only the core's record repo,
  # inside the app data dir, has a pointer that names a core slug: refuse a campaign write that may run there.
  if [ "$cwrite" = 1 ] && [ "$CORE_PTR" = 1 ]; then
    local d inapp=0
    canon "${CWD_IN:-/}" /
    for d in "${CORE_DATA[@]}"; do inside "$CANON" "$d" && inapp=1; done
    if [ "$inapp" = 1 ] || [[ "$NS" == *vajradesktop* || "$NS" == *git-dir* || "$NS" == *git-common-dir* || "$NS" == *git-work-tree* ]] \
       || [[ " $X " == *" cd "* || " $X " == *" pushd "* || " $X " == *" -c "* || " $X " == *" --chdir"* ]]; then
      block "$why may not change directory or git repository: the app's own campaign is the active one there"
    fi
  fi
  return 0
}
core_campaign_guard
GIT_READ=" status log diff show rev-parse ls-files ls-tree cat-file blame grep describe shortlog reflog remote branch tag config "
SYSROOTS=" /etc /usr /var /bin /sbin /lib /boot /dev /sys /opt /root /home /System /Library /Applications /private /private/etc /private/var /Users /Volumes /usr/local /usr/bin /usr/lib "

is_interp() { [[ "$SHELLS" == *" $1 "* || "$INTERPS" == *" $1 "* || "$1" =~ ^python[0-9.]+$ ]]; }

# A file name, or a file-name glob, that names an operator-only script -> 0 and NS_HIT.
# Matched in memory, never against the disk: the name decides, whatever the cwd, ~
# form or shell (bash, zsh **/) the glob is expanded by. Any case (APFS is
# case-insensitive). A glob with no letters besides .sh (*, *.sh) is not judged.
NS_HIT=""
not_script() {
  local n lit="${1//[\*\?\[\]]/}" r=1
  shopt -s nocasematch
  if [[ "$NOT_SCRIPTS" == *" $1 "* ]]; then
    r=0; NS_HIT="$1"
  elif [[ "$1" == *[\*\?\[]* ]] && [[ "${lit%.sh}" == *[a-z]* ]]; then
    for n in $NOT_SCRIPTS; do
      if [[ "$n" == $1 ]]; then r=0; NS_HIT="$n"; break; fi
    done
  fi
  shopt -u nocasematch
  return $r
}

# The tokenizer splits brace and zsh glob groups ({install,x}.sh, inst(a)ll.sh) at
# , ( ), so they are judged here on the whole command: each group is replaced by each
# of its , or | alternatives and by *, and no variant's file name may name an
# operator-only script. (Strict: a read of such a name is refused too.)
RE_GROUP='[{(]([^{}() ]*)[})]'
if [[ "$N" == *[\{\(]* ]]; then
  set -f
  for k in $N; do
    case "$k" in *[\{\(]*) ;; *) continue ;; esac
    vs=( "$k" ) i=0
    while [ "$i" -lt "${#vs[@]}" ] && [ "${#vs[@]}" -le 64 ]; do
      g="${vs[$i]}"; i=$((i + 1))
      if [[ "$g" =~ $RE_GROUP ]]; then
        m="${BASH_REMATCH[0]}" alts="${BASH_REMATCH[1]//[,|]/ }"
        vs+=( "${g/"$m"/*}" )
        for a in $alts; do vs+=( "${g/"$m"/$a}" ); done
      elif [ "$g" != "$k" ] && not_script "${g##*/}"; then
        block "${k##*/} names ${NS_HIT}, which changes Vajra's wiring or integrity state; the operator runs it in their own terminal"
      fi
    done
  done
  set +f
fi

# Expand a token into candidate canonical paths -> CANDS array.
CANDS=()
candidates() { # token cwd
  local tok="$1" base="$2" e pat k=0
  local exp=()
  CANDS=()
  case "$tok" in *://*|"") return 0 ;; esac
  canon "$tok" "$base"; CANDS=( "$CANON" )
  case "$tok" in
    *[\*\?\[]*)
      pat="$tok"
      case "$pat" in "~"|"~/"*) pat="$HOME${pat#\~}" ;; esac
      shopt -s nullglob dotglob
      case "$pat" in /*) exp=( $pat ) ;; *) exp=( "$base"/$pat ) ;; esac
      shopt -u nullglob dotglob
      for e in ${exp[@]+"${exp[@]}"}; do
        k=$((k + 1)); [ "$k" -gt 64 ] && break
        canon "$e" "$base"; CANDS=( "${CANDS[@]}" "$CANON" )
      done
      ;;
  esac
  return 0
}

# Copiers (cp, gcp, rsync, ditto, install, ginstall) may READ protected
# sources when they write only outside every protected area. copy_layout parses
# one copier segment (eval_segment's SEG, n, wi, wb, SEGCWD) and sets
#   CP_OK=1   the layout is understood and the destination is safe
#   CP_SRC    " i j ... " token indexes of the source operands
# Anything it does not model leaves CP_OK=0, so the old rule (only readers may
# touch protected paths) applies. Unknown long options are assumed to take the
# next word as their value, so a misparse can only hide a source, never turn a
# destination or option value into one. Never relaxed: hard or symbolic links
# (the link would alias the protected file), --parents/--relative, rsync
# --remove-source-files, remote operands, install -d. The key, its directory and
# its ancestors stay blocked by the recursive-copy rule above.
CP_OK=0 CP_SRC=" "
cp_dest_ok() { # path -> 0 if a copy may land there (not protected, not an ancestor of ~/.claude)
  canon "$1" "$SEGCWD"
  classify "$CANON"
  [ -z "$PKIND" ] || return 1
  inside "$CL" "$CANON" && return 1
  key_anc "$CANON" && return 1
  return 0
}
copy_layout() {
  local c tok opt j ch want="" tdir="" tset=0 argl="" blk="" longok="" longblk="" dest d s dcands=() ops=() opi=()
  CP_OK=0; CP_SRC=" "
  case "$wb" in
    cp|gcp)
      argl="St"; blk="ls"
      longblk=" --link --symbolic-link --parents "
      longok=" --archive --attributes-only --backup --copy-contents --dereference --force --interactive --no-dereference --no-clobber --one-file-system --preserve --recursive --remove-destination --reflink --strip-trailing-slashes --update --verbose --no-target-directory --debug --keep-directory-symlink --context " ;;
    install|ginstall)
      argl="gmoStBfNTh"; blk="dlD"
      longblk=" --directory --strip-program "
      longok=" --backup --compare --preserve-timestamps --strip --verbose --no-target-directory --preserve-context --debug " ;;
    rsync)
      argl="eBfMT@"; blk="eR"
      longblk=" --remove-source-files --remove-sent-files --relative --link-dest --rsh --rsync-path --daemon --server --sender "
      longok=" --archive --recursive --verbose --quiet --checksum --update --links --copy-links --copy-unsafe-links --safe-links --hard-links --perms --executability --times --group --owner --devices --specials --dry-run --whole-file --one-file-system --existing --ignore-existing --delete --delete-before --delete-during --delete-after --delete-excluded --force --compress --progress --partial --human-readable --stats --itemize-changes --mkpath --inplace --append --sparse --omit-dir-times --no-perms --no-times --no-owner --no-group --size-only --ignore-times --keep-dirlinks --acls --xattrs --numeric-ids --prune-empty-dirs --no-motd " ;;
    ditto)
      longok=" --keepParent --sequesterRsrc --rsrc --norsrc --extattr --noextattr --qtn --noqtn --acl --noacl --persistRootless --nopersistRootless --hfsCompression --nohfsCompression --preserveHFSCompression --nopreserveHFSCompression --nocache --clone --noclone " ;;
    *) return 0 ;;
  esac
  for ((c = wi + 1; c < n; c++)); do
    tok="${SEG[$c]}"
    if [ -n "$want" ]; then
      [ "$want" = t ] && { tdir="$tok"; tset=1; }
      want=""; continue
    fi
    case "$tok" in
      ">"|"<"|__DUP__) c=$((c + 1)); continue ;;
      --)
        for ((c = c + 1; c < n; c++)); do
          case "${SEG[$c]}" in ">"|"<"|__DUP__) c=$((c + 1)) ;; *) ops+=( "${SEG[$c]}" ); opi+=( "$c" ) ;; esac
        done
        break ;;
      --?*)
        opt="${tok%%=*}"
        if [[ "$longok" == *" $opt "* ]]; then continue; fi
        # Exact blocker or an abbreviation of one (getopt accepts prefixes).
        for j in $longblk; do [[ "$j" == "$opt"* ]] && return 0; done
        if [ "$wb" != rsync ] && [ "$wb" != ditto ] && [[ "--target-directory" == "$opt"* && "${#opt}" -ge 3 ]]; then
          if [[ "$tok" == *=* ]]; then tdir="${tok#*=}"; tset=1; else want=t; fi
          continue
        fi
        [[ "$tok" == *=* ]] || want=v
        continue ;;
      -?*)
        for ((j = 1; j < ${#tok}; j++)); do
          ch="${tok:j:1}"
          [[ "$blk" == *"$ch"* ]] && return 0
          if [[ "$argl" == *"$ch"* ]]; then
            if [ $((j + 1)) -lt "${#tok}" ]; then
              [ "$ch" = t ] && [ "$wb" != rsync ] && { tdir="${tok:j+1}"; tset=1; }
            else
              want=v; [ "$ch" = t ] && [ "$wb" != rsync ] && want=t
            fi
            break
          fi
        done
        continue ;;
    esac
    ops+=( "$tok" ); opi+=( "$c" )
  done
  [ -z "$want" ] || return 0
  if [ "$tset" = 1 ]; then
    dest="$tdir"; [ "${#ops[@]}" -ge 1 ] || return 0
  else
    [ "${#ops[@]}" -ge 2 ] || return 0
    dest="${ops[${#ops[@]}-1]}"
    ops=( "${ops[@]:0:${#ops[@]}-1}" ); opi=( "${opi[@]:0:${#opi[@]}-1}" )
  fi
  # Remote operands (host:path, rsync://) are never part of a relaxed copy.
  for tok in "$dest" "${ops[@]}"; do
    case "$tok" in ""|*://*) return 0 ;; [!/]*:*) [[ "${tok%%/*}" == *:* ]] && return 0 ;; esac
  done
  candidates "$dest" "$SEGCWD"; dcands=( ${CANDS[@]+"${CANDS[@]}"} )
  [ "${#dcands[@]}" -gt 0 ] || return 0
  for d in "${dcands[@]}"; do cp_dest_ok "$d" || return 0; done
  for ((j = 0; j < ${#ops[@]}; j++)); do
    candidates "${ops[$j]}" "$SEGCWD"
    for s in ${CANDS[@]+"${CANDS[@]}"}; do
      # A source directory holding .claude/ could merge it into the destination.
      [ -e "$s/.claude" ] && return 0
      for d in "${dcands[@]}"; do cp_dest_ok "${d%/}/${s##*/}" || return 0; done
    done
    CP_SRC="$CP_SRC${opi[$j]} "
  done
  CP_OK=1
  return 0
}

# env -C/--chdir moves the cwd for one command only (Sol r4485195331): run the
# segment with that cwd, then restore it unless the segment was a cd/pushd.
eval_segment() {
  local saved="$SEGCWD" saved_unk="$CWD_UNK"
  SEG_CD=0
  eval_segment_body
  local rc=$?
  [ "$SEG_CD" = 1 ] || { SEGCWD="$saved"; CWD_UNK="$saved_unk"; }
  return $rc
}

eval_segment_body() { # uses SEG array
  local n=${#SEG[@]} i=0 w="" wi=-1 wb tok reader=0 script=0 script_i=-1 hit=0 kanc=0 c k gsub_i=-1 dscript=0
  SEG_HIT=0
  [ "$n" -gt 0 ] || return 0
  # First command word, skipping VAR=value prefixes and wrappers.
  while [ "$i" -lt "$n" ]; do
    tok="${SEG[$i]}"
    if [[ "$tok" == *=* && "$tok" != -* && "$tok" != /* ]]; then i=$((i + 1)); continue; fi
    # Wrappers by basename (/usr/bin/env, /usr/bin/nohup); env's own options are consumed
    # so `env -i -u X ./script` reaches ./script. -S keeps its string as the command word.
    if [[ "$WRAPPERS" == *" ${tok##*/} "* ]]; then
      i=$((i + 1))
      if [ "${tok##*/}" = env ]; then
        while [ "$i" -lt "$n" ]; do
          case "${SEG[$i]}" in
            -C|--chdir)
              [ $((i + 1)) -lt "$n" ] && { amb_check "${SEG[$((i + 1))]}" "$SEGCWD" all; canon "${SEG[$((i + 1))]}" "$SEGCWD"; SEGCWD="$CANON"; }
              i=$((i + 2)) ;;
            --chdir=*) amb_check "${SEG[$i]#--chdir=}" "$SEGCWD" all; canon "${SEG[$i]#--chdir=}" "$SEGCWD"; SEGCWD="$CANON"; i=$((i + 1)) ;;
            -C?*) amb_check "${SEG[$i]#-C}" "$SEGCWD" all; canon "${SEG[$i]#-C}" "$SEGCWD"; SEGCWD="$CANON"; i=$((i + 1)) ;;
            -u|-P|--unset) i=$((i + 2)) ;;
            -S|--split-string) i=$((i + 1)); break ;;
            --) i=$((i + 1)); break ;;
            -*) i=$((i + 1)) ;;
            *) break ;;
          esac
        done
      fi
      continue
    fi
    if [ "$tok" = "-u" ] || [ "$tok" = "-n" ]; then i=$((i + 2)); continue; fi
    break
  done
  [ "$i" -lt "$n" ] || return 0
  w="${SEG[$i]}"; wi=$i
  wb="${w##*/}"

  # cd / pushd: move the effective cwd for later segments.
  if [ "$wb" = cd ] || [ "$wb" = pushd ]; then
    SEG_CD=1
    # A cd target the analysis cannot resolve to one dir near a protected path leaves the real cwd unknown
    # (CWD_UNK): later relative writes and non-readers are refused, reads still run (#148 F1/F5).
    local cdt="" cdunk=$CWD_UNK
    for ((c = wi + 1; c < n; c++)); do case "${SEG[$c]}" in -P|-L|-e|-@|--) ;; *) cdt="${SEG[$c]}"; break ;; esac; done
    CWD_UNK=0
    for ((c = wi + 1; c < n; c++)); do amb_check "${SEG[$c]}" "$SEGCWD" cd; done
    # From an unknown cwd only an absolute or ~ target (or none: HOME) is known again.
    if [ "$CWD_UNK" = 0 ] && [ "$cdunk" = 1 ]; then case "$cdt" in /*|"~"|"~/"*|"") ;; *) CWD_UNK=1 ;; esac; fi
    if [ $((wi + 1)) -lt "$n" ] && [ "${SEG[$((wi + 1))]}" != "-" ]; then
      canon "${SEG[$((wi + 1))]}" "$SEGCWD"; SEGCWD="$CANON"
    else
      SEGCWD="$CHOME"
    fi
    return 0
  fi

  # Reader classification.
  if [[ "$READERS" == *" $wb "* ]]; then
    reader=1
    if [ "$wb" = env ] || [ "$wb" = printenv ]; then [ $((wi + 1)) -lt "$n" ] && reader=0; fi
    if [ "$wb" = rg ]; then
      for ((c = wi + 1; c < n; c++)); do case "${SEG[$c]}" in --pre|--pre=*) reader=0 ;; esac; done
    fi
  elif [ "$wb" = find ]; then
    reader=1
    for ((c = wi + 1; c < n; c++)); do
      case "${SEG[$c]}" in -delete|-exec|-execdir|-ok|-okdir|-fprint*|-fls) reader=0 ;; esac
    done
  elif [ "$wb" = git ]; then
    c=$((wi + 1))
    local gitc=0
    while [ "$c" -lt "$n" ]; do
      case "${SEG[$c]}" in
        -c|--exec-path*|--config-env*) gitc=1; c=$((c + 2)) ;;
        -C|--git-dir|--work-tree) c=$((c + 2)) ;;
        -*) c=$((c + 1)) ;;
        *) break ;;
      esac
    done
    gsub_i=$c
    if [ "$gitc" = 0 ] && [ "$c" -lt "$n" ] && [[ "$GIT_READ" == *" ${SEG[$c]} "* ]]; then
      reader=1
      local sub="${SEG[$c]}" a
      for ((k = c + 1; k < n; k++)); do
        a="${SEG[$k]}"
        case "$a" in --output*|-O*|--open-files-in-pager*|--ext-diff|--textconv) reader=0 ;; esac
        case "$sub" in
          branch) case "$a" in -a|-r|-v|-vv|-l|--list|--all|--show-current|--no-color|--color|--contains|--merged|--no-merged|--sort=*) ;; *) reader=0 ;; esac ;;
          tag) case "$a" in -l|--list|-n|-n*|--contains|--sort=*) ;; *) reader=0 ;; esac ;;
          remote) case "$a" in -v|--verbose|show|get-url) ;; *) [ "$k" -gt $((c + 1)) ] || reader=0 ;; esac ;;
          reflog) case "$a" in show|-n*|--date=*) ;; *) [ "$k" -gt $((c + 1)) ] || reader=0 ;; esac ;;
          config) [ "$k" -eq $((c + 1)) ] && case "$a" in --get|--get-all|--get-regexp|--list|-l|--show-origin) ;; *) reader=0 ;; esac ;;
        esac
      done
    fi
  fi

  # Pipe into a shell or interpreter with no script argument executes stdin.
  if [ "$PIPE_IN" = 1 ] && is_interp "$wb"; then
    local has_arg=0 dash_s=0
    for ((c = wi + 1; c < n; c++)); do
      case "${SEG[$c]}" in
        -s) dash_s=1 ;;
        -) ;;
        -m|-c|-e|-r|--eval) has_arg=1 ;;
        -*) ;;
        *) has_arg=1; break ;;
      esac
    done
    if [ "$has_arg" = 0 ] || [ "$dash_s" = 1 ]; then block "pipe into ${wb} (executes piped input)"; fi
  fi

  # Vajra's own scripts are allowed (except the operator-only ones).
  local cand_i=-1 sname
  if is_interp "$wb"; then
    for ((c = wi + 1; c < n; c++)); do
      case "${SEG[$c]}" in -*) ;; *) cand_i=$c; break ;; esac
    done
  else
    cand_i=$wi
  fi
  if [ "$cand_i" -ge 0 ]; then
    # Knowledge-ledger decisions are the user's: approve/reject/forget need a
    # TTY, and the user trust tier comes only from the router's /vajra remember
    # (it runs inside the UserPromptSubmit hook, never as a tool call). Matched
    # by file name wherever the script lives (installed copy or a dev clone).
    # Operator-only scripts too: a worktree copy of generate-manifest.sh still
    # signs into the real ~/.claude/vajra (incident 2026-11-05), so the name decides.
    # Run, not named: the script is the command word or follows a shell or
    # interpreter, also inside a bash -c string (cat or grep of it is fine).
    local runs=0
    for ((c = wi; c < n; c++)); do
      tok="${SEG[$c]//[\"\']/}"
      set -f; local words=( $tok ); set +f
      for k in ${words[@]+"${words[@]}"}; do
        if not_script "${k##*/}" && { [ "$runs" = 1 ] || [ "$c" = "$wi" ]; }; then
          block "${k##*/} names ${NS_HIT}, which changes Vajra's wiring or integrity state, from any copy of the skill; the operator runs it in their own terminal"
        fi
        [[ "$SHELLS$INTERPS" == *" ${k##*/} "* ]] && runs=1
        if [ "$c" = "$wi" ]; then case "$k" in .|source) runs=1 ;; esac; fi
      done
    done
    if [ "${SEG[$cand_i]##*/}" = vajra_kb.py ]; then
      local kbsub="" kprev=""
      for ((c = cand_i + 1; c < n; c++)); do
        tok="${SEG[$c]}"
        if [ -z "$kbsub" ]; then
          case "$tok" in -*) continue ;; *) kbsub="$tok" ;; esac
          case "$kbsub" in
            desktop-enroll|desktop-revoke|desktop-consume|desktop-repair) block "vajra_kb.py ${kbsub} is for the user's own terminal and the Vajra Desktop app, not the agent" ;;
            approve|reject|forget) block "vajra_kb.py ${kbsub} is a user decision and needs a TTY; ask the user to run it in their own terminal: python3 ~/.claude/skills/vajra/scripts/vajra_kb.py ${kbsub} <id>" ;;
          esac
          kprev="$tok"
          continue
        fi
        if [ "$kbsub" = add ]; then
          shopt -s nocasematch
          # argparse accepts any unambiguous prefix of --source (--s, --so,
          # --sou, ...), with = or a space. vajra_kb.py also refuses --source
          # user without the router's user token; this is an extra layer.
          local kopt="${tok%%=*}" kpopt="${kprev%%=*}"
          if [[ ( "$tok" == --s*=* && "--source" == "$kopt"* && "${tok#*=}" == user ) || \
                ( "$kprev" == --s* && "$kprev" != *=* && "--source" == "$kpopt"* && "$tok" == user ) ]]; then
            shopt -u nocasematch
            block "vajra_kb.py add --source user is only for the user's own /vajra remember prompt; record agent knowledge with --source proposed (or verified with evidence)"
          fi
          shopt -u nocasematch
        fi
        kprev="$tok"
      done
    fi
    # Pins decide what `add` installs, so moving them is the user's review, and
    # nothing installs while the integrity flag stands. Matched by file name, as above.
    if [ "${SEG[$cand_i]##*/}" = vajra_skills.py ]; then
      for ((c = cand_i + 1; c < n; c++)); do
        case "${SEG[$c]}" in
          -*) ;;
          upstream) block "vajra_skills.py upstream moves skill pins; the skill-pins Action runs it and a person reviews the PR" ;;
          add) if [ "$TAMPER" = 1 ]; then block "integrity violation is unresolved; no skill installs (see scripts/vajra-doctor.sh)"; fi; break ;;
          *) break ;;
        esac
      done
    fi
    canon "${SEG[$cand_i]}" "$SEGCWD"
    if inside "$CANON" "$SKILL/scripts" && [ "${#CANON}" -gt $(( ${#SKILL} + 9 )) ]; then
      sname="${CANON:${#SKILL}+9}"
      if [[ "$sname" != */* && ( "$sname" == *.sh || "$sname" == *.py || "$sname" == *.js ) ]]; then
        if [[ "$NOT_SCRIPTS" == *" $sname "* ]]; then
          block "${sname} changes Vajra's wiring or integrity state; the operator runs it, not the agent"
        fi
        if [ "$sname" = atman.sh ]; then
          for ((c = cand_i + 1; c < n; c++)); do
            case "${SEG[$c]}" in
              -*) ;;
              approve|reject) block "atman approve/reject is a human decision and needs a TTY; ask the user to run it in their own terminal: bash ~/.claude/skills/vajra/scripts/atman.sh ${SEG[$c]} <id>" ;;
              *) break ;;
            esac
          done
        fi
        script=1; script_i=$cand_i
        [[ "$DATA_SCRIPTS" == *" $sname "* ]] && dscript=1
      fi
    fi
  fi

  # Recursive deletes of /, system roots, HOME or the working directory.
  if [ "$wb" = rm ] || [ "$wb" = find ]; then
    local recursive=0 deleting=1 first_target=1
    if [ "$wb" = rm ]; then
      for ((c = wi + 1; c < n; c++)); do
        case "${SEG[$c]}" in --recursive) recursive=1 ;; --*) ;; -*[rR]*) recursive=1 ;; esac
      done
    else
      recursive=1; deleting=0
      for ((c = wi + 1; c < n; c++)); do
        case "${SEG[$c]}" in
          -delete) deleting=1 ;;
          -exec|-execdir|-ok) case "${SEG[$((c + 1))]:-}" in rm|*/rm|sh|bash|zsh|shred|unlink) deleting=1 ;; esac ;;
        esac
      done
    fi
    if [ "$recursive" = 1 ] && [ "$deleting" = 1 ]; then
      for ((c = wi + 1; c < n; c++)); do
        tok="${SEG[$c]}"
        case "$tok" in -*|">"|"<"|__DUP__) continue ;; esac
        if [ "$wb" = find ] && [ "$first_target" = 0 ]; then break; fi
        first_target=0
        if [ "$tok" = "*" ] && { inside "$CHOME" "$SEGCWD" || [[ "$SYSROOTS" == *" $SEGCWD "* ]] || [ "$SEGCWD" = / ]; }; then
          block "recursive delete of * in $SEGCWD"
        fi
        canon "$tok" "$SEGCWD"
        if [ "$CANON" = "/" ] || [[ "$SYSROOTS" == *" $CANON "* ]] || inside "$CHOME" "$CANON"; then
          block "recursive delete of $CANON"
        fi
        if [ "$wb" = rm ] && inside "$SEGCWD" "$CANON"; then
          block "recursive delete of the working directory or its parent ($CANON)"
        fi
      done
    fi
  fi

  # curl/wget uploading a local file from outside the working directory.
  if [ "$wb" = curl ] || [ "$wb" = wget ]; then
    local nextfile=0 f
    for ((c = wi + 1; c < n; c++)); do
      tok="${SEG[$c]}"; f=""
      if [ "$nextfile" = 1 ]; then
        f="$tok"; nextfile=0
      else
        case "$tok" in
          -T|--upload-file|--post-file|--body-file) nextfile=1; continue ;;
          --post-file=*|--body-file=*|--upload-file=*) f="${tok#*=}" ;;
          *://*) ;;
          *@*) f="${tok#*@}" ;;
        esac
      fi
      if [ -z "$f" ] || [ "$f" = "-" ]; then continue; fi
      canon "$f" "$SEGCWD"
      inside "$CANON" "$SEGCWD" || block "network upload of a file outside the working directory ($CANON)"
    done
  fi

  # Recursive readers, finders that run commands, archivers and copiers
  # would reach the HMAC key through its directory or an ancestor of it.
  local rmode="" skip=0 nolast=0 op_from=$((wi + 1)) last_i=-1
  case "$wb" in
    grep|egrep|fgrep|zgrep)
      for ((c = wi + 1; c < n; c++)); do
        case "${SEG[$c]}" in
          --recursive|--dereference-recursive|--directories=recurse|-drecurse) rmode=search ;;
          -d) [ "${SEG[$((c + 1))]:-}" = recurse ] && rmode=search ;;
          --*) ;;
          -*[rR]*) rmode=search ;;
        esac
      done ;;
    rg|ag|ack|ack-grep|ugrep|ug|pt|sift) rmode=search ;;
    git) if [ "$gsub_i" -ge 0 ] && [ "${SEG[$gsub_i]:-}" = grep ]; then rmode=search; op_from=$((gsub_i + 1)); fi ;;
    find|gfind)
      for ((c = wi + 1; c < n; c++)); do
        case "${SEG[$c]}" in -exec|-execdir|-ok|-okdir) rmode=find ;; esac
      done ;;
    fd|fdfind)
      for ((c = wi + 1; c < n; c++)); do
        case "${SEG[$c]}" in -x|-X|--exec|--exec-batch|--exec=*|--exec-batch=*) rmode=search ;; esac
      done ;;
    tar|gtar|bsdtar)
      rmode=copy
      for ((c = wi + 1; c < n; c++)); do
        tok="${SEG[$c]}"
        case "$tok" in
          --extract|--get|--list) rmode="" ;;
          --*) ;;
          -*x*|-*t*) rmode="" ;;
          *) if [ "$c" = $((wi + 1)) ]; then case "$tok" in *x*|*t*) rmode="" ;; esac; fi ;;
        esac
      done ;;
    zip|7z|7za|7zz|rar|cpio|pax|jar|xar) rmode=copy ;;
    diff|gdiff|colordiff)
      for ((c = wi + 1; c < n; c++)); do
        case "${SEG[$c]}" in --recursive) rmode=copy ;; --*) ;; -*r*) rmode=copy ;; esac
      done ;;
    cp|gcp|ditto|rsync|scp|rclone|install|ginstall)
      rmode=copy; nolast=1
      for ((c = wi + 1; c < n; c++)); do
        case "${SEG[$c]}" in -t|--target-directory|--target-directory=*|-t?*) nolast=0 ;; esac
      done ;;
  esac
  if [ -n "$rmode" ]; then
    if [ "$rmode" = search ]; then
      # Default target is the cwd; the first operand is the pattern unless
      # -e/-f/--regexp/--file supplied it.
      reach_check "$SEGCWD" "${wb} in $SEGCWD"
      skip=1
      for ((c = op_from; c < n; c++)); do
        case "${SEG[$c]}" in --regexp*|--file*) skip=0 ;; --*) ;; -*[ef]*) skip=0 ;; esac
      done
    fi
    local ops=()
    if [ "$rmode" = find ]; then
      # find [-H|-L|-P|-E|-X|-d|-s|-x] [-f path] path... expression; GNU find
      # defaults to the cwd when no path is given.
      c=$op_from
      while [ "$c" -lt "$n" ]; do
        case "${SEG[$c]}" in -H|-L|-P|-E|-X|-d|-s|-x|-O*) c=$((c + 1)) ;; -f) c=$((c + 1)); break ;; *) break ;; esac
      done
      case "${SEG[$c]:-}" in ""|-*|"!"|";") ops=( "$SEGCWD" ) ;; esac
    fi
    for ((c = op_from; c < n; c++)); do
      tok="${SEG[$c]}"
      case "$tok" in
        ">"|"<"|__DUP__) c=$((c + 1)); continue ;;
        -*|"!") continue ;;
      esac
      if [ "$skip" = 1 ]; then skip=0; continue; fi
      ops+=( "$tok" )
    done
    if [ "$nolast" = 1 ] && [ "${#ops[@]}" -gt 1 ]; then
      ops=( "${ops[@]:0:${#ops[@]}-1}" )
    fi
    for tok in ${ops[@]+"${ops[@]}"}; do
      candidates "$tok" "$SEGCWD"
      for cc in ${CANDS[@]+"${CANDS[@]}"}; do
        reach_check "$cc" "${wb} over $cc"
      done
    done
  fi

  # Names listed from the key's directory must not be fed to xargs/parallel.
  if [ "$PIPE_KTAINT" = 1 ]; then
    case "$wb" in xargs|parallel|gxargs) block "file names from a directory containing the Vajra HMAC key piped into ${wb}" ;; esac
  fi

  if [ "$TAMPER" = 1 ] && [ "$reader" = 0 ] && [ "$script" = 0 ]; then
    block "integrity violation is unresolved; only read-only commands run (see scripts/vajra-doctor.sh)"
  fi

  # Protected paths: readers may look; nothing else may touch them.
  local redirect=0 part cc dataarg=0 fastcwd=0
  # Fast path for plain words: when the cwd is neither inside ~/.claude nor an
  # ancestor of it, a word with no / ~ glob characters (and not . or ..) that
  # is not a symlink resolves to cwd/word, which cannot be protected.
  # A glob with no / or ~ that does not start with . matches only entries
  # of the cwd (never . or ..), so it is expanded only when the cwd holds a
  # symlink. Both facts are cached per cwd.
  if [ "$SEGCWD" != "$FAST_CWD" ]; then
    FAST_CWD="$SEGCWD"; FAST_OK=1; FAST_LINKS=0
    local e
    for e in "$CL" "$DCTL" "$DCTL2" "$DDATA" "$DDATA2"; do
      if inside "$SEGCWD" "$e" || inside "$e" "$SEGCWD"; then FAST_OK=0; fi
    done
    shopt -s nullglob dotglob
    for e in "$SEGCWD"/*; do [ -L "$e" ] && { FAST_LINKS=1; break; }; done
    shopt -u nullglob dotglob
  fi
  fastcwd=$FAST_OK
  CP_OK=0; CP_SRC=" "
  local copier=0 asg=0
  case "$wb" in cp|gcp|ditto|rsync|install|ginstall) copier=1; copy_layout ;; esac
  case "$wb" in export|declare|local|readonly|typeset) asg=1 ;; esac
  local parts=()
  for ((c = 0; c < n; c++)); do  # from 0: VAR=value prefixes count too
    tok="${SEG[$c]}"
    dataarg=0
    if [ "$dscript" = 1 ] && [ "$c" -gt "$script_i" ]; then
      case "${SEG[$((c - 1))]}" in -o|--out|--output|--to|--dest) ;; *) dataarg=1 ;; esac
      case "$tok" in -o?*|--out=*|--output=*|--to=*|--dest=*) dataarg=0 ;; esac
    fi
    case "$tok" in
      ">") redirect=1; continue ;;
      "<") continue ;;
      __DUP__) c=$((c + 1)); continue ;;
    esac
    if [ "$c" = "$script_i" ]; then redirect=0; continue; fi
    if [ "$c" = "$wi" ] && [ "$cand_i" != "$wi" ]; then continue; fi
    case "$tok" in
      *=*) parts=( "${tok#*=}" ) ;;
      -*~/*) parts=( "~/${tok#*~/}" ) ;;   # -o~/.claude (attached flag value)
      -*/*) parts=( "/${tok#*/}" ) ;;      # -o/abs/path
      -*) parts=() ;;
      *) parts=( "$tok" ) ;;
    esac
    for part in ${parts[@]+"${parts[@]}"}; do
      # A redirect target, or an argument of a command that may write, must not be ambiguous near a protected root.
      # A source operand of an understood copy (CP_OK) is a read: the copy-out rule below judges it.
      # An export/declare/local/readonly/typeset value is not a path the command writes (#148 F5).
      if [ "$redirect" = 1 ] || { [ "$reader" = 0 ] && [ "$dataarg" = 0 ] && [ "$c" -gt "$wi" ] \
           && ! { [ "$CP_OK" = 1 ] && [[ "$CP_SRC" == *" $c "* ]]; } && ! { [ "$asg" = 1 ] && [[ "$tok" == *=* ]]; }; }; then
        amb_check "$part" "$SEGCWD" all
        if [ "$CWD_UNK" = 1 ]; then
          case "$part" in /*|"~"|"~/"*) ;; *) block "the cwd is unknown after a cd to a glob or variable near a protected Vajra path; cd to the literal path first" ;; esac
        fi
      elif [ "$c" -gt "$wi" ]; then
        # #148 F3: a reader's ambiguous word near a protected path taints the pipe, as a literal protected path does.
        amb_check "$part" "$SEGCWD" read && hit=1
      fi
      if [ "$fastcwd" = 1 ]; then
        case "$part" in
          ""|.|..|*/*|*"~"*|.*[\*\?\[]*) ;;
          *[\*\?\[]*) [ "$FAST_LINKS" = 1 ] || continue ;;
          *) [ -L "$SEGCWD/$part" ] || continue ;;
        esac
      fi
      candidates "$part" "$SEGCWD"
      for cc in ${CANDS[@]+"${CANDS[@]}"}; do
        noread "$cc" && block "workers may not read Vajra Desktop control files or Claude Code transcripts: $cc"
        key_anc "$cc" && kanc=1
        classify "$cc"
        case "$PKIND" in
          key) block "the Vajra HMAC key is never a command target" ;;
          core|ancestor)
            [ "$redirect" = 1 ] && block "redirect into protected path $cc"
            [ "$dataarg" = 1 ] && continue
            hit=1
            # A copier may read a protected source (not an ancestor of one)
            # into a destination outside every protected area. A source that holds
            # the control dir (the <data> root, an ancestor before ADR-16) never is.
            if [ "$CP_OK" = 1 ] && [ "$PKIND" = core ] && [ "$part" = "$tok" ] && [[ "$CP_SRC" == *" $c "* ]] \
               && ! { inside "$DCTL" "$cc" && ! same_dir "$DCTL" "$cc"; } \
               && ! { inside "$DCTL2" "$cc" && ! same_dir "$DCTL2" "$cc"; }; then continue; fi
            [ "$copier" = 1 ] && [ "$reader" = 0 ] && block "only read-only commands may touch protected path $cc (a copy may read protected files only into a destination outside every protected area; never into one, never via links)"
            [ "$reader" = 1 ] || block "only read-only commands may touch protected path $cc"
            ;;
        esac
      done
    done
    redirect=0
  done

  if [ "$PIPE_TAINT" = 1 ] && [ "$reader" = 0 ]; then
    block "protected path piped into a non-reader (${wb})"
  fi
  # #148 F1/F5: after an ambiguous cd near a protected path only readers may run.
  [ "$reader" = 0 ] && [ "$CWD_UNK" = 1 ] && block "the cwd is unknown after a cd to a glob or variable near a protected Vajra path; only read-only commands may follow it"
  if [ "$reader" = 0 ] && [ "$script" = 0 ]; then
    classify "$SEGCWD"
    [ "$PKIND" = core ] && block "only read-only commands may run inside protected directory $SEGCWD"
  fi
  SEG_HIT=$hit
  # A listing in ~/.claude or ~/.claude/vajra itself, or of a key ancestor, names the key.
  SEG_KANC=$kanc
  if [ "$SEGCWD" = "$VS_C" ] || [ "$SEGCWD" = "$CL" ]; then SEG_KANC=1; fi
  return 0
}

run_pass() { # judge every segment of TOKS from the starting cwd
  SEGCWD="$CWD" CWD_UNK=0
  FAST_CWD="" FAST_OK=0 FAST_LINKS=0
  PIPE_IN=0 PIPE_TAINT=0 PIPE_KTAINT=0 SEG_HIT=0 SEG_KANC=0
  SEG=()
  for tok in ${TOKS[@]+"${TOKS[@]}"} ";"; do
    case "$tok" in
      ";"|PIPE)
        eval_segment
        if [ "$tok" = PIPE ]; then
          PIPE_IN=1
          [ "$SEG_HIT" = 1 ] && PIPE_TAINT=1
          [ "$SEG_KANC" = 1 ] && PIPE_KTAINT=1
        else
          PIPE_IN=0; PIPE_TAINT=0; PIPE_KTAINT=0
        fi
        SEG=()
        ;;
      *) SEG+=( "$tok" ) ;;
    esac
  done
}
APPSUP_PAT='[Aa][Pp][Pp][Ll][Ii][Cc][Aa][Tt][Ii][Oo][Nn] [Ss][Uu][Pp][Pp][Oo][Rr][Tt]'
T2="${T//$APPSUP_PAT/Application__SP__Support}"
# AMB_DYN (new in #148) reads assignment values from the joined view when the command names "Application
# Support": the split view cuts a <work> path at the space and would see ~/Library as its literal dir.
if [ "$T2" != "$T" ]; then
  set -f; DTOKS=( $T2 ); set +f
  amb_dyn_scan ${DTOKS[@]+"${DTOKS[@]}"}
else
  amb_dyn_scan ${TOKS[@]+"${TOKS[@]}"}
fi
run_pass

# Second pass, additive only: the pass above has already run unchanged. If the
# command names "Application Support" (any case), rerun the same segment loop
# with that phrase joined into one token, so a path with that space (the Vajra
# Desktop control dir) is judged as one path. It can only add blocks.
if [ "$T2" != "$T" ]; then
  set -f
  TOKS=( $T2 )
  set +f
  run_pass
fi

allow
