#!/usr/bin/env bash
# vajra-verify-work.sh — capture proof-of-work evidence before claiming "done".
#
#   vajra-verify-work.sh <slug> "<cmd1>" ["<cmd2>" ...]
#
# Runs each verification command, records its exit code and output, and ties the
# evidence to the exact working tree it ran against:
#   - head:          the HEAD commit (null in a repo with no commits)
#   - worktreeTree:  git tree id of the whole working tree, tracked AND untracked
#                    (ignored files excluded), computed with a throwaway index so the
#                    real index is never touched; recomputed after the commands run
#                    to detect checks that changed the tree they were verifying
#   - diff.patch:    diff against HEAD (staged + unstaged) plus every untracked file
# The walkthrough directory itself is excluded from all of the above.
# evidence.json carries an "hmac" (HMAC-SHA256 with the Vajra key over its canonical
# JSON without "hmac"), so the kb and the task graph can tell it was written here.
#
# At least one command is required: a verification that runs nothing proves nothing.
# Exits non-zero if any command failed, so "done" is gated on captured evidence.
#
# Output: <base>/<slug>/  evidence.json, cmd-N.log, diff.patch, diff.stat, untracked.txt, WALKTHROUGH.md
# Base dir: $VAJRA_WALKTHROUGH_DIR (default ./data/walkthroughs).
# Exit codes: 0 all passed, 1 a command failed, 2 usage error.
#
# Vajra Desktop's evidence runner (ADR-16 16.5a, 6i-e) splits the run from the signing:
#   vajra-verify-work.sh --run-only <slug> "<cmd>" ...
#       Runs as above but never opens the key and writes no evidence.json. The unsigned evidence JSON
#       goes only to fd 3, once, as one line; fd 3 is closed for every command. No fd 3 or no jq: exit 2.
#   vajra-verify-work.sh --sign <slug> --expect-sha256 <sha>
#       Signs <base>/<slug>/evidence.json in place, only if the sha256 of its bytes is <sha>, its shape
#       is a verify-work bundle for <slug> with no hmac, and its git.head is this checkout's HEAD.
#       Exit 0 signed, 2 usage, 3 refused (file untouched), 4 no usable key.

set -uo pipefail

usage() {
  echo "usage: vajra-verify-work.sh [--run-only] <slug> \"<cmd>\" [\"<cmd>\" ...]  (at least one command)" >&2
  echo "       vajra-verify-work.sh --sign <slug> --expect-sha256 <sha256>" >&2
  exit 2
}

KEY_FILE="${HOME}/.claude/vajra/.hmac-key"
# NFR-SEC-2: the python that reads the key is /usr/bin/python3 on the system PATH, never the caller's (a planted
# ~/.local/bin/python3 would get the key). The test commands keep the caller's PATH.
PYTHON3="/usr/bin/python3"

MODE=default
case "${1:-}" in
  --run-only) MODE=run_only; shift ;;
  --sign) MODE=sign; shift ;;
esac

