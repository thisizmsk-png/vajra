#!/usr/bin/env bash
# Tests for scripts/vajra_tasks.py (campaign task graph). Runs in a temp HOME; never touches ~/.claude.
# Run: bash tests/tasks/run-tasks.sh
set -uo pipefail
command -v python3 >/dev/null || { echo "SKIP: python3 required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PY="$REPO/scripts/vajra_tasks.py"
VW="$REPO/scripts/vajra-verify-work.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vajra-tasks-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
export HOME="$WORK/home"
mkdir -p "$HOME"
CD="$HOME/.claude/vajra/campaigns"
C=camp-one
T() { python3 "$PY" "$@" --campaign "$C"; }
TJ() { python3 "$PY" "$@" --campaign "$C" --json; }
field() { python3 -c 'import json,sys; d=json.load(sys.stdin)
for k in sys.argv[1].split("."): d=d[int(k)] if k.isdigit() else d[k]
print(json.dumps(d) if isinstance(d,(list,dict)) else d)' "$1"; }

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }
rc() { "$@" >/dev/null 2>&1; echo $?; }

# --- usage and input validation ---
[ "$(rc python3 "$PY")" = 2 ]; check "no command -> exit 2" $?
[ "$(rc python3 "$PY" ready)" = 2 ]; check "missing --campaign -> exit 2" $?
[ "$(rc python3 "$PY" ready --campaign ../../etc)" = 2 ]; check "traversal slug -> exit 2" $?
[ "$(rc T add t1 --title a --owner bhima)" = 1 ]; check "add into a missing campaign -> exit 1" $?
[ ! -e "$CD/$C" ]; check "missing campaign is not created" $?
mkdir -p "$CD/$C"
[ "$(rc T add 'bad id' --title a --owner bhima)" = 2 ]; check "invalid task id -> exit 2" $?
[ "$(rc T add t1 --title a --owner 'Bad Agent')" = 2 ]; check "invalid owner -> exit 2" $?
[ "$(rc T add t1 --title a --owner bhima --files '../etc')" = 2 ]; check "path traversal in files -> exit 2" $?
[ "$(rc T add t1 --title a --owner bhima --files '/abs')" = 2 ]; check "absolute path in files -> exit 2" $?
[ "$(rc T add t1 --title a --owner bhima --deps t1)" = 2 ]; check "self dependency -> exit 2" $?
[ ! -e "$CD/$C/tasks.json" ]; check "refused adds write nothing" $?

# --- add / ready / status ---
T add t1 --title "flag" --owner bhima --files 'src/export/,test/*.ts' --acceptance ac-1 --branch vajra/r1/t1 >/dev/null; check "add t1" $?
T add t2 --title "docs" --owner abhimanyu --deps t1 --files README.md --acceptance ac-2 >/dev/null; check "add t2 depending on t1" $?
T add t3 --title "ci" --owner hanuman --files .github/ >/dev/null; check "add t3" $?
[ "$(stat -f %Lp "$CD/$C/tasks.json" 2>/dev/null || stat -c %a "$CD/$C/tasks.json")" = "600" ]; check "tasks.json mode 600" $?
[ "$(TJ status t1 | field task.status)" = ready ]; check "t1 without deps is ready" $?
[ "$(TJ status t2 | field task.status)" = pending ]; check "t2 waits for t1 (pending)" $?
[ "$(TJ status t1 | field task.files)" = '["src/export/", "test/*.ts"]' ]; check "files globs stored" $?
[ "$(TJ status t1 | field task.attempts)" = 0 ]; check "attempts start at 0" $?
[ "$(T ready | tr '\n' ' ')" = "t1 t3 " ]; check "ready lists t1 and t3" $?
[ "$(TJ add t1 --title "flag" --owner bhima --files 'src/export/,test/*.ts' --acceptance ac-1 | field result)" = unchanged ]; check "re-adding the same task is a no-op" $?
[ "$(TJ add t3 --title "ci config" --owner hanuman --files .github/ | field result)" = updated ]; check "re-adding with a new title updates it" $?
[ "$(rc T status nope)" = 1 ]; check "unknown task -> exit 1" $?

# --- validate ---
[ "$(rc T validate)" = 2 ]; check "validate: unknown agents (no ~/.claude/agents) -> exit 2" $?
{ TJ validate 2>/dev/null || true; } | grep -q 'unknown agent bhima'; check "validate --json names the unknown agent" $?
mkdir -p "$HOME/.claude/agents"; for a in bhima abhimanyu hanuman nakula; do echo "# $a" > "$HOME/.claude/agents/$a.md"; done
T validate >/dev/null; check "validate passes with known agents" $?
T add t9 --title x --owner nakula --deps ghost --files x/ >/dev/null
[ "$(rc T validate)" = 2 ]; check "validate: unknown dependency -> exit 2" $?
T dep t9 ghost --remove >/dev/null; check "dep --remove" $?
T validate >/dev/null; check "validate passes again" $?
[ "$(rc T dep t1 t2)" = 2 ]; check "dep that closes a cycle is refused (exit 2)" $?
[ "$(TJ status t1 | field task.deps)" = '[]' ]; check "refused dep leaves t1 unchanged" $?
T add t9 --title x --owner nakula --deps t2 --files x/ >/dev/null
T add t2 --title docs --owner abhimanyu --deps t1,t9 --files README.md --acceptance ac-2 >/dev/null
[ "$(rc T validate)" = 2 ]; check "validate: cycle created through add -> exit 2" $?
{ TJ validate 2>/dev/null || true; } | grep -q 'dependency cycle'; check "validate names the cycle" $?
T add t2 --title docs --owner abhimanyu --deps t1 --files README.md --acceptance ac-2 >/dev/null
T add t9 --title x --owner nakula --files x/ >/dev/null
T validate >/dev/null; check "cycle removed" $?

# --- claim (atomic) ---
[ "$(rc T claim t2 w1)" = 3 ]; check "claiming a pending task -> exit 3" $?
T claim t1 bhima-1 >/dev/null; check "claim ready t1" $?
[ "$(TJ status t1 | field task.status)" = claimed ] && [ "$(TJ status t1 | field task.attempts)" = 1 ]; check "claimed, attempts 1" $?
T claim t1 bhima-1 >/dev/null; check "same worker re-claim is idempotent" $?
[ "$(rc T claim t1 other)" = 3 ]; check "another worker cannot claim it (exit 3)" $?
OK=0
for i in 1 2 3 4 5 6 7 8; do ( python3 "$PY" claim t3 "w$i" --campaign "$C" >/dev/null 2>&1 && echo ok > "$WORK/claim.$i" ) & done
wait
OK="$(ls "$WORK"/claim.* 2>/dev/null | wc -l | tr -d ' ')"
[ "$OK" = 1 ]; check "8 concurrent claims: exactly one wins ($OK)" $?
python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$CD/$C/tasks.json"; check "tasks.json still valid JSON after concurrent writes" $?
[ ! -d "$CD/$C/.tasks.lock" ]; check "lock released" $?

# --- report ---
T report t1 in_progress >/dev/null; check "report in_progress" $?
[ "$(TJ status t1 | field task.attempts)" = 1 ]; check "claim then in_progress is one attempt" $?
T report t1 done --summary "implemented the flag" >/dev/null; check "report done" $?
[ "$(TJ status t1 | field task.status)" = in_progress ]; check "report done is only a claim: status stays in_progress" $?
[ "$(TJ status t1 | field task.report.status)" = done ]; check "the done claim is recorded" $?
[ "$(TJ status t2 | field task.status)" = pending ]; check "dependents stay pending on a done claim" $?
[ "$(rc T report t1 finished)" = 2 ]; check "unknown report status -> exit 2" $?
T report t1 failed >/dev/null
[ "$(rc T report t1 progress)" = 3 ]; check "report progress on a failed task -> exit 3 (retry goes through claim)" $?
[ "$(rc T report t2 in_progress)" = 3 ]; check "report in_progress on a pending task -> exit 3" $?
[ "$(TJ status t2 | field task.status)" = pending ]; check "refused report leaves t2 pending" $?
T claim t1 bhima-1 >/dev/null; check "a failed task is retried by claiming it again" $?
T report t1 progress >/dev/null; check "report progress after the re-claim" $?
[ "$(TJ status t1 | field task.attempts)" = 2 ]; check "the retry is a new attempt" $?
LONG="$(python3 -c 'print("x"*900)')"
T report t1 question --summary "$LONG" >/dev/null
[ "$(TJ status t1 | field task.report.summary | wc -c | tr -d ' ')" = 501 ]; check "summary capped at 500" $?
[ "$(rc T claim t2 w2)" = 3 ]; check "a pending task cannot be claimed even as a retry" $?
T claim t1 bhima-1 >/dev/null && T report t1 in_progress >/dev/null; check "question -> claim -> in_progress" $?
T add t5 --title ready-one --owner bhima --files t5/ >/dev/null
T report t5 in_progress >/dev/null; check "report in_progress on a ready task (no claim) is allowed" $?
[ "$(TJ status t5 | field task.attempts)" = 1 ]; check "it starts an attempt" $?

# --- close needs a claimed task, done deps and green, signed evidence for this repo ---
KEYF="$HOME/.claude/vajra/.hmac-key"
mkdir -p "$HOME/.claude/vajra"
python3 -c 'import secrets; print(secrets.token_hex(32))' > "$KEYF"
# sign FILE [--force]: add the contract hmac (sort_keys, (',', ':'), UTF-8) unless one is present.
sign() { python3 - "$1" "$KEYF" "${2:-}" <<'PY'
import hashlib, hmac, json, sys
p, kf, force = sys.argv[1], sys.argv[2], sys.argv[3] == "--force"
ev = json.load(open(p))
if "hmac" in ev and not force:
    sys.exit(0)
ev.pop("hmac", None)
key = bytes.fromhex(open(kf).read().strip())
ev["hmac"] = hmac.new(key, json.dumps(ev, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8"), hashlib.sha256).hexdigest()
json.dump(ev, open(p, "w"))
PY
}
# edit FILE PYCODE: change the evidence (ev) in place, keeping its hmac.
edit() { python3 -c 'import json,sys; ev=json.load(open(sys.argv[1])); exec(sys.argv[2]); json.dump(ev, open(sys.argv[1],"w"))' "$1" "$2"; }
[ "$(rc T close t1 --evidence "$WORK/none.json")" = 3 ]; check "close without an evidence file -> exit 3" $?
if command -v jq >/dev/null && command -v git >/dev/null; then
  R="$WORK/repo"; mkdir -p "$R"; git -C "$R" init -q; git -C "$R" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init
  echo 'data/' > "$R/.gitignore"; git -C "$R" add .gitignore; git -C "$R" -c user.email=t@t -c user.name=t commit -q -m ignore
  RC() { ( cd "$R" && rc python3 "$PY" "$@" --campaign "$C" ); }
  RT() { ( cd "$R" && python3 "$PY" "$@" --campaign "$C" ); }
  vw() { ( cd "$R" && bash "$VW" "$@" >/dev/null 2>&1 ); }
  WT="$R/data/walkthroughs"
  vw red "git rev-parse HEAD" false; sign "$WT/red/evidence.json"
  vw green "git rev-parse HEAD" "test -f .gitignore"; check "verify-work produced green evidence" $?
  sign "$WT/green/evidence.json"
  GREEN="$WT/green/evidence.json"
  mkdir -p "$WT/fake"; echo '{"allPassed": true}' > "$WT/fake/evidence.json"
  [ "$(RC close t1 --evidence "$WT/fake/evidence.json")" = 3 ]; check "close with a non verify-work file -> exit 3" $?
  [ "$(RC close t1 --evidence "$WT/red/evidence.json")" = 3 ]; check "close with red evidence -> exit 3" $?
  mkdir -p "$WT/unsigned" "$WT/tampered" "$WT/trivial" "$WT/unstable" "$WT/nohead"
  cp "$GREEN" "$WT/unsigned/evidence.json"; edit "$WT/unsigned/evidence.json" 'ev.pop("hmac")'
  [ "$(RC close t1 --evidence "$WT/unsigned/evidence.json")" = 3 ]; check "close with unsigned evidence -> exit 3" $?
  cp "$GREEN" "$WT/tampered/evidence.json"; edit "$WT/tampered/evidence.json" 'ev["commands"][0]["cmd"]="make test"'
  [ "$(RC close t1 --evidence "$WT/tampered/evidence.json")" = 3 ]; check "close with tampered evidence (hmac no longer verifies) -> exit 3" $?
  vw trivial-run true "echo ok" ":" "exit 0"; mv "$WT/trivial-run/evidence.json" "$WT/trivial/evidence.json"; sign "$WT/trivial/evidence.json" --force
  [ "$(RC close t1 --evidence "$WT/trivial/evidence.json")" = 3 ]; check "close with only trivial checks (true, echo, :, exit 0) -> exit 3" $?
  cp "$GREEN" "$WT/unstable/evidence.json"; edit "$WT/unstable/evidence.json" 'ev["git"]["treeStable"]=False'; sign "$WT/unstable/evidence.json" --force
  [ "$(RC close t1 --evidence "$WT/unstable/evidence.json")" = 3 ]; check "close with treeStable false (signed) -> exit 3" $?
  cp "$GREEN" "$WT/nohead/evidence.json"; edit "$WT/nohead/evidence.json" 'ev["git"]=None'; sign "$WT/nohead/evidence.json" --force
  [ "$(RC close t1 --evidence "$WT/nohead/evidence.json")" = 3 ]; check "close with no git section (signed) -> exit 3" $?
  # location: only <repo_root>/data/walkthroughs/<slug>/evidence.json
  mkdir -p "$WORK/elsewhere" "$R/data/other" "$WT/a/b" "$WT/link"
  cp "$GREEN" "$WORK/elsewhere/evidence.json"; cp "$GREEN" "$R/data/other/evidence.json"; cp "$GREEN" "$WT/a/b/evidence.json"
  ln -s "$WORK/elsewhere/evidence.json" "$WT/link/evidence.json"
  [ "$(RC close t1 --evidence "$WORK/elsewhere/evidence.json")" = 3 ]; check "close with evidence outside the repo -> exit 3" $?
  [ "$(RC close t1 --evidence "$R/data/other/evidence.json")" = 3 ]; check "close with evidence outside data/walkthroughs -> exit 3" $?
  [ "$(RC close t1 --evidence "$WT/a/b/evidence.json")" = 3 ]; check "close with evidence nested too deep -> exit 3" $?
  [ "$(RC close t1 --evidence "$WT/link/evidence.json")" = 3 ]; check "close through a symlink that leaves data/walkthroughs -> exit 3" $?
  [ "$(rc T close t1 --evidence "$GREEN")" = 3 ]; check "close run outside the evidence's repo -> exit 3" $?
  # no key: refused, and no key is created
  NOKEY="$WORK/nokey"; mkdir -p "$NOKEY/.claude/vajra"; cp -R "$CD" "$NOKEY/.claude/vajra/"
  [ "$( cd "$R" && HOME="$NOKEY" rc python3 "$PY" close t1 --evidence "$GREEN" --campaign "$C")" = 3 ]; check "close without a Vajra key -> exit 3" $?
  [ ! -e "$NOKEY/.claude/vajra/.hmac-key" ]; check "no HMAC key is created" $?
  # state: only claimed or in_progress, and every dependency done
  [ "$(RC close t5 --evidence "$GREEN")" = 0 ]; check "close an in_progress task with green evidence" $?
  T add t6 --title not-claimed --owner bhima --files t6/ >/dev/null
  [ "$(RC close t6 --evidence "$GREEN")" = 3 ]; check "close a ready (unclaimed) task -> exit 3" $?
  [ "$(TJ status t6 | field task.status)" = ready ]; check "refused close leaves t6 ready" $?
  T claim t6 bhima-6 >/dev/null; T dep t6 t9 >/dev/null
  [ "$(RC close t6 --evidence "$GREEN")" = 3 ]; check "close with a dependency not done -> exit 3" $?
  { RT close t6 --evidence "$GREEN" --json 2>/dev/null || true; } | grep -q 'not done: t9'; check "the refusal names the open dependency" $?
  # head: the current HEAD, or the commit the caller names
  OLD="$(git -C "$R" rev-parse HEAD)"
  git -C "$R" -c user.email=t@t -c user.name=t commit -q --allow-empty -m later
  [ "$(TJ status t1 | field task.status)" = in_progress ]; check "refused closes leave the task open" $?
  [ "$(RC close t1 --evidence "$GREEN")" = 3 ]; check "close with evidence for an older HEAD -> exit 3" $?
  [ "$(RC close t1 --evidence "$GREEN" --head zzz)" = 2 ]; check "close with a bad --head -> exit 2" $?
  [ "$(RC close t1 --evidence "$GREEN" --head "$(git -C "$R" rev-parse HEAD)")" = 3 ]; check "close --head naming another commit -> exit 3" $?
  [ "$(RC close t1 --evidence "$GREEN" --commit zzz)" = 2 ]; check "close with a bad --commit -> exit 2" $?
  [ "$(RC close t1 --evidence "$GREEN" --he "$OLD")" = 2 ]; check "abbreviated options are refused (allow_abbrev=False)" $?
  RT close t1 --evidence "$GREEN" --head "${OLD:0:12}" --commit b111111 --branch vajra/r1/t1 --base-commit aaaaaaa --changed 'src/export/a.ts,test/x.ts' >/dev/null; check "close with green evidence for the named --head" $?
  [ "$(TJ status t1 | field task.status)" = done ]; check "t1 done" $?
  [ "$(TJ status t1 | field task.commit)" = b111111 ] && [ "$(TJ status t1 | field task.changed)" = '["src/export/a.ts", "test/x.ts"]' ]; check "close records commit and changed files" $?
  TJ status t1 | field task.evidence.sha256 | grep -Eq '^[0-9a-f]{64}$'; check "evidence hash recorded" $?
  [ "$(TJ status t1 | field task.evidence.head)" = "$OLD" ]; check "evidence head recorded" $?
  [ "$(TJ status t2 | field task.status)" = ready ]; check "closing t1 makes t2 ready" $?
  [ "$(rc T report t1 in_progress)" = 3 ]; check "a done task cannot be reopened by report (exit 3)" $?
  [ "$(TJ add t1 --title other --owner bhima | field result)" = exists ]; check "add on a done task keeps it" $?
  [ "$(TJ status t1 | field task.title)" = flag ]; check "done task definition unchanged" $?
  [ "$( cd "$R" && python3 "$PY" close t1 --evidence "$GREEN" --campaign "$C" --json | field result)" = "already done" ]; check "close is idempotent" $?
else
  echo "  SKIP: close/evidence checks need jq and git"
fi

# --- claim --stacked (vajra-deliver: dependents build on reported-done dependencies) ---
C=camp-stacked; mkdir -p "$CD/$C"
T add s1 --title base --owner bhima --files s1/ >/dev/null
T add s2 --title dependent --owner abhimanyu --deps s1 --files s2/ >/dev/null
[ "$(rc T claim s2 abhimanyu --stacked)" = 3 ]; check "stacked claim refused while the dependency is only ready" $?
T claim s1 bhima >/dev/null; T report s1 in_progress >/dev/null
[ "$(rc T claim s2 abhimanyu --stacked)" = 3 ]; check "stacked claim refused while the dependency is in progress without a done report" $?
T report s1 done --summary built >/dev/null
[ "$(rc T claim s2 abhimanyu)" = 3 ]; check "plain claim still needs the dependency done" $?
T claim s2 abhimanyu --stacked >/dev/null; check "stacked claim accepted once the dependency reported done" $?
T report s2 in_progress >/dev/null; check "the stacked task reports in_progress" $?
[ "$(rc T claim s2 abhimanyu --stacked)" = 0 ]; check "the same worker re-claims a stacked task idempotently" $?
[ "$(rc T claim s2 abhimanyu --stack)" = 2 ]; check "abbreviated --stack is refused" $?
if [ -n "${GREEN:-}" ] && [ -n "${R:-}" ]; then
  # GREEN is for $OLD; name it with --head.
  [ "$(RC close s2 --evidence "$GREEN" --head "$OLD")" = 3 ]; check "a stacked task cannot close before its dependency" $?
  [ "$(RC close s1 --evidence "$GREEN" --head "$OLD")" = 0 ] && [ "$(RC close s2 --evidence "$GREEN" --head "$OLD")" = 0 ]; check "closing in dependency order closes both" $?
fi

# --- waves / graph / list ---
C=camp-waves; mkdir -p "$CD/$C"
T add a --title a --owner bhima --files src/ >/dev/null
T add b --title b --owner bhima --files src/x.ts >/dev/null
T add c --title c --owner bhima --files docs/ >/dev/null
T add d --title d --owner bhima --deps a,c --files test/ >/dev/null
T add e --title e --owner bhima --files lib/ >/dev/null
[ "$(T waves)" = '[["a", "c", "e"], ["b", "d"]]' ]; check "waves: deps order them, overlapping files split them ($(T waves))" $?
[ "$(T waves --max-in-flight 2)" = '[["a", "c"], ["e"], ["b", "d"]]' ]; check "waves --max-in-flight 2 splits each wave into batches" $?
[ "$(rc T waves --max-in-flight 0)" = 2 ]; check "bad --max-in-flight -> exit 2" $?
G="$(T graph)"
printf '%s\n' "$G" | head -1 | grep -qx 'graph TD'; check "graph is Mermaid" $?
printf '%s\n' "$G" | grep -q -- '-->'; check "graph has dependency edges" $?
T add q --title 'evil"] --> x[' --owner bhima --files q/ >/dev/null
! T graph | grep -q 'evil"]'; check "graph escapes titles" $?
[ "$(TJ list | field tasks.0.id)" = a ]; check "list --json" $?

# --- integrity ---
echo 'not json' > "$CD/$C/tasks.json"
[ "$(rc T list)" = 1 ]; check "corrupt tasks.json -> exit 1" $?
echo '{"schema":"other","tasks":[]}' > "$CD/$C/tasks.json"
[ "$(rc T list)" = 1 ]; check "foreign schema -> exit 1" $?
rm -f "$CD/$C/tasks.json"
mkdir "$CD/$C/.tasks.lock"; touch -t 202001010000 "$CD/$C/.tasks.lock"
T add z --title z --owner bhima --files z/ >/dev/null; check "a stale lock is broken" $?

echo "----------------------------------------"
echo "tasks: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
