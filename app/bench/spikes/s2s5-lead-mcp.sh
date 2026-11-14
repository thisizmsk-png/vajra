#!/usr/bin/env bash
# Spikes S2 and S5 (ADR-16 16.12), for 6d2.
#
#   S2: under `--sandbox read-only`, can a Codex lead still call its `vajra_run` MCP tools and read the project, while
#       it cannot write its lead dir? Do N3's `-c mcp_servers={...only vajra_run...}` and `-c notify=[]` switch off the
#       user's own servers and notify?
#   S5: does each harness's MCP tool-call timeout let a `wait_s` = 110 `await_any` come back?
#
#   bash app/bench/spikes/s2s5-lead-mcp.sh codex  [wait_s]   # S2 + S5 for Codex, with tool_timeout_sec=130
#   bash app/bench/spikes/s2s5-lead-mcp.sh codex-short-timeout [wait_s]  # S5 control: tool_timeout_sec=20 must cut a longer wait
#   bash app/bench/spikes/s2s5-lead-mcp.sh claude [wait_s]   # S5 for Claude Code, lead shape (--strict-mcp-config)
#
# Everything lives in a new mktemp dir: a temp HOME, CODEX_HOME and CLAUDE_CONFIG_DIR (never the real ~/.codex or
# ~/.claude), a project dir, a lead dir, a fake `vajra_run` MCP server (s2s5_mcp.py) and a loopback model stub
# (s2s5_stub.py). Key values are placeholders the stub ignores, and nothing leaves 127.0.0.1. No settings file is
# written for Claude: the S5 question is the MCP timeout alone, so only an `--mcp-config` file is used.
# The verdict is printed by s2s5_check.py.
set -u

MODE=${1:-codex}
WAIT=${2:-110}
HERE=$(cd "$(dirname "$0")" && pwd -P) || exit 2
for f in s2s5_stub.py s2s5_mcp.py s2s5_check.py; do [ -f "$HERE/$f" ] || { echo "FAIL: $f is not next to this script"; exit 2; }; done
case "$MODE" in codex|codex-short-timeout) BIN=$(command -v codex || true) ;; claude) BIN=$(command -v claude || true) ;;
  *) echo "FAIL: mode is codex, codex-short-timeout or claude"; exit 2 ;; esac
[ -n "$BIN" ] && [ -x "$BIN" ] || { echo "FAIL: no $MODE binary on PATH"; exit 2; }
"$BIN" --version 2>/dev/null | head -1 | sed "s/^/$MODE: /"

SP=$(mktemp -d "${TMPDIR:-/tmp}/vajra-s2s5.XXXXXX") || exit 2
SP=$(cd "$SP" && pwd -P)
H=$SP/home; PROJ=$SP/proj; LEAD=$SP/data/runs/r1/lead; M=$SP/m
mkdir -p "$H" "$PROJ/src" "$LEAD" "$M" "$SP/stub"
STUB=""
cleanup() { if [ -n "$STUB" ]; then kill "$STUB" 2>/dev/null; fi; }
trap cleanup EXIT
echo "spike dir: $SP"

echo "s2-project-marker" > "$PROJ/src/marker.txt"
echo "the brief" > "$LEAD/brief.md"
chmod 0555 "$LEAD"
TOKEN_FILE=$SP/data/runs/r1/token
( umask 077; echo "spike-token-not-a-secret" > "$TOKEN_FILE" )

# A user MCP server and notify that N3 must switch off: each leaves a marker if it ever runs.
cat > "$SP/user-mcp.sh" <<EOF
#!/bin/sh
echo "\$1" >> "$M/user-mcp-ran"
exit 1
EOF
cat > "$SP/user-notify.sh" <<EOF
#!/bin/sh
echo notified >> "$M/user-notify-ran"
EOF
chmod 755 "$SP/user-mcp.sh" "$SP/user-notify.sh"

/usr/bin/python3 -I - "$PROJ" "$LEAD" "$WAIT" "$MODE" > "$SP/stub/plan.json" <<'PY'
import json, sys
proj, lead, wait, mode = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
if mode == "claude":
    plan = [{"read": proj + "/src/marker.txt"}, {"mcp": "probe"}, {"mcp": "await_any", "args": {"wait_s": wait}}, {"say": "done"}]
else:
    plan = [{"shell": "cat " + proj + "/src/marker.txt"}, {"shell": "echo x > " + lead + "/lead-write && echo WROTE"},
            {"shell": "echo x > " + proj + "/src/proj-write && echo WROTE"}, {"mcp": "probe"},
            {"mcp": "await_any", "args": {"wait_s": wait}}, {"say": "done"}]
print(json.dumps(plan))
PY

/usr/bin/python3 -I "$HERE/s2s5_stub.py" "$SP/stub/plan.json" "$SP/stub/log" "$SP/stub/port" > "$SP/stub/out" 2>&1 &
STUB=$!
for _ in $(seq 1 50); do [ -s "$SP/stub/port" ] && break; sleep 0.1; done
PORT=$(cat "$SP/stub/port" 2>/dev/null)
[ -n "$PORT" ] || { echo "FAIL: the stub did not start"; exit 2; }

