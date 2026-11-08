#!/usr/bin/env bash
# atman.sh: the Atman learning loop CLI.
#
#   atman.sh status              integrity, recent failures, flagged skills/agents, counts
#   atman.sh propose             turn verified signals into proposals (proposals/ only)
#   atman.sh review              list pending proposals with their evidence, and
#                                proposed knowledge records (scripts/vajra_kb.py pending)
#   atman.sh approve <id>        human only: route -> learned-routes.json,
#                                skill/agent change -> patches/<id>.patch (you apply it),
#                                kb-<id> -> delegated to vajra_kb.py approve (into the kb)
#   atman.sh reject <id> [why]   human only (kb-<id> -> vajra_kb.py reject)
#   atman.sh log                 approval / rejection history
#   atman.sh pause | resume      stop / restart proposing (observing continues)
#   atman.sh observe | dream | verify
#
# Nothing here writes to config/, the skill core, skills or agents.
# NFR-SEC-2: atman.py reads the HMAC key, so it is started as /usr/bin/python3 on the system PATH, never a python3 a
# same-uid worker planted in a user dir such as ~/.local/bin.
PATH="/usr/bin:/bin:/usr/sbin:/sbin"; export PATH
PYTHON3="/usr/bin/python3"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
[ -x "$PYTHON3" ] || { echo "atman: python3 is required at $PYTHON3 (xcode-select --install provides it)" >&2; exit 1; }
exec "$PYTHON3" -I "${HERE}/../atman/atman.py" "$@"
