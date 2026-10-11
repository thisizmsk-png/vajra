#!/usr/bin/env bash
# Atman v2: practice-log contract, chain verification, proposals, human approval,
# learned routes, patch files, pause/resume and chain-preserving prune.
# Runs entirely in a temp HOME. Run: bash tests/atman/run-atman.sh
set -uo pipefail
for c in jq openssl python3 git; do
  command -v "$c" >/dev/null || { echo "SKIP: $c required"; exit 0; }
done
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# ---------------------------------------------------------------- 1. contract
hook c1 PostToolUse Read '{"file_path":"/x"}'
skill_call c1 code-review
bash_fail c1 "boom SECRET-TOKEN-123 at /Users/someone/proj/file.js:12" vidura
hook c1 PostToolUseFailure Bash '{"command":"bash scripts/vajra-verify-work.sh --cmd \"npm test\""}' '"error":"exit 1"'
hook c1 PostToolUse Bash '{"command":"ls"}' '"tool_response":{"exit_code":2}'

[ "$(wc -l < "$LOG" | tr -d ' ')" = "5" ]; check "every tool call logged (5 lines)" $?
want='["agent","error_sha256","event","hmac","outcome","prevHmac","seq","session_id","skill","tool","ts"]'
[ "$(sed -n 1p "$LOG" | jq -c 'keys')" = "$want" ]; check "line has exactly the contract fields" $?
[ "$(sed -n 2p "$LOG" | jq -r '.skill')" = "code-review" ]; check "skill = tool_input.skill for Skill calls" $?
[ "$(sed -n 1p "$LOG" | jq -r '.skill')" = "null" ]; check "skill null for non-Skill calls" $?
l3="$(sed -n 3p "$LOG")"
[ "$(echo "$l3" | jq -r '.outcome')" = "failure" ]; check "PostToolUseFailure -> outcome failure" $?
[ "$(echo "$l3" | jq -r '.event')" = "PostToolUseFailure" ]; check "event recorded" $?
[ "$(echo "$l3" | jq -r '.agent')" = "vidura" ]; check "agent = agent_type" $?
[ "$(echo "$l3" | jq -r '.error_sha256 | length')" = "64" ]; check "error stored as sha256" $?
[ "$(sed -n 1p "$LOG" | jq -r '.error_sha256')" = "null" ]; check "no error hash on success" $?
! grep -rq "SECRET-TOKEN" "$VDIR"; check "no raw error text anywhere in state dir" $?
[ "$(sed -n 4p "$LOG" | jq -r '.signal')" = "verify-work" ]; check "verify-work failure carries signal" $?
[ "$(sed -n 5p "$LOG" | jq -r '.outcome')" = "failure" ]; check "non-zero exit_code in tool_response -> failure" $?
[ "$(sed -n 1p "$LOG" | jq -r '.prevHmac')" = "genesis" ]; check "chain starts at genesis" $?
[ "$(sed -n 2p "$LOG" | jq -r '.prevHmac')" = "$(sed -n 1p "$LOG" | jq -r '.hmac')" ]; check "chain links" $?
payload="$(sed -n 3p "$LOG" | jq -c 'del(.hmac)')"
mac="$(printf '%s' "$payload" | openssl dgst -sha256 -mac HMAC -macopt "hexkey:${KEY}" -hex | awk '{print $NF}')"
[ "$mac" = "$(sed -n 3p "$LOG" | jq -r '.hmac')" ]; check "HMAC verifies with openssl over jq -c del(.hmac)" $?
[ -f "$VDIR/events/c1.jsonl" ]; check "event log still written for replay" $?

# Same error with different paths/numbers hashes the same (clustering).
bash_fail c1 "boom SECRET-TOKEN-999 at /tmp/other/place.js:7" vidura
[ "$(sed -n 3p "$LOG" | jq -r '.error_sha256')" = "$(tail -1 "$LOG" | jq -r '.error_sha256')" ]; check "normalized error clusters" $?

# Hostile names: sanitized, never escape the state dir.
hook "../../../etc" PostToolUse Skill '{"skill":"../../evil/$(rm -rf ~)"}' '"agent_type":"../x/y"'
[ "$(tail -1 "$LOG" | jq -r '.skill')" = "evilrm-rf" ]; check "skill name sanitized" $?
[ "$(tail -1 "$LOG" | jq -r '.agent')" = "xy" ]; check "agent name sanitized" $?
[ -f "$VDIR/events/etc.jsonl" ] && [ ! -e "$HOME/etc.jsonl" ]; check "session id cannot traverse out of events/" $?

