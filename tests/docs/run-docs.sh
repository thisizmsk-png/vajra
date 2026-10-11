#!/usr/bin/env bash
# Docs honesty checks: SKILL.md frontmatter and size, one doc per subcommand,
# every repo path the docs mention exists, no retired v0.4 claims.
# Run: bash tests/docs/run-docs.sh
set -uo pipefail
command -v python3 >/dev/null || { echo "SKIP: python3 required"; exit 0; }
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 -I - "$REPO" <<'PY'
import ast, re, sys
from pathlib import Path
repo = Path(sys.argv[1]); fails = []; passes = 0
def check(name, ok):
    global passes
    if ok: passes += 1
    else: fails.append(name); print("FAIL ::", name)

skill = (repo / "SKILL.md").read_text()
m = re.match(r"^---\n(.*?)\n---\n", skill, re.S)
check("SKILL.md has frontmatter", bool(m))
keys = [l.split(":", 1)[0].strip() for l in (m.group(1).splitlines() if m else []) if l and not l.startswith(" ")]
allowed = {"name", "description", "argument-hint", "allowed-tools", "model",
           "disable-model-invocation", "user-invocable", "when_to_use"}
check("frontmatter keys valid: %s" % keys, set(keys) <= allowed and {"name", "description", "argument-hint"} <= set(keys))
check("no invalid 'invocation'/'args' keys", "invocation" not in keys and "args" not in keys)
n = skill.count("\n")
check("SKILL.md about 80 lines (%d)" % n, n <= 100)
check("SKILL.md: bare ask launches vajra-deliver", "vajra-deliver" in skill)

src = (repo / "scripts" / "vajra_route.py").read_text()
subs = ast.literal_eval(re.search(r"SUBCOMMANDS = (\{.*?\n\})", src, re.S).group(1))
for doc in sorted(set(subs.values())):
    check("docs/commands/%s.md exists" % doc, (repo / "docs" / "commands" / (doc + ".md")).is_file())
for ref in re.findall(r"docs/commands/([a-z-]+)\.md", skill):
    check("SKILL.md ref docs/commands/%s.md exists" % ref, (repo / "docs" / "commands" / (ref + ".md")).is_file())
for c in ["plan", "fleet", "campaign", "verify-work", "ralph", "review", "cost", "replay", "repomap", "redteam", "atman", "doctor", "help"]:
    check("router knows subcommand %s" % c, c in subs)

docs = [repo / "SKILL.md", repo / "README.md", repo / "CONTRIBUTING.md",
        repo / "docs/architecture/ARCHITECTURE.md", repo / "engine/router.md",
        repo / "memory/tiered-memory.md"] + sorted((repo / "docs/commands").glob("*.md"))
pref = r"(?:scripts|hooks|workflows|engine|tests|config|atman|steering|redteam|memory|fleet|evals|bundled-agents|docs)"
path_re = re.compile(r"(?<![\w./~-])(" + pref + r"/[A-Za-z0-9_.\-/]*[A-Za-z0-9_/])")
retired = [r"\bTier 4\b", r"Authorization Gate", r"\bvajra\.db\b", r"better-sqlite3 is", r"tier3Keywords", r"/vajra act\b", r"\bRE2\b"]
for d in docs:
    text = d.read_text()
    rel = d.relative_to(repo)
    t = text.replace("~/.claude/skills/vajra/", "").replace("SKILL/", "")
    if d.name == "ARCHITECTURE.md":
        t = t.split("## Removed in v0.5")[0]
    found = set()
    for mm in path_re.finditer(t):
        if any(ch in t[mm.end():mm.end() + 2] for ch in "*<"):
            continue  # a glob or template, not a concrete path
        found.add(mm.group(1))
    for p in sorted(found):
        check("%s: referenced path %s exists" % (rel, p), (repo / p).exists())
    if d.name != "ARCHITECTURE.md":
        for pat in retired:
            check("%s: no retired claim /%s/" % (rel, pat), not re.search(pat, text))
    if d.suffix == ".md" and ("docs/commands" in str(rel) or d.name in ("README.md", "CONTRIBUTING.md", "SKILL.md")):
        check("%s: no em/en dashes" % rel, "—" not in text and "–" not in text)
check(".github/workflows/ci.yml exists", (repo / ".github/workflows/ci.yml").is_file())
ci = (repo / ".github/workflows/ci.yml").read_text() if (repo / ".github/workflows/ci.yml").is_file() else ""
check("CI runs tests/run-all.sh", "bash tests/run-all.sh" in ci)
pkg = (repo / "package.json").read_text()
check("package.json has no better-sqlite3", "better-sqlite3" not in pkg)
check("engine.ts removed", not (repo / "engine/engine.ts").exists())
print("----------------------------------------")
print("docs: %d passed, %d failed" % (passes, len(fails)))
sys.exit(1 if fails else 0)
PY
