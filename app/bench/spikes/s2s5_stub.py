"""Loopback model stub for spikes S2 and S5 (ADR-16 16.12). Speaks just enough of the Responses API (Codex) and the
Messages API (Claude Code) to drive a fixed plan of tool calls. It never leaves 127.0.0.1 and ignores any key.

    python3 -I s2s5_stub.py <plan.json> <log.jsonl> <portfile>

A plan step is one of:
    {"shell": "<command>"}              the harness's shell tool (Codex) or Bash (Claude)
    {"read": "<absolute path>"}         Claude's Read tool (Codex has none; use shell)
    {"mcp": "<tool suffix>", "args": {}} the first offered tool whose name ends with the suffix
    {"say": "<text>"}                   a closing assistant message

The step is the number of tool results already in the conversation, so each turn advances by one.
"""
import http.server
import json
import sys
import time

plan, log, portfile = json.load(open(sys.argv[1])), sys.argv[2], sys.argv[3]


def note(rec):
    rec["t"] = round(time.time(), 3)
    with open(log, "a") as f:
        f.write(json.dumps(rec) + "\n")


def pick(names, step):
    """The tool name and input for a plan step, given the offered tool names."""
    if "mcp" in step:
        n = next((n for n in names if n.endswith(step["mcp"])), None)
        return n, step.get("args", {})
    if "read" in step:
        return "Read", {"file_path": step["read"]}
    cmd = step["shell"]
    if "Bash" in names:
        return "Bash", {"command": cmd, "description": "spike"}
    if "exec_command" in names:
        return "exec_command", {"cmd": cmd}
    if "shell_command" in names:
        return "shell_command", {"command": cmd}
    return "shell", {"command": ["/bin/sh", "-c", cmd]}


class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def do_GET(self):
        body = b'{"data":[],"models":[]}'
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def sse(self):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.send_header("connection", "close")
        self.end_headers()
        self.close_connection = True

    def ev(self, t, d):
        d["type"] = t
        self.wfile.write(f"event: {t}\ndata: {json.dumps(d)}\n\n".encode())
        self.wfile.flush()

    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("content-length", 0)) or 0)
        try:
            b = json.loads(raw or b"{}")
        except ValueError:
            b = {}
        if "count_tokens" in self.path:
            body = b'{"input_tokens":1}'
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path.rstrip("/").endswith("/responses"):
            self.do_responses(b)
        else:
            self.messages(b)

    def do_responses(self, b):
        items = b.get("input", []) if isinstance(b.get("input"), list) else []
        outs = [i for i in items if isinstance(i, dict) and i.get("type") == "function_call_output"]
        k = len(outs)
        names, ns_of = [], {}
        for t in b.get("tools", []):
            if not isinstance(t, dict):
                continue
            for inner in t.get("tools", []) if t.get("type") == "namespace" else [t]:
                n = (t["name"] + "__" + inner["name"]) if t.get("type") == "namespace" else inner.get("name")
                if n:
                    names.append(n)
                    if t.get("type") == "namespace":
                        ns_of[n] = (t["name"], inner["name"])
        results = [{"id": o.get("call_id"), "text": str(o.get("output"))[:600]} for o in outs]
        note({"api": "responses", "k": k, "tools": names, "results": results})
        step = plan[k] if k < len(plan) else {"say": "done"}
        self.sse()
        self.ev("response.created", {"response": {"id": f"resp_{k}"}})
        if "say" in step:
            item = {"type": "message", "role": "assistant", "id": f"msg_{k}",
                    "content": [{"type": "output_text", "text": step["say"]}]}
        else:
            name, args = pick(names, step)
            item = {"type": "function_call", "id": f"fc_{k}", "call_id": f"call_{k}", "name": name or "missing_tool",
                    "arguments": json.dumps(args)}
            if name in ns_of:
                item["namespace"], item["name"] = ns_of[name]
        self.ev("response.output_item.done", {"output_index": 0, "item": item})
        self.ev("response.completed", {"response": {"id": f"resp_{k}", "usage": {
            "input_tokens": 1, "input_tokens_details": {"cached_tokens": 0}, "output_tokens": 1,
            "output_tokens_details": {"reasoning_tokens": 0}, "total_tokens": 2}}})

    def messages(self, b):
        blocks = [c for m in b.get("messages", []) if isinstance(m.get("content"), list) for c in m["content"]]
        res = [c for c in blocks if isinstance(c, dict) and c.get("type") == "tool_result"]
        k = len(res)
        names = [t.get("name") for t in b.get("tools", []) if isinstance(t, dict)]

        def text(c):
            x = c.get("content")
            return x if isinstance(x, str) else " ".join(i.get("text", "") for i in x or [] if isinstance(i, dict))

        results = [{"id": c.get("tool_use_id"), "is_error": c.get("is_error") is True, "text": text(c)[:600]} for c in res]
        note({"api": "messages", "k": k, "tools": names, "results": results})
        # Side requests (titles, summaries) offer no tools: answer them with text and do not advance the plan.
        step = plan[k] if (names and k < len(plan)) else {"say": "done"}
        self.sse()
        self.ev("message_start", {"message": {"id": f"msg_{k}", "type": "message", "role": "assistant",
                                              "model": b.get("model", "m"), "content": [], "stop_reason": None,
                                              "usage": {"input_tokens": 1, "output_tokens": 1}}})
        if "say" in step:
            self.ev("content_block_start", {"index": 0, "content_block": {"type": "text", "text": ""}})
            self.ev("content_block_delta", {"index": 0, "delta": {"type": "text_delta", "text": step["say"]}})
            stop = "end_turn"
        else:
            name, args = pick(names, step)
            self.ev("content_block_start", {"index": 0, "content_block": {"type": "tool_use", "id": f"tu_{k}",
                                                                          "name": name or "missing_tool", "input": {}}})
            self.ev("content_block_delta", {"index": 0, "delta": {"type": "input_json_delta", "partial_json": json.dumps(args)}})
            stop = "tool_use"
        self.ev("content_block_stop", {"index": 0})
        self.ev("message_delta", {"delta": {"stop_reason": stop}, "usage": {"output_tokens": 1}})
        self.ev("message_stop", {})


srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
open(portfile, "w").write(str(srv.server_address[1]))
srv.serve_forever()
