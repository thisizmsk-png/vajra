#!/usr/bin/env bash
# vajra-worktree-clean.sh: remove the git worktrees one vajra-deliver run used.
#
#   bash ~/.claude/skills/vajra/scripts/vajra-worktree-clean.sh \
#     --commits <sha,sha,...> [--branch-prefix vajra/<run_id>/] [--allow <dir,dir,...>] [--dry-run]
#
# Run from the repository (the user's checkout). Claude Code does not remove an
# agent's worktree when it holds untracked files, and every delivery leaves
# some (the walkthrough evidence, a .venv), so each run left its worktrees under
# <repo>/.claude/worktrees/. Once the write-back has recorded the evidence (the
# closed tasks and the knowledge records keep its sha and name), they can go.
#
# A worktree from `git worktree list --porcelain` is a candidate only if ALL hold:
#   - it is not the main worktree and not the checkout this runs in;
#   - its path is below a .claude/worktrees/ directory;
#   - its HEAD is detached at one of --commits (prefix match), or it holds a
#     branch under --branch-prefix;
#   - it is not locked.
# A candidate is removed (git worktree remove --force) only if it has no
# tracked change at all (staged or not) and its only untracked files are under
# the --allow directories (repo-relative). Everything else is kept and named.
# Then `git worktree prune`. One line per worktree on stdout:
#   removed <path>
#   kept <path>: <reason>
# and a final "worktrees: N removed, M kept". Exit 0 (also when some were
# kept), 64 on bad arguments, 1 outside a git repository. bash 3.2 compatible.

set -uo pipefail

COMMITS="" PREFIX="" ALLOW="" DRY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --commits) COMMITS="${2:-}"; shift 2 ;;
    --branch-prefix) PREFIX="${2:-}"; shift 2 ;;
    --allow) ALLOW="${2:-}"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    -h|--help) sed -n '2,27p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1 (see --help)" >&2; exit 64 ;;
  esac
done

# Inputs are code-built by the workflow; validate them anyway.
IFS=, read -r -a SHAS <<< "$COMMITS"
for s in ${SHAS[@]+"${SHAS[@]}"}; do
  [[ "$s" =~ ^[0-9a-f]{7,40}$ ]] || { echo "bad commit: $s" >&2; exit 64; }
done
if [ -n "$PREFIX" ] && ! [[ "$PREFIX" =~ ^vajra/[a-z0-9][a-z0-9-]{2,40}/$ ]]; then
  echo "bad branch prefix: $PREFIX (want vajra/<run_id>/)" >&2; exit 64
fi
[ "${#SHAS[@]}" -gt 0 ] || [ -n "$PREFIX" ] || { echo "nothing to match: give --commits and/or --branch-prefix" >&2; exit 64; }
IFS=, read -r -a DIRS <<< "$ALLOW"
for d in ${DIRS[@]+"${DIRS[@]}"}; do
  if ! [[ "$d" =~ ^[A-Za-z0-9._-]+(/[A-Za-z0-9._-]+)*$ ]] || [[ "/$d/" == */../* || "/$d/" == */./* || "/$d/" == /.git/* ]]; then
    echo "bad --allow directory: $d" >&2; exit 64
  fi
done

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo "not inside a git work tree" >&2; exit 1; }
phys() { ( cd -P -- "$1" 2>/dev/null && pwd ) || printf '%s' "$1"; }
TOP="$(phys "$(git rev-parse --show-toplevel)")"

match_commit() { # full-sha -> 0 if it starts with one of SHAS
  local s
  for s in ${SHAS[@]+"${SHAS[@]}"}; do case "$1" in "$s"*) return 0 ;; esac; done
  return 1
}

REMOVED=0 KEPT=0
keep() { echo "kept $1: $2"; KEPT=$((KEPT + 1)); }

consider() { # path head branch detached locked index
  local p="$1" head="$2" branch="$3" detached="$4" locked="$5" idx="$6" pp ours=0 tracked untracked out
  [ -n "$p" ] || return 0
  [ "$idx" -gt 0 ] || return 0                    # the main worktree, never
  pp="$(phys "$p")"
  [ "$pp" = "$TOP" ] && return 0                  # the checkout this runs in, never
  if [ "$detached" = 1 ] && match_commit "$head"; then ours=1; fi
  if [ -n "$PREFIX" ] && [[ "$branch" == "refs/heads/$PREFIX"* ]]; then ours=1; fi
  [ "$ours" = 1 ] || return 0                     # not this run's
  case "$pp/" in */.claude/worktrees/?*) ;; *) keep "$p" "not under .claude/worktrees/ (left alone)"; return 0 ;; esac
  [ "$locked" = 1 ] && { keep "$p" "locked (in use?)"; return 0; }
  [ -d "$p" ] || return 0                         # already gone: prune handles it
  if ! tracked="$(git -C "$p" status --porcelain --untracked-files=no 2>&1)"; then keep "$p" "git status failed"; return 0; fi
  if [ -n "$tracked" ]; then keep "$p" "uncommitted changes to tracked files"; return 0; fi
  local spec=( -- . ) d
  for d in ${DIRS[@]+"${DIRS[@]}"}; do spec+=( ":(exclude)$d" ); done
  if ! untracked="$(git -C "$p" status --porcelain --untracked-files=all "${spec[@]}" 2>&1)"; then keep "$p" "git status failed"; return 0; fi
  if [ -n "$untracked" ]; then
    keep "$p" "untracked files outside ${ALLOW:-the allowed directories}: $(printf '%s\n' "$untracked" | head -3 | sed 's/^?? //' | tr '\n' ' ' | sed 's/ $//')"
    return 0
  fi
  if [ "$DRY" = 1 ]; then echo "would remove $p"; REMOVED=$((REMOVED + 1)); return 0; fi
  if out="$(git worktree remove --force "$p" 2>&1)"; then
    echo "removed $p"; REMOVED=$((REMOVED + 1))
  else
    keep "$p" "git worktree remove failed: $(printf '%s' "$out" | head -1)"
  fi
}

LIST="$(git worktree list --porcelain 2>/dev/null)" || { echo "git worktree list failed" >&2; exit 1; }
P="" HEADSHA="" BR="" DET=0 LOCK=0 I=-1
while IFS= read -r line; do
  case "$line" in
    "worktree "*) consider "$P" "$HEADSHA" "$BR" "$DET" "$LOCK" "$I"; I=$((I + 1)); P="${line#worktree }"; HEADSHA="" BR="" DET=0 LOCK=0 ;;
    "HEAD "*) HEADSHA="${line#HEAD }" ;;
    "branch "*) BR="${line#branch }" ;;
    detached) DET=1 ;;
    locked|"locked "*) LOCK=1 ;;
  esac
done <<< "$LIST"
consider "$P" "$HEADSHA" "$BR" "$DET" "$LOCK" "$I"

[ "$DRY" = 1 ] || git worktree prune >/dev/null 2>&1 || true
echo "worktrees: ${REMOVED} removed, ${KEPT} kept"
exit 0
