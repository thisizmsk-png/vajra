#!/usr/bin/env bash
# NFR-SEC-2: the hooks and scripts that hold or reach the HMAC key (every hook, vajra-campaign.sh, vajra-replay.sh,
# vajra-doctor.sh, vajra-verify-work.sh's signing step) call openssl and python3 only at /usr/bin and pin PATH to the root-owned system
# dirs only (never the same-uid-writable Homebrew prefix),
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
for t in openssl jq awk tr mktemp python3 git nc; do
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

# 5. hooks/post-tool-use.sh: atman.py (started as /usr/bin/python3) HMAC-signs a practice-log line with the key.
SK="$HOME/.claude/skills/vajra"
mkdir -p "$SK/atman" "$SK/hooks" "$SK/scripts" "$SK/config" "$VS/kb"
cp "$REPO/atman/atman.py" "$SK/atman/"
cp "$REPO/hooks/post-tool-use.sh" "$REPO/hooks/user-prompt-submit.sh" "$REPO/hooks/pre-tool-use.sh" "$REPO/hooks/desktop-control.sh" "$SK/hooks/"
cp "$REPO/scripts/vajra_route.py" "$REPO/scripts/vajra_kb.py" "$SK/scripts/"; cp "$REPO/config/default.json" "$SK/config/"
printf '%s' '{"tool":"testing","session_id":"pin5"}' | hostile "$SK/hooks/post-tool-use.sh" >/dev/null 2>&1
[ -s "$VS/practice-log.jsonl" ] && tail -n 1 "$VS/practice-log.jsonl" | jq -e '.hmac | test("^[0-9a-f]{64}$")' >/dev/null
check "post-tool-use: atman wrote a signed practice-log line" $?
no_plant_ran; check "post-tool-use: no planted tool ran" $?

# 6. hooks/user-prompt-submit.sh: vajra_route.py mints the user-tier token with the key for `/vajra remember`.
P6="$TEST_HOME/proj6"; mkdir -p "$P6"; git -C "$P6" init -q
out="$(jq -cn --arg d "$P6" '{session_id:"s6",cwd:$d,hook_event_name:"UserPromptSubmit",prompt:"/vajra remember use cursor pagination for list endpoints"}' \
  | hostile "$SK/hooks/user-prompt-submit.sh")"
printf '%s' "$out" | jq -r '.hookSpecificOutput.additionalContext // empty' | grep -qi "saved"; check "user-prompt-submit: the remember was saved (user token verified)" $?
no_plant_ran; check "user-prompt-submit: no planted tool ran" $?

# 7. The guard and the desktop-control hook run outside the guard too: they never start a planted tool.
KEYF="$VS/.hmac-key"
jq -cn --arg f "$KEYF" --arg d "$TEST_HOME" '{tool_name:"Read",tool_input:{file_path:$f},cwd:$d}' | hostile "$SK/hooks/pre-tool-use.sh" >/dev/null 2>&1
[ $? -eq 2 ]; check "pre-tool-use: still blocks a Read of the key" $?
jq -cn --arg f "$TEST_HOME/x" --arg d "$TEST_HOME" '{tool_name:"Read",tool_input:{file_path:$f},cwd:$d}' | hostile "$SK/hooks/pre-tool-use.sh" >/dev/null 2>&1
check "pre-tool-use: still allows an ordinary Read" $?
printf '%s' '{"hook_event_name":"PostToolUse","session_id":"s7","tool_name":"Read"}' | hostile "$SK/hooks/desktop-control.sh" >/dev/null 2>&1
check "desktop-control: exit 0" $?
no_plant_ran; check "pre-tool-use / desktop-control: no planted tool ran" $?

# 8. scripts/vajra-verify-work.sh signs evidence.json with the key. Its test commands keep the caller's PATH (a
# worker's own tools), so only python3 and env are planted here: neither may run at the key boundary (a bare
# `env PATH=... python3` is looked up on the caller's PATH before env applies its PATH). The launcher is /usr/bin/env.
PY_ONLY="$TEST_HOME/py-only"; mkdir -p "$PY_ONLY"; cp "$PLANT/python3" "$PY_ONLY/"
printf '#!/bin/sh\necho env >> "%s"\nexit 97\n' "$MARK" > "$PY_ONLY/env"; chmod 755 "$PY_ONLY/env"
( cd "$TEST_HOME" && VAJRA_WALKTHROUGH_DIR="$TEST_HOME/wt" /usr/bin/env PATH="$PY_ONLY:$PATH" "$BASH_BIN" "$REPO/scripts/vajra-verify-work.sh" pin8 "true" ) >/dev/null 2>&1
jq -e '.hmac | test("^[0-9a-f]{64}$")' "$TEST_HOME/wt/pin8/evidence.json" >/dev/null 2>&1; check "verify-work: evidence.json is signed" $?
no_plant_ran; check "verify-work: neither the planted python3 nor the planted env ran" $?

