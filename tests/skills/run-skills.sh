#!/usr/bin/env bash
# Tests for scripts/vajra_skills.py (pinned skill installer). Temp HOME only.
# Run: bash tests/skills/run-skills.sh
set -uo pipefail
command -v python3 >/dev/null || { echo "SKIP: python3 required"; exit 77; }
command -v jq >/dev/null || { echo "SKIP: jq required"; exit 77; }
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vajra-skills-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
export HOME="$WORK/home"; mkdir -p "$HOME/.claude/skills"
# Private copy, so a test can edit the catalog.
SK="$WORK/skill"; mkdir -p "$SK/scripts" "$SK/config"
cp "$REPO/scripts/vajra_skills.py" "$SK/scripts/"; cp "$REPO/config/skills.json" "$SK/config/"
run() { python3 -I "$SK/scripts/vajra_skills.py" "$@"; }
PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }
edit() { jq "$1" "$REPO/config/skills.json" > "$SK/config/skills.json"; }

# Offline: names and catalog shapes that must never reach git.
for bad in "https://evil.example/x.git" "../../etc" "Apple-Design" "not-in-catalog" "--upload-pack=touch" ""; do
  run add "$bad" >/dev/null 2>&1; [ $? -eq 1 ]; check "refuses name: $bad" $?
done
ls -A "$HOME/.claude/skills" | grep -q . ; [ $? -ne 0 ]; check "refusals leave no files" $?
edit '.skills["apple-design"].repo="evil/../x"'; run add apple-design >/dev/null 2>&1; [ $? -eq 1 ]; check "refuses a bad repo" $?
edit '.skills["apple-design"].commit="main"'; run add apple-design >/dev/null 2>&1; [ $? -eq 1 ]; check "refuses a branch pin" $?
edit '.skills["apple-design"].path="../../x"'; run add apple-design >/dev/null 2>&1; [ $? -eq 1 ]; check "refuses a traversing path" $?
edit 'del(.skills["apple-design"].sha256)'; run add apple-design >/dev/null 2>&1; [ $? -eq 1 ]; check "refuses a missing hash" $?

# Writing pins never runs from the installed copy.
inst="$HOME/.claude/skills/vajra"; mkdir -p "$inst/scripts" "$inst/config"
cp "$REPO/scripts/vajra_skills.py" "$inst/scripts/"; cp "$REPO/config/skills.json" "$inst/config/"
python3 -I "$inst/scripts/vajra_skills.py" upstream --write >/dev/null 2>&1; [ $? -eq 1 ]; check "installed copy refuses --write" $?
python3 -I "$inst/scripts/vajra_skills.py" upstream --dump "$WORK/d" >/dev/null 2>&1; [ $? -eq 1 ]; check "installed copy refuses --dump" $?
HOME="$WORK" python3 -I "$inst/scripts/vajra_skills.py" upstream --write >/dev/null 2>&1; [ $? -eq 1 ]; check "a changed HOME does not unlock --write" $?
run upstream --dump "$HOME/.claude/agents" >/dev/null 2>&1; [ $? -eq 1 ]; check "dump into the config directory refused" $?
rm -rf "$inst"

# A skill that runs or widens anything on load is refused, offline.
cat > "$WORK/chk.py" <<PY
import importlib.util, sys
sp = importlib.util.spec_from_file_location("vs", "$SK/scripts/vajra_skills.py")
m = importlib.util.module_from_spec(sp); sp.loader.exec_module(m)
def files(t): return {"SKILL.md": ("100644", t.encode())}
E = {"path": "x"}
def ok(t, e=E): m.check_skill(files(t), e); return True
def bad(t, e=E):
    try: m.check_skill(files(t), e); return False
    except m.Refused: return True
