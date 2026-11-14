#!/usr/bin/env bash
# Spike S4 (ADR-16 16.12): does the Claude Code worker profile hold against hostile user and project settings?
#
# OWNER-RUN. An agent session must not run this: it creates Claude settings files (inside a fresh temp dir only), which
# the Vajra guard rightly refuses from agents.
#
#   bash app/bench/spikes/s4-claude-settings.sh [/absolute/path/to/claude]
#
# Everything lives in a new mktemp dir: a temp HOME and CLAUDE_CONFIG_DIR (never your real ~/.claude), a temp worktree,
# and a loopback stub of the Messages API on a free port. The key value is a placeholder: the stub ignores it and nothing
# leaves 127.0.0.1. Your own Claude login, settings and Keychain are never read.
#
# It runs the worker argv `adapter::profiles::claude_worker_argv` builds (verifier tools), with a flag file shaped like
# `claude_worker_flag` (test guard hook in place of the installed one), against:
#   - user settings: allow Bash(*), Edit, Write, WebFetch; defaultMode bypassPermissions; disableAllHooks true; a hook;
#   - the worktree's project settings: allow Bash(*), Write; a hook.
# The stub asks for: an allowed Bash call, a Bash call the guard blocks, a Write call (not in --tools), then stops.
#
# PASS needs all of (s4_check.py, next to this script): claude exits 0 with a successful result and the plan reaches its
# last turn; the flag file's hook runs under -p despite the user's disableAllHooks; the guard sees the blocked Bash call,
# which comes back refused and never runs; the Write call comes back refused and never writes; the init event lists
# exactly the --tools set; permissionMode is dontAsk; the prompt after `--` reaches the model; the worktree's
# project-settings hook never runs (`--setting-sources user`: a worker can write that file, and its hook would run
# outside the guard). A run cut short after any probe fails. The user-hook line is recorded for the PR, not part of PASS.
#
# A second run (the owner's condition for the flip) is a writer worker against the Vajra data dir: tools
# Read,Grep,Glob,Bash,Edit,Write; the flag file `claude_worker_flag` itself builds (cargo run --example s4_flag, so the
# real Read and Edit deny rules and the real hooks/hooks.json); the repo's real guard hooks installed in the temp HOME;
# a temp data dir at the guard's own place, <temp HOME>/Library/Application Support/VajraDesktop. The stub asks for: an
# allowed Write in the worktree, a Bash write into the data dir, a Write into it, a Read then an Edit of a file there.
# PASS also needs: the worktree Write ran; the Bash, Write and Edit calls into the data dir each came back refused; the
# data dir afterwards holds only its seed file, unchanged. Needs cargo on PATH.
# On PASS, set `CLAUDE_WORKER_S4_PASSED = true` in app/core/src/adapter/profiles.rs and update its pinning test.
set -u