# 8b. scripts/atman.sh (the Atman CLI) starts atman.py, which reads the key: never a planted python3.
hostile "$REPO/scripts/atman.sh" verify >/dev/null 2>&1; check "atman.sh verify: exit 0" $?
no_plant_ran; check "atman.sh: no planted tool ran" $?

# 8c. A tool the hooks need is missing from the pinned dirs: never a silent break. Copies whose pinned PATH (and python3)
# point at an empty dir stand in for a machine where jq and python3 live only in a user dir.
NOBIN="$TEST_HOME/nobin"; mkdir -p "$NOBIN"
for h in pre-tool-use session-start; do
  sed -e "s#^PATH=\"/usr/bin[^\"]*\"; export PATH\$#PATH=\"$NOBIN\"; export PATH#" -e "s#^PYTHON3=\"/usr/bin/python3\"\$#PYTHON3=\"$NOBIN/python3\"#" \
    "$SK/hooks/$h.sh" > "$SK/hooks/$h-notools.sh"
done
err="$(printf '%s' '{"tool_name":"Read","tool_input":{"file_path":"/tmp/x"},"cwd":"/tmp"}' | /bin/bash "$SK/hooks/pre-tool-use-notools.sh" 2>&1 >/dev/null)"; rc=$?
[ "$rc" -eq 2 ]; check "no jq: the guard fails closed (exit 2)" $?
printf '%s' "$err" | grep -q "every tool call is blocked" && printf '%s' "$err" | grep -q "a jq in /opt/homebrew/bin, /usr/local/bin" \
  && printf '%s' "$err" | grep -q "~/.local/bin, ~/.nix-profile/bin or an asdf/mise shim"; check "no jq: the guard says where jq must be" $?
out="$(printf '%s' '{"session_id":"s","source":"startup"}' | /bin/bash "$SK/hooks/session-start-notools.sh" 2>/dev/null)"
printf '%s' "$out" | grep -q "^Vajra: jq was not found, so the guard blocks every tool call"; check "no jq: session-start says so" $?
printf '%s' "$out" | grep -q "^Vajra: python3 was not found at $NOBIN/python3"; check "no python3: session-start says so" $?

# 8d. The system dirs have no jq (macOS 14) and a same-uid-writable dir that used to be on the pin (the Homebrew prefix)
# holds a planted jq. Copies whose system dirs point at SYS (every system tool except jq) and whose /opt/homebrew/bin
# points at the writable plant dir: the plant never runs, and the guard still decides through /usr/bin/python3.
SYS="$TEST_HOME/sys-nojq"; WPLANT="$TEST_HOME/brew-bin"; mkdir -p "$SYS" "$WPLANT"
for d in /usr/bin /bin /usr/sbin /sbin; do
  for f in "$d"/*; do b="${f##*/}"; [ "$b" = jq ] || [ -e "$SYS/$b" ] || ln -s "$f" "$SYS/$b"; done
done
printf '#!/bin/sh\necho jq >> "%s"\nexit 97\n' "$MARK" > "$WPLANT/jq"; chmod 755 "$WPLANT/jq"
nojq_copy() { # src dst
  sed -e "s#/usr/bin:/bin:/usr/sbin:/sbin#$SYS#g" -e "s#/opt/homebrew/bin#$WPLANT#g" "$1" > "$2"
}
[ ! -e "$SYS/jq" ] && [ -x "$SYS/python3" ]; check "sanity: SYS has python3 and no jq" $?
for h in pre-tool-use session-start pre-compact desktop-control post-tool-use user-prompt-submit; do
  nojq_copy "$REPO/hooks/$h.sh" "$SK/hooks/$h-sysnojq.sh"
done
for sc in vajra-verify-work vajra-replay vajra-doctor vajra-campaign atman; do
  nojq_copy "$REPO/scripts/$sc.sh" "$SK/scripts/$sc-sysnojq.sh"
done
G="$SK/hooks/pre-tool-use-sysnojq.sh"
same=0; n=0
while IFS= read -r p; do
  n=$((n + 1))
  a_err="$(printf '%s' "$p" | /bin/bash "$SK/hooks/pre-tool-use.sh" 2>&1 >/dev/null)"; a_rc=$?
  b_err="$(printf '%s' "$p" | /bin/bash "$G" 2>&1 >/dev/null)"; b_rc=$?
  # A payload jq cannot parse blocks on both paths; only the wording differs (the python path names the missing jq).
  case "$a_err" in *"unparseable hook payload"*) a_err="unparseable"; case "$b_err" in *"jq was not found"*) b_err="unparseable" ;; esac ;; esac
  if [ "$a_rc" = "$b_rc" ] && [ "$a_err" = "$b_err" ]; then same=$((same + 1)); else echo "  differs ($a_rc vs $b_rc): $p"; fi
