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

# --- verify --print-report: verify and emit the report in one process, from one private snapshot ---
C create rp --feature "ac-1|x" >/dev/null 2>&1; PD="$CD/rp"
C report --campaign rp --bytes "$NB" --check "$SHA" --base64 "$B64" >/dev/null
printf '%s\n' "$REP" > "$WORK/expect-report"
C verify rp --print-report > "$WORK/out" 2>/dev/null; rc=$?
[ "$rc" -eq 0 ] && cmp -s "$WORK/out" "$WORK/expect-report"; check "verify --print-report prints exactly the signed report bytes" $?
[ "$(C verify rp)" = "rp: signature ok" ]; check "plain verify is unchanged (no report bytes)" $?
C verify rep3 --print-report > "$WORK/out" 2>/dev/null; [ $? -eq 4 ] && [ ! -s "$WORK/out" ]; check "a campaign with no recorded report exits 4 and prints nothing" $?
printf 'forged\n' > "$CD/rep3/report.md"
C verify rep3 --print-report > "$WORK/out" 2>/dev/null; [ $? -eq 4 ] && [ ! -s "$WORK/out" ]; check "a planted report.md with no recorded hash exits 4 and prints nothing" $?
rm -f "$CD/rep3/report.md"
cp "$PD/report.md" "$WORK/good-report"; printf 'tampered\n' >> "$PD/report.md"
C verify rp --print-report > "$WORK/out" 2>/dev/null; [ $? -eq 3 ] && [ ! -s "$WORK/out" ]; check "an edited report exits 3 and prints nothing" $?
rm -f "$PD/report.md"; ln -s "$WORK/good-report" "$PD/report.md"
C verify rp --print-report > "$WORK/out" 2>/dev/null; [ $? -eq 3 ] && [ ! -s "$WORK/out" ]; check "a report.md swapped for a symlink exits 3 and prints nothing" $?
rm -f "$PD/report.md"; cp "$WORK/good-report" "$PD/report.md"
C verify rp --print-report >/dev/null 2>&1; check "restoring the exact bytes verifies again" $?
cp "$PD/campaign.json" "$WORK/cj"; jq '.goal = "other"' "$WORK/cj" > "$PD/campaign.json"
C verify rp --print-report > "$WORK/out" 2>/dev/null; [ $? -eq 3 ] && [ ! -s "$WORK/out" ]; check "an edited campaign.json exits 3 and prints nothing" $?
cp "$WORK/cj" "$PD/campaign.json"
C verify rp --bogus >/dev/null 2>&1; [ $? -ne 0 ]; check "verify refuses an unknown option" $?
[ -z "$(ls -A "${TMPDIR:-/tmp}" 2>/dev/null | grep '^vajra-verify\.')" ]; check "the private snapshot is removed" $?

# printed bytes are exactly the hashed bytes: trailing newlines kept, a NUL byte fails closed, a swap cannot be printed
save_report() { # slug text-via-printf-format
  local slug="$1" fmt="$2" b n
  C create "$slug" --feature "ac-1|x" >/dev/null 2>&1
  b="$(printf "$fmt" | openssl base64 -A)"; n="$(printf "$fmt" | wc -c | tr -d ' ')"
  C report --campaign "$slug" --bytes "$n" --check "$(fnv "$b")" --base64 "$b" >/dev/null 2>&1
}
save_report rq1 'no trailing newline'; printf 'no trailing newline' > "$WORK/exp1"
C verify rq1 --print-report > "$WORK/out" 2>/dev/null; [ $? -eq 0 ] && cmp -s "$WORK/out" "$WORK/exp1"; check "print-report keeps a report with no trailing newline exactly" $?
save_report rq2 'two blank lines\n\n\n'; printf 'two blank lines\n\n\n' > "$WORK/exp2"
C verify rq2 --print-report > "$WORK/out" 2>/dev/null; [ $? -eq 0 ] && cmp -s "$WORK/out" "$WORK/exp2"; check "print-report keeps trailing blank lines exactly" $?
save_report rq3 'a\0b'
C verify rq3 --print-report > "$WORK/out" 2>/dev/null; [ $? -eq 3 ] && [ ! -s "$WORK/out" ]; check "a report with a NUL byte fails closed (exit 3, nothing printed)" $?
# a same-user swap of the snapshot's report.md right before the report is read must never reach the output
mkdir -p "$WORK/shim"; REALCAT="$(command -v cat)"
cat > "$WORK/shim/cat" <<SHIM
#!/bin/bash
for a in "\$@"; do case "\$a" in */vajra-verify.*/report.md) printf 'FORGED' > "\$a" ;; esac; done
exec "$REALCAT" "\$@"
SHIM
chmod +x "$WORK/shim/cat"
PATH="$WORK/shim:$PATH" C verify rq1 --print-report > "$WORK/out" 2>/dev/null; rc=$?
! grep -q FORGED "$WORK/out"; check "a report.md swapped in the snapshot is never printed (rc=$rc)" $?
rm -rf "$WORK/shim"
# TOCTOU on the metadata: a signed campaign with NO recorded report and a planted report.md. If campaign.json in the
# snapshot is swapped (here: gains a matching reportSha256) as the hash is first read, nothing downstream may trust it:
# the recorded hash comes from the bytes that were copied once, so the planted text must never be printed.
C create tu --feature "ac-1|x" >/dev/null 2>&1; printf 'PLANTED\n' > "$CD/tu/report.md"
mkdir -p "$WORK/shim"; REALJQ="$(command -v jq)"; PLANTED_SHA="$(printf 'PLANTED\n' | openssl dgst -sha256 -r | awk '{print $1}')"
cat > "$WORK/shim/jq" <<SHIM
#!/bin/bash
hit=0; file=""
for a in "\$@"; do [ "\$a" = '.reportSha256 // ""' ] && hit=1; case "\$a" in */vajra-verify.*/campaign.json) file="\$a" ;; esac; done
if [ "\$hit" = 1 ] && [ -n "\$file" ] && [ ! -e "$WORK/jq-swapped" ]; then
  : > "$WORK/jq-swapped"; "$REALJQ" --arg h "$PLANTED_SHA" '.reportSha256 = \$h' "\$file" > "\$file.n" && mv "\$file.n" "\$file"
fi
exec "$REALJQ" "\$@"
SHIM
chmod +x "$WORK/shim/jq"
PATH="$WORK/shim:$PATH" C verify tu --print-report > "$WORK/out" 2>/dev/null; rc=$?
[ -e "$WORK/jq-swapped" ]; check "the swap hook fired (test is live)" $?
[ "$rc" -ne 0 ] && ! grep -q PLANTED "$WORK/out"; check "campaign.json swapped after the copy cannot authorize a planted report (rc=$rc)" $?
rm -rf "$WORK/shim" "$WORK/jq-swapped" "$CD/tu"

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

echo "----------------------------------------"
echo "campaign: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