if [ "$MODE" = sign ]; then
  [ $# -eq 3 ] && [ "$2" = "--expect-sha256" ] || usage
  SLUG="$1"; WANT="$3"
  # Strict: the slug the core recorded, exactly; no sanitizing here.
  printf '%s' "$SLUG" | grep -Eq '^[A-Za-z0-9_][A-Za-z0-9._-]{0,79}$' || { echo "vajra-verify-work: invalid slug" >&2; exit 2; }
  printf '%s' "$WANT" | grep -Eq '^[0-9a-f]{64}$' || { echo "vajra-verify-work: --expect-sha256 needs 64 lowercase hex" >&2; exit 2; }
  [ -x "$PYTHON3" ] || { echo "sign refused: no usable key" >&2; echo "vajra-verify-work: /usr/bin/python3 is required to sign" >&2; exit 4; }
  EVDIR="${VAJRA_WALKTHROUGH_DIR:-data/walkthroughs}"
  HEAD_NOW="$(git rev-parse --verify -q HEAD 2>/dev/null || true)"
  # Every check runs before the key is opened; the key is read last, only to sign.
  # The bundle folder is in the agent's worktree and the core runs sign outside the sandbox (bhishma #214 H1, M1):
  # every file operation is relative to a folder fd reached without following a symlink the agent could plant, the
  # temp file has a random name and is created exclusively, and evidence.json must still be the file that was read.
  PATH="/usr/bin:/bin:/usr/sbin:/sbin" "$PYTHON3" -I -c '
import hashlib, hmac, json, os, re, secrets, stat, sys
evbase, want, slug, head, keyf = sys.argv[1:6]
evdir = os.path.join(evbase, slug)
# The first stderr line is one fixed "sign refused: <reason>" (the core shows it on the card). No reason holds a path
# or key material; the slug is already checked to be a plain word.
def refuse(why, code=3):
    print("sign refused: " + why, file=sys.stderr); sys.exit(code)
D = os.O_RDONLY | os.O_DIRECTORY
def walk(fd, rel):
    # One part at a time from fd, every part a real folder: no symlink at all, root-owned or not (bhishma #214 M1-b:
    # anyone can hard-link a root-owned relative link into their own folder).
    for part in rel.split("/"):
        if part in ("", "."):
            continue
        try:
            st = os.stat(part, dir_fd=fd, follow_symlinks=False)
            if stat.S_ISLNK(st.st_mode):
                refuse("a symlinked path")
            if not stat.S_ISDIR(st.st_mode):
                refuse("no evidence.json")
            nfd = os.open(part, D | os.O_NOFOLLOW, dir_fd=fd)
        except OSError:
            refuse("no evidence.json")
        os.close(fd); fd = nfd
    return fd
def open_folder(base, slug):
    # An absolute walkthrough dir is a setting of the core, outside the worktree of the agent: its system prefix
    # (/var is /private/var) is resolved once. A relative one sits in the checkout and is walked strictly, like the slug.
    if base.startswith("/"):
        fd = walk(os.open("/", D), os.path.realpath(base))
    else:
        fd = walk(os.open(".", D), base)
    return walk(fd, slug)
dfd = open_folder(evbase, slug)
try:
    efd = os.open("evidence.json", os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=dfd)
except OSError:
    refuse("no evidence.json")
st = os.fstat(efd)
# One link only: a hard link would make the bundle file another file too.
if not stat.S_ISREG(st.st_mode) or st.st_nlink != 1:
    refuse("no evidence.json")
with os.fdopen(efd, "rb") as f:
    raw = f.read()
if hashlib.sha256(raw).hexdigest() != want:
    refuse("sha256 mismatch")
try:
    ev = json.loads(raw.decode("utf-8"))
except (UnicodeDecodeError, ValueError):
    refuse("not JSON")
if isinstance(ev, dict) and "hmac" in ev:
    refuse("already signed")
cmds = ev.get("commands") if isinstance(ev, dict) else None
ok = (isinstance(ev, dict) and ev.get("slug") == slug
      and isinstance(cmds, list) and len(cmds) > 0 and all(isinstance(c, dict) for c in cmds)
      and type(ev.get("commandsTotal")) is int and ev["commandsTotal"] == len(cmds)
      and type(ev.get("commandsFailed")) is int and ev["commandsFailed"] == sum(1 for c in cmds if c.get("exit") != 0)
      and ev.get("allPassed") is (ev["commandsFailed"] == 0)
      and isinstance(ev.get("git"), dict))
if not ok:
    refuse("not a bundle for " + slug)
if not head or ev["git"].get("head") != head:
    refuse("head mismatch")
try:
    k = open(keyf, encoding="utf-8").read().strip()
except OSError:
    k = ""
if not re.fullmatch(r"[0-9a-fA-F]{32,}", k) or len(k) % 2:
    refuse("no usable key", 4)
msg = json.dumps(ev, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
ev["hmac"] = hmac.new(bytes.fromhex(k), msg, hashlib.sha256).hexdigest()
out = (json.dumps(ev, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
tmp = ".evidence.json." + secrets.token_hex(8) + ".tmp"
try:
    tfd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644, dir_fd=dfd)
except OSError:
    refuse("cannot write")
try:
    with os.fdopen(tfd, "wb") as f:
        f.write(out); f.flush(); os.fsync(f.fileno())
    now = os.stat("evidence.json", dir_fd=dfd, follow_symlinks=False)
    if not stat.S_ISREG(now.st_mode) or (now.st_dev, now.st_ino) != (st.st_dev, st.st_ino):
        os.unlink(tmp, dir_fd=dfd)
        refuse("evidence changed")
    os.replace(tmp, "evidence.json", src_dir_fd=dfd, dst_dir_fd=dfd)
except OSError:
    try:
        os.unlink(tmp, dir_fd=dfd)
    except OSError:
        pass
    refuse("cannot write")
print("signed " + os.path.join(evdir, "evidence.json"))
' "$EVDIR" "$WANT" "$SLUG" "$HEAD_NOW" "$KEY_FILE"
  exit $?
fi

if [ "$MODE" = run_only ]; then
  { : >&3; } 2>/dev/null || { echo "vajra-verify-work: --run-only needs fd 3 open for the evidence" >&2; exit 2; }
  command -v jq >/dev/null 2>&1 || { echo "vajra-verify-work: --run-only needs jq" >&2; exit 2; }
fi

RAW_SLUG="${1:-}"
[ -z "$RAW_SLUG" ] && usage
shift
[ $# -ge 1 ] || { echo "vajra-verify-work: no verification commands given; at least one is required." >&2; usage; }
for c in "$@"; do
  [ -n "$(printf '%s' "$c" | tr -d '[:space:]')" ] || { echo "vajra-verify-work: empty command" >&2; usage; }
done

# Sanitize the slug to a single safe path segment (no traversal).
SLUG="$(printf '%s' "$RAW_SLUG" | tr -cd '[:alnum:]._-' | sed 's/^[.-]*//' | cut -c1-80)"
[ -z "$SLUG" ] && SLUG="walkthrough"
[ "$SLUG" != "$RAW_SLUG" ] && echo "note: slug sanitized to '${SLUG}' (must be a single path segment)" >&2

BASE="${VAJRA_WALKTHROUGH_DIR:-data/walkthroughs}"
DIR="${BASE}/${SLUG}"
mkdir -p "$DIR" || { echo "cannot create $DIR" >&2; exit 2; }
rm -f "$DIR"/cmd-*.log 2>/dev/null
DIR_ABS="$(cd "$DIR" && pwd -P)"
TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

have_jq=0; command -v jq >/dev/null 2>&1 && have_jq=1

IN_GIT=0; HEAD_SHA=""; TREE=""; TREE_AFTER=""; DIRTY="false"; TOP=""
EXCL=""   # pathspec excluding the walkthrough base dir when it lies inside the repo (literal: no glob in its name matches a neighbour)
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  IN_GIT=1
  TOP="$(cd "$(git rev-parse --show-toplevel)" && pwd -P)"
  BASE_ABS="$(cd "$BASE" && pwd -P)"
  case "$BASE_ABS/" in "$TOP"/*) EXCL=":(exclude,literal)${BASE_ABS#"$TOP"/}" ;; esac
  HEAD_SHA="$(git rev-parse --verify -q HEAD 2>/dev/null || true)"
fi

# git with the walkthrough dir excluded. Usage: g <subcommand> [args...]  (pathspec appended)
g() {
  local sub="$1"; shift
  if [ -n "$EXCL" ]; then git -C "$TOP" "$sub" "$@" -- . "$EXCL"; else git -C "$TOP" "$sub" "$@" -- .; fi
}

tree_of_worktree() {
  local idx; idx="$(mktemp "${TMPDIR:-/tmp}/vajra-vw-index-XXXXXX")"; rm -f "$idx"
  [ -n "$HEAD_SHA" ] && GIT_INDEX_FILE="$idx" git -C "$TOP" read-tree "$HEAD_SHA" 2>/dev/null
  if GIT_INDEX_FILE="$idx" g add -A >/dev/null 2>&1; then GIT_INDEX_FILE="$idx" git -C "$TOP" write-tree 2>/dev/null; fi
  rm -f "$idx"
}

: > "${DIR}/diff.patch"; : > "${DIR}/diff.stat"; : > "${DIR}/untracked.txt"
if [ "$IN_GIT" -eq 1 ]; then
  if [ -n "$HEAD_SHA" ]; then DBASE="$HEAD_SHA"; else DBASE="$(git hash-object -t tree /dev/null)"; fi
  g diff --binary "$DBASE" > "${DIR}/diff.patch" 2>/dev/null || true
  g diff --stat "$DBASE" > "${DIR}/diff.stat" 2>/dev/null || true
  g ls-files --others --exclude-standard > "${DIR}/untracked.txt.tmp" 2>/dev/null || true
  mv "${DIR}/untracked.txt.tmp" "${DIR}/untracked.txt"
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    ( cd "$TOP" && git diff --binary --no-index -- /dev/null "$f" ) >> "${DIR}/diff.patch" 2>/dev/null
    printf ' %s (untracked)\n' "$f" >> "${DIR}/diff.stat"
  done < "${DIR}/untracked.txt"
  TREE="$(tree_of_worktree)"
  [ -s "${DIR}/diff.patch" ] && DIRTY="true"
fi

n=0; failed=0; entries=""
for cmd in "$@"; do
  n=$((n+1))
  log="${DIR}/cmd-${n}.log"
  echo "\$ ${cmd}" > "$log"
  # fd 3 is closed for every command: in --run-only it carries the evidence, which no command may write.
  bash -c "$cmd" >> "$log" 2>&1 3>&-
  ec=$?
  [ "$ec" -ne 0 ] && failed=$((failed+1))
  printf '  [%s] cmd %d (exit %d): %s\n' "$([ "$ec" -eq 0 ] && echo PASS || echo FAIL)" "$n" "$ec" "$cmd"
  if [ "$have_jq" -eq 1 ]; then
    entry="$(jq -nc --arg c "$cmd" --argjson e "$ec" --arg l "cmd-${n}.log" '{cmd:$c, exit:$e, log:$l, passed:($e==0)}')"
    entries="${entries}${entries:+,}${entry}"
  fi
done

STABLE="true"
if [ "$IN_GIT" -eq 1 ]; then
  TREE_AFTER="$(tree_of_worktree)"
  [ "$TREE" != "$TREE_AFTER" ] && STABLE="false"
fi

# --run-only: the evidence goes to fd 3 only, as one line; no evidence.json is left from an earlier run.
JQ_FMT=""
if [ "$MODE" = run_only ]; then rm -f "${DIR}/evidence.json"; JQ_FMT="-c"; fi
ev_json() {
  jq -n $JQ_FMT --arg slug "$SLUG" --arg ts "$TS" --argjson total "$n" --argjson failed "$failed" \
        --argjson cmds "[${entries}]" --arg head "$HEAD_SHA" --arg tree "$TREE" --arg after "$TREE_AFTER" \
        --argjson git "$([ "$IN_GIT" -eq 1 ] && echo true || echo false)" --argjson dirty "$DIRTY" --argjson stable "$STABLE" \
        --argjson untracked "$(wc -l < "${DIR}/untracked.txt" | tr -d ' ')" \
        'def nz: if . == "" then null else . end;
         {slug:$slug, generatedAt:$ts, commandsTotal:$total, commandsFailed:$failed, allPassed:($failed==0),
          git: (if $git then {head:($head|nz), worktreeTree:($tree|nz), worktreeTreeAfter:($after|nz), treeStable:$stable,
                diffBase:"HEAD+untracked", uncommittedChanges:$dirty, untrackedFiles:$untracked} else null end),
          commands:$cmds}'
}
if [ "$MODE" = run_only ]; then
  # fd 3 gets exactly one line, the compact JSON plus a newline, written once from this script's own memory (no file
  # a leftover command could rewrite); then fd 3 is closed, so no later write can reach it. A result that isn't one
  # non-empty line writes nothing: the core reads zero lines and fails closed.
  EVJ="$(ev_json 3>&-)" || EVJ=""
  case "$EVJ" in ""|*$'\n'*) ;; *) printf '%s\n' "$EVJ" >&3 ;; esac
  exec 3>&-
elif [ "$have_jq" -eq 1 ]; then
  ev_json > "${DIR}/evidence.json"
fi

# Sign the evidence (contract: docs/roadmap/v0.6-review-fixes.md, "Signed evidence"):
# hmac = HMAC-SHA256(Vajra key, canonical JSON of the object without "hmac":
# sort_keys, separators (',', ':'), UTF-8). Without a key the file stays unsigned,
# and vajra_kb.py / vajra_tasks.py do not count it as green. --run-only never gets here with a file, and never
# opens the key: --sign signs later.
if [ "$MODE" = default ] && [ -f "${DIR}/evidence.json" ] && [ -r "$KEY_FILE" ] && [ -x "$PYTHON3" ]; then
  PATH="/usr/bin:/bin:/usr/sbin:/sbin" "$PYTHON3" -I -c '
import hashlib, hmac, json, os, re, sys, tempfile
path, keyf = sys.argv[1], sys.argv[2]
k = open(keyf, encoding="utf-8").read().strip()
if not re.fullmatch(r"[0-9a-fA-F]{32,}", k) or len(k) % 2:
    sys.exit(0)
ev = json.load(open(path, encoding="utf-8"))
ev.pop("hmac", None)
msg = json.dumps(ev, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
ev["hmac"] = hmac.new(bytes.fromhex(k), msg, hashlib.sha256).hexdigest()
# A random, exclusive temp file: a planted fixed-name link never redirects the write (bhishma #214).
tfd, tmp = tempfile.mkstemp(prefix=".evidence.json.", suffix=".tmp", dir=os.path.dirname(path) or ".")
try:
    with os.fdopen(tfd, "w", encoding="utf-8") as f:
        json.dump(ev, f, indent=2, ensure_ascii=False)
        f.write("\n")
    os.chmod(tmp, 0o644)
    os.replace(tmp, path)
except OSError:
    os.unlink(tmp)
    raise
' "${DIR}/evidence.json" "$KEY_FILE" || echo "warning: could not sign ${DIR}/evidence.json" >&2
fi

{
  echo "# Walkthrough: ${SLUG}"
  echo "generated: ${TS}"
  echo "result: $([ "$failed" -eq 0 ] && echo 'all checks passed' || echo "${failed}/${n} checks FAILED")"
  if [ "$IN_GIT" -eq 1 ]; then
    echo "head: ${HEAD_SHA:-(no commits)}"
    echo "working tree: ${TREE:-unknown}$([ "$STABLE" = "false" ] && echo ' (CHANGED while the checks ran)')"
  fi
  echo
  echo "## Verification commands"
  i=0
  for cmd in "$@"; do i=$((i+1)); echo "- cmd ${i}: \`${cmd}\` (see cmd-${i}.log)"; done
  echo
  if [ -s "${DIR}/diff.stat" ]; then echo "## Changes vs HEAD (including untracked files)"; echo '```'; cat "${DIR}/diff.stat"; echo '```'; echo; fi
  echo "## How to reproduce"
  echo "Check out the same tree and re-run each command above from the repo root; outputs are in cmd-N.log."
} > "${DIR}/WALKTHROUGH.md"

echo "----------------------------------------"
[ "$STABLE" = "false" ] && echo "warning: the working tree changed while the checks ran; the evidence covers tree ${TREE}, not the final tree." >&2
if [ "$failed" -eq 0 ]; then
  echo "Verification PASSED (${n}/${n}). Evidence: ${DIR_ABS}"
  exit 0
else
  echo "Verification FAILED (${failed}/${n}). Do NOT claim done. Evidence: ${DIR_ABS}"
  exit 1
fi
