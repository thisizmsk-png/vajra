"""The S4 verdict (ADR-16 16.12), from what `s4-claude-settings.sh` recorded.

    python3 -I s4_check.py <spike dir> <marker dir> <tools csv> <claude exit status> \
        <data-run dir> <data-run worktree> <data dir> <data-run tools csv> <data-run claude exit status> \
        <key folder> <data-run ticket tmp>

PASS means every probe in the plan ran to its end. It is never the absence of a bad outcome alone (#147 codex). Claude
has to exit 0 with a successful result event. The stub has to have served the plan's last turn. The allowed Bash call
must come back as a success, and the guard-blocked Bash call and the Write call (not in --tools) must each have been
issued and come back as an error. The guard must have seen the blocked call: a Bash hook input whose
tool_input.command names it, parsed from the guard's record, not a text search of it. A run that stops after its first call
(an error, or the 180 s alarm) fails.

The data-dir run (a writer worker with the real flag file and the real guard) must also run to its end; its Write into
the worktree must succeed; its Bash call into the Vajra data dir must come back refused by the Vajra guard, and its
Write and Edit calls refused by the flag file's deny rules (each result's text names its source; any other error fails);
and the data dir must hold only its seed file, unchanged. Its two name-free Bash calls (a read of the stand-in key and a
write into the data dir, whose commands name no locked path) must each come back saying "Operation not permitted", the
OS sandbox's refusal, with the stand-in key's text in no tool result; and its Bash writes into the ticket tmp and the
worktree must have run.
"""
import json
import os
import sys

# The stub's plan: tool_use ids are `tu_<step>`; step 3 is the closing "done".
ALLOWED, BLOCKED, WRITE, DONE = "tu_0", "tu_1", "tu_2", 3
# The data-dir run's plan: a worktree Write, then a Bash write, a Write, a Read and an Edit into the data dir, then the
# sandbox probes: a name-free key read, a name-free data-dir write, and Bash writes into the ticket tmp and worktree.
D_OK, D_BASH, D_WRITE, D_EDIT = "tu_0", "tu_1", "tu_2", "tu_4"
D_KEY, D_DATA, D_TMP, D_DONE = "tu_5", "tu_6", "tu_7", 8
SEED = "seed\n"
KEY_STAND_IN = "s4-stand-in"
# What each refusal's tool result must say: the guard's block, Claude Code's permission deny, and the OS sandbox's EPERM.
GUARD_REFUSAL = "Vajra guard blocked"
DENY_REFUSAL = "denied by your permission settings"
SANDBOX_REFUSAL = "operation not permitted"


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


def run(sp):
    events = [e for e in lines(sp + "/events.jsonl") if isinstance(e, dict)]
    init = next((e for e in events if e.get("type") == "system" and e.get("subtype") == "init"), {})
    result = next((e for e in events if e.get("type") == "result"), {})
    log = lines(sp + "/log")
    results = {}
    for r in log:
        for t in r.get("results", []):
            results[t.get("id")] = t
    return init, result, log, results


def data_verdict(sp, wt, data, tools, status, kd, tt):
    init, result, log, results = run(sp)
    got = sorted(t for t in init.get("tools", []) if not t.startswith("mcp__"))
    err = lambda i: i in results and results[i].get("is_error") is True
    # Refused by the guard or the deny rules, not any error: a shell error or "read the file first" is not proof (#163).
    refused_by = lambda i, source: err(i) and source in (results[i].get("text") or "")
    # The sandbox's EPERM reaches the model as the command's own output, and the Bash call need not be an error.
    sandboxed = lambda i: i in results and SANDBOX_REFUSAL in (results[i].get("text") or "").lower()
    leaked = any(KEY_STAND_IN in (t.get("text") or "") for t in results.values())
    key_intact = os.path.isfile(kd + "/s4-key") and open(kd + "/s4-key").read() == KEY_STAND_IN + "\n"
    left = sorted(os.listdir(data)) if os.path.isdir(data) else None
    seed = open(data + "/seed.json").read() if os.path.isfile(data + "/seed.json") else None
    checks = [
        ("data-dir run: claude exited 0", status == 0),
        ("data-dir run: claude reported a successful result", result.get("subtype") == "success" and result.get("is_error") is False),
        ("data-dir run: the plan reached its last turn", any(r.get("k") == D_DONE and r.get("tools") for r in log)),
        ("data-dir run: init lists exactly the --tools set", got == sorted(tools)),
        ("data-dir run: a Write into the worktree ran", os.path.exists(wt + "/ok-written") and D_OK in results and not err(D_OK)),
        ("data-dir run: the Vajra guard refused a Bash write into the data dir", refused_by(D_BASH, GUARD_REFUSAL)),
        ("data-dir run: the deny rules refused a Write into the data dir", refused_by(D_WRITE, DENY_REFUSAL)),
        ("data-dir run: the deny rules refused an Edit in the data dir", refused_by(D_EDIT, DENY_REFUSAL)),
        ("data-dir run: the OS sandbox refused a key read that names no locked path", sandboxed(D_KEY)),
        ("data-dir run: the OS sandbox refused a data-dir write that names no locked path", sandboxed(D_DATA)),
        ("data-dir run: the stand-in key reached no tool result and is unchanged", not leaked and key_intact),
        ("data-dir run: Bash wrote in the ticket tmp and the worktree", D_TMP in results and not err(D_TMP)
            and os.path.exists(tt + "/tmp-ok") and os.path.exists(wt + "/bash-ok")),
        ("data-dir run: the data dir holds only its seed file, unchanged", left == ["seed.json"] and seed == SEED),
    ]
    why = {name: (results.get(i, {}).get("text") or "")[:160].replace("\n", " ")
           for name, i in (("Bash", D_BASH), ("Write", D_WRITE), ("Edit", D_EDIT), ("sandboxed key read", D_KEY), ("sandboxed data write", D_DATA))}
    return checks, why


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
    if len(sys.argv) != 12:
        print("FAIL  usage: s4_check.py <sp> <m> <tools> <status> <data sp> <data wt> <data dir> <data tools> <data status> <key dir> <data tmp>")
        print("S4 FAIL")
        sys.exit(2)
    code = lambda s: int(s) if s.isdigit() else -1
    sp, m, tools, status = sys.argv[1], sys.argv[2], sys.argv[3].split(","), sys.argv[4]
    checks, init = verdict(sp, m, tools, code(status))
    dchecks, why = data_verdict(sys.argv[5], sys.argv[6], sys.argv[7], sys.argv[8].split(","), code(sys.argv[9]), sys.argv[10], sys.argv[11])
    checks += dchecks
    ok = True
    for name, passed in checks:
        print(("ok    " if passed else "FAIL  ") + name)
        ok &= passed
    print("info  init tools:", init.get("tools"))
    print("info  the user's own hook ran:", os.path.exists(m + "/user-hook"))
    for tool, text in why.items():
        print("info  data-dir run: the %s refusal said: %s" % (tool, text))
    print("S4 PASS" if ok else "S4 FAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
