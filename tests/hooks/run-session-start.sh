#!/usr/bin/env bash
# hooks/session-start.sh: local-signature integrity check, campaign line,
# post-compaction pointer. Output goes to stdout; the hook always exits 0.
#
# Run: bash tests/hooks/run-session-start.sh

set -uo pipefail
for t in jq openssl; do command -v "$t" >/dev/null || { echo "SKIP: $t required"; exit 0; }; done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_HOME="$(mktemp -d "${TMPDIR:-/tmp}/vajra-ss-XXXXXX")"
trap 'chmod -R u+w "$TEST_HOME" 2>/dev/null; rm -rf "$TEST_HOME"' EXIT
export HOME="$TEST_HOME"
SKILL="$HOME/.claude/skills/vajra"
VS="$HOME/.claude/vajra"
FLAG="$VS/.integrity-violation"
mkdir -p "$SKILL/hooks" "$SKILL/scripts" "$SKILL/config" "$VS"
cp "$REPO/hooks/session-start.sh" "$REPO/hooks/pre-tool-use.sh" "$SKILL/hooks/"
cp "$REPO/scripts/generate-manifest.sh" "$SKILL/scripts/"
echo '{"a":1}' > "$SKILL/config/default.json"
cat > "$SKILL/scripts/vajra-campaign.sh" <<'SH'
#!/usr/bin/env bash
[ "$1 $2" = "active --brief" ] && echo "login-flow: 2/5 features passing"
SH
SS="$SKILL/hooks/session-start.sh"
run_ss() { printf '%s' "${1:-{\"session_id\":\"s\",\"source\":\"startup\"\}}" | bash "$SS"; }

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); echo "FAIL :: $1"; fi; }

# 1. Not installed (no key, no signature): warn, never lock.
bash "$SKILL/scripts/generate-manifest.sh" >/dev/null 2>&1
out="$(run_ss)"; rc=$?
[ "$rc" -eq 0 ]; check "exit 0 when not set up" $?
[ ! -f "$FLAG" ]; check "no tamper flag when unsigned" $?
printf '%s' "$out" | grep -q "not set up"; check "warns that the manifest is unsigned" $?
printf '%s' "$out" | grep -q "Vajra campaign: login-flow"; check "prints active campaign line" $?

# 2. Key + local signature, clean tree.
openssl rand -hex 32 > "$VS/.hmac-key"; chmod 600 "$VS/.hmac-key"
bash "$SKILL/scripts/generate-manifest.sh" >/dev/null 2>&1
[ -s "$VS/manifest.sig" ]; check "generate-manifest signs into ~/.claude/vajra/manifest.sig" $?
[ ! -e "$SKILL/manifest.json.sig" ]; check "no signature shipped next to manifest.json" $?
out="$(run_ss)"; rc=$?
[ "$rc" -eq 0 ] && [ ! -f "$FLAG" ]; check "clean signed tree: no flag" $?
printf '%s' "$out" | grep -q "INTEGRITY"; [ $? -ne 0 ]; check "clean signed tree: no violation text" $?
bash "$SS" --check; check "--check exits 0 on a clean tree" $?

# 3. A shipped signature made with someone else's key is ignored.
echo "deadbeef" > "$SKILL/manifest.json.sig"
run_ss >/dev/null; [ ! -f "$FLAG" ]; check "foreign manifest.json.sig does not trip the check" $?
rm -f "$SKILL/manifest.json.sig"

