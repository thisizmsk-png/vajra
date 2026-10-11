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
command -v jq >/dev/null 2>&1 || block "jq is not installed, so the payload cannot be checked (fail-closed). Install jq."

PAYLOAD="$(cat)"
CWD_IN="" CMD="" P1="" P2="" P3=""
PARSED="$(jq -r '
  def s: if type == "string" then . elif . == null then "" else tostring end;
  (.tool_input | if type == "object" then . else {} end) as $i
  | @sh "TOOL=\(.tool_name | s) CWD_IN=\(.cwd | s) CMD=\($i.command | s) P1=\($i.file_path | s) P2=\($i.path | s) P3=\($i.notebook_path | s)"
' <<<"$PAYLOAD" 2>/dev/null)" || block "unparseable hook payload (fail-closed)"
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
)

PKIND=""
classify() { shopt -s nocasematch; classify_ "$1"; shopt -u nocasematch; return 0; }
classify_() { # canonical path -> PKIND = key|core|ancestor|""
  local p="$1" r rest
  PKIND=""
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
  local rc=1
  shopt -s nocasematch
  if [[ "$1" == "$2" || "$1" == "$2"/* ]]; then rc=0; fi
  shopt -u nocasematch
  return $rc
}

# 0 if the canonical path is the HMAC key, its directory, or any ancestor of
# it (~/.claude, HOME, /Users, /): a recursive read or copy there reaches the key.
key_anc() { [ "$1" = / ] || inside "$KEY" "$1"; }

TAMPER=0
[ -f "$VS/.integrity-violation" ] && TAMPER=1

# ---------------------------------------------------------------------------
# File tools
# ---------------------------------------------------------------------------
case "$TOOL" in
  Write|Edit|MultiEdit|NotebookEdit)
    for f in "$P1" "$P2" "$P3"; do
      [ -n "$f" ] || continue
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
      # Grep reads file contents recursively: the key's directory or any
      # ancestor of it (~/.claude, HOME, /) would include the key.
      if [ "$TOOL" = Grep ] && key_anc "$CANON"; then
        block "Grep over $CANON would search the Vajra HMAC key; give a path below it (for example your project directory)"
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

# Normalized copy: quotes and backslashes removed (they change how the shell
# splits words, not which programs run), whitespace collapsed, newlines become
# command separators, $HOME expanded.
# A standalone empty argument ("" or '') becomes the token __E__, so an empty
# grep pattern still counts as the pattern operand.
N=" ${CMD//$'\t'/ } "
RE_EMPTY=" ((\"\"|'')+) "
while [[ "$N" =~ $RE_EMPTY ]]; do N="${N/"${BASH_REMATCH[0]}"/ __E__ }"; done
N="${N# }"; N="${N% }"
N="${N//\"/}"; N="${N//\'/}"; N="${N//\\/}"
N="${N//$'\n'/ ; }"; N="${N//$'\r'/ ; }"; N="${N//$'\t'/ }"
N="${N//'${HOME}'/$HOME}"; N="${N//'$HOME'/$HOME}"
while [[ "$N" == *"  "* ]]; do N="${N//  / }"; done

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
shopt -s nocasematch
[[ "$N" == *".hmac-key"* ]] && block "the Vajra HMAC key is never a command target"
[[ "$N" == *VAJRA_ATMAN_ASSUME_HUMAN* ]] && block "VAJRA_ATMAN_ASSUME_HUMAN is for a human at a terminal, not the agent"
shopt -u nocasematch

RE_RF="(^| )-[[:alnum:]]*([rR][[:alnum:]]*f|f[[:alnum:]]*[rR])[[:alnum:]]*( [^;|&]*)? (/|/\*|~|~/|~/\*|${HOME_RE}|${HOME_RE}/|${HOME_RE}/\*)( |;|\||&|$)"
# N2 also splits on the punctuation interpreters use around argv lists,
# so python -c "subprocess.run(['rm','-rf','/'])" is seen as rm -rf /.
N2=" $N "; N2="${N2//,/ }"; N2="${N2//[/ }"; N2="${N2//]/ }"; N2="${N2//(/ }"; N2="${N2//)/ }"
while [[ "$N2" == *"  "* ]]; do N2="${N2//  / }"; done
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
T=" $N "
T="${T//'$('/ ; }"; T="${T//\`/ ; }"
T="${T//(/ ; }"; T="${T//)/ ; }"; T="${T//,/ }"
T="${T//>&/ __DUP__ }"; T="${T//&>/ > }"
T="${T//&/ ; }"; T="${T//;/ ; }"; T="${T//|/ PIPE }"
T="${T//>/ > }"; T="${T//</ < }"
set -f
TOKS=( $T )
set +f

# Readers: commands with no file-writing or command-running mode. (sort -o,
# uniq OUT, tree -o, xxd -r IN OUT, yq -i write files, so they are not here.)
READERS=" cat less more head tail grep egrep fgrep rg ag ack wc diff cmp file stat ls du sha1sum sha256sum sha512sum shasum md5 md5sum cksum cut nl od hexdump strings column bat realpath readlink dirname basename echo printf test [ true false jq tr pwd which type whoami id date env printenv "
SHELLS=" sh bash zsh dash ksh fish "
INTERPS=" python python2 python3 perl ruby node php lua Rscript deno bun "
WRAPPERS=" sudo command nohup time nice ionice "
NOT_SCRIPTS=" install.sh install-skills.sh generate-manifest.sh "
GIT_READ=" status log diff show rev-parse ls-files ls-tree cat-file blame grep describe shortlog reflog remote branch tag config "
SYSROOTS=" /etc /usr /var /bin /sbin /lib /boot /dev /sys /opt /root /home /System /Library /Applications /private /private/etc /private/var /Users /Volumes /usr/local /usr/bin /usr/lib "

is_interp() { [[ "$SHELLS" == *" $1 "* || "$INTERPS" == *" $1 "* || "$1" =~ ^python[0-9.]+$ ]]; }

SEGCWD="$CWD"
PIPE_IN=0
PIPE_TAINT=0
PIPE_KTAINT=0
SEG_HIT=0
SEG_KANC=0

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

eval_segment() { # uses SEG array
  local n=${#SEG[@]} i=0 w="" wi=-1 wb tok reader=0 script=0 script_i=-1 hit=0 kanc=0 c k gsub_i=-1
  SEG_HIT=0
  [ "$n" -gt 0 ] || return 0
  # First command word, skipping VAR=value prefixes and wrappers.
  while [ "$i" -lt "$n" ]; do
    tok="${SEG[$i]}"
    if [[ "$tok" == *=* && "$tok" != -* && "$tok" != /* ]]; then i=$((i + 1)); continue; fi
    if [[ "$WRAPPERS" == *" $tok "* ]]; then i=$((i + 1)); continue; fi
    if [ "$tok" = "-u" ] || [ "$tok" = "-n" ]; then i=$((i + 2)); continue; fi
    break
  done
  [ "$i" -lt "$n" ] || return 0
  w="${SEG[$i]}"; wi=$i
  wb="${w##*/}"

  # cd / pushd: move the effective cwd for later segments.
  if [ "$wb" = cd ] || [ "$wb" = pushd ]; then
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
    cp|gcp|ditto|rsync|scp|rclone)
      rmode=copy; nolast=1
      for ((c = wi + 1; c < n; c++)); do
        case "${SEG[$c]}" in -t|--target-directory|--target-directory=*|-t?*) nolast=0 ;; esac
      done ;;
  esac
  if [ -n "$rmode" ]; then
    if [ "$rmode" = search ]; then
      # Default target is the cwd; the first operand is the pattern unless
      # -e/-f/--regexp/--file supplied it.
      key_anc "$SEGCWD" && block "${wb} would recursively read $SEGCWD, which contains the Vajra HMAC key; cd into a directory below it (for example your project) first"
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
      ops=( ${ops[@]+"${ops[@]}"} "$tok" )
    done
    if [ "$nolast" = 1 ] && [ "${#ops[@]}" -gt 1 ]; then
      ops=( "${ops[@]:0:${#ops[@]}-1}" )
    fi
    for tok in ${ops[@]+"${ops[@]}"}; do
      candidates "$tok" "$SEGCWD"
      for cc in ${CANDS[@]+"${CANDS[@]}"}; do
        key_anc "$cc" && block "${wb} over $cc would reach the Vajra HMAC key; name a path below it instead"
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
  local redirect=0 part cc
  local parts=()
  for ((c = wi; c < n; c++)); do
    tok="${SEG[$c]}"
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
      candidates "$part" "$SEGCWD"
      for cc in ${CANDS[@]+"${CANDS[@]}"}; do
        key_anc "$cc" && kanc=1
        classify "$cc"
        case "$PKIND" in
          key) block "the Vajra HMAC key is never a command target" ;;
          core|ancestor)
            [ "$redirect" = 1 ] && block "redirect into protected path $cc"
            hit=1
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
    *) SEG=( ${SEG[@]+"${SEG[@]}"} "$tok" ) ;;
  esac
done

allow
