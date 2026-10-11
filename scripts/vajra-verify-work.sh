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

set -uo pipefail

usage() { echo "usage: vajra-verify-work.sh <slug> \"<cmd>\" [\"<cmd>\" ...]  (at least one command)" >&2; exit 2; }

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
EXCL=""   # pathspec excluding the walkthrough base dir when it lies inside the repo
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  IN_GIT=1
  TOP="$(cd "$(git rev-parse --show-toplevel)" && pwd -P)"
  BASE_ABS="$(cd "$BASE" && pwd -P)"
  case "$BASE_ABS/" in "$TOP"/*) EXCL=":(exclude)${BASE_ABS#"$TOP"/}" ;; esac
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
  bash -c "$cmd" >> "$log" 2>&1
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

if [ "$have_jq" -eq 1 ]; then
  jq -n --arg slug "$SLUG" --arg ts "$TS" --argjson total "$n" --argjson failed "$failed" \
        --argjson cmds "[${entries}]" --arg head "$HEAD_SHA" --arg tree "$TREE" --arg after "$TREE_AFTER" \
        --argjson git "$([ "$IN_GIT" -eq 1 ] && echo true || echo false)" --argjson dirty "$DIRTY" --argjson stable "$STABLE" \
        --argjson untracked "$(wc -l < "${DIR}/untracked.txt" | tr -d ' ')" \
        'def nz: if . == "" then null else . end;
         {slug:$slug, generatedAt:$ts, commandsTotal:$total, commandsFailed:$failed, allPassed:($failed==0),
          git: (if $git then {head:($head|nz), worktreeTree:($tree|nz), worktreeTreeAfter:($after|nz), treeStable:$stable,
                diffBase:"HEAD+untracked", uncommittedChanges:$dirty, untrackedFiles:$untracked} else null end),
          commands:$cmds}' > "${DIR}/evidence.json"
fi

# Sign the evidence (contract: docs/roadmap/v0.6-review-fixes.md, "Signed evidence"):
# hmac = HMAC-SHA256(Vajra key, canonical JSON of the object without "hmac":
# sort_keys, separators (',', ':'), UTF-8). Without a key the file stays unsigned,
# and vajra_kb.py / vajra_tasks.py do not count it as green.
KEY_FILE="${HOME}/.claude/vajra/.hmac-key"
if [ -f "${DIR}/evidence.json" ] && [ -r "$KEY_FILE" ] && command -v python3 >/dev/null 2>&1; then
  python3 -I -c '
import hashlib, hmac, json, os, re, sys
path, keyf = sys.argv[1], sys.argv[2]
k = open(keyf, encoding="utf-8").read().strip()
if not re.fullmatch(r"[0-9a-fA-F]{32,}", k) or len(k) % 2:
    sys.exit(0)
ev = json.load(open(path, encoding="utf-8"))
ev.pop("hmac", None)
msg = json.dumps(ev, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
ev["hmac"] = hmac.new(bytes.fromhex(k), msg, hashlib.sha256).hexdigest()
tmp = path + ".tmp"
with open(tmp, "w", encoding="utf-8") as f:
    json.dump(ev, f, indent=2, ensure_ascii=False)
    f.write("\n")
os.replace(tmp, path)
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
