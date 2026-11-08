#!/usr/bin/env bash
# NFR-SEC-2: the scripts that hold the HMAC key (scripts/vajra-campaign.sh, scripts/vajra-replay.sh, hooks/session-start.sh,
# hooks/pre-compact.sh) call openssl only at /usr/bin/openssl and pin PATH to the system dirs first,
# so an openssl/jq/awk/tr/mktemp a same-uid worker plants in ~/.local/bin (or anywhere early on the
# caller's PATH) never runs and never sees the key.
#
# Each planted tool only appends its own name to a marker file and exits 97. It never records its
# arguments, so even a failure here cannot copy the key anywhere. The test never reads the key.
#
# Run: bash tests/hooks/run-openssl-pin.sh

set -uo pipefail
[ -x /usr/bin/openssl ] || { echo "SKIP: /usr/bin/openssl required"; exit 0; }
for t in jq git; do command -v "$t" >/dev/null || { echo "SKIP: $t required"; exit 0; }; done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-pin-XXXXXX")"
trap 'chmod -R u+w "$TEST_HOME" 2>/dev/null; rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"
VS="$HOME/.claude/vajra"
BASH_BIN=/bin/bash

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); echo "FAIL :: $1"; fi; }

# The planted tools, in ~/.local/bin, the first dir on the PATH every script is started with.
PLANT="$HOME/.local/bin"; MARK="$TEST_HOME/planted-ran"
mkdir -p "$PLANT"
for t in openssl jq awk tr mktemp; do
  printf '#!/bin/sh\necho %s >> "%s"\nexit 97\n' "$t" "$MARK" > "$PLANT/$t"
  chmod 755 "$PLANT/$t"
done
HOSTILE_PATH="$PLANT:$PATH"
# Runs a script the way a worker-controlled caller would: its PATH starts with the planted dir.
hostile() { env PATH="$HOSTILE_PATH" "$BASH_BIN" "$@"; }
no_plant_ran() { [ ! -e "$MARK" ]; }

# 0. The plants work: a bare `openssl` on that PATH is the planted one.
env PATH="$HOSTILE_PATH" /bin/sh -c 'openssl version' >/dev/null 2>&1
[ -s "$MARK" ] && grep -qx openssl "$MARK"; check "sanity: a bare openssl on the hostile PATH runs the plant" $?
rm -f "$MARK"

# 1. vajra-campaign.sh: creates the key, signs, verifies, passes a feature, all on the hostile PATH.
CS="$REPO/scripts/vajra-campaign.sh"
hostile "$CS" create pin-check --goal "pin" --feature "ac-1|one" --feature "ac-2|two" >/dev/null 2>&1
check "campaign create succeeds" $?
[ -s "$VS/.hmac-key" ]; check "campaign create made the key" $?
out="$(hostile "$CS" verify pin-check 2>&1)"
[ "$out" = "pin-check: signature ok" ]; check "campaign verify: signature ok" $?
hostile "$CS" pass ac-1 --campaign pin-check --evidence "pin test" >/dev/null 2>&1
check "campaign pass succeeds (verify + re-sign)" $?
hostile "$CS" verify pin-check >/dev/null 2>&1; check "campaign still verifies after pass" $?
hostile "$CS" show pin-check >/dev/null 2>&1; check "campaign show succeeds" $?
no_plant_ran; check "campaign: no planted tool ran" $?
# The signature is real: a tampered state fails verify (so verify is not trivially passing).
jq '.features[1].passes = true' "$VS/campaigns/pin-check/features.json" > "$TEST_HOME/f.tmp"
cp "$TEST_HOME/f.tmp" "$VS/campaigns/pin-check/features.json"
hostile "$CS" verify pin-check >/dev/null 2>&1; [ $? -ne 0 ]; check "campaign verify fails on tampered state" $?
no_plant_ran; check "campaign (tampered): no planted tool ran" $?

# 2. hooks/session-start.sh: verifies the manifest signature with the key.
SKILL="$HOME/.claude/skills/vajra"; FLAG="$VS/.integrity-violation"
mkdir -p "$SKILL/hooks" "$SKILL/scripts" "$SKILL/config"
cp "$REPO/hooks/session-start.sh" "$SKILL/hooks/"
cp "$REPO/scripts/generate-manifest.sh" "$SKILL/scripts/"
echo '{"a":1}' > "$SKILL/config/default.json"
printf '#!/usr/bin/env bash\nexit 0\n' > "$SKILL/scripts/vajra-campaign.sh"
# The operator signs from a clean terminal (the normal PATH, not the hostile one).
VAJRA_OPERATOR_SIGN=1 "$BASH_BIN" "$SKILL/scripts/generate-manifest.sh" >/dev/null 2>&1
[ -s "$VS/manifest.sig" ]; check "setup: manifest signed" $?
out="$(printf '%s' '{"session_id":"s","source":"startup"}' | hostile "$SKILL/hooks/session-start.sh")"; rc=$?
[ "$rc" -eq 0 ] && [ ! -f "$FLAG" ]; check "session-start: clean signed tree, no flag" $?
printf '%s' "$out" | grep -qE "INTEGRITY|not set up|cannot verify"; [ $? -ne 0 ]; check "session-start: signature checked, no warning" $?
no_plant_ran; check "session-start: no planted tool ran" $?
echo "# change" >> "$SKILL/config/default.json"
printf '%s' '{"session_id":"s","source":"startup"}' | hostile "$SKILL/hooks/session-start.sh" >/dev/null
[ -f "$FLAG" ] && grep -q "config/default.json" "$FLAG"; check "session-start: a modified file is still flagged" $?
no_plant_ran; check "session-start (modified): no planted tool ran" $?

# 3. hooks/pre-compact.sh: HMAC-signs the checkpoint event with the key.
W="$TEST_HOME/work"; mkdir -p "$W"
git -C "$W" init -q; git -C "$W" config user.email t@t; git -C "$W" config user.name t
echo a > "$W/a.txt"; git -C "$W" add a.txt; git -C "$W" commit -qm init
printf '%s' "{\"session_id\":\"pc1\",\"cwd\":\"$W\",\"hook_event_name\":\"PreCompact\",\"trigger\":\"auto\"}" \
  | (cd / && hostile "$REPO/hooks/pre-compact.sh") >/dev/null; rc=$?
[ "$rc" -eq 0 ]; check "pre-compact: exit 0" $?
EV="$VS/events/pc1.jsonl"
[ -s "$EV" ] && tail -n 1 "$EV" | jq -e '.kind == "checkpoint" and (.hmac | test("^[0-9a-f]{64}$"))' >/dev/null
check "pre-compact: checkpoint event carries a 64-hex hmac" $?
no_plant_ran; check "pre-compact: no planted tool ran" $?

# 4. scripts/vajra-replay.sh: re-checks every event HMAC with the key.
RS="$REPO/scripts/vajra-replay.sh"
out="$(hostile "$RS" pc1 2>&1)"; rc=$?
[ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q "chain INTACT, all HMACs valid"; check "replay: the signed checkpoint verifies" $?
no_plant_ran; check "replay: no planted tool ran" $?
# The check is real: a changed event fails it.
jq -c '.gitRef = "refs/vajra/compact/forged"' "$EV" > "$VS/events/pc2.jsonl"
out="$(hostile "$RS" pc2 2>&1)"; rc=$?
[ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "HMAC-BAD"; check "replay: a changed event is HMAC-BAD" $?
no_plant_ran; check "replay (changed): no planted tool ran" $?

echo "----------------------------------------"
echo "openssl-pin: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
