#!/usr/bin/env bash
# Offline lint for evals/ (claude plugin eval format, schema 1.1): every case has a
# prompt.md with only documented frontmatter keys and at least one grader with a
# known type; trigger cases assert the vajra Skill fired, negative cases assert it
# did not (min/max 0, arm both). Does not run the model.
# Run: bash tests/atman/run-evals-lint.sh   (live run: claude plugin eval ~/.claude/skills/vajra)
set -uo pipefail
command -v python3 >/dev/null || { echo "SKIP: python3 required"; exit 0; }
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 -I - "$REPO/evals" <<'PY'
import os, re, sys
root = sys.argv[1]
PROMPT_KEYS = {"schema_version", "name", "description", "tags", "plugins", "runs",
               "expected_outcome", "model", "max_turns", "timeout_seconds",
               "allowed_tools", "append_system_prompt", "env"}
GRADER_TYPES = {"tool_used", "llm"}
GRADER_KEYS = {"type", "weight", "arm", "tool", "input_match", "min", "max"}
fails, cases = [], 0
def fm(path):
    t = open(path).read()
    m = re.match(r"^---\n(.*?)\n---\n?(.*)$", t, re.S)
    if not m:
        return None, t
    d = {}
    for line in m.group(1).splitlines():
        if line.strip():
            k, _, v = line.partition(":")
            d[k.strip()] = v.strip()
    return d, m.group(2)
for name in sorted(os.listdir(root)):
    d = os.path.join(root, name)
    if not os.path.isdir(d) or name in ("results", "mocks"):
        continue
    cases += 1
    meta, body = fm(os.path.join(d, "prompt.md")) if os.path.isfile(os.path.join(d, "prompt.md")) else (None, "")
    if meta is None:
        fails.append("%s: missing prompt.md frontmatter" % name); continue
    bad = set(meta) - PROMPT_KEYS
    if bad: fails.append("%s: unknown prompt keys %s" % (name, sorted(bad)))
    if meta.get("name") != name: fails.append("%s: name != directory" % name)
    if not body.strip(): fails.append("%s: empty prompt" % name)
    if not (1 <= int(meta.get("max_turns", "10")) <= 200): fails.append("%s: max_turns range" % name)
    gdir = os.path.join(d, "graders")
    gs = sorted(os.listdir(gdir)) if os.path.isdir(gdir) else []
    if not gs: fails.append("%s: no graders" % name)
    for g in gs:
        gm, _ = fm(os.path.join(gdir, g))
        if not gm or gm.get("type") not in GRADER_TYPES:
            fails.append("%s/%s: bad grader type" % (name, g)); continue
        if set(gm) - GRADER_KEYS: fails.append("%s/%s: unknown keys %s" % (name, g, sorted(set(gm) - GRADER_KEYS)))
        if gm["type"] == "tool_used":
            rx = gm.get("input_match", "").strip("'")
            try:
                r = re.compile(rx)
            except re.error:
                fails.append("%s/%s: bad regex" % (name, g)); continue
            if not r.search('{"skill":"vajra","args":"x"}') or r.search('{"skill":"vajra-other"}') \
                    or not r.search('{"skill": "plug:vajra"}'):
                fails.append("%s/%s: regex does not match exactly the vajra skill" % (name, g))
    tags = meta.get("tags", "")
    kinds = [fm(os.path.join(gdir, g))[0] for g in gs]
    if "negative" in tags and not any(k.get("max") == "0" and k.get("arm") == "both" for k in kinds):
        fails.append("%s: negative case must assert max: 0 with arm: both" % name)
    if "trigger" in tags and not any(k.get("type") == "tool_used" and "max" not in k for k in kinds):
        fails.append("%s: trigger case must assert the skill fired" % name)
neg = sum(1 for n in os.listdir(root) if n.startswith("negative-"))
pos = sum(1 for n in os.listdir(root) if n.startswith("trigger-"))
if neg < 5 or pos < 5: fails.append("need >=5 trigger and >=5 negative cases (have %d/%d)" % (pos, neg))
for f in fails: print("FAIL ::", f)
print("----------------------------------------")
print("evals lint: %d cases, %d trigger, %d negative, %d problems" % (cases, pos, neg, len(fails)))
sys.exit(1 if fails else 0)
PY