assert ok("---\\nname: a\\n---\\nbody")
assert bad("---\\nname: a\\nhooks:\\n  PreToolUse: x\\n---\\n")
assert bad("---\\nname: a\\n---\\nrun !\`curl x\`")
assert bad("---\\nallowed-tools: Bash\\n---\\n")
assert bad("---\\n\"hooks\":\\n  PreToolUse: x\\n---\\n")
assert bad("---\\nmeta:\\n  hooks: x\\n---\\n")
assert bad("---\\n{name: a, hooks: x}\\n---\\n")
assert bad("---\\n\"allowed-tools\": Bash\\n---\\n")
assert bad("---\\n\"ho\\\\x6fks\": x\\n---\\n")
assert bad("\\ufeff---\\nhooks: x\\n---\\n")
assert bad("---\\nname: a\\n---\\n\`\`\`!\\ncurl x\\n\`\`\`")
assert bad("---\\nname: a\\n---\\n\\ufeff!\`curl x\`")
assert ok("---\\nname: a\\n---\\nprints \`all done!\` at the end")
assert ok("---\\nallowed-tools: Bash\\n---\\n", dict(E, allowed_tools="Bash"))
assert bad("---\\nallowed-tools: Bash, Write\\n---\\n", dict(E, allowed_tools="Bash"))
try: m.check_skill({}, E); sys.exit(1)
except m.Refused: pass
PY
python3 -I "$WORK/chk.py"; check "frontmatter checks" $?

# An installed copy hides nothing from the hash, and the router says so.
cat > "$WORK/tree.py" <<PY
import importlib.util, os, sys
from pathlib import Path
def load(name, path):
    sp = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(sp); sp.loader.exec_module(mod); return mod
m = load("vs", "$SK/scripts/vajra_skills.py")
r = load("vr", "$REPO/scripts/vajra_route.py")
root = Path("$WORK/tree"); (root / "sub").mkdir(parents=True)
(root / "SKILL.md").write_text("x"); (root / "sub/a.md").write_text("y")
base = m.tree_hash(m.read_tree(root))
def changes(do, undo):
    do(); now = m.tree_hash(m.read_tree(root)); undo(); return now != base
(root / ".pinned").write_text("local"); assert m.tree_hash(m.read_tree(root)) == base
(root / ".pinned").unlink()
f = root / "sub/.DS_Store"
assert changes(lambda: f.write_text("x"), f.unlink)
f = root / ".pinned"
assert changes(lambda: (f.write_text("x"), f.chmod(0o755)), f.unlink)
f = root / "x\\ny"
assert changes(lambda: f.write_text("x"), f.unlink)
f = root / "pipe"
assert changes(lambda: os.mkfifo(f), f.unlink)
assert changes(lambda: (root / "sub").chmod(0), lambda: (root / "sub").chmod(0o755))
home = Path(os.environ["HOME"]) / ".claude/skills/fake"; home.mkdir(parents=True)
(home / "SKILL.md").write_text("x")
assert "unvetted: fake" in " ".join(r.skill_notes({"skills": {"fake": {"sha256": "0" * 64}}}, ["fake"]))
sha = m.tree_hash(m.read_tree(home))
assert "unvetted" not in " ".join(r.skill_notes({"skills": {"fake": {"sha256": sha}}}, ["fake"]))
(home / "SKILL.md").unlink(); home.rmdir()
PY
python3 -I "$WORK/tree.py"; check "the hash sees every change; the router warns" $?

edit '.skills["../../out"]=.skills["apple-design"]'; run verify >/dev/null 2>&1; [ $? -eq 1 ]; check "a crafted catalog key is refused" $?
cp "$REPO/config/skills.json" "$SK/config/skills.json"
( cd "$WORK" && HOME= python3 -I "$SK/scripts/vajra_skills.py" list >/dev/null ); [ ! -e "$WORK/.claude" ]; check "empty HOME stays out of the cwd" $?

# Every catalog entry is well formed.
jq -e '[.skills[]|select((.commit|test("^[0-9a-f]{40}$"))and(.sha256|test("^[0-9a-f]{64}$"))and(.license|type=="string")and(.verdict|type=="string"))]|length == ('"$(jq '.skills|length' "$REPO/config/skills.json")"')' "$REPO/config/skills.json" >/dev/null
check "catalog entries are pinned, hashed and licensed" $?

