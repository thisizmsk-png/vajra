#!/usr/bin/env bash
# Tests for scripts/vajra-campaign.sh (HMAC-signed campaign state). Runs in a temp HOME,
# under the bash running this file (CI also runs it with macOS /bin/bash 3.2).
# Run: bash tests/run-campaign.sh
set -uo pipefail
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 0; }
command -v openssl >/dev/null || { echo "SKIP: openssl required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO/scripts/vajra-campaign.sh"
BASH_BIN="${BASH:-bash}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vajra-campaign-XXXXXX")"
trap '[ -s "$WORK/hang.pid" ] && kill "$(cat "$WORK/hang.pid")" 2>/dev/null; rm -rf "$WORK"' EXIT
export HOME="$WORK"
C() { "$BASH_BIN" "$SCRIPT" "$@"; }
CD="$HOME/.claude/vajra/campaigns"

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

# --- empty state ---
[ -z "$(C active --brief)" ]; check "active --brief prints nothing with no campaign" $?
C active --brief >/dev/null; check "active --brief exits 0 with no campaign" $?
[ ! -e "$HOME/.claude/vajra/.hmac-key" ]; check "active --brief creates no state" $?

# --- create ---
C create "Login Flow!" --goal "Users can log in" --feature "ac-1|valid password logs in" --feature "ac-2|bad password rejected" >/dev/null 2>&1
check "create succeeds" $?
D="$CD/login-flow"
[ -f "$D/campaign.json" ] && [ -f "$D/features.json" ] && [ -f "$D/progress.md" ] && [ -f "$D/state.sig" ] && [ -d "$D/checkpoints" ]
check "slug sanitized; all state files created" $?
[ "$(jq -r '.features | map(.passes) | any' "$D/features.json")" = "false" ]; check "features start with passes=false" $?
[ "$(stat -c %a "$HOME/.claude/vajra/.hmac-key" 2>/dev/null || stat -f %Lp "$HOME/.claude/vajra/.hmac-key" 2>/dev/null)" = "600" ]; check "key created mode 600" $?
KEY="$(cat "$HOME/.claude/vajra/.hmac-key")"
! grep -rqF "$KEY" "$CD"; check "key never written into campaign state" $?
OUT="$(C list; C show; C show --json; C active --brief; C resume 2>&1)"
! printf '%s' "$OUT" | grep -qF "$KEY"; check "key never printed" $?
C create login-flow >/dev/null 2>&1; [ $? -ne 0 ]; check "duplicate create refused" $?
C create bad --feature "../x|y" >/dev/null 2>&1; [ $? -ne 0 ]; check "invalid feature id refused" $?
C create dup --feature "a|1" --feature "a|2" >/dev/null 2>&1; [ $? -ne 0 ]; check "duplicate feature id refused" $?
C create "../../etc" >/dev/null 2>&1; [ ! -e "$HOME/.claude/etc" ] && [ ! -e "$WORK/../etc/campaign.json" ]; check "traversal slug stays inside campaigns/" $?
[ -d "$CD/etc" ]; check "traversal slug sanitized to a single segment" $?
C close etc --status abandoned >/dev/null 2>&1

# make login-flow the active one again
C resume login-flow >/dev/null 2>&1; check "resume valid campaign" $?

# --- active --brief ---
B="$(C active --brief)"
[ "$(printf '%s\n' "$B" | wc -l | tr -d ' ')" = "1" ]; check "active --brief prints exactly one line" $?
printf '%s' "$B" | grep -q "login-flow: 0/2 features passing; next: ac-1"; check "brief shows counts and next feature" $?
case "$B" in "login-flow: "*) true ;; *) false ;; esac; check "brief starts with the slug (no 'Vajra campaign' prefix; callers add it)" $?

# --- pass / progress ---
C pass ac-1 --evidence "npm test: 12 passed" >/dev/null; check "pass succeeds" $?
[ "$(jq -r '.features[] | select(.id=="ac-1") | .passes' "$D/features.json")" = "true" ]; check "pass flips passes" $?
[ "$(jq -r '.features[] | select(.id=="ac-1") | .description' "$D/features.json")" = "valid password logs in" ]; check "pass keeps description" $?
grep -q "pass: ac-1 (evidence: npm test: 12 passed)" "$D/progress.md"; check "pass logged to progress.md" $?
C pass nope >/dev/null 2>&1; [ $? -ne 0 ]; check "unknown feature refused" $?
C progress "$(printf 'line one\nline two')" >/dev/null; check "progress succeeds" $?
grep -q "line one line two" "$D/progress.md"; check "progress flattened to one line" $?
C pass ac-1 --revoke >/dev/null && [ "$(jq -r '.features[0].passes' "$D/features.json")" = "false" ]; check "--revoke sets passes=false" $?
C pass ac-1 >/dev/null
C verify >/dev/null; check "signature valid after mutations" $?

# --- checkpoint ---
C checkpoint "after ac-1" >/dev/null; check "checkpoint 1" $?
C progress "more work" >/dev/null
C checkpoint >/dev/null; check "checkpoint 2" $?
[ -f "$D/checkpoints/0001.json" ] && [ -f "$D/checkpoints/0002.json" ]; check "checkpoint files written" $?
[ "$(jq -r .prevHmac "$D/checkpoints/0002.json")" = "$(jq -r .hmac "$D/checkpoints/0001.json")" ]; check "checkpoints are chained" $?

# --- tamper detection: only the script may change state ---
cp "$D/features.json" "$WORK/features.bak"
jq '.features[1].passes = true' "$WORK/features.bak" > "$D/features.json"
C verify >/dev/null 2>&1; [ $? -eq 3 ]; check "hand-flipped passes detected (exit 3)" $?
C pass ac-2 >/dev/null 2>&1; [ $? -eq 3 ]; check "mutation refused on tampered state" $?
R="$(C active --brief)"; printf '%s' "$R" | grep -q "^login-flow: state signature INVALID"; check "brief warns on tampered state" $?
C resume >/dev/null 2>&1; [ $? -eq 3 ]; check "resume refuses tampered state" $?
R="$(C resume 2>&1)"; printf '%s' "$R" | grep -q -- "--from-checkpoint 2"; check "resume points at latest valid checkpoint" $?
cp "$WORK/features.bak" "$D/features.json"
C verify >/dev/null; check "restoring the exact bytes verifies again" $?

