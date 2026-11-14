"""A fake `vajra_run` MCP server for spikes S2 and S5 (stdio, newline-delimited JSON-RPC).

    python3 -I s2s5_mcp.py <log.jsonl>

Tools: `probe` answers at once with VAJRA_RUN and whether the token file is readable (never its contents);
`await_any{wait_s}` sleeps wait_s seconds and then answers, the way the real one parks. Every call is logged with its
start and end time, so the checker can tell a call the harness gave up on from one that came back.
"""
import json
import os
import sys
import threading
import time

log = sys.argv[1]
lock = threading.Lock()


def note(rec):
    rec["t"] = round(time.time(), 3)
    with lock, open(log, "a") as f:
        f.write(json.dumps(rec) + "\n")


def send(msg):
    with lock:
        sys.stdout.write(json.dumps(msg) + "\n")
        sys.stdout.flush()


TOOLS = [
    {"name": "probe", "description": "Spike probe: answers at once.", "inputSchema": {"type": "object", "properties": {}}},
    {"name": "await_any", "description": "Spike await: sleeps wait_s seconds.",
     "inputSchema": {"type": "object", "properties": {"wait_s": {"type": "integer"}}, "required": ["wait_s"]}},
]


def call(rid, name, args):
    note({"ev": "call_start", "name": name, "args": args})
    if name == "probe":
        tf = os.environ.get("VAJRA_RUN_TOKEN_FILE", "")
        text = json.dumps({"run": os.environ.get("VAJRA_RUN"), "token_file_readable": bool(tf) and os.access(tf, os.R_OK)})
    elif name == "await_any":
        w = int(args.get("wait_s", 0))
        time.sleep(w)
        text = json.dumps({"state": "waited", "elapsed_s": w})
    else:
        send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "no such tool"}})
        return
    note({"ev": "call_end", "name": name})
    send({"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": text}], "isError": False}})


note({"ev": "start", "pid": os.getpid(), "argv_has_token": any("tok" in a.lower() and "file" not in a.lower() for a in sys.argv[2:])})
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    method, rid = m.get("method"), m.get("id")
    if method == "initialize":
        pv = (m.get("params") or {}).get("protocolVersion", "2025-06-18")
        send({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": pv, "capabilities": {"tools": {}},
                                                      "serverInfo": {"name": "vajra_run", "version": "0"}}})
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": rid, "result": {"tools": TOOLS}})
    elif method == "tools/call":
        p = m.get("params") or {}
        threading.Thread(target=call, args=(rid, p.get("name"), p.get("arguments") or {}), daemon=True).start()
    elif method == "notifications/cancelled":
        note({"ev": "cancelled", "params": m.get("params")})
    elif rid is not None:
        send({"jsonrpc": "2.0", "id": rid, "result": {}})
note({"ev": "eof"})
