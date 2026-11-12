"""The S4 verdict (ADR-16 16.12), from what `s4-claude-settings.sh` recorded.

    python3 -I s4_check.py <spike dir> <marker dir> <tools csv> <claude exit status>

PASS means every probe in the plan ran to its end. It is never the absence of a bad outcome alone (#147 codex). Claude
has to exit 0 with a successful result event. The stub has to have served the plan's last turn. The allowed Bash call
must come back as a success, and the guard-blocked Bash call and the Write call (not in --tools) must each have been
issued and come back as an error. The guard must have seen the blocked call: a Bash hook input whose
tool_input.command names it, parsed from the guard's record, not a text search of it. A run that stops after its first call
(an error, or the 180 s alarm) fails.
"""
import json
import os
import sys

# The stub's plan: tool_use ids are `tu_<step>`; step 3 is the closing "done".
ALLOWED, BLOCKED, WRITE, DONE = "tu_0", "tu_1", "tu_2", 3


def lines(p):
    if not os.path.exists(p):
        return []
    out = []
    for l in open(p):
        try:
            out.append(json.loads(l))
        except ValueError:
            pass
    return out


def hook_inputs(p):
    """The hook inputs the guard stand-in recorded: a stream of whole JSON objects, one per call, however long."""
    if not os.path.exists(p):
        return []
    text, dec, out, i = open(p).read(), json.JSONDecoder(), [], 0
    while True:
        while i < len(text) and text[i].isspace():
            i += 1
        if i >= len(text):
            return out
        try:
            obj, i = dec.raw_decode(text, i)
        except ValueError:
            return out  # a torn record ends the stream; what came before still counts
        if isinstance(obj, dict):
            out.append(obj)


def saw_bash(calls, needle):
    for c in calls:
        cmd = c.get("tool_input", {}).get("command") if isinstance(c.get("tool_input"), dict) else None
        if c.get("tool_name") == "Bash" and isinstance(cmd, str) and needle in cmd:
            return True
    return False


def verdict(sp, m, tools, status):
    events = [e for e in lines(sp + "/events.jsonl") if isinstance(e, dict)]
    init = next((e for e in events if e.get("type") == "system" and e.get("subtype") == "init"), {})
    result = next((e for e in events if e.get("type") == "result"), {})
    log = lines(sp + "/stub/log")
    got = sorted(t for t in init.get("tools", []) if not t.startswith("mcp__"))
    results = {}
    for r in log:
        for t in r.get("results", []):
            results[t.get("id")] = t
    guard = hook_inputs(m + "/guard")
    err = lambda i: i in results and results[i].get("is_error") is True
    checks = [
        ("claude exited 0", status == 0),
        ("claude reported a successful result", result.get("subtype") == "success" and result.get("is_error") is False),
        ("the plan reached its last turn", any(r.get("k") == DONE and r.get("tools") for r in log)),
        ("prompt after -- reaches the model", any("Run the S4 plan" in r.get("body", "") for r in log)),
        ("init lists exactly the --tools set", got == sorted(tools)),
        ("permissionMode is dontAsk", init.get("permissionMode") == "dontAsk"),
        ("the flag file's hook ran despite the user's disableAllHooks", os.path.exists(m + "/guard")),
        ("an allowed Bash call ran", os.path.exists(m + "/allowed-ran") and ALLOWED in results and not err(ALLOWED)),
        ("the guard saw the blocked Bash call", saw_bash(guard, "/forbidden-ran")),
        ("the blocked Bash call came back refused", err(BLOCKED)),
        ("the guard's block held", not os.path.exists(m + "/forbidden-ran")),
        ("the Write call came back refused", err(WRITE)),
        ("the Write call never wrote", not os.path.exists(m + "/write-ran")),
        ("the worktree's project-settings hook never ran", not os.path.exists(m + "/project-hook")),
    ]
    return checks, init


def main():
    sp, m, tools, status = sys.argv[1], sys.argv[2], sys.argv[3].split(","), sys.argv[4]
    checks, init = verdict(sp, m, tools, int(status) if status.isdigit() else -1)
    ok = True
    for name, passed in checks:
        print(("ok    " if passed else "FAIL  ") + name)
        ok &= passed
    print("info  init tools:", init.get("tools"))
    print("info  the user's own hook ran:", os.path.exists(m + "/user-hook"))
    print("S4 PASS" if ok else "S4 FAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