echo "evil" >> "$D/progress.md"
C verify >/dev/null 2>&1; [ $? -eq 3 ]; check "edited progress.md detected" $?
C resume login-flow --from-checkpoint 2 >/dev/null; check "restore from checkpoint 2" $?
C verify >/dev/null && ! grep -q evil "$D/progress.md"; check "restored state is valid and clean" $?

# definition change, even re-signed with the real key, is caught by the definition hash
jq '.features[0].description = "changed"' "$D/features.json" > "$WORK/f2" && cp "$WORK/f2" "$D/features.json"
C verify >/dev/null 2>&1; [ $? -eq 3 ]; check "feature definition change detected" $?
C resume login-flow --from-checkpoint 2 >/dev/null 2>&1

# forged checkpoint (bad hmac) is not restorable
jq '.features.features[1].passes = true' "$D/checkpoints/0002.json" > "$WORK/cp" && cp "$WORK/cp" "$D/checkpoints/0002.json"
C resume login-flow --from-checkpoint 2 >/dev/null 2>&1; [ $? -eq 3 ]; check "forged checkpoint refused" $?
C resume login-flow --from-checkpoint 1 >/dev/null; check "earlier valid checkpoint still restorable" $?

# --- show / list ---
C show --json | jq -e '.signature_valid == true and (.features | length) == 2' >/dev/null; check "show --json" $?
C list | grep -q '^\* login-flow'; check "list marks active campaign" $?

# --- close ---
C close >/dev/null; check "close" $?
[ "$(jq -r .status "$D/campaign.json")" = "done" ]; check "status done" $?
[ -z "$(C active --brief)" ]; check "closed campaign not shown by active --brief" $?
C resume login-flow >/dev/null 2>&1; [ $? -ne 0 ]; check "closed campaign cannot be resumed" $?

# --- close by slug, as vajra-deliver does at delivery ---
C create deliver-20261011-0930 --feature "ac-1|x" >/dev/null 2>&1
C resume login-flow >/dev/null 2>&1 || true
C close deliver-20261011-0930 --status done >/dev/null; check "close <slug> --status done" $?
[ "$(jq -r .status "$CD/deliver-20261011-0930/campaign.json")" = "done" ]; check "named campaign closed" $?
C verify deliver-20261011-0930 >/dev/null; check "closed campaign signature still valid" $?
C close deliver-20261011-0930 --status bogus >/dev/null 2>&1; [ $? -ne 0 ]; check "close rejects an unknown status" $?

# --- usage ---
C >/dev/null 2>&1; [ $? -eq 2 ]; check "no command -> usage (exit 2)" $?
C bogus >/dev/null 2>&1; [ $? -eq 2 ]; check "unknown command -> usage (exit 2)" $?

