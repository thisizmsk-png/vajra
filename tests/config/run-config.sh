#!/usr/bin/env bash
# Checks config/*.json: valid JSON, steering rules resolve, narrowed CDK/lambda
# patterns, agent names exist. Run: bash tests/config/run-config.sh
set -uo pipefail
command -v python3 >/dev/null || { echo "SKIP: python3 required"; exit 0; }
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 -I - "$REPO" <<'PY'
import json, re, sys
from pathlib import Path
repo = Path(sys.argv[1]); fails = []; passes = 0
def check(name, ok):
    global passes
    if ok: passes += 1
    else: fails.append(name); print("FAIL ::", name)

cfgs = {}
for p in sorted((repo / "config").glob("*.json")):
    try: cfgs[p.name] = json.loads(p.read_text()); check(p.name + " parses", True)
    except Exception as e: check(p.name + " parses (%s)" % e, False)

def glob_re(g):
    out, i = "", 0
    while i < len(g):
        if g.startswith("**/", i): out += "(?:.*/)?"; i += 3
        elif g.startswith("**", i): out += ".*"; i += 2
        elif g[i] == "*": out += "[^/]*"; i += 1
        elif g[i] == "?": out += "[^/]"; i += 1
        else: out += re.escape(g[i]); i += 1
    return re.compile("^" + out + "$", re.I)
def applies(rule, path):
    pos = [g for g in rule["filePatterns"] if not g.startswith("!")]
    neg = [g[1:] for g in rule["filePatterns"] if g.startswith("!")]
    return any(glob_re(g).match(path) for g in pos) and not any(glob_re(g).match(path) for g in neg)

st = cfgs.get("steering.json", {})
rules = {r["id"]: r for r in st.get("rules", [])}
for r in rules.values():
    check("rule file exists: " + r["file"], (repo / r["file"]).is_file())
for agent, ids in st.get("agentSteering", {}).items():
    check("steering agent exists: " + agent, (repo / "bundled-agents" / (agent + ".md")).is_file())
    check("steering ids resolve for " + agent, all(i in rules for i in ids))
cdk, lam = rules.get("aws-cdk"), rules.get("lambda")
for path, rule, want in [
    ("lib/utils.ts", cdk, False), ("lib/app/server.ts", cdk, False),
    ("infra/cdk/lib/api.ts", cdk, True), ("infra/lib/api-stack.ts", cdk, True),
    ("lib/NetworkStack.ts", cdk, True), ("cdk.json", cdk, True),
    ("src/handlers/http.ts", lam, False), ("src/ErrorHandler.java", lam, False),
    ("services/lambda/orders.py", lam, True), ("src/resize.lambda.ts", lam, True),
    ("fn/lambda_function.py", lam, True)]:
    check("%s %s %s" % (rule["id"], "applies to" if want else "skips", path), applies(rule, path) == want)
check("duryodhana steering is not security", "security" not in st.get("agentSteering", {}).get("duryodhana", []))

d = cfgs.get("default.json", {})
check("no tier3Keywords (removed)", "tier3Keywords" not in d.get("routing", {}))
check("no sqlite dbPath", "campaigns" not in d)
check("deliver workflow named", d.get("cortex", {}).get("deliverWorkflow") == "vajra-deliver")
for name, crew in d.get("cortex", {}).get("fleetCrews", {}).items():
    check("crew %s workflow is vajra-%s" % (name, name), crew.get("workflow") == "vajra-" + name)
check("five crews incl. sabha", sorted(d.get("cortex", {}).get("fleetCrews", {})) ==
      ["architecture-review", "feature-build", "incident-response", "sabha", "security-audit"])
print("----------------------------------------")
print("config: %d passed, %d failed" % (passes, len(fails)))
sys.exit(1 if fails else 0)
PY
