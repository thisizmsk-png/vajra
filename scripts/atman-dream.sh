#!/usr/bin/env bash
# atman-dream.sh: prune the practice log (order and HMAC chain preserved, old lines
# archived), then write new proposals. Never edits config/, skills or agents.
# Kept for compatibility; same as: scripts/atman.sh dream
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
exec bash "${HERE}/atman.sh" dream "$@"
