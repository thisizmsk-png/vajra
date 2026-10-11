#!/usr/bin/env bash
# atman-observe.sh: read-only report of failure clusters per skill/agent (last 7 days).
# Kept for compatibility; same as: scripts/atman.sh observe
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
exec bash "${HERE}/atman.sh" observe "$@"
