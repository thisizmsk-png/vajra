#!/usr/bin/env bash
# Tests for `vajra_tasks.py check-evidence` (read-only green check over read_evidence()).
# Runs in a temp HOME with a temp campaign dir and a temp git repo; never touches ~/.claude.
# Run: bash tests/tasks/run-check-evidence.sh
set -uo pipefail
command -v python3 >/dev/null || { echo "SKIP: python3 required"; exit 0; }
command -v git >/dev/null || { echo "SKIP: git required"; exit 0; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PY="$REPO/scripts/vajra_tasks.py"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vajra-check-evidence-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
export HOME="$WORK/home"
CD="$HOME/.claude/vajra/campaigns/camp-ce"
mkdir -p "$CD"
echo '{"schema":"vajra.tasks/v1","tasks":[]}' > "$CD/tasks.json"
KEYF="$HOME/.claude/vajra/.hmac-key"
python3 -c 'import secrets; print(secrets.token_hex(32))' > "$KEYF"

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

R="$WORK/repo"; mkdir -p "$R"; git -C "$R" init -q
echo 'data/' > "$R/.gitignore"; git -C "$R" add .gitignore
git -C "$R" -c user.email=t@t -c user.name=t commit -q -m init
OLD="$(git -C "$R" rev-parse HEAD)"
WT="$R/data/walkthroughs"

# mk SLUG [PYCODE] [--nosign]: write signed green evidence for HEAD to $WT/SLUG/evidence.json,
# after applying PYCODE to the evidence dict (ev). With --nosign the hmac is left out.
mk() { mkdir -p "$WT/$1"; python3 - "$WT/$1/evidence.json" "$KEYF" "$OLD" "${2:-}" "${3:-}" <<'PY'
import hashlib, hmac, json, sys
p, kf, head, code, nosign = sys.argv[1:6]
ev = {"allPassed": True, "commandsTotal": 1, "commandsFailed": 0,
      "commands": [{"cmd": "test -f .gitignore", "exit": 0, "passed": True}],
      "git": {"head": head, "treeStable": True}}
exec(code)
if nosign != "--nosign":
    key = bytes.fromhex(open(kf).read().strip())
    ev["hmac"] = hmac.new(key, json.dumps(ev, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8"), hashlib.sha256).hexdigest()
json.dump(ev, open(p, "w"))
PY
}

# Fixtures, all built before the checksum is taken.
mk green
mk notverify 'ev = {"allPassed": True}'
mk red 'ev["commands"][0]["exit"] = 1; ev["commandsFailed"] = 1; ev["allPassed"] = False'
mk trivial 'ev["commands"] = [{"cmd": c, "exit": 0, "passed": True} for c in ("true", "echo ok", ":", "exit 0")]; ev["commandsTotal"] = 4'
mk unstable 'ev["git"]["treeStable"] = False'
mk nohead 'ev["git"] = {"treeStable": True}'
mk otherhead 'ev["git"]["head"] = "f" * 40'
mk unsigned '' --nosign
mk tampered
python3 -c 'import json,sys; p=sys.argv[1]; ev=json.load(open(p)); ev["commands"][0]["cmd"]="make test"; json.dump(ev, open(p,"w"))' "$WT/tampered/evidence.json"
mkdir -p "$R/data/other"; cp "$WT/green/evidence.json" "$R/data/other/evidence.json"
mkdir -p "$WT/bad"; echo 'not json' > "$WT/bad/evidence.json"
NOKEY="$WORK/nokey"; mkdir -p "$NOKEY/.claude/vajra"
NOGIT="$WORK/nogit"; mkdir -p "$NOGIT"
git -C "$R" -c user.email=t@t -c user.name=t commit -q --allow-empty -m later
NEW="$(git -C "$R" rev-parse HEAD)"
mk greennew "ev[\"git\"][\"head\"] = \"$NEW\""

# snap: a hash of every path and file content under HOME, the campaign dir and the repo.
snap() { python3 - "$HOME" "$R" <<'PY'
import hashlib, os, sys
h = hashlib.sha256()
for top in sys.argv[1:]:
    for d, dirs, files in sorted(os.walk(top)):
        dirs.sort()
        h.update(("D " + d + "\n").encode())
        for f in sorted(files):
            p = os.path.join(d, f)
            h.update(("F " + p + "\n").encode())
            if not os.path.islink(p):
                h.update(open(p, "rb").read())
print(h.hexdigest())
PY
}
# E7 fixtures: archive the green bundle through vajra-campaign.sh (temp HOME, its own key file).
CAMP="$REPO/scripts/vajra-campaign.sh"
bash "$CAMP" create arc-ce --feature "ac-1|x" >/dev/null 2>&1
printf '{"schema":"vajra.tasks/v1","tasks":[{"id":"t1","status":"done"},{"id":"t2","status":"done"},{"id":"t3","status":"done"},{"id":"t4","status":"done"}]}\n' > "$HOME/.claude/vajra/campaigns/arc-ce/tasks.json"
LATER="$(git -C "$R" rev-parse HEAD)"; git -C "$R" checkout -q --detach "$OLD"
printf 'diff\n' > "$WT/green/diff.patch"; printf 'ok\n' > "$WT/green/cmd-1.log"
( cd "$R" && bash "$CAMP" archive-evidence arc-ce t1 "$WT/green" '["npm ci"]' >/dev/null )
ARC="$HOME/.claude/vajra/campaigns/arc-ce/evidence/t1/data/walkthroughs/green"
( cd "$R" && bash "$CAMP" archive-evidence arc-ce t2 "$WT/green" '[]' >/dev/null )
ARC2="$HOME/.claude/vajra/campaigns/arc-ce/evidence/t2/data/walkthroughs/green"
printf 'edited\n' >> "$ARC2/cmd-1.log"
( cd "$R" && bash "$CAMP" archive-evidence arc-ce t3 "$WT/green" '[]' >/dev/null )
ARC3="$HOME/.claude/vajra/campaigns/arc-ce/evidence/t3/data/walkthroughs/green"
python3 -c 'import json,sys; p=sys.argv[1]; m=json.load(open(p)); m["files"]["setup.json"]="0"*64; json.dump(m, open(p,"w"))' "$ARC3/manifest.json"
( cd "$R" && bash "$CAMP" archive-evidence arc-ce t4 "$WT/green" '[]' >/dev/null )
ARC4="$HOME/.claude/vajra/campaigns/arc-ce/evidence/t4/data/walkthroughs/green"
printf 'x\n' > "$ARC4/extra.txt"
git -C "$R" checkout -q --detach "$LATER"
bash "$CAMP" create arc-other --feature "ac-1|x" >/dev/null 2>&1
# Review #1: a signed archive copied to another task or another campaign must not pass.
cp -R "$HOME/.claude/vajra/campaigns/arc-ce/evidence/t1" "$HOME/.claude/vajra/campaigns/arc-ce/evidence/t9"
mkdir -p "$HOME/.claude/vajra/campaigns/arc-other/evidence"
cp -R "$HOME/.claude/vajra/campaigns/arc-ce/evidence/t1" "$HOME/.claude/vajra/campaigns/arc-other/evidence/t1"
ARC9="$HOME/.claude/vajra/campaigns/arc-ce/evidence/t9/data/walkthroughs/green"
ARCO="$HOME/.claude/vajra/campaigns/arc-other/evidence/t1/data/walkthroughs/green"

BEFORE="$(snap)"

# ce [ENV...] -- ARGS: run check-evidence --json in the repo; sets OUT and RC.
ce() { OUT="$( cd "$R" && python3 "$PY" check-evidence "$@" --json 2>/dev/null )"; RC=$?; }
# is_green / is_refused PATTERN: assert the JSON shape of OUT.
shape() { printf '%s' "$OUT" | python3 -c 'import json,re,sys
d = json.load(sys.stdin); want, pat = sys.argv[1], sys.argv[2]
ok = d.get("ok") is True and set(d) >= {"ok", "green", "reasons"} and isinstance(d["reasons"], list)
if want == "green":
    ok = ok and d["green"] is True and d["reasons"] == []
else:
    ok = ok and d["green"] is False and len(d["reasons"]) == 1 and isinstance(d["reasons"][0], str) and re.search(pat, d["reasons"][0]) is not None
sys.exit(0 if ok else 1)' "$1" "${2:-}" 2>/dev/null; }
green() { ce "$2" "${@:3}"; [ "$RC" = 0 ] && shape green; check "$1 -> {ok:true,green:true,reasons:[]} exit 0 (rc=$RC out=$OUT)" $?; }
refused() { local name="$1" pat="$2"; shift 2; [ "$RC" = 0 ] && shape refused "$pat"; check "$name -> {ok:true,green:false,reasons:[\"$pat\"]} exit 0 (rc=$RC out=$OUT)" $?; }

# --- ac-1: green ---
green "ac-1 green: signed green evidence for --head" "$WT/green/evidence.json" --head "$OLD"
green "ac-1 green: signed green evidence for the current HEAD" "$WT/greennew/evidence.json"
green "ac-1 green: abbreviated --head" "$WT/green/evidence.json" --head "${OLD:0:12}"

# --- ac-1: one case per Fail(3) refusal branch of read_evidence() ---
OUT="$( cd "$NOGIT" && GIT_CEILING_DIRECTORIES="$WORK" python3 "$PY" check-evidence "$WT/green/evidence.json" --json 2>/dev/null )"; RC=$?
refused "ac-1 refusal: not run inside a git repository" "not run inside a git repository"
ce "$R/data/other/evidence.json"; refused "ac-1 refusal: wrong location" "data/walkthroughs/<slug>/evidence.json"
ce "$WT/notverify/evidence.json"; refused "ac-1 refusal: not a verify-work evidence.json" "not a verify-work evidence.json"
ce "$WT/red/evidence.json" --head "$OLD"; refused "ac-1 refusal: not green" "not a green verify-work run"
ce "$WT/trivial/evidence.json" --head "$OLD"; refused "ac-1 refusal: every check trivial" "every check is trivial"
ce "$WT/unstable/evidence.json" --head "$OLD"; refused "ac-1 refusal: tree not stable" "working tree was not stable"
ce "$WT/nohead/evidence.json" --head "$OLD"; refused "ac-1 refusal: no head recorded" "no head commit recorded"
ce "$WT/otherhead/evidence.json"; refused "ac-1 refusal: head is not the current HEAD" "is not the current HEAD"
ce "$WT/green/evidence.json"; refused "ac-1 refusal: evidence for an older HEAD" "is not the current HEAD"
ce "$WT/green/evidence.json" --head "$NEW"; refused "ac-1 refusal: head is not the named --head" "is not $NEW"
OUT="$( cd "$R" && HOME="$NOKEY" python3 "$PY" check-evidence "$WT/green/evidence.json" --head "$OLD" --json 2>/dev/null )"; RC=$?
refused "ac-1 refusal: no Vajra key" "no Vajra key"
[ ! -e "$NOKEY/.claude/vajra/.hmac-key" ]; check "ac-1 no key is created" $?
ce "$WT/unsigned/evidence.json" --head "$OLD"; refused "ac-1 refusal: hmac missing" "signature \\(hmac\\) missing or invalid"
ce "$WT/tampered/evidence.json" --head "$OLD"; refused "ac-1 refusal: hmac invalid (tampered)" "signature \\(hmac\\) missing or invalid"

# --- ac-1: unreadable input exits 1 ---
ce "$WT/missing/evidence.json"; [ "$RC" = 1 ]; check "ac-1 unreadable: missing file -> exit 1 (rc=$RC)" $?
printf '%s' "$OUT" | grep -q '"ok": false'; check "ac-1 unreadable: --json reports ok:false" $?
ce "$WT/bad/evidence.json"; [ "$RC" = 1 ]; check "ac-1 unreadable: invalid JSON -> exit 1 (rc=$RC)" $?

# --- ac-1: bad usage exits 2 ---
ce; [ "$RC" = 2 ]; check "ac-1 usage: no evidence path -> exit 2 (rc=$RC)" $?
ce "$WT/green/evidence.json" --head zzz; [ "$RC" = 2 ]; check "ac-1 usage: invalid --head -> exit 2 (rc=$RC)" $?
ce "$WT/green/evidence.json" --he "$OLD"; [ "$RC" = 2 ]; check "ac-1 usage: abbreviated option refused -> exit 2 (rc=$RC)" $?
ce "$WT/green/evidence.json" --bogus; [ "$RC" = 2 ]; check "ac-1 usage: unknown option -> exit 2 (rc=$RC)" $?

# --- E7: --archived <slug> checks evidence archived in that campaign, plus its signed manifest ---
green "E7 archived green evidence with a valid manifest" "$ARC/evidence.json" --archived arc-ce --head "$OLD"
ce "$ARC/evidence.json" --head "$OLD"; refused "E7 archived path without --archived is outside the repo" 'data/walkthroughs'
ce "$WT/green/evidence.json" --archived arc-ce --head "$OLD"; refused "E7 --archived refuses a repo path" 'evidence/<task>'
ce "$ARC/evidence.json" --archived arc-other --head "$OLD"; refused "E7 --archived names another campaign" 'evidence/<task>'
ce "$ARC2/evidence.json" --archived arc-ce --head "$OLD"; refused "E7 a file changed after archiving" 'manifest'
ce "$ARC3/evidence.json" --archived arc-ce --head "$OLD"; refused "E7 an edited manifest (hmac no longer verifies)" 'manifest'
ce "$ARC4/evidence.json" --archived arc-ce --head "$OLD"; refused "E7 a file not in the manifest" 'manifest'
ce "$ARC/evidence.json" --archived arc-ce; refused "E7 archived evidence still checks --head against the repo" 'head'
ce "$ARC9/evidence.json" --archived arc-ce --head "$OLD"; refused "E7 an archive copied to another task" 'manifest'
ce "$ARCO/evidence.json" --archived arc-other --head "$OLD"; refused "E7 an archive copied to another campaign" 'manifest'
ce "$ARC/evidence.json" --archived ../x --head "$OLD"; [ "$RC" = 2 ]; check "E7 invalid --archived slug -> exit 2 (rc=$RC)" $?

# --- ac-1: human output without --json ---
( cd "$R" && python3 "$PY" check-evidence "$WT/green/evidence.json" --head "$OLD" 2>/dev/null ) | grep -q '^green'; check "ac-1 human output says green" $?
( cd "$R" && python3 "$PY" check-evidence "$WT/red/evidence.json" --head "$OLD" 2>/dev/null ) | grep -q '^not green'; check "ac-1 human output says not green" $?

# --- ac-1: read-only ---
[ "$(snap)" = "$BEFORE" ]; check "ac-1 read-only: no file under HOME, the campaign dir or the repo changed" $?
[ ! -d "$CD/.tasks.lock" ]; check "ac-1 read-only: no campaign lock taken" $?

echo "----------------------------------------"
echo "check-evidence: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