done <<EOF
{"tool_name":"Read","tool_input":{"file_path":"$TEST_HOME/x"},"cwd":"$TEST_HOME"}
{"tool_name":"Read","tool_input":{"file_path":"$VS/.hmac-key"},"cwd":"$TEST_HOME"}
{"tool_name":"Bash","tool_input":{"command":"cat ~/.claude/vajra/.hmac-key"},"cwd":"$TEST_HOME"}
{"tool_name":"Bash","tool_input":{"command":"echo 'it s' && echo don't; ls"},"cwd":"$TEST_HOME"}
{"tool_name":"Grep","tool_input":{"pattern":"x","path":"$VS"},"cwd":"$TEST_HOME","agent_id":"a1"}
{"tool_name":"Edit","tool_input":{"file_path":"$SK/hooks/pre-tool-use.sh"},"cwd":"$TEST_HOME"}
{"tool_name":"Bash","tool_input":{"command":"echo \$(printf %s x)\nrm -rf /"},"cwd":"$TEST_HOME"}
{"tool_name":"NotebookEdit","tool_input":{"notebook_path":7},"cwd":null}
{"tool_name":"Read","tool_input":"not an object","cwd":"$TEST_HOME"}
{"tool_input":{"file_path":"/tmp/x"}}
[1,2]
not json
EOF
[ "$same" = "$n" ]; check "no system jq: the python3 guard decides every payload as the jq guard does ($same/$n)" $?
jq -cn --arg f "$TEST_HOME/x" --arg d "$TEST_HOME" '{tool_name:"Read",tool_input:{file_path:$f},cwd:$d}' | /bin/bash "$G" >/dev/null 2>&1
check "no system jq: the guard still allows a plain read" $?
jq -cn --arg f "$VS/.hmac-key" --arg d "$TEST_HOME" '{tool_name:"Read",tool_input:{file_path:$f},cwd:$d}' | /bin/bash "$G" >/dev/null 2>&1
[ $? -eq 2 ]; check "no system jq: the guard still blocks a key read" $?
p='{"session_id":"s8d","source":"startup","cwd":"'"$TEST_HOME"'","hook_event_name":"PostToolUse","tool_name":"Read","tool":"testing","prompt":"hi"}'
out="$(printf '%s' "$p" | /bin/bash "$SK/hooks/session-start-sysnojq.sh" 2>/dev/null)"
printf '%s' "$out" | grep -q "^Vajra: jq was not found in /usr/bin, /bin, /usr/sbin or /sbin. The guard parses with /usr/bin/python3"
check "no system jq: session-start says the guard uses python3 and what needs jq" $?
for h in pre-compact desktop-control post-tool-use user-prompt-submit; do
  printf '%s' "$p" | /bin/bash "$SK/hooks/$h-sysnojq.sh" >/dev/null 2>&1
done
( cd "$TEST_HOME" && VAJRA_WALKTHROUGH_DIR="$TEST_HOME/wt" /bin/bash "$SK/scripts/vajra-verify-work-sysnojq.sh" pin8d "true" ) >/dev/null 2>&1
for sc in vajra-replay vajra-doctor atman; do /bin/bash "$SK/scripts/$sc-sysnojq.sh" >/dev/null 2>&1 </dev/null; done
/bin/bash "$SK/scripts/vajra-campaign-sysnojq.sh" verify pin-check >/dev/null 2>&1
no_plant_ran; check "no system jq: the planted jq in the writable dir never ran (hooks and scripts)" $?
rm -f "$MARK"

# 9. One PATH everywhere: every hook is pinned, every pin is the same string of root-owned system dirs (none of them
# writable by this uid), and the bridge's BASE_PATH (#70), when present, starts with it.
WANT="/usr/bin:/bin:/usr/sbin:/sbin"
for h in "$REPO"/hooks/*.sh; do
  grep -qxF "PATH=\"$WANT\"; export PATH" "$h"; check "pinned: ${h##*/}" $?
done
others="$(grep -rhoE '(PATH="|os\.environ\["PATH"\] = ")/usr/bin[^"]*"' "$REPO/hooks" "$REPO/scripts" "$REPO/atman" | sed -E 's/.*"(\/usr\/bin[^"]*)"/\1/' | sort -u)"
[ "$others" = "$WANT" ]; check "every pinned PATH is the same string" $?
nw=0; IFS=: read -r -a pin_dirs <<<"$WANT"; for d in "${pin_dirs[@]}"; do [ -w "$d" ] && nw=1; done
[ "$nw" = 0 ]; check "no pinned dir is writable by this uid" $?
BR="$REPO/app/core/src/vajra/bridge.rs"
if grep -q 'BASE_PATH: \[' "$BR" 2>/dev/null; then
  base="$(grep -E 'BASE_PATH: \[' "$BR" | sed -E 's/.*= \[(.*)\];.*/\1/' | tr -d '" ' | tr ',' ':')"
  case "$base" in "$WANT"|"$WANT":*) true ;; *) false ;; esac; check "bridge::BASE_PATH starts with the pin" $?
fi

echo "----------------------------------------"
echo "openssl-pin: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