# Garbage input never fails the hook.
printf 'not json' | bash "$HOOK"; check "garbage stdin exits 0" $?
printf '' | bash "$HOOK"; check "empty stdin exits 0" $?

bash "$ATMAN" verify >/dev/null; check "verify passes on an intact log" $?

# ---------------------------------------------------------- 2. concurrency
before="$(wc -l < "$LOG" | tr -d ' ')"
for i in 1 2 3 4 5 6 7 8 9 10 11 12; do hook "p$i" PostToolUse Read '{}' & done; wait
after="$(wc -l < "$LOG" | tr -d ' ')"
[ "$((after - before))" = "12" ]; check "12 parallel hooks -> 12 lines" $?
bash "$ATMAN" verify >/dev/null; check "chain intact after parallel appends" $?
[ "$(jq -r '.seq' "$LOG" | sort -n | uniq -d | wc -l | tr -d ' ')" = "0" ]; check "seq numbers unique" $?

# ------------------------------------------------------ 3. routing proposals
for s in r1 r2 r3; do route_log "$s" null deploy staging the; skill_call "$s" ci-cd; done
route_log r4 ci-cd deploy; skill_call r4 ci-cd
route_log r5 null docs; skill_call r5 code-review          # one-off: below support
bash "$ATMAN" propose >/dev/null; check "propose exits 0" $?
rid="$(for i in $(proposal_ids); do [ "$(prop_field "$i" '.kind+":"+(.word//"")')" = "route:deploy" ] && echo "$i"; done)"
[ -n "$rid" ]; check "route proposal for 'deploy' -> ci-cd" $?
[ "$(prop_field "$rid" '.skill')" = "ci-cd" ]; check "proposed skill is the one actually used" $?
[ "$(prop_field "$rid" '.evidence.precision == 1')" = "true" ]; check "precision computed on history (4/4)" $?
[ "$(prop_field "$rid" '.evidence.router_misses')" = "3" ]; check "router misses counted" $?
! ls "$VDIR/proposals" | xargs -I{} jq -r '.word // empty' "$VDIR/proposals/{}" | grep -qx 'docs'; check "below-support word not proposed" $?
! ls "$VDIR/proposals" | xargs -I{} jq -r '.word // empty' "$VDIR/proposals/{}" | grep -qx 'the'; check "stopwords never proposed" $?
n1="$(proposal_ids | wc -l | tr -d ' ')"
bash "$ATMAN" propose >/dev/null
[ "$(proposal_ids | wc -l | tr -d ' ')" = "$n1" ]; check "propose is idempotent" $?
bash "$ATMAN" review | grep -q "$rid"; check "review lists the pending proposal" $?

# ---------------------------------------------------------- 4. human gate
bash "$ATMAN" approve "$rid" </dev/null >/dev/null 2>&1; rc=$?
[ "$rc" = "3" ]; check "approve refuses without a human terminal (exit 3)" $?
[ ! -f "$VDIR/learned-routes.json" ]; check "nothing written when refused" $?
bash "$ATMAN" approve "../../etc/passwd" >/dev/null 2>&1; [ $? = 2 ]; check "malformed id rejected" $?

export VAJRA_ATMAN_ASSUME_HUMAN=1
bash "$ATMAN" approve "$rid" >/dev/null; check "approve as human" $?
[ "$(jq -r '.routes[] | select(.word=="deploy") | .skill' "$VDIR/learned-routes.json")" = "ci-cd" ]; check "learned-routes.json has the route" $?
[ "$(prop_field "$rid" '.status')" = "approved" ]; check "proposal marked approved" $?
bash "$ATMAN" approve "$rid" >/dev/null 2>&1; [ $? = 1 ]; check "cannot approve twice" $?
sid="$(for i in $(proposal_ids); do [ "$(prop_field "$i" '.word//""')" = "staging" ] && echo "$i"; done)"
bash "$ATMAN" reject "$sid" "too broad" >/dev/null; check "reject as human" $?
[ "$(prop_field "$sid" '.status')" = "rejected" ]; check "proposal marked rejected" $?
bash "$ATMAN" propose >/dev/null
[ "$(prop_field "$sid" '.status')" = "rejected" ]; check "rejected proposal is not re-opened" $?
bash "$ATMAN" log | grep -q "reject"; check "log shows history" $?

