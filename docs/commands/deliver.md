# /vajra <ask> (and /vajra deliver <ask>)

The default. A bare ask, meaning anything whose first word is not a subcommand,
goes to the saved `vajra-deliver` workflow. `scripts/install.sh` installs it
into `~/.claude/workflows/vajra-deliver.js`.

## What to do

1. Launch the `vajra-deliver` workflow (Workflow tool, or `/vajra-deliver`) with
   the ask verbatim, a run id and the session id:
   `args: {"ask": "<the ask>", "run_id": "<UTC now as YYYYMMDD-HHMM>", "session_id": "<session_id>"}`,
   for example `"run_id": "20261011-0930"` (get it with `date -u +%Y%m%d-%H%M`).
   Branch names (`vajra/<run_id>/...`) and the campaign (`deliver-<run_id>`)
   come from it, so a rerun of the same ask doesn't overwrite the last one.
   The workflow never reads the clock itself. `session_id` is the hook
   payload's `session_id`, which the router's context line gives you. It
   must match `^[A-Za-z0-9-]{8,}$`, otherwise the workflow stops at `input`.
   The knowledge write-back passes it as `vajra_kb.py add --session`. Without
   it, no knowledge records are written, because `vajra_kb.py` refuses
   proposed and verified writes without a session id; the report lists this
   as an open risk. Don't ask the user which crew,
   agent or step to use, and don't pre-plan in the main session.
2. If the router's context line lists crews, agents or skills, pass them along
   as hints: `args.hints = {"crews": [...], "agents": [...], "skills": [...]}`.
   The workflow is free to ignore them. Optional: `max_rounds` (default 3),
   `base` (a commit sha to build on instead of `HEAD`), `maxInFlight` (how
   many workers run at once, 1 to 16, default 4).
3. If an active campaign matches the ask, pass its slug (`args.campaign`) so
   progress is recorded there. Otherwise the workflow creates one with
   `scripts/vajra-campaign.sh`.

## What the workflow does

1. **Understand.** The first agent searches the project knowledge base
   (`vajra_kb.py search "<ask>" --format ids --k 5`, the ask passed as a
   quoted argument). The hits, one-line pointers, go to the next agents as
   data. Then drona restates the ask, acceptance criteria, scope and
   assumptions. It picks the most reasonable reading of anything ambiguous and
   records it.
2. **Plan.** Breaks the work into tasks, each with an owner agent picked from
   the roster by charter, the `files` (paths or globs) it may change, its
   `deps`, and up to 5 design decisions worth remembering. The workflow
   registers the tasks in the campaign's task graph with
   `scripts/vajra_tasks.py` (`add`, then `validate`), under ids namespaced by
   the run (`<run_id>.t1`).
3. **Execute.** Tasks run in waves: a wave starts once every task it depends
   on has finished, and tasks whose files overlap go in separate waves. Each
   wave runs in parallel, at most `maxInFlight` workers at once. The script
   computes the waves itself with the same algorithm as
   `vajra_tasks.py waves`. Writers run in git worktrees.
4. **Workers.** Before any other work, each worker claims its task with
   `vajra_tasks.py claim <id> <owner> --stacked`. `--stacked` lets a task
   whose dependencies have a `done` report (not yet closed) be claimed, since
   dependents build on their dependencies' unverified commits within one run.
   If the claim exits 3 (another worker holds the task, or it is not ready),
   the worker stops and returns `blocked` with the error. Any other failure,
   such as a task graph that could not be registered, is not fatal. The
   same worker re-claiming its task in a later fix round is a no-op. Then the
   worker reports `in_progress`. Task ids are namespaced by run, so a resumed
   campaign gets fresh tasks; there is no "already done" shortcut. The worker
   reports through `vajra_tasks.py report` and returns
   `{status: progress|done|blocked|question, summary, artifacts, evidence_cmds}`.
   `done` is only a claim. `progress` reruns the task next round from its
   partial commit; `blocked` and `question` come back to you. Workers pass on
   artifact paths and commits, never another worker's summary.
5. **Verify.** vidura runs real checks against every criterion and lists the
   `setup_cmds` it ran first to prepare its fresh worktree (for example
   `npm ci`). arjuna reviews
   read-only, and bhishma does too when the work touches security. The gate
   is computed in code from that evidence.
6. **Fix loop.** Repeats until every criterion has passing evidence, up to 3
   rounds.
7. **Deliver.** Merges worker branches into one local branch
   (`vajra/<run_id>/integration`) and leaves it free to check out. Then it
   writes back in a fresh worktree detached at the integration commit. It
   runs vidura's `setup_cmds` (at most 5), then reruns the passing checks
   through `scripts/vajra-verify-work.sh`, which writes a signed
   `data/walkthroughs/deliver-<run_id>/evidence.json`. It closes each task in
   dependency order with
   `vajra_tasks.py close --evidence <evidence.json> --head <integration commit>`.
   It records knowledge with
   `vajra_kb.py add --source verified --evidence <evidence.json> --session <session_id>`:
   one `delivery` record, the plan's decisions, and one `lesson` for each fix
   round that went from failing to passing.
   - `close` needs a `claimed` or `in_progress` task whose dependencies are
     all `done`, plus green evidence.
   - Green means every command exited 0 and `treeStable` is true. `git.head`
     must be the named commit, the file must be
     `<repo_root>/data/walkthroughs/<slug>/evidence.json`, and its `hmac`
     must verify with the Vajra key. Not every command may be trivial
     (`true`, `:`, `echo`, `exit 0`).
   - If the knowledge gate refuses a verified record (exit 3), the workflow
     runs its `--source proposed` twin, so the record waits in `pending/`
     instead of being dropped. The report counts these records.
   - If the rerun is not green, no task is closed and every record goes in as
     `--source proposed`, waiting for `atman.sh review`.
   - Write-back failures are open risks, never fatal.
   Finally it closes the campaign and returns a markdown report: what was
   built, evidence per criterion, knowledge recorded, how to run it, open
   risks, cleanup of the `vajra/<run_id>/*` work branches.

## Handling the result

The workflow returns `{status, node, artifacts, question?}`; on `done` it also
returns `branch`, `commit`, `report` (markdown text, not a file) and
`pending_approval`.

- `done`: show `report` as is (it names the branch and the evidence per
  criterion). Nothing is written to disk besides the branches, the campaign
  (with its `tasks.json` task graph), the knowledge records and the
  walkthrough evidence. `artifacts.knowledge` says whether the records went in
  as verified, as proposed, or not at all (`none`, for example with no
  `session_id`). It also says whether the tasks were closed and how many
  refused verified records fell back to proposed (`proposed_fallbacks`).
- `need_input`: ask the user `question` once, then relaunch with the answer
  appended to the ask, a new `run_id`, the same `session_id`, and
  `"campaign": artifacts.campaign` when the result has one. After the fix loop gives up (`node: fix_loop`),
  `/vajra continue` relaunches with just the campaign; see
  [campaign.md](campaign.md).
- `rejected` or `error`: show `node` and the reason. Don't retry silently.

The workflow never pushes, opens a PR, deploys, runs migrations or spends
money. Those come back in the report as a ready plan. Run them only after the
user approves through Claude Code's permission prompt.

If `~/.claude/workflows/vajra-deliver.js` is missing, say so and suggest
`bash ~/.claude/skills/vajra/scripts/install.sh` and `/vajra doctor`. Don't
improvise the workflow by hand.