# 4. Modified file -> flag naming it; the guard then blocks mutations.
echo "# change" >> "$SKILL/config/default.json"
out="$(run_ss)"; rc=$?
[ "$rc" -eq 0 ]; check "exit 0 even on violation" $?
[ -f "$FLAG" ] && grep -q "config/default.json" "$FLAG"; check "flag names the modified file" $?
printf '%s' "$out" | grep -q "INTEGRITY VIOLATION"; check "violation on stdout (model sees it)" $?
printf '%s' "$out" | grep -q "own terminal: bash ~/.claude/skills/vajra/scripts/generate-manifest.sh"; check "violation points at generate-manifest.sh in the user's terminal" $?
printf '%s' "$out" | grep -q "scripts/install.sh"; [ $? -ne 0 ]; check "violation does not suggest install.sh (it refuses to sign)" $?
printf '%s' '{"tool_name":"Write","tool_input":{"file_path":"/tmp/x"},"cwd":"/tmp"}' | bash "$SKILL/hooks/pre-tool-use.sh" 2>/dev/null; rc=$?
[ "$rc" -eq 2 ]; check "guard blocks Write while flagged" $?
bash "$SS" --check >/dev/null; [ $? -ne 0 ]; check "--check exits 1 on a modified tree" $?
bash "$SKILL/scripts/generate-manifest.sh" >/dev/null 2>&1
run_ss >/dev/null; [ ! -f "$FLAG" ]; check "flag clears after operator re-signs" $?

# 5. Manifest edited to match a tampered file -> signature mismatch.
echo "# evil" >> "$SKILL/config/default.json"
h="$(shasum -a 256 "$SKILL/config/default.json" 2>/dev/null | cut -d' ' -f1 || sha256sum "$SKILL/config/default.json" | cut -d' ' -f1)"
jq --arg h "$h" '.files["config/default.json"] = $h' "$SKILL/manifest.json" > "$SKILL/m.tmp" && mv "$SKILL/m.tmp" "$SKILL/manifest.json"
run_ss >/dev/null
[ -f "$FLAG" ] && grep -q "signature" "$FLAG"; check "re-hashed manifest fails the signature" $?
bash "$SKILL/scripts/generate-manifest.sh" >/dev/null 2>&1

# 6. Unlisted script dropped into scripts/ -> flagged.
echo 'echo pwned' > "$SKILL/scripts/evil.sh"
run_ss >/dev/null
[ -f "$FLAG" ] && grep -q "scripts/evil.sh" "$FLAG"; check "unlisted script flagged" $?
rm -f "$SKILL/scripts/evil.sh"
mkdir -p "$SKILL/scripts/__pycache__"; echo x > "$SKILL/scripts/__pycache__/r.pyc"
run_ss >/dev/null; [ ! -f "$FLAG" ]; check "__pycache__ is ignored" $?

# 7. Deleted file -> flagged.
mv "$SKILL/config/default.json" "$TEST_HOME/held.json"
run_ss >/dev/null; [ -f "$FLAG" ] && grep -q "missing: config/default.json" "$FLAG"; check "deleted file flagged" $?
mv "$TEST_HOME/held.json" "$SKILL/config/default.json"
run_ss >/dev/null

# 8. After compaction, point at the pinned checkpoint.
mkdir -p "$VS/compaction"
printf 'git snapshot: abc\ngit ref: refs/vajra/compact/20261011T000000Z-s\n' > "$VS/compaction/s-x.md"
out="$(run_ss '{"session_id":"s","source":"compact"}')"
printf '%s' "$out" | grep -q "refs/vajra/compact/20261011T000000Z-s"; check "compact source points at the checkpoint ref" $?

# 9. Campaign prefix appears exactly once, whatever the brief prints.
for brief in "login-flow: 2/5 features passing" "Vajra campaign: login-flow: 2/5 features passing" "Vajra campaign login-flow: 2/5 features passing"; do
  printf '#!/usr/bin/env bash\necho "%s"\n' "$brief" > "$SKILL/scripts/vajra-campaign.sh"
  out="$(run_ss)"
  [ "$(printf '%s\n' "$out" | grep -c '^Vajra campaign: login-flow: 2/5 features passing$')" = 1 ] && ! printf '%s' "$out" | grep -q 'Vajra campaign: Vajra campaign'
  check "one campaign prefix for brief: $brief" $?
done
: > "$SKILL/scripts/vajra-campaign.sh"
out="$(run_ss)"; printf '%s' "$out" | grep -q "Vajra campaign"; [ $? -ne 0 ]; check "no campaign line when the brief is empty" $?

# 10. Garbage stdin never fails the hook.
printf 'not json' | bash "$SS" >/dev/null 2>&1; check "garbage stdin: exit 0" $?

echo "----------------------------------------"
echo "session-start.sh: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