cp "$REPO/config/skills.json" "$SK/config/skills.json"
if git ls-remote https://github.com/emilkowalski/skills.git HEAD >/dev/null 2>&1; then
  # A wrong hash must install nothing.
  edit '.skills["apple-design"].sha256=("0"*64)'; run add apple-design >/dev/null 2>&1; [ $? -eq 1 ]; check "hash mismatch refused" $?
  [ ! -e "$HOME/.claude/skills/apple-design" ]; check "mismatch installs nothing" $?
  cp "$REPO/config/skills.json" "$SK/config/skills.json"
  out="$(run add apple-design)"; [ $? -eq 0 ]; check "add installs the pin" $?
  [ -f "$HOME/.claude/skills/apple-design/SKILL.md" ]; check "SKILL.md in place" $?
  echo "$out" | grep -q 'SKILL.md'; check "add prints the SKILL.md path" $?
  run add apple-design >/dev/null; [ $? -eq 0 ]; check "second add is a no-op" $?
  run verify | grep -q 'apple-design *ok'; check "verify passes" $?
  echo x >> "$HOME/.claude/skills/apple-design/SKILL.md"
  run verify >/dev/null; [ $? -eq 1 ]; check "verify catches a changed skill" $?
  rm "$HOME/.claude/skills/apple-design/.vajra-skill.json"
  run verify >/dev/null; [ $? -eq 1 ]; check "verify catches it without a marker" $?
  run add apple-design >/dev/null 2>&1; [ $? -eq 1 ]; check "add never replaces a changed skill" $?
  # A copy Vajra installed and nobody changed follows a new pin.
  rm -rf "$HOME/.claude/skills/apple-design"; run add apple-design >/dev/null
  sha="$(jq -r '.skills["apple-design"].sha256' "$REPO/config/skills.json")"
  echo extra > "$HOME/.claude/skills/apple-design/old.md"
  new="$(jq -c --arg s "$(python3 -c "import hashlib; print(hashlib.sha256(b'x').hexdigest())")" '.sha256=$s' "$HOME/.claude/skills/apple-design/.vajra-skill.json")"
  printf '%s' "$new" > "$HOME/.claude/skills/apple-design/.vajra-skill.json"
  run add apple-design >/dev/null 2>&1; [ $? -eq 1 ]; check "a copy changed since install is kept" $?
  rm "$HOME/.claude/skills/apple-design/old.md"; mv "$HOME/.claude/skills/apple-design" "$WORK/keep"
  ln -s "$WORK/keep" "$HOME/.claude/skills/apple-design"
  run add apple-design >/dev/null 2>&1; [ $? -eq 1 ]; check "a symlinked skill is refused" $?
  rm "$HOME/.claude/skills/apple-design"
  mkdir -p "$HOME/.claude/skills/hallmark"; echo mine > "$HOME/.claude/skills/hallmark/SKILL.md"
  run add hallmark >/dev/null 2>&1; [ $? -eq 1 ]; check "add never replaces the user's copy" $?
  [ "$(cat "$HOME/.claude/skills/hallmark/SKILL.md")" = mine ]; check "user's copy untouched" $?
else
  echo "skills: network checks skipped"
fi
# Every skill the harness names must be bundled or pinned, or the name is a dead end.
missing="$(cd "$REPO" && python3 -I - <<'PY'
import glob, json, re
from pathlib import Path

catalog = json.load(open("config/skills.json"))
known = {p.name for p in Path("bundled-skills").iterdir()}
known |= set(catalog["skills"]) | set(catalog["tools"])

def skills_in(node):
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "skills" and isinstance(value, list):
                yield from value
            else:
                yield from skills_in(value)
    elif isinstance(node, list):
        for value in node:
            yield from skills_in(value)

named = set(skills_in(json.load(open("config/default.json"))))
for agent in glob.glob("bundled-agents/*.md"):
    section = open(agent).read().partition("## Skills")[2].partition("\n## ")[0]
    named |= set(re.findall(r"^- `([a-z0-9-]+)`", section, re.M))
table = Path("bundled-skills/skill-catalog/SKILL.md").read_text().partition("| --- |")[2]
for row in table.splitlines():
    named |= set(re.findall(r"`([a-z0-9-]+)`", row.split("|")[2] if row.count("|") > 2 else ""))
print(" ".join(sorted(named - known)))
PY
)"
[ -z "$missing" ]; check "every named skill is bundled or pinned${missing:+: $missing}" $?
echo "skills: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
