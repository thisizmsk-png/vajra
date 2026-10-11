# /vajra <ask> (and /vajra deliver <ask>)

The default. A bare ask, meaning anything whose first word is not a subcommand,
goes to the saved `vajra-deliver` workflow. `scripts/install.sh` installs it
into `~/.claude/workflows/vajra-deliver.js`.

## What to do

1. Launch the `vajra-deliver` workflow (Workflow tool, or `/vajra-deliver`) with
   the ask verbatim and a run id:
   `args: {"ask": "<the ask>", "run_id": "<UTC now as YYYYMMDD-HHMM>"}`, for
   example `"run_id": "20261011-0930"` (get it with `date -u +%Y%m%d-%H%M`).
   Branch names (`vajra/<run_id>/...`) and the campaign (`deliver-<run_id>`)
   come from it, so a rerun of the same ask doesn't overwrite the last one.
   The workflow never reads the clock itself. Don't ask the user which crew,
   agent or step to use, and don't pre-plan in the main session.
2. If the router's context line lists crews, agents or skills, pass them along
   as hints: `args.hints = {"crews": [...], "agents": [...], "skills": [...]}`.
   The workflow is free to ignore them. Optional: `max_rounds` (default 3),
   `base` (a commit sha to build on instead of `HEAD`).
3. If an active campaign matches the ask, pass its slug (`args.campaign`) so
   progress is recorded there. Otherwise the workflow creates one with
   `scripts/vajra-campaign.sh`.

## What the workflow does

1. **Understand.** Restates the ask, acceptance criteria, scope and
   assumptions. It picks the most reasonable reading of anything ambiguous and
   records it.
2. **Plan.** Breaks the work into tasks, each with an owner agent picked from
   the roster by charter, plus dependencies.
3. **Execute.** Independent tasks run in parallel; writers run in git worktrees.
4. **Verify.** vidura runs real checks against every criterion. arjuna reviews
   read-only, and bhishma does too when the work touches security.
5. **Fix loop.** Repeats until every criterion has passing evidence, up to 3
   rounds.
6. **Deliver.** Merges worker branches into one local branch
   (`vajra/<run_id>/integration`), leaves it free to check out, closes the
   campaign and returns a markdown report: what was built, evidence per
   criterion, how to run it, open risks, cleanup of the `vajra/<run_id>/*`
   work branches.

## Handling the result

The workflow returns `{status, node, artifacts, question?}`; on `done` it also
returns `branch`, `commit`, `report` (markdown text, not a file) and
`pending_approval`.

- `done`: show `report` as is (it names the branch and the evidence per
  criterion). Nothing is written to disk besides the branches and the campaign.
- `need_input`: ask the user `question` once, then relaunch with the answer
  appended to the ask, a new `run_id`, and `"campaign": artifacts.campaign`
  when the result has one. After the fix loop gives up (`node: fix_loop`),
  `/vajra continue` relaunches with just the campaign; see
  [campaign.md](campaign.md).
- `rejected` or `error`: show `node` and the reason. Don't retry silently.

The workflow never pushes, opens a PR, deploys, runs migrations or spends
money. Those come back in the report as a ready plan. Run them only after the
user approves through Claude Code's permission prompt.

If `~/.claude/workflows/vajra-deliver.js` is missing, say so and suggest
`bash ~/.claude/skills/vajra/scripts/install.sh` and `/vajra doctor`. Don't
improvise the workflow by hand.