# ------------------------------------------------ 5. skill delta -> patch file
SK="$HOME/.claude/skills/testing"; mkdir -p "$SK"
printf -- '---\nname: testing\ndescription: test things\n---\n\n# Testing\n\n- Write tests first.\n' > "$SK/SKILL.md"
cp "$SK/SKILL.md" "$TEST_HOME/SKILL.orig"
for s in d1 d2; do
  skill_call "$s" testing
  bash_fail "$s" "npm ERR! missing script: test in /a/$s"
  bash_fail "$s" "npm ERR! missing script: test in /b/$s/x"
done
bash "$ATMAN" propose >/dev/null
did="$(for i in $(proposal_ids); do [ "$(prop_field "$i" '.kind+":"+(.target//"")+":"+(.op//"")')" = "skill:testing:add" ] && echo "$i"; done)"
[ -n "$did" ]; check "skill delta 'add' proposed for recurring failure" $?
[ "$(prop_field "$did" '.evidence.failures')" = "4" ]; check "delta carries failure count" $?
[ "$(prop_field "$did" '.evidence.sessions')" = "2" ]; check "delta carries session count" $?
aid="$(for i in $(proposal_ids); do [ "$(prop_field "$i" '.kind+":"+(.target//"")')" = "agent:vidura" ] && echo "$i"; done)"
[ -z "$aid" ] || [ "$(prop_field "$aid" '.evidence.sessions')" -ge 2 ]; check "agent delta respects session gate" $?

bash "$ATMAN" approve "$did" >/dev/null; check "approve skill delta" $?
PATCH="$VDIR/patches/$did.patch"
[ -f "$PATCH" ]; check "patch file written" $?
cmp -s "$SK/SKILL.md" "$TEST_HOME/SKILL.orig"; check "skill file NOT modified by approve" $?
git -C "$SK" apply --check "$PATCH" 2>/dev/null; check "patch applies with git apply" $?
git -C "$SK" apply "$PATCH" && grep -q "atman:$did" "$SK/SKILL.md"; check "applied patch adds tagged bullet" $?

# ------------------------------------------------ 6. retire an ineffective bullet
sleep 1
skill_call d3 testing
i=0; while [ $i -lt 22 ]; do bash_fail d3 "npm ERR! missing script: test in /c/$i"; i=$((i+1)); done
bash "$ATMAN" propose >/dev/null
tid="$(for i in $(proposal_ids); do [ "$(prop_field "$i" '(.op//"")+":"+(.ref//"")')" = "retire:$did" ] && echo "$i"; done)"
[ -n "$tid" ]; check "retire proposed when failure rate did not drop" $?
bash "$ATMAN" approve "$tid" >/dev/null; check "approve retire" $?
git -C "$SK" apply "$VDIR/patches/$tid.patch" 2>/dev/null && cmp -s "$SK/SKILL.md" "$TEST_HOME/SKILL.orig"
check "retire patch removes the bullet" $?

# ------------------------------------------------ 7. never touches config/ or core
CORE="$HOME/.claude/skills/vajra"; mkdir -p "$CORE/config"
echo '{"routing":{}}' > "$CORE/config/default.json"; echo "core" > "$CORE/SKILL.md"
sum_before="$(cat "$CORE/config/default.json" "$CORE/SKILL.md" | openssl dgst -sha256)"
bash "$ATMAN" dream >/dev/null; bash "$ATMAN" propose >/dev/null
[ "$(cat "$CORE/config/default.json" "$CORE/SKILL.md" | openssl dgst -sha256)" = "$sum_before" ]
check "dream/propose/approve never write config/ or the skill core" $?

