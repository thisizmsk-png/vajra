# /vajra plan <task>

Plan first, then act on the plan the user approves.

`/vajra plan and build a CSV export` (next word `and`, `&` or `then`) is not
plan mode: it is a bare ask, so follow `deliver.md`. Legacy `act`, `explore`
and `phase` land here only when nothing follows them.

1. Enter Claude Code's native plan mode (the user can also press Shift+Tab).
   Explore read-only.
2. Write the plan with the template in `~/.claude/skills/vajra/engine/plan-mode.md`:
   goal, acceptance criteria, steps with an owner agent each, risks, and the
   verification commands for `verify-work`.
3. Present it with ExitPlanMode. The user edits or approves it there; nothing
   runs before approval.
4. After approval, run the steps (delegate each to its owner agent), then
   `/vajra verify-work` with the plan's commands.

There is no `.phase` file any more: native plan mode is the read-only gate.
