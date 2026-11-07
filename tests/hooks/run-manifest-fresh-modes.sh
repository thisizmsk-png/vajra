#!/usr/bin/env bash
# run-manifest-fresh.sh is strict by default and only warns when VAJRA_MANIFEST_FRESH=warn (set by CI for pull_request).
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
command -v jq >/dev/null || { echo "SKIP: jq missing"; exit 77; }
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
mkdir -p "$T/tests/hooks"
cp "$REPO/tests/hooks/run-manifest-fresh.sh" "$T/tests/hooks/"
printf 'hello\n' > "$T/a.txt"
if command -v sha256sum >/dev/null 2>&1; then H="$(sha256sum "$T/a.txt" | cut -c1-64)"; else H="$(shasum -a 256 "$T/a.txt" | cut -c1-64)"; fi
git -C "$T" init -q
git -C "$T" add a.txt tests/hooks/run-manifest-fresh.sh
RUN="$T/tests/hooks/run-manifest-fresh.sh"
if command -v sha256sum >/dev/null 2>&1; then HS="$(sha256sum "$RUN" | cut -c1-64)"; else HS="$(shasum -a 256 "$RUN" | cut -c1-64)"; fi
P=0; F=0
ok()  { P=$((P+1)); }
bad() { F=$((F+1)); echo "FAIL :: $1"; }
manifest() { printf '{"files":{"a.txt":"%s","tests/hooks/run-manifest-fresh.sh":"%s"}}\n' "$1" "$HS" > "$T/manifest.json"; git -C "$T" add manifest.json 2>/dev/null; }
run() { env -u GITHUB_ACTIONS -u VAJRA_MANIFEST_FRESH "$@" bash "$RUN" 2>&1; }

manifest "$H"
out="$(run)"; [ $? -eq 0 ] && ok || bad "fresh manifest, default mode must pass: $out"
out="$(run VAJRA_MANIFEST_FRESH=warn)"; [ $? -eq 0 ] && ok || bad "fresh manifest, warn mode must pass: $out"

manifest "0000000000000000000000000000000000000000000000000000000000000000"
out="$(run)"; rc=$?
[ "$rc" -ne 0 ] && echo "$out" | grep -q "stale for a.txt" && ok || bad "stale manifest, default mode must block and name the file (rc=$rc): $out"
out="$(run VAJRA_MANIFEST_FRESH=strict)"; [ $? -ne 0 ] && ok || bad "an unknown mode value must stay strict"
out="$(run VAJRA_MANIFEST_FRESH=)"; [ $? -ne 0 ] && ok || bad "an empty mode value must stay strict"
out="$(run VAJRA_MANIFEST_FRESH=warn)"; rc=$?
[ "$rc" -eq 0 ] && echo "$out" | grep -q "stale for a.txt" && echo "$out" | grep -q "^WARN:" && ok || bad "stale manifest, warn mode must print the stale entry and exit 0 (rc=$rc): $out"
out="$(env GITHUB_ACTIONS=true VAJRA_MANIFEST_FRESH=warn bash "$RUN" 2>&1)"
echo "$out" | grep -q "^::warning::" && ok || bad "warn mode on Actions must emit an annotation: $out"

echo "manifest-fresh modes: $P passed, $F failed"
[ "$F" -eq 0 ]
