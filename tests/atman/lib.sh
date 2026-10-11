# Shared helpers for tests/atman/run-*.sh (sourced, not run).
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOOK="$REPO/hooks/post-tool-use.sh"
ATMAN="$REPO/scripts/atman.sh"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-atman-XXXXXX")"
trap 'rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"
unset VAJRA_ATMAN_ASSUME_HUMAN VAJRA_ATMAN_MAX_ENTRIES
VDIR="$HOME/.claude/vajra"
LOG="$VDIR/practice-log.jsonl"
RLOG="$VDIR/routing-log.jsonl"
mkdir -p "$VDIR"
KEY="$(openssl rand -hex 32)"
printf '%s' "$KEY" > "$VDIR/.hmac-key"; chmod 600 "$VDIR/.hmac-key"

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }
summary() {
  echo "----------------------------------------"
  echo "$1: ${PASS} passed, ${FAIL} failed"
  [ "$FAIL" -eq 0 ]
}

# hook <session> <event> <tool> [tool_input-json] [extra-json-fields]
hook() {
  local s="$1" ev="$2" tool="$3" in="${4:-}" extra="${5:-}"
  [ -z "$in" ] && in='{}'
  [ -n "$extra" ] && extra=",$extra"
  printf '{"session_id":"%s","transcript_path":"/tmp/t.jsonl","cwd":"%s","hook_event_name":"%s","tool_name":"%s","tool_input":%s,"tool_response":{}%s}' \
    "$s" "$TEST_HOME" "$ev" "$tool" "$in" "$extra" | bash "$HOOK"
}
skill_call() { hook "$1" PostToolUse Skill "{\"skill\":\"$2\"}"; }
bash_fail() { hook "$1" PostToolUseFailure Bash '{"command":"npm test"}' "\"error\":\"$2\"${3:+,\"agent_type\":\"$3\"}"; }

# route_log <session> <router-skill|null> <word>...   (SUGGESTED='["a","b"]' adds the suggested field)
route_log() {
  local s="$1" rs="$2"; shift 2
  local words="" w sug=""
  for w in "$@"; do words="${words:+$words,}\"$w\""; done
  [ "$rs" = "null" ] || rs="\"$rs\""
  [ -n "${SUGGESTED:-}" ] && sug=",\"suggested\":$SUGGESTED"
  printf '{"ts":"%s","session_id":"%s","prompt_sha256":"%s","words":[%s],"route":{"tier":1,"skill":%s,"agent":null,"pattern":null}%s}\n' \
    "$(date -u -v-2S +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d '2 seconds ago' +%Y-%m-%dT%H:%M:%SZ)" \
    "$s" "$(printf '%s' "$s$*" | openssl dgst -sha256 | awk '{print $NF}')" "$words" "$rs" "$sug" >> "$RLOG"
}
proposal_ids() { ls "$VDIR/proposals" 2>/dev/null | sed 's/\.json$//'; }
prop_field() { jq -r "$2" "$VDIR/proposals/$1.json"; }