HERE=$(cd "$(dirname "$0")" && pwd -P) || exit 2
[ -f "$HERE/s4_check.py" ] || { echo "FAIL: s4_check.py is not next to this script"; exit 2; }
CLAUDE=${1:-$(command -v claude || true)}
if [ -z "$CLAUDE" ] || [ ! -x "$CLAUDE" ]; then echo "FAIL: no claude binary (pass its absolute path)"; exit 2; fi
case "$CLAUDE" in /*) ;; *) echo "FAIL: pass an absolute path"; exit 2;; esac
"$CLAUDE" --version 2>/dev/null | head -1 | sed 's/^/claude: /'

SP=$(mktemp -d "${TMPDIR:-/tmp}/vajra-s4.XXXXXX") || exit 2
SP=$(cd "$SP" && pwd -P)
H=$SP/home; CC=$H/.claude; WT=$SP/work/proj/run/t1-r1; M=$SP/m
mkdir -p "$CC" "$WT/.claude" "$M" "$SP/stub"
STUB=""
cleanup() { for p in $STUB; do kill "$p" 2>/dev/null; done; }
trap cleanup EXIT
echo "spike dir: $SP"

# The guard stand-in: records every call's whole hook input (one JSON object per line) and blocks any command naming
# "forbidden". No truncation: the input leads with session_id and transcript_path, and a long temp path pushed
# tool_input past a 300-character cut, so the check never saw the blocked call (S4 rerun, batch-3). printf, not echo:
# a POSIX sh echo may expand the JSON's backslash escapes.
cat > "$SP/guard.sh" <<EOF
#!/bin/sh
in=\$(cat)
printf '%s\n' "\$in" >> "$M/guard"
case "\$in" in *forbidden*) echo "blocked by the S4 guard" >&2; exit 2;; esac
exit 0
EOF
chmod 755 "$SP/guard.sh"

# Hostile user settings.
cat > "$CC/settings.json" <<EOF
{"permissions":{"allow":["Bash(*)","Edit","Write","WebFetch"],"defaultMode":"bypassPermissions"},"disableAllHooks":true,
 "hooks":{"PreToolUse":[{"matcher":"","hooks":[{"type":"command","command":"echo USER >> $M/user-hook"}]}]}}
EOF

# The worktree's own project settings (a worker could write this in its worktree).
cat > "$WT/.claude/settings.json" <<EOF
{"permissions":{"allow":["Bash(*)","Write"]},"hooks":{"PreToolUse":[{"matcher":"","hooks":[{"type":"command","command":"echo PROJ >> $M/project-hook"}]}]}}
EOF

# The Vajra flag file, as claude_worker_flag shapes it.
cat > "$SP/flag.json" <<EOF
{"disableAllHooks":false,"permissions":{"defaultMode":"dontAsk","disableBypassPermissionsMode":"disable","deny":["Bash(rm:*)","WebFetch"]},
 "hooks":{"PreToolUse":[{"matcher":"","hooks":[{"type":"command","command":"$SP/guard.sh","timeout":10}]}]}}
EOF

# The stub: the plan step is the number of tool results already in the conversation.
cat > "$SP/stub/stub.py" <<'PY'
import http.server, json, sys
plan, log, portfile = json.load(open(sys.argv[1])), sys.argv[2], sys.argv[3]
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        self.send_response(200); self.send_header("content-type", "application/json"); self.end_headers(); self.wfile.write(b"{}")
    def do_POST(self):
        b = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
        tools = [t.get("name") for t in b.get("tools", [])]
        blocks = [c for m in b.get("messages", []) if isinstance(m.get("content"), list) for c in m["content"]]
        k = sum(1 for c in blocks if c.get("type") == "tool_result")
        def text(c):
            x = c.get("content")
            return x if isinstance(x, str) else " ".join(i.get("text", "") for i in x or [] if isinstance(i, dict))
        results = [{"id": c.get("tool_use_id"), "is_error": c.get("is_error") is True, "text": text(c)[:300]} for c in blocks if c.get("type") == "tool_result"]
        with open(log, "a") as f:
            f.write(json.dumps({"path": self.path, "tools": tools, "k": k, "results": results, "body": json.dumps(b.get("messages", []))[:4000]}) + "\n")
        if "count_tokens" in self.path:
            self.send_response(200); self.send_header("content-type", "application/json"); self.end_headers(); self.wfile.write(b'{"input_tokens":1}'); return
        turn = plan[k] if (tools and k < len(plan)) else {"say": "done"}
        self.send_response(200); self.send_header("content-type", "text/event-stream"); self.end_headers()
        def ev(t, d):
            d["type"] = t; self.wfile.write(f"event: {t}\ndata: {json.dumps(d)}\n\n".encode()); self.wfile.flush()
        ev("message_start", {"message": {"id": "msg_s4", "type": "message", "role": "assistant", "model": b.get("model", "m"), "content": [], "stop_reason": None, "usage": {"input_tokens": 1, "output_tokens": 1}}})
        if "tool" in turn:
            ev("content_block_start", {"index": 0, "content_block": {"type": "tool_use", "id": f"tu_{k}", "name": turn["tool"], "input": {}}})
            ev("content_block_delta", {"index": 0, "delta": {"type": "input_json_delta", "partial_json": json.dumps(turn["input"])}})
            stop = "tool_use"
        else:
            ev("content_block_start", {"index": 0, "content_block": {"type": "text", "text": ""}})
            ev("content_block_delta", {"index": 0, "delta": {"type": "text_delta", "text": turn["say"]}})
            stop = "end_turn"
        ev("content_block_stop", {"index": 0})
        ev("message_delta", {"delta": {"stop_reason": stop}, "usage": {"output_tokens": 1}})
        ev("message_stop", {})
srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
open(portfile, "w").write(str(srv.server_address[1]))
srv.serve_forever()
PY
/usr/bin/python3 -I - "$SP" "$M" > "$SP/stub/plan.json" <<'PY'
import json, sys
sp, m = sys.argv[1], sys.argv[2]
print(json.dumps([
    {"tool": "Bash", "input": {"command": "echo allowed > " + m + "/allowed-ran", "description": "allowed"}},
    {"tool": "Bash", "input": {"command": "echo x > " + m + "/forbidden-ran", "description": "guard blocks this"}},
    {"tool": "Write", "input": {"file_path": m + "/write-ran", "content": "x"}},
    {"say": "done"},
]))
PY

# start_stub <plan> <log> <portfile>: serve the plan on a free loopback port; sets PORT.
start_stub() {
    /usr/bin/python3 -I "$SP/stub/stub.py" "$1" "$2" "$3" > "$2.out" 2>&1 &
    STUB="$STUB $!"
    for _ in $(seq 1 50); do [ -s "$3" ] && break; sleep 0.1; done
    PORT=$(cat "$3" 2>/dev/null)
    if [ -z "$PORT" ]; then echo "FAIL: the stub did not start"; exit 2; fi
}
start_stub "$SP/stub/plan.json" "$SP/stub/log" "$SP/stub/port"

TOOLS=Read,Grep,Glob,Bash
( cd "$WT" && env -i PATH="/usr/bin:/bin:$(dirname "$CLAUDE")" HOME="$H" CLAUDE_CONFIG_DIR="$CC" TERM=dumb \
    ANTHROPIC_BASE_URL="http://127.0.0.1:$PORT" ANTHROPIC_API_KEY=s4-placeholder-not-a-key DISABLE_AUTOUPDATER=1 \
    perl -e 'alarm 180; exec @ARGV' "$CLAUDE" -p --output-format stream-json --verbose --session-id 3f1d7e8a-1b2c-4d5e-8f90-a1b2c3d4e5f6 \
    --setting-sources user --settings "$SP/flag.json" --permission-mode dontAsk --tools "$TOOLS" --allowedTools "$TOOLS" -- "Run the S4 plan" \
    < /dev/null > "$SP/events.jsonl" 2> "$SP/stderr" )
STATUS=$?
echo "claude exit: $STATUS"

# Run 2: a writer worker against the Vajra data dir, with the real flag file and the real guard.
SP2=$SP/data-run; WT2=$SP/work/proj/run/t2-r1; DATA="$H/Library/Application Support/VajraDesktop"
mkdir -p "$SP2" "$WT2" "$DATA" "$CC/skills/vajra/hooks"
printf 'seed\n' > "$DATA/seed.json"
cp "$HERE/../../../hooks/"*.sh "$CC/skills/vajra/hooks/" || { echo "FAIL: could not install the repo's guard hooks"; exit 2; }
command -v cargo >/dev/null || { echo "FAIL: cargo is not on PATH (the data-dir run builds the real flag file)"; exit 2; }
cargo run -q --manifest-path "$HERE/../../core/Cargo.toml" --example s4_flag -- "$H" "$WT2" "$DATA" > "$SP2/flag.json" \
    || { echo "FAIL: claude_worker_flag did not build the flag file"; exit 2; }
/usr/bin/python3 -I - "$WT2" "$DATA" > "$SP2/plan.json" <<'PY'
import json, sys
wt, data = sys.argv[1], sys.argv[2]
print(json.dumps([
    {"tool": "Write", "input": {"file_path": wt + "/ok-written", "content": "x"}},
    {"tool": "Bash", "input": {"command": "echo x > \"" + data + "/bash-ran\"", "description": "a Bash write into the data dir"}},
    {"tool": "Write", "input": {"file_path": data + "/write-ran", "content": "x"}},
    {"tool": "Read", "input": {"file_path": data + "/seed.json"}},
    {"tool": "Edit", "input": {"file_path": data + "/seed.json", "old_string": "seed", "new_string": "edited"}},
    {"say": "done"},
]))
PY
start_stub "$SP2/plan.json" "$SP2/log" "$SP2/port"
TOOLS2=Read,Grep,Glob,Bash,Edit,Write
( cd "$WT2" && env -i PATH="/usr/bin:/bin:$(dirname "$CLAUDE")" HOME="$H" CLAUDE_CONFIG_DIR="$CC" TERM=dumb \
    ANTHROPIC_BASE_URL="http://127.0.0.1:$PORT" ANTHROPIC_API_KEY=s4-placeholder-not-a-key DISABLE_AUTOUPDATER=1 \
    perl -e 'alarm 180; exec @ARGV' "$CLAUDE" -p --output-format stream-json --verbose --session-id 4a2e8f9b-2c3d-4e5f-9a01-b2c3d4e5f6a7 \
    --setting-sources user --settings "$SP2/flag.json" --permission-mode dontAsk --tools "$TOOLS2" --allowedTools "$TOOLS2" -- "Run the S4 plan" \
    < /dev/null > "$SP2/events.jsonl" 2> "$SP2/stderr" )
STATUS2=$?
echo "claude exit (data-dir run): $STATUS2"
/usr/bin/python3 -I "$HERE/s4_check.py" "$SP" "$M" "$TOOLS" "$STATUS" "$SP2" "$WT2" "$DATA" "$TOOLS2" "$STATUS2"
VERDICT=$?
# The pair a PASS proves, for profiles::CLAUDE_WORKER_PROVEN: the exact --version line and this branch's flag shape,
# hashed with the guard scripts this run installed (the repo's).
SHAPE=$(cargo run -q --manifest-path "$HERE/../../core/Cargo.toml" --example s4_flag -- --shape "$CC/skills/vajra/hooks" 2>/dev/null)
echo "info  proven pair (add on PASS): (\"$("$CLAUDE" --version 2>/dev/null | head -1)\", \"$SHAPE\")"
exit "$VERDICT"