# --- report (E4, Q3): the Deliver step saves report.md, bound into the signed state ---
C create rep --feature "ac-1|x" >/dev/null 2>&1
RD="$CD/rep"
REP="$(printf '# Delivery: x\n\nBranch `b` at `c`.\n- ünïcode ✓\n')"
B64="$(printf '%s\n' "$REP" | openssl base64 -A)"
NB="$(printf '%s\n' "$REP" | wc -c | tr -d ' ')"
fnv() { python3 -c 'import sys
h = 0x811c9dc5
for b in sys.argv[1].encode(): h = ((h ^ b) * 0x01000193) & 0xffffffff
print("%08x" % h)' "$1"; }
SHA="$(fnv "$B64")"
C report --campaign rep --bytes "$NB" --check "$SHA" --base64 "$B64" >/dev/null; check "report saved" $?
[ "$(cat "$RD/report.md")" = "$REP" ]; check "report.md is the exact text (multi-line, UTF-8)" $?
[ "$(stat -c %a "$RD/report.md" 2>/dev/null || stat -f %Lp "$RD/report.md")" = 600 ]; check "report.md mode 600" $?
C verify rep >/dev/null 2>&1; check "state verifies with the report" $?
C close rep --status done >/dev/null && C report --campaign rep --bytes "$NB" --check "$SHA" --base64 "$B64" >/dev/null; check "report works after close (Deliver closes first)" $?
printf 'tampered\n' >> "$RD/report.md"
C verify rep >/dev/null 2>&1; [ $? -ne 0 ]; check "edited report.md fails verify" $?
C report --campaign rep --bytes "$NB" --check "$SHA" --base64 "$B64" >/dev/null 2>&1; [ $? -ne 0 ]; check "report refuses a campaign whose state is invalid" $?
C create rep2 --feature "ac-1|x" >/dev/null 2>&1
C report --campaign rep2 --bytes "$NB" --check "$SHA" --base64 "$B64" >/dev/null && rm -f "$CD/rep2/report.md"
C verify rep2 >/dev/null 2>&1; [ $? -ne 0 ]; check "deleted report.md fails verify" $?
C create rep3 --feature "ac-1|x" >/dev/null 2>&1
C report --campaign rep3 --bytes 999 --check "$SHA" --base64 "$B64" >/dev/null 2>&1; [ $? -ne 0 ] && [ ! -e "$CD/rep3/report.md" ]; check "byte count mismatch refused, nothing written" $?
C report --campaign rep3 --bytes 4 --check "$SHA" --base64 '!!!!' >/dev/null 2>&1; [ $? -ne 0 ]; check "bad base64 refused" $?
C report --campaign rep3 --bytes "$NB" --check 00000000 --base64 "$B64" >/dev/null 2>&1; [ $? -ne 0 ] && [ ! -e "$CD/rep3/report.md" ]
check "check mismatch refused (a same-length copy error is caught)" $?
SWAP="$(printf '%s' "$B64" | sed 's/^\(.\)\(.\)/\2\1/')"
C report --campaign rep3 --bytes "$NB" --check "$SHA" --base64 "$SWAP" >/dev/null 2>&1; [ $? -ne 0 ] && [ ! -e "$CD/rep3/report.md" ]
check "two swapped base64 characters are caught" $?
C report --campaign rep3 --bytes "$NB" --base64 "$B64" >/dev/null 2>&1; [ $? -ne 0 ]; check "--check is required" $?
# The temp file is never a planted link: report.md.tmp pointing outside is neither followed nor needed.
C create rep4 --feature "ac-1|x" >/dev/null 2>&1; ln -s "$WORK/victim" "$CD/rep4/report.md.tmp"
C report --campaign rep4 --bytes "$NB" --check "$SHA" --base64 "$B64" >/dev/null; [ $? -eq 0 ] && [ ! -e "$WORK/victim" ]
check "a planted report.md.tmp link is not written through" $?
BIG="$(head -c 70000 /dev/zero | tr '\0' x | openssl base64 -A)"
C report --campaign rep3 --bytes 70000 --check "$SHA" --base64 "$BIG" >/dev/null 2>&1; [ $? -ne 0 ]; check "report over 64 KiB refused" $?
C verify rep3 >/dev/null 2>&1; check "campaign without a report still verifies" $?

# --- archive-evidence (E7, Q13): the verification bundle + the setup list, once per delivery, signed ---
# Run from a git repo, as the write-back step does; the evidence must be signed and green for --head,
# and at least one task of the campaign's graph must have been closed on exactly this evidence.
C create arc --feature "ac-1|x" >/dev/null 2>&1
AD="$CD/arc"
RP="$WORK/repo"; mkdir -p "$RP"; git -C "$RP" init -q; git -C "$RP" -c user.email=t@t -c user.name=t commit -q --allow-empty -m i
RHEAD="$(git -C "$RP" rev-parse HEAD)"
# mkev DIR [PYCODE]: signed green verify-work evidence for RHEAD (temp test key), PYCODE edits ev first.
mkev() { mkdir -p "$1"; python3 - "$1/evidence.json" "$HOME/.claude/vajra/.hmac-key" "$RHEAD" "${2:-}" <<'PY'
import hashlib, hmac, json, sys
p, kf, head, code = sys.argv[1:5]
ev = {"allPassed": True, "commandsTotal": 1, "commandsFailed": 0,
      "commands": [{"cmd": "test -f x", "exit": 0, "passed": True}], "git": {"head": head, "treeStable": True}}
exec(code)
key = bytes.fromhex(open(kf).read().strip())
ev["hmac"] = hmac.new(key, json.dumps(ev, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode(), hashlib.sha256).hexdigest()
json.dump(ev, open(p, "w"))
PY
}
esha() { openssl dgst -sha256 -r < "$1/evidence.json" | awk '{print $1}'; }
WK="$RP/data/walkthroughs"; SRC="$WK/deliver-20261105-1200"
mkev "$SRC"; printf 'diff\n' > "$SRC/diff.patch"; printf ' 1 file\n' > "$SRC/diff.stat"
: > "$SRC/untracked.txt"; printf 'ok\n' > "$SRC/cmd-1.log"; printf 'ok2\n' > "$SRC/cmd-12.log"; printf 'nope\n' > "$SRC/secret.txt"
# Each fixture's evidence is distinct (its own check command), so its sha256 names only its own task.
for d in unsigned red oldhead slink hlink other lnk; do mkev "$WK/$d" "ev['commands'][0]['cmd'] = 'test -f $d'"; done
jq 'del(.hmac)' "$WK/unsigned/evidence.json" > "$WK/u.tmp" && mv "$WK/u.tmp" "$WK/unsigned/evidence.json"
mkev "$WK/red" 'ev["commands"][0]["exit"] = 1; ev["commandsFailed"] = 1; ev["allPassed"] = False'
mkev "$WK/oldhead" 'ev["git"]["head"] = "f" * 40'
ln -s /etc/hosts "$WK/slink/cmd-2.log"
printf 'other\n' > "$WORK/hl-target"; ln "$WORK/hl-target" "$WK/hlink/cmd-3.log"
# tasks.json as close leaves it: run-prefixed ids, each done task records the evidence sha256 it closed on.
jq -n --arg s "$(esha "$SRC")" --arg o "$(esha "$WK/other")" --arg u "$(esha "$WK/unsigned")" --arg r "$(esha "$WK/red")" \
  --arg sl "$(esha "$WK/slink")" --arg hl "$(esha "$WK/hlink")" --arg ln "$(esha "$WK/lnk")" \
  '{schema: "vajra.tasks/v1", tasks: [
    {id: "r1.t1", status: "done", evidence: {sha256: $s}}, {id: "r1.t2", status: "in_progress", evidence: null},
    {id: "r1.t3", status: "done", evidence: {sha256: $s}}, {id: "r1.t4", status: "done", evidence: {sha256: $o}},
    {id: "x.u", status: "done", evidence: {sha256: $u}}, {id: "x.r", status: "done", evidence: {sha256: $r}},
    {id: "x.s", status: "done", evidence: {sha256: $sl}}, {id: "x.h", status: "done", evidence: {sha256: $hl}},
    {id: "x.l", status: "done", evidence: {sha256: $ln}}]}' > "$AD/tasks.json"
A() { ( cd "$RP" && C archive-evidence "$@" ); }
SETUP='["npm ci","python3 -m venv .venv && .venv/bin/pip install -r requirements.txt"]'
A arc "$SRC" "$RHEAD" "$SETUP" >/dev/null; check "archive-evidence succeeds" $?
DST="$AD/evidence/deliver-20261105-1200"
[ -f "$DST/evidence.json" ] && [ -f "$DST/diff.patch" ] && [ -f "$DST/diff.stat" ] && [ -f "$DST/untracked.txt" ] && [ -f "$DST/cmd-1.log" ] && [ -f "$DST/cmd-12.log" ]
check "bundle files copied to evidence/<slug>" $?
cmp -s "$SRC/evidence.json" "$DST/evidence.json"; check "evidence.json is copied byte-exact" $?
[ ! -e "$DST/secret.txt" ]; check "only allowlisted names are copied" $?
[ "$(jq -c . "$DST/setup.json")" = "$(printf '%s' "$SETUP" | jq -c .)" ]; check "setup.json is the exact list, untruncated" $?
jq -e --arg h "$RHEAD" '.files["setup.json"] and .files["evidence.json"] and .campaign == "arc" and .evidence == "deliver-20261105-1200"
  and .head == $h and .tasks == ["r1.t1", "r1.t3"] and (.hmac | test("^[0-9a-f]{64}$"))' "$DST/manifest.json" >/dev/null
