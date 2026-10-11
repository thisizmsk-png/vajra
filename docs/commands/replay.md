# /vajra replay [session]

Runs `bash ~/.claude/skills/vajra/scripts/vajra-replay.sh [session]`
(`--list` shows the available sessions). It reconstructs the tool-call trace
from the session's HMAC-chained event log and verifies the chain, so edits,
reordering or deletions show up as a broken link.

Show the trace summary and the integrity verdict. If the chain fails, say so
plainly and don't trust that log.
