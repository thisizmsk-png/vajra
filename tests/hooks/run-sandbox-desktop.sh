#!/usr/bin/env bash
# E8 (Q10, AC-3.4.10): the shipped sandbox profile denies writes to the Vajra Desktop
# app dir (control queue included); enable-sandbox.sh prints it and writes nothing;
# vajra-doctor reports whether the rule is active. Temp HOME only.
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
command -v jq >/dev/null || { echo "SKIP: jq missing"; exit 77; }
P=0; F=0
ok() { if [ "$2" -eq 0 ]; then P=$((P+1)); else F=$((F+1)); echo "FAIL :: $1"; fi; }
DESK="~/Library/Application Support/VajraDesktop"

jq -e --arg d "$DESK" '.settings.sandbox.filesystem.denyWrite | index($d) != null' "$REPO/config/sandbox.json" >/dev/null
ok "profile denyWrite includes the VajraDesktop app dir" $?
jq -e '.optional.vajraDesktopControl == null' "$REPO/config/sandbox.json" >/dev/null
ok "no longer an opt-in extra" $?

H="$(mktemp -d)"; trap 'rm -rf "$H"' EXIT
mkdir -p "$H/.claude"
# Print-only means nothing under the Claude dir changes. (On Linux the script runs `npm ls -g`, which
# may write npm's own cache and logs under HOME; that is not Claude or Vajra state.)
CL_DIR="$H/.claude"
before="$(find "$CL_DIR" -type f | sort | xargs -I{} shasum {} 2>/dev/null)"
out="$(HOME="$H" bash "$REPO/scripts/enable-sandbox.sh" 2>&1)"
printf '%s' "$out" | grep -q 'Application Support/VajraDesktop'; ok "enable-sandbox shows the denyWrite entry" $?
[ "$before" = "$(find "$CL_DIR" -type f | sort | xargs -I{} shasum {} 2>/dev/null)" ] && [ ! -e "$CL_DIR/settings.json" ]
ok "enable-sandbox writes nothing in the Claude dir" $?

# The bare temp HOME has no install, so doctor exits 1; only its Desktop line matters.
doc() { HOME="$H" bash "$REPO/scripts/vajra-doctor.sh" > "$H/doc.log" 2>&1; grep -i "desktop" "$H/doc.log"; }
jq -n '{sandbox: {enabled: true, filesystem: {denyWrite: ["~/.claude/hooks"]}}}' > "$H/.claude/settings.json"
doc | grep -q '^WARN .*Vajra Desktop.*not active'; ok "doctor warns when the rule is missing" $?
jq -n --arg d "$DESK" '{sandbox: {enabled: true, filesystem: {denyWrite: [$d]}}}' > "$H/.claude/settings.json"
doc | grep -q '^PASS .*Vajra Desktop.*active'; ok "doctor passes when the rule is set and the sandbox is on" $?
jq -n --arg d "$DESK" '{sandbox: {enabled: false, filesystem: {denyWrite: [$d]}}}' > "$H/.claude/settings.json"
doc | grep -q '^WARN .*Vajra Desktop.*not active'; ok "doctor warns when the sandbox is off" $?
jq -n --arg d "$HOME/Library/Application Support/VajraDesktop/" '{sandbox: {enabled: true, filesystem: {denyWrite: [$d]}}}' | sed "s#$HOME#$H#" > "$H/.claude/settings.json"
doc | grep -q '^PASS .*Vajra Desktop.*active'; ok "doctor accepts the absolute path with a trailing slash" $?

echo "sandbox desktop: $P passed, $F failed"
[ "$F" -eq 0 ]