check "manifest names campaign, evidence, head, the tasks closed on it and every file, with an hmac" $?
python3 - "$DST/manifest.json" "$HOME/.claude/vajra/.hmac-key" <<'PY'
import hashlib, hmac, json, sys
m = json.load(open(sys.argv[1])); key = bytes.fromhex(open(sys.argv[2]).read().strip()); h = m.pop("hmac")
sys.exit(h != hmac.new(key, json.dumps(m, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode(), hashlib.sha256).hexdigest())
PY
check "manifest hmac is the signed-evidence convention (key bytes, canonical JSON)" $?
[ "$(jq -r '.files["cmd-1.log"]' "$DST/manifest.json")" = "$(openssl dgst -sha256 -r < "$DST/cmd-1.log" | awk '{print $1}')" ]; check "manifest sha256 matches the file" $?
[ "$(stat -c %a "$DST/evidence.json" 2>/dev/null || stat -f %Lp "$DST/evidence.json")" = 600 ]; check "archived files mode 600" $?
! grep -rqF "$KEY" "$AD/evidence"; check "key never written into the archive" $?
[ "$(ls -A "$AD/evidence")" = deliver-20261105-1200 ]; check "no scratch files left in the archive dir" $?
grep -q 'evidence archived: deliver-20261105-1200 (r1.t1, r1.t3)' "$AD/progress.md"; check "progress names the archive and its tasks" $?
C verify arc >/dev/null 2>&1; check "campaign state still valid after the archive" $?
A arc "$SRC" "$RHEAD" "$SETUP" >/dev/null 2>&1; [ $? -ne 0 ]; check "an archive is written once (no overwrite)" $?
A arc "$WK/other" "$RHEAD" '["a","b","c","d","e","f"]' >/dev/null 2>&1; [ $? -ne 0 ]; check "more than 5 setup commands refused" $?
A arc "$WK/other" "$RHEAD" "[\"$(head -c 1001 /dev/zero | tr '\0' a)\"]" >/dev/null 2>&1; [ $? -ne 0 ]; check "setup command over 1000 chars refused" $?
A arc "$WK/other" "$RHEAD" '{"cmd":1}' >/dev/null 2>&1; [ $? -ne 0 ]; check "setup must be an array of strings" $?
A arc "$WK/other" zzz '[]' >/dev/null 2>&1; [ $? -ne 0 ]; check "an invalid head refused" $?
A arc "$WK" "$RHEAD" '[]' >/dev/null 2>&1; [ $? -ne 0 ]; check "a dir without evidence.json refused" $?
# Review #2: only signed, green evidence for --head, and only evidence a task of the graph closed on.
A arc "$WK/unsigned" "$RHEAD" '[]' >/dev/null 2>&1; [ $? -ne 0 ] && [ ! -e "$AD/evidence/unsigned" ]; check "unsigned evidence refused" $?
A arc "$WK/red" "$RHEAD" '[]' >/dev/null 2>&1; [ $? -ne 0 ]; check "red evidence refused" $?
A arc "$WK/oldhead" "$RHEAD" '[]' >/dev/null 2>&1; [ $? -ne 0 ]; check "evidence for another commit refused" $?
mkev "$WK/noclose" 'ev["commands"][0]["cmd"] = "test -f noclose"'; A arc "$WK/noclose" "$RHEAD" '[]' >/dev/null 2>&1; [ $? -ne 0 ] && [ ! -e "$AD/evidence/noclose" ]
check "evidence no closed task was closed on is refused" $?
# Review #2/#3: never through a planted link.
A arc "$WK/slink" "$RHEAD" '[]' >/dev/null 2>&1; [ $? -ne 0 ] && [ ! -e "$AD/evidence/slink" ]; check "a symlinked bundle file refuses the archive" $?
A arc "$WK/hlink" "$RHEAD" '[]' >/dev/null 2>&1; [ $? -ne 0 ] && [ ! -e "$AD/evidence/hlink" ]; check "a hard-linked bundle file refuses the archive" $?
ln -s "$WK/other" "$WK/lnkdir"; A arc "$WK/lnkdir" "$RHEAD" '[]' >/dev/null 2>&1; [ $? -ne 0 ]; check "a symlinked bundle dir is refused" $?
mkdir -p "$WORK/elsewhere"; ln -s "$WORK/elsewhere" "$AD/evidence/lnk"
A arc "$WK/lnk" "$RHEAD" '[]' >/dev/null 2>&1; [ $? -ne 0 ] && [ -z "$(ls -A "$WORK/elsewhere")" ]; check "a planted evidence/<slug> link is refused" $?
C create arc2 --feature "ac-1|x" >/dev/null 2>&1; cp "$AD/tasks.json" "$CD/arc2/tasks.json"; ln -s "$WORK/elsewhere" "$CD/arc2/evidence"
A arc2 "$WK/other" "$RHEAD" '[]' >/dev/null 2>&1; [ $? -ne 0 ] && [ -z "$(ls -A "$WORK/elsewhere")" ]; check "a symlinked evidence dir is refused" $?
A arc "$WK/other" "$RHEAD" '[]' >/dev/null; check "an empty setup list is fine" $?
printf 'x' >> "$AD/campaign.json"; mkev "$WK/late" 'ev["commands"][0]["cmd"] = "test -f late"'
jq --arg s "$(esha "$WK/late")" '.tasks += [{id: "r2.t1", status: "done", evidence: {sha256: $s}}]' "$AD/tasks.json" > "$AD/t.tmp" && mv "$AD/t.tmp" "$AD/tasks.json"
A arc "$WK/late" "$RHEAD" '[]' >/dev/null 2>&1; [ $? -ne 0 ] && [ ! -e "$AD/evidence/late" ]; check "refused when the campaign state is invalid" $?

# --- an option given without its value is refused, never a hang ---
# bounded_cmd SECONDS CMD...: run CMD directly in its own process group for at most SECONDS; 0 if it exited
# non-zero in time, 1 if it was still running (the whole group is then killed, so nothing is orphaned).
# CMD is run directly, never through a shell function: with a function, $! is the subshell, and killing it
# leaves the real process running with ppid 1 (issue #16).
bounded_cmd() {
  local secs="$1" p i=0 rc; shift
  set -m
  "$@" >/dev/null 2>&1 & p=$!
  set +m
  while kill -0 "$p" 2>/dev/null && [ "$i" -lt $((secs * 10)) ]; do sleep 0.1; i=$((i+1)); done
  if kill -0 "$p" 2>/dev/null; then
    { kill -TERM -- "-$p"; sleep 0.2; kill -KILL -- "-$p"; wait "$p"; } 2>/dev/null
    return 1
  fi
  wait "$p"; rc=$?; [ "$rc" -ne 0 ]
}
bounded() { bounded_cmd 5 "$BASH_BIN" "$SCRIPT" "$@"; }

# a hung case is reaped: the stub and the child it spawned (its pid is recorded, never found by pattern) are gone
STUB="$WORK/hang.sh"; printf '#!/bin/bash\nsleep 300 &\necho $! > "%s/hang.pid"\nwait\n' "$WORK" > "$STUB"
bounded_cmd 1 "$BASH_BIN" "$STUB"; [ $? -eq 1 ]; check "bounded_cmd reports a timeout" $?
sleep 0.3; [ -s "$WORK/hang.pid" ] && ! kill -0 "$(cat "$WORK/hang.pid")" 2>/dev/null; check "bounded_cmd reaps a hung case and its children" $?
bounded_cmd 1 "$BASH_BIN" -c 'exit 3'; check "bounded_cmd returns 0 for a fast non-zero exit" $?
! bounded_cmd 1 "$BASH_BIN" -c 'exit 0'; check "bounded_cmd returns non-zero for a fast success" $?
for args in "report --campaign" "report --bytes" "report --check" "close --status" "resume --from-checkpoint" "create nogoal-1 --goal" "fail --campaign"; do
  bounded $args; check "$args with no value exits with an error" $?
done

# --- verify-digest: the script's only key use for the core's held-bytes path (stdin: four hex lines, no slug, no files) ---
C create vd1 --feature "ac-1|x" >/dev/null 2>&1; VD="$CD/vd1"
sha() { openssl dgst -sha256 -r < "$1" | awk '{print $1}'; }
VSIG="$(tr -d ' \r\n' < "$VD/state.sig")"
vd_in() { printf 'campaign %s\nfeatures %s\nprogress %s\nsig %s\n' "$(sha "$VD/campaign.json")" "$(sha "$VD/features.json")" "$(sha "$VD/progress.md")" "$1"; }
vd_in "$VSIG" | C verify-digest >/dev/null 2>&1; check "verify-digest: the signed hashes and signature verify (exit 0)" $?
vd_in "$(printf '%064d' 0)" | C verify-digest >/dev/null 2>&1; [ $? -eq 3 ]; check "verify-digest: a different signature is exit 3" $?
printf 'campaign %s\nfeatures %s\nprogress %s\nsig %s\n' "$(printf '%064d' 1)" "$(sha "$VD/features.json")" "$(sha "$VD/progress.md")" "$VSIG" | C verify-digest >/dev/null 2>&1; [ $? -eq 3 ]; check "verify-digest: a changed campaign hash is exit 3" $?
for bad in "" "campaign x" "$(vd_in "$VSIG")
extra 1" "$(vd_in "$VSIG" | sed 's/^campaign/Campaign/')" "$(vd_in "$VSIG" | tr 'a-f' 'A-F')"; do
  printf '%s\n' "$bad" | C verify-digest >/dev/null 2>&1; [ $? -eq 2 ]; check "verify-digest: malformed input is exit 2 ($(printf '%s' "$bad" | head -c 20 | tr '\n' '|'))" $?
done
vd_in "$VSIG" | awk 'NR==1{a=$0}NR==2{b=$0}NR==3{c=$0}NR==4{d=$0}END{print a;print c;print b;print d}' | C verify-digest >/dev/null 2>&1; [ $? -eq 2 ]; check "verify-digest: lines out of order are exit 2" $?
{ vd_in "$VSIG" | tr -d '\n'; printf '\n'; } | C verify-digest >/dev/null 2>&1; [ $? -eq 2 ]; check "verify-digest: lines joined by nothing are exit 2" $?
{ vd_in "$VSIG" | sed '1s/$/\r/'; } | C verify-digest >/dev/null 2>&1; [ $? -eq 2 ]; check "verify-digest: a CR is exit 2" $?
{ vd_in "$VSIG"; printf 'x%.0s' $(seq 1 600); } 2>/dev/null | C verify-digest >/dev/null 2>&1; [ $? -eq 2 ]; check "verify-digest: more than 512 bytes is exit 2" $?
{ vd_in "$VSIG" | head -c 100; printf '\0'; vd_in "$VSIG" | tail -c +102; } | C verify-digest >/dev/null 2>&1; [ $? -eq 2 ]; check "verify-digest: a NUL is exit 2" $?
vd_in "$VSIG" | C verify-digest vd1 >/dev/null 2>&1; [ $? -eq 2 ]; check "verify-digest: any argument (a slug) is exit 2" $?
# it must not open the campaign dir at all: the result is the same with the campaigns dir unreadable
vd_in "$VSIG" > "$WORK/vd-in"; chmod 000 "$CD"; C verify-digest < "$WORK/vd-in" >/dev/null 2>&1; rc=$?; chmod 700 "$CD"
[ "$rc" -eq 0 ]; check "verify-digest: same result with the campaigns dir unreadable (it never opens it)" $?
# review #34: the temp file is removed on every exit, including exit 0 (the EXIT trap runs after the function returned)
for vd_case in "0:$VSIG" "3:$(printf '%064d' 0)" "2:"; do
  want="${vd_case%%:*}"; s="${vd_case#*:}"; VT="$WORK/vd-tmp-$want"; mkdir -p "$VT"
  if [ "$want" = 2 ]; then printf 'junk\n'; else vd_in "$s"; fi | TMPDIR="$VT" C verify-digest >/dev/null 2>"$WORK/vd-err-$want"; rc=$?
  [ "$rc" -eq "$want" ]; check "verify-digest: exit $want with a scratch TMPDIR" $?
  [ -z "$(ls -A "$VT")" ]; check "verify-digest: no vajra-digest.* temp file left after exit $want" $?
done
[ ! -s "$WORK/vd-err-0" ]; check "verify-digest: a valid check writes nothing to stderr (no unbound variable from the trap)" $?

# --- the active pointer is per repo: two concurrent runs in different repos never see each other's campaign ---
if command -v git >/dev/null 2>&1; then
  RA="$WORK/repo-a"; RB="$WORK/repo-b"; NR="$WORK/no-repo"
  for r in "$RA" "$RB"; do
    mkdir -p "$r"; git -C "$r" init -q; git -C "$r" config user.email t@t; git -C "$r" config user.name t
    git -C "$r" commit -q --allow-empty -m init
  done
  mkdir -p "$NR"
  in_a() { ( cd "$RA" && C "$@" ); }; in_b() { ( cd "$RB" && C "$@" ); }; in_nr() { ( cd "$NR" && C "$@" ); }
  in_a create run-a --feature "x|in repo a" >/dev/null 2>&1; check "repo A: create" $?
  in_a active --brief | grep -q "^run-a: "; check "repo A: its banner names run-a" $?
  [ -z "$(in_b active --brief)" ]; check "repo B: no banner before it has a campaign" $?
  in_b pass x >/dev/null 2>&1; [ $? -ne 0 ]; check "repo B: a slug-less command does not reach repo A's campaign" $?
  [ "$(jq -r '.features[0].passes' "$CD/run-a/features.json")" = false ]; check "repo A's campaign untouched by repo B" $?
  in_b create run-b --feature "y|in repo b" >/dev/null 2>&1; check "repo B: create" $?
  in_a active --brief | grep -q "^run-a: "; check "repo A: still run-a after repo B created run-b" $?
  in_b active --brief | grep -q "^run-b: "; check "repo B: banner names run-b" $?
  in_b pass y >/dev/null 2>&1; check "repo B: slug-less pass goes to run-b" $?
  [ "$(jq -r '.features[0].passes' "$CD/run-b/features.json")" = true ] && [ "$(jq -r '.features[0].passes' "$CD/run-a/features.json")" = false ]
  check "repo B: pass changed run-b only" $?
  [ "$(in_a active)" = run-a ] && [ "$(in_b active)" = run-b ]; check "active names each repo's own campaign" $?
  # A linked worktree is the same repo: it shares the pointer.
  git -C "$RA" worktree add -q "$WORK/repo-a-wt" 2>/dev/null
  ( cd "$WORK/repo-a-wt" && C active --brief ) | grep -q "^run-a: "; check "a worktree of repo A sees run-a" $?
  [ -z "$(in_nr active --brief)" ]; check "outside any repo: no banner from either repo" $?
  in_a close >/dev/null 2>&1; check "repo A: slug-less close closes run-a" $?
  [ -z "$(in_a active --brief)" ] && in_b active --brief | grep -q "^run-b: "; check "closing repo A's campaign leaves repo B's banner" $?
  in_b resume run-a >/dev/null 2>&1; [ $? -ne 0 ]; check "a closed campaign cannot be resumed" $?
  # The key is the knowledge ledger's project id: sha256 of the absolute git common dir, first 16 hex.
  cd_b="$(git -C "$RB" rev-parse --path-format=absolute --git-common-dir)"
  kb="$(printf '%s' "$cd_b" | openssl dgst -sha256 -r | cut -c1-16)"
  [ "$(tr -d ' \r\n' < "$CD/.active-$kb")" = run-b ]; check "repo B's pointer is .active-<project id>" $?
  # A pre-upgrade global pointer is never read: it could name another repo's campaign.
  printf 'run-b\n' > "$CD/.active"
  [ -z "$(in_a active --brief)" ] && [ -z "$(in_nr active --brief)" ]; check "the old global .active is ignored" $?
  rm -f "$CD/.active"

  # #84 review: Git before 2.31 does not know --path-format; it echoes the option as its own line and still exits 0.
  # A fake old git (it prints the option, then runs the real git without it) and a copy of the script that calls it:
  # two repo roots must still get two pointers. (A copy, not a PATH entry: the key holder may pin its own PATH.)
  REAL_GIT="$(command -v git)"; OLDGIT="$WORK/old-git"
  cat > "$OLDGIT" <<OG
#!/bin/sh
for a in "\$@"; do [ "\$a" = "--path-format=absolute" ] && echo "--path-format=absolute"; done
n=\$#; i=0
while [ \$i -lt \$n ]; do a="\$1"; shift; i=\$((i+1)); [ "\$a" = "--path-format=absolute" ] || set -- "\$@" "\$a"; done
exec "$REAL_GIT" "\$@"
OG
  chmod 755 "$OLDGIT"
  og_out="$(cd "$RA" && "$OLDGIT" rev-parse --path-format=absolute --git-common-dir)"; og_rc=$?
  [ "$og_rc" -eq 0 ] && [ "${og_out%%
*}" = "--path-format=absolute" ]
  check "fake old git echoes --path-format and exits 0, like git before 2.31" $?
  OLD_SCRIPT="$WORK/old-git-campaign.sh"
  sed "s#git -c core.fsmonitor= rev-parse#\"$OLDGIT\" -c core.fsmonitor= rev-parse#g" "$SCRIPT" > "$OLD_SCRIPT"
  [ "$(grep -c "$OLDGIT" "$OLD_SCRIPT")" = 2 ]; check "the copy calls the fake old git for both lookups" $?
  RC="$WORK/repo-c"; RD="$WORK/repo-d"
  for r in "$RC" "$RD"; do mkdir -p "$r"; git -C "$r" init -q; done
  OC() { "$BASH_BIN" "$OLD_SCRIPT" "$@"; }
  ( cd "$RC" && OC create run-c --feature "c|in repo c" ) >/dev/null 2>&1; check "old git: create in repo C" $?
  [ -z "$(cd "$RD" && OC active --brief)" ]; check "old git: repo D (at its root) does not see repo C's campaign" $?
  ( cd "$RD" && OC pass c ) >/dev/null 2>&1; [ $? -ne 0 ]; check "old git: a slug-less pass in repo D cannot reach repo C" $?
  ( cd "$RC" && OC active --brief ) | grep -q "^run-c: "; check "old git: repo C still sees run-c" $?
  ( cd "$RC/" && mkdir -p sub && cd sub && OC active --brief ) | grep -q "^run-c: "; check "old git: a subfolder of repo C shares its pointer" $?
  [ ! -e "$CD/.active-global" ] || [ "$(tr -d ' \r\n' < "$CD/.active-global")" != run-c ]; check "old git: repo C's pointer is not the global one" $?
  ( cd "$RC" && OC close ) >/dev/null 2>&1
fi

# --- lock owner and unlock --if-stale (6h-e) ---
C create lk --feature "f1|one" >/dev/null 2>&1; check "lock: create a campaign" $?
LD="$CD/lk"
[ ! -e "$LD/.lock" ]; check "lock: a normal create leaves no lock" $?
C pass f1 --campaign lk >/dev/null 2>&1; check "lock: a normal pass succeeds" $?
[ ! -e "$LD/.lock" ]; check "lock: a normal pass leaves no lock and no owner file" $?

# A copy that holds the lock for a while, so a test can SIGKILL it with the lock held.
HOLD_SCRIPT="$WORK/hold-campaign.sh"
awk '{print} /^  jq -n --argjson pid/ {print "  sleep 30"}' "$SCRIPT" > "$HOLD_SCRIPT"
grep -q '^  sleep 30' "$HOLD_SCRIPT"; check "lock: the hold copy was built" $?
# hold_lock: start a holder in its own process group; sets HP (the script's pid) once the owner file is written.
hold_lock() {
  set -m; "$BASH_BIN" "$HOLD_SCRIPT" pass f1 --campaign lk --revoke >/dev/null 2>&1 & HP=$!; set +m
  local i=0; while [ ! -f "$LD/.lock/owner.json" ] && [ "$i" -lt 50 ]; do sleep 0.1; i=$((i+1)); done
}
kill_holder() { kill -9 -- "-$HP" 2>/dev/null; wait "$HP" 2>/dev/null; }

hold_lock
[ -f "$LD/.lock/owner.json" ] && [ "$(jq -r .pid "$LD/.lock/owner.json")" = "$HP" ]; check "lock: owner.json records the holder's pid" $?
[ -n "$(jq -r .start "$LD/.lock/owner.json")" ]; check "lock: owner.json records a start time" $?
C unlock lk --if-stale >/dev/null 2>&1; [ $? -ne 0 ] && [ -d "$LD/.lock" ]; check "lock: a live owner with the same start time is kept" $?
kill_holder
[ -d "$LD/.lock" ] && [ -f "$LD/.lock/owner.json" ]; check "lock: SIGKILL leaves the lock and its owner file" $?
C unlock lk --if-stale >/dev/null 2>&1; check "lock: unlock --if-stale on a killed owner succeeds" $?
[ ! -e "$LD/.lock" ]; check "lock: --if-stale removed the owner file and the lock" $?
C pass f1 --campaign lk >/dev/null 2>&1; check "lock: the campaign is usable again" $?

# a reused pid: the pid is alive (this shell) but the recorded start time is not its start time
mkdir "$LD/.lock"; jq -n --argjson pid "$$" '{pid: $pid, start: "Mon Jan  1 00:00:00 1990"}' > "$LD/.lock/owner.json"
C unlock lk --if-stale >/dev/null 2>&1; [ ! -e "$LD/.lock" ]; check "lock: a reused pid (different start time) is treated as dead" $?
# the same pid with the matching start time is live
mkdir "$LD/.lock"; SELF_START="$(TZ=UTC LC_ALL=C ps -o lstart= -p $$ | tr -s ' ' | sed 's/^ //; s/ $//')"
jq -n --argjson pid "$$" --arg s "$SELF_START" '{pid: $pid, start: $s}' > "$LD/.lock/owner.json"
C unlock lk --if-stale >/dev/null 2>&1; [ $? -ne 0 ] && [ -d "$LD/.lock" ]; check "lock: a live pid with its real start time is kept" $?
rm -f "$LD/.lock/owner.json"; rmdir "$LD/.lock"
# a live owner is kept whatever time zone the writer and the unlocker run in
set -m; TZ=Asia/Kolkata "$BASH_BIN" "$HOLD_SCRIPT" pass f1 --campaign lk --revoke >/dev/null 2>&1 & HP=$!; set +m
i=0; while [ "$(jq -r .pid "$LD/.lock/owner.json" 2>/dev/null)" != "$HP" ] && [ "$i" -lt 50 ]; do sleep 0.1; i=$((i+1)); done
TZ=America/Los_Angeles C unlock lk --if-stale >/dev/null 2>&1; [ $? -ne 0 ] && [ -d "$LD/.lock" ]; check "lock: a live owner is kept when the unlocker runs in another time zone" $?
TZ=Pacific/Auckland C unlock lk --if-stale >/dev/null 2>&1; [ $? -ne 0 ] && [ -d "$LD/.lock" ]; check "lock: a live owner is kept in a third time zone" $?
kill_holder; C unlock lk --if-stale >/dev/null 2>&1
# no owner file: never removed
mkdir "$LD/.lock"
C unlock lk --if-stale >/dev/null 2>&1; [ $? -ne 0 ] && [ -d "$LD/.lock" ]; check "lock: a lock with no owner file is kept" $?
printf 'not json' > "$LD/.lock/owner.json"
C unlock lk --if-stale >/dev/null 2>&1; [ $? -ne 0 ] && [ -d "$LD/.lock" ]; check "lock: an unreadable owner file keeps the lock" $?
rm -f "$LD/.lock/owner.json"; rmdir "$LD/.lock"
C unlock lk >/dev/null 2>&1; [ $? -ne 0 ]; check "lock: unlock without --if-stale is refused" $?
C unlock "../x" --if-stale >/dev/null 2>&1; [ $? -ne 0 ]; check "lock: unlock refuses an invalid slug" $?
C unlock lk --if-stale >/dev/null 2>&1; check "lock: unlock with no lock held is a no-op success" $?

# interleave: unlocker A deletes a dead owner's lock and pauses (still inside reclamation); a writer then takes the
# lock; unlocker B, which saw the same dead owner, must not delete the writer's lock.
SLOW_UNLOCK="$WORK/slow-unlock.sh"
awk '{print} /^  release_lock \|\| true/ {print "  sleep 3"}' "$SCRIPT" > "$SLOW_UNLOCK"
grep -q '^  sleep 3' "$SLOW_UNLOCK"; check "race: the slow-unlock copy was built" $?
mkdir "$LD/.lock"; jq -n '{pid: 999999, start: "Mon Jan  1 00:00:00 1990"}' > "$LD/.lock/owner.json"
"$BASH_BIN" "$SLOW_UNLOCK" unlock lk --if-stale >/dev/null 2>&1 & UA=$!
i=0; while [ -e "$LD/.lock" ] && [ "$i" -lt 50 ]; do sleep 0.1; i=$((i+1)); done
[ ! -e "$LD/.lock" ]; check "race: unlocker A removed the dead owner's lock" $?
hold_lock
[ -f "$LD/.lock/owner.json" ] && [ "$(jq -r .pid "$LD/.lock/owner.json")" = "$HP" ]; check "race: a writer took the released lock" $?
C unlock lk --if-stale >/dev/null 2>&1; [ $? -ne 0 ]; check "race: unlocker B refuses (the lock is a live writer's)" $?
wait "$UA" 2>/dev/null
[ -d "$LD/.lock" ] && [ "$(jq -r .pid "$LD/.lock/owner.json")" = "$HP" ]; check "race: the writer's lock and owner record are intact" $?
kill_holder; C unlock lk --if-stale >/dev/null 2>&1
# handoff: unlocker U reads writer A's record and pauses; A finishes; writer B wants the lock; U resumes. U must not
# delete B's lock (A's release waits for U, so the lock generation U checked stays put until U is done).
SHORT_A="$WORK/short-a.sh"; PAUSE_U="$WORK/pause-u.sh"
awk '{print} /^  jq -n --argjson pid/ {print "  sleep 1"}' "$SCRIPT" > "$SHORT_A"
awk '{print} /^  pid="\$\(jq -r/ {print "  sleep 3"}' "$SCRIPT" > "$PAUSE_U"
grep -q '^  sleep 1' "$SHORT_A" && grep -q '^  sleep 3' "$PAUSE_U"; check "handoff: the test copies were built" $?
"$BASH_BIN" "$SHORT_A" pass f1 --campaign lk >/dev/null 2>&1 & AP=$!
i=0; while [ ! -f "$LD/.lock/owner.json" ] && [ "$i" -lt 50 ]; do sleep 0.1; i=$((i+1)); done
"$BASH_BIN" "$PAUSE_U" unlock lk --if-stale >/dev/null 2>&1 & UP=$!
sleep 0.5
set -m; "$BASH_BIN" "$HOLD_SCRIPT" pass f1 --campaign lk --revoke >/dev/null 2>&1 & HP=$!; set +m
wait "$UP"; URC=$?
wait "$AP" 2>/dev/null
i=0; while [ "$(jq -r .pid "$LD/.lock/owner.json" 2>/dev/null)" != "$HP" ] && [ "$i" -lt 100 ]; do sleep 0.1; i=$((i+1)); done
[ "$URC" -ne 0 ]; check "handoff: the unlocker refused (it never saw A as gone)" $?
[ -d "$LD/.lock" ] && [ "$(jq -r .pid "$LD/.lock/owner.json")" = "$HP" ]; check "handoff: writer B holds an intact lock" $?
kill_holder; C unlock lk --if-stale >/dev/null 2>&1
[ ! -e "$LD/.lock" ]; check "handoff: nothing is left behind" $?
# a reclaim-flock holder killed with SIGKILL must not strand the campaign: the next unlock recovers on its own
PYH="$WORK/flock-holder.py"
cat > "$PYH" <<'PYEOF'
import fcntl, os, sys, time
fd = os.open(sys.argv[1] + "/.lock.reclaim", os.O_RDWR | os.O_CREAT, 0o600)
fcntl.flock(fd, fcntl.LOCK_EX)
open(sys.argv[2], "w").write("held")
time.sleep(60)
PYEOF
mkdir "$LD/.lock"; jq -n '{pid: 999999, start: "x"}' > "$LD/.lock/owner.json"
rm -f "$WORK/held"
set -m; /usr/bin/python3 -I "$PYH" "$LD" "$WORK/held" >/dev/null 2>&1 & FH=$!; set +m
i=0; while [ ! -f "$WORK/held" ] && [ "$i" -lt 50 ]; do sleep 0.1; i=$((i+1)); done
[ -f "$WORK/held" ]; check "mutex: a flock holder is running" $?
kill -9 -- "-$FH" 2>/dev/null; wait "$FH" 2>/dev/null
[ -e "$LD/.lock.reclaim" ]; check "mutex: the killed holder left its flock file behind" $?
C unlock lk --if-stale >/dev/null 2>&1; [ ! -e "$LD/.lock" ]; check "mutex: unlock recovers after the holder was SIGKILLed, no manual cleanup" $?
C pass f1 --campaign lk >/dev/null 2>&1 && [ ! -e "$LD/.lock" ]; check "mutex: a later writer still releases its lock normally" $?

echo "----------------------------------------"
echo "campaign: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
