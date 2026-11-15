"""The S2 and S5 verdicts (ADR-16 16.12), from what `s2s5-lead-mcp.sh` recorded.

    python3 -I s2s5_check.py <mode> <spike dir> <marker dir> <wait_s> <exit status> <project dir> <lead dir>

PASS means every probe ran to its end and gave the answer the profile depends on, never only the absence of a bad
outcome. In `codex-short-timeout` mode (tool_timeout_sec=20, run it with a wait above 20) the await must be cut short: it
proves the key is honoured, so the profile's 130 is a real bound and not an ignored setting.
"""
import json
import os
import sys


def lines(p):
    out = []
    if os.path.exists(p):
        for l in open(p):
            try:
                out.append(json.loads(l))
            except ValueError:
                pass
    return out


def main():
    mode, sp, m, wait, status, proj, lead = sys.argv[1:8]
    wait, status = int(wait), int(status)
    stub = lines(sp + "/stub/log")
    mcp = lines(m + "/mcp-log")
    results = {}
    for r in stub:
        for x in r.get("results", []):
            results[x.get("id")] = x.get("text", "")
    offered = sorted({n for r in stub for n in r.get("tools", []) if n})
    rid = (lambda k: f"tu_{k}") if mode == "claude" else (lambda k: f"call_{k}")
    last = 3 if mode == "claude" else 5
    await_k = last - 1
    starts = [e for e in mcp if e.get("ev") == "call_start" and e.get("name") == "await_any"]
    ends = [e for e in mcp if e.get("ev") == "call_end" and e.get("name") == "await_any"]
    seen = [r for r in stub if r.get("k") == last]
    took = round(seen[0]["t"] - starts[0]["t"], 1) if (seen and starts) else None
    got_wait = "waited" in results.get(rid(await_k), "")
    checks = [
        ("the harness reached the stub", bool(stub)),
        ("the vajra_run probe tool was offered", any(n.endswith("probe") for n in offered)),
        ("the vajra_run await_any tool was offered", any(n.endswith("await_any") for n in offered)),
        ("the probe call reached the MCP server", any(e.get("ev") == "call_start" and e.get("name") == "probe" for e in mcp)),
        ("the probe answer came back with VAJRA_RUN", '"run": "r1"' in results.get(rid(last - 2), "")),
        ("the token is not in the MCP server's argv", all(not e.get("argv_has_token") for e in mcp if e.get("ev") == "start")),
        ("the await_any call reached the MCP server", bool(starts)),
    ]
    if mode == "codex-short-timeout":
        checks.append(("the await was cut short at tool_timeout_sec", not got_wait and took is not None and took < wait))
    else:
        checks += [
            ("the harness exited 0", status == 0),
            ("the plan reached its last turn", bool(seen)),
            (f"the {wait} s await_any came back with its answer", got_wait and bool(ends)),
        ]
    if mode == "claude":
        events = [e for e in lines(sp + "/events.jsonl") if isinstance(e, dict)]
        init = next((e for e in events if e.get("type") == "system" and e.get("subtype") == "init"), {})
        servers = {s.get("name"): s.get("status") for s in init.get("mcp_servers", []) if isinstance(s, dict)}
        builtin = sorted(t for t in init.get("tools", []) if not t.startswith("mcp__"))
        checks += [
            ("the Read of the project came back", "s2-project-marker" in results.get("tu_0", "")),
            ("init lists vajra_run as connected", servers.get("vajra_run") == "connected"),
            ("init lists only vajra_run as an MCP server", list(servers) == ["vajra_run"]),
            ("init built-in tools are exactly Read, Grep, Glob", builtin == ["Glob", "Grep", "Read"]),
        ]
    else:
        checks += [
            ("S2: the project read came back", "s2-project-marker" in results.get("call_0", "")),
            ("S2: the lead dir write was refused", "WROTE" not in results.get("call_1", "") and not os.path.exists(lead + "/lead-write")),
            ("S2: the project write was refused", "WROTE" not in results.get("call_2", "") and not os.path.exists(proj + "/src/proj-write")),
            ("N3: the user's own MCP server never ran", not os.path.exists(m + "/user-mcp-ran")),
            ("N3: the user's notify never ran", not os.path.exists(m + "/user-notify-ran")),
            ("N3: no user MCP tool was offered", not any("user_mcp" in n for n in offered)),
        ]
    ok = all(c for _, c in checks)
    if os.path.exists(m + "/user-mcp-ran"):
        print("user MCP servers that started: " + ", ".join(open(m + "/user-mcp-ran").read().split()))
    for name, c in checks:
        print(("  ok   " if c else "  FAIL ") + name)
    print(f"offered tools: {offered}")
    print(f"await_any: asked {wait} s, answered after {took} s" if took is not None else "await_any: no answer seen")
    for k in range(last + 1):
        t = results.get(rid(k))
        if t is not None:
            print(f"result {k}: {t[:200]!r}")
    print(("PASS" if ok else "FAIL") + f" ({mode})")
    sys.exit(0 if ok else 1)


main()
