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
trap 'rm -rf "$WORK"' EXIT
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
[ "$(stat -f %Lp "$HOME/.claude/vajra/.hmac-key" 2>/dev/null || stat -c %a "$HOME/.claude/vajra/.hmac-key")" = "600" ]; check "key created mode 600" $?
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

echo "----------------------------------------"
echo "campaign: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
