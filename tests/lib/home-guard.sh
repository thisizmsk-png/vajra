# Real-home tripwire for the test runner. home_fp <home> prints a content hash of each file
# no test may change: Vajra's manifest signature and tamper flag, the installed manifest,
# Claude Code settings and the agents folder. Content, not mtime, so another process
# rewriting the same bytes is not a false alarm. Never names the HMAC key. Bash 3.2.
home_fp() {
  local c="$1/.claude" f
  for f in "$c/vajra/manifest.sig" "$c/vajra/.integrity-violation" \
           "$c/skills/vajra/manifest.json" "$c/settings.json" "$c/settings.local.json" \
           "$c"/agents/*; do
    if [ -f "$f" ]; then echo "$f $(shasum -a 256 < "$f" | cut -c1-64)"; else echo "$f absent"; fi
  done
}