MCP_ARGS="[\"-I\",\"$HERE/s2s5_mcp.py\",\"$M/mcp-log\"]"
MCP_ENV="{VAJRA_RUN=\"r1\",VAJRA_RUN_TOKEN_FILE=\"$TOKEN_FILE\"}"
START=$(date +%s)
if [ "$MODE" = claude ]; then
  CC=$H/.claude; mkdir -p "$CC"
  /usr/bin/python3 -I - "$HERE/s2s5_mcp.py" "$M/mcp-log" "$TOKEN_FILE" > "$SP/data/runs/r1/vajra-run.json" <<'PY'
import json, sys
print(json.dumps({"mcpServers": {"vajra_run": {"type": "stdio", "command": "/usr/bin/python3", "args": ["-I", sys.argv[1], sys.argv[2]],
                                              "env": {"VAJRA_RUN": "r1", "VAJRA_RUN_TOKEN_FILE": sys.argv[3]}}}}))
PY
  TOOLS=Read,Grep,Glob
  ALLOWED=Read,Grep,Glob,mcp__vajra_run__probe,mcp__vajra_run__await_any
  ( cd "$LEAD" && env -i PATH="/usr/bin:/bin:$(dirname "$BIN")" HOME="$H" CLAUDE_CONFIG_DIR="$CC" TERM=dumb \
      ANTHROPIC_BASE_URL="http://127.0.0.1:$PORT" ANTHROPIC_API_KEY=s5-placeholder-not-a-key DISABLE_AUTOUPDATER=1 \
      perl -e 'alarm 300; exec @ARGV' "$BIN" -p --output-format stream-json --verbose \
      --session-id 5e5e5e5e-1b2c-4d5e-8f90-a1b2c3d4e5f6 --setting-sources "" --permission-mode dontAsk \
      --tools "$TOOLS" --allowedTools "$ALLOWED" --mcp-config "$SP/data/runs/r1/vajra-run.json" --strict-mcp-config \
      -- "Run the S5 plan" < /dev/null > "$SP/events.jsonl" 2> "$SP/stderr" )
  STATUS=$?
else
  CH=$SP/codex-home; mkdir -p "$CH"
  cat > "$CH/config.toml" <<EOF
model_provider = "stub"
notify = ["$SP/user-notify.sh"]

[model_providers.stub]
name = "Stub"
base_url = "http://127.0.0.1:$PORT/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0

[mcp_servers.user_mcp]
command = "$SP/user-mcp.sh"
args = ["user_mcp"]

[mcp_servers.vajra_run]
command = "$SP/user-mcp.sh"
args = ["user_vajra_run"]
EOF
  # N3 plus the S2 fix: only vajra_run, only its tools, pre-approved (read-only + approval_policy=never refuses an
  # MCP call that needs approval).
  TT=",default_tools_approval_mode=\"approve\",enabled_tools=[\"probe\",\"await_any\"]"
  [ "$MODE" = codex ] && TT="$TT,tool_timeout_sec=130"
  [ "$MODE" = codex-short-timeout ] && TT="$TT,tool_timeout_sec=20"
  # N3 variant (env N3, default ignore): table = only the -c mcp_servers table (FAILS: the user's servers still start);
  # disable = also enabled=false per user server name (FAILS the same way); ignore = --ignore-user-config, with the
  # provider passed as -c keys, and the lead's sub-agent and goal tools off (PASSES on codex 0.166).
  N3=${N3:-ignore}
  EXTRA=()
  case "$N3" in
    disable) EXTRA=(-c "mcp_servers.user_mcp.enabled=false") ;;
    ignore) EXTRA=(--ignore-user-config -c features.multi_agent=false -c features.goals=false -c 'model_provider="stub"' -c 'model_providers.stub.name="Stub"'
                   -c "model_providers.stub.base_url=\"http://127.0.0.1:$PORT/v1\"" -c 'model_providers.stub.wire_api="responses"'
                   -c 'model_providers.stub.request_max_retries=0' -c 'model_providers.stub.stream_max_retries=0') ;;
  esac
  echo "N3 variant: $N3"
  ( cd "$LEAD" && env -i PATH="/usr/bin:/bin:$(dirname "$BIN")" HOME="$H" CODEX_HOME="$CH" TERM=dumb \
      perl -e 'alarm 300; exec @ARGV' "$BIN" exec --json --skip-git-repo-check --sandbox read-only -C "$LEAD" -m stub-model \
      -c 'approval_policy="never"' -c 'notify=[]' ${EXTRA[@]+"${EXTRA[@]}"} \
      -c "mcp_servers={vajra_run={command=\"/usr/bin/python3\",args=$MCP_ARGS,env=$MCP_ENV$TT}}" \
      -- "Run the S2 plan" < /dev/null > "$SP/events.jsonl" 2> "$SP/stderr" )
  STATUS=$?
fi
END=$(date +%s)
echo "$MODE exit: $STATUS after $((END - START)) s"
/usr/bin/python3 -I "$HERE/s2s5_check.py" "$MODE" "$SP" "$M" "$WAIT" "$STATUS" "$PROJ" "$LEAD"