# ------------------------------------- 7b. agent-made Skill calls label routes
# Under "/vajra <ask>" the skills are called by the deliver workflow's agents.
agent_skill_call() { hook "$1" PostToolUse Skill "{\"skill\":\"$2\"}" "\"agent_type\":\"$3\""; }
word_props() { for i in $(proposal_ids); do [ "$(prop_field "$i" '.kind+":"+(.word//"")')" = "route:$1" ] && echo "$i"; done; }
for s in ag1 ag2 ag3; do route_log "$s" null tailwind; hook "$s" PostToolUse Workflow '{}'; agent_skill_call "$s" frontend-design nakula; done
# Self-reinforcement: agents calling the router's own suggestion never label.
for s in sr1 sr2 sr3; do route_log "$s" api-design graphql; agent_skill_call "$s" api-design bhima; done
# A main-session Skill call wins over an agent-made one for the same prompt.
for s in mp1 mp2 mp3; do route_log "$s" null pagination; agent_skill_call "$s" api-design bhima; skill_call "$s" database-design; done
# The vajra skill itself is never a label, from agents either.
for s in vj1 vj2 vj3; do route_log "$s" null widgetry; agent_skill_call "$s" vajra arjuna; done
bash "$ATMAN" propose >/dev/null
tid="$(word_props tailwind)"
[ -n "$tid" ] && [ "$(prop_field "$tid" '.skill')" = "frontend-design" ]; check "agent-made Skill calls label the routed prompt" $?
[ -n "$tid" ] && [ "$(prop_field "$tid" '.evidence.router_misses')" = "3" ]; check "agent-labeled misses counted" $?
[ -z "$(word_props graphql)" ]; check "agent call of the router's own suggestion is not learned" $?
pid_="$(word_props pagination)"
[ -n "$pid_" ] && [ "$(prop_field "$pid_" '.skill')" = "database-design" ]; check "main-session Skill call wins over agent call" $?
[ -z "$(word_props widgetry)" ]; check "agent vajra calls never label" $?
python3 -I - "$REPO/atman" <<'PY'
import sys; sys.path.insert(0, sys.argv[1]); import atman
r = {"t": 1000.0, "router_skill": None}
calls = [(1000.0 + atman.ROUTE_WINDOW_SECONDS + 60, "late-skill", True)]
assert atman.route_label(r, calls, None) == "late-skill"          # agent window is longer
assert atman.route_label(r, calls, 1100.0) is None                # cut at the next routed prompt
assert atman.route_label(r, [(1000.0 + atman.AGENT_ROUTE_WINDOW_SECONDS + 1, "x", True)], None) is None
assert atman.route_label(r, [(1000.0 + atman.ROUTE_WINDOW_SECONDS + 60, "x", False)], None) is None
rs = {"t": 1000.0, "router_skill": "api-design"}
assert atman.route_label(rs, [(1001.0, "api-design", True)], None) is None   # router's own hint
assert atman.route_label(rs, [(1001.0, "api-design", False)], None) == "api-design"  # main-session hit
PY
check "route_label windows and router-suggestion exclusion" $?

# ------------------------------------------------ 8. pause / resume
bash "$ATMAN" pause >/dev/null
n="$(wc -l < "$LOG")"; hook z1 PostToolUse Read '{}'
[ "$(wc -l < "$LOG")" -gt "$n" ]; check "paused: practice log still records" $?
bash "$ATMAN" status | grep -q PAUSED; check "status shows PAUSED" $?
np="$(proposal_ids | wc -l)"
for s in q1 q2 q3; do route_log "$s" null lint; skill_call "$s" eslint-fix; done
bash "$ATMAN" propose | grep -q paused; check "paused: propose refuses" $?
[ "$(proposal_ids | wc -l)" = "$np" ]; check "paused: no proposals written" $?
bash "$ATMAN" resume >/dev/null
bash "$ATMAN" propose >/dev/null
[ "$(proposal_ids | wc -l)" -gt "$np" ]; check "resumed: proposals again" $?

# ------------------------------------------------ 9. prune keeps chain order
total="$(wc -l < "$LOG" | tr -d ' ')"
tail_before="$(tail -5 "$LOG")"
VAJRA_ATMAN_MAX_ENTRIES=5 bash "$ATMAN" dream >/dev/null
[ "$(wc -l < "$LOG" | tr -d ' ')" = "5" ]; check "pruned to 5 entries" $?
[ "$(tail -5 "$LOG")" = "$tail_before" ]; check "kept the newest entries in original order" $?
[ "$(cat "$VDIR"/practice-archive/*.jsonl | wc -l | tr -d ' ')" = "$((total - 5))" ]; check "pruned entries archived" $?
bash "$ATMAN" verify >/dev/null; check "chain verifies after prune (signed anchor)" $?
hook z2 PostToolUse Read '{}'
bash "$ATMAN" verify >/dev/null; check "chain verifies after prune + append" $?
bash "$ATMAN" status >/dev/null; check "status exits 0" $?
bash "$REPO/scripts/atman-observe.sh" >/dev/null; check "atman-observe.sh wrapper works" $?

summary "atman"
