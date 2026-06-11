#!/usr/bin/env bash
# Test for scripts/vajra-ralph.sh — hard-reset iteration loop control.
# Uses a mock agent (CLAUDE_CMD) so no real model is invoked.
# Run: bash tests/run-ralph.sh
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO/scripts/vajra-ralph.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vajra-ralph-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0; FAIL=0
check() { if [ "$2" -eq 0 ]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); echo "FAIL :: $1"; fi; }

# Mock agent: checks off the first unchecked todo in $RALPH_TEST_PROGRESS.
MOCK_TICK="$WORK/mock_tick.sh"
cat > "$MOCK_TICK" <<'EOF'
#!/usr/bin/env bash
# ignore the -p prompt; tick the first unchecked box in the known progress file
awk 'd==0 && /- \[ \]/{sub(/\[ \]/,"[x]"); d=1} {print}' "$RALPH_TEST_PROGRESS" > "$RALPH_TEST_PROGRESS.tmp" \
  && mv "$RALPH_TEST_PROGRESS.tmp" "$RALPH_TEST_PROGRESS"
exit 0
EOF
chmod +x "$MOCK_TICK"

# Mock agent that does nothing (never progresses).
MOCK_NOOP="$WORK/mock_noop.sh"
printf '#!/usr/bin/env bash\nexit 0\n' > "$MOCK_NOOP"; chmod +x "$MOCK_NOOP"

# Mock agent that writes RALPH-COMPLETE on first call.
MOCK_DONE="$WORK/mock_done.sh"
cat > "$MOCK_DONE" <<'EOF'
#!/usr/bin/env bash
echo "RALPH-COMPLETE" >> "$RALPH_TEST_PROGRESS"
exit 0
EOF
chmod +x "$MOCK_DONE"

mkprog() { printf '# Goal: test\n\n## Steps\n- [ ] one\n- [ ] two\n- [ ] three\n' > "$1"; }

# 1. converges: 3 todos, mock ticks 1/iter → done in 3
P="$WORK/p1.md"; mkprog "$P"
out="$(CLAUDE_CMD="$MOCK_TICK" RALPH_TEST_PROGRESS="$P" bash "$SCRIPT" "$P" 25 2>&1)"; rc=$?
[ "$rc" -eq 0 ]; check "converges -> exit 0" $?
echo "$out" | grep -q "complete after 3 iteration"; check "stops exactly when all todos checked (3)" $?
! grep -qE '^\s*-\s*\[ \]' "$P"; check "no unchecked todos remain" $?

# 2. max-iters cap: noop agent, max 2 → exit 1
P2="$WORK/p2.md"; mkprog "$P2"
out2="$(CLAUDE_CMD="$MOCK_NOOP" RALPH_TEST_PROGRESS="$P2" bash "$SCRIPT" "$P2" 2 2>&1)"; rc2=$?
[ "$rc2" -eq 1 ]; check "no progress + max-iters -> exit 1" $?
echo "$out2" | grep -q "hit max 2"; check "reports hitting the cap" $?

# 3. RALPH-COMPLETE marker short-circuits
P3="$WORK/p3.md"; mkprog "$P3"
CLAUDE_CMD="$MOCK_DONE" RALPH_TEST_PROGRESS="$P3" bash "$SCRIPT" "$P3" 25 >/dev/null 2>&1; rc3=$?
[ "$rc3" -eq 0 ]; check "RALPH-COMPLETE marker -> exit 0" $?

# 4. already-done progress: 0 iterations
P4="$WORK/p4.md"; printf '# Goal\n- [x] a\n- [x] b\n' > "$P4"
out4="$(CLAUDE_CMD="$MOCK_NOOP" RALPH_TEST_PROGRESS="$P4" bash "$SCRIPT" "$P4" 25 2>&1)"; rc4=$?
[ "$rc4" -eq 0 ]; check "already-done -> exit 0" $?
echo "$out4" | grep -q "after 0 iteration"; check "already-done runs 0 iterations" $?

# 5. usage / missing file
bash "$SCRIPT" >/dev/null 2>&1; [ $? -eq 2 ]; check "no args -> usage error" $?
bash "$SCRIPT" "$WORK/nope.md" >/dev/null 2>&1; [ $? -eq 2 ]; check "missing file -> error" $?

echo "----------------------------------------"
echo "ralph: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
