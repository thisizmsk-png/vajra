# Vajra Verification — Proof-of-Work Before "Done"

The dominant failure mode of agent harnesses is **claiming completion without
evidence** — agents "prioritize appearing helpful over being correct," game
tests, and report success that isn't real. Vajra closes this with a hard rule:

> **"Done" requires captured evidence, not the agent's say-so.** Before claiming
> a task is complete, run the verification commands, capture their real output,
> and produce a walkthrough artifact. If a check fails, it is not done.

## The gate

```
/vajra verify-work <slug> "<cmd1>" "<cmd2>" ...
  → runs scripts/vajra-verify-work.sh
  → captures: git diff, each command's exit + output, a WALKTHROUGH.md
  → exits NON-ZERO if any command failed
```

The agent MUST NOT report success while this gate is red. The walkthrough bundle
(`data/walkthroughs/{slug}/`) is the proof a human or another agent can inspect:
`evidence.json` (machine-readable), `cmd-N.log` (captured output), `diff.patch`,
`WALKTHROUGH.md` (summary).

## What to verify (pick what the change can be exercised by)

- **Tests** — the test command for the touched code, with captured output
  (`npm test`, `pytest`, `cargo test`, `go test`).
- **Build / typecheck / lint** — `tsc --noEmit`, `ruff`, `mypy`, `npm run build`.
- **Behavior** — for a previewable change, a screenshot/recording (use the
  preview tooling); for a CLI change, the command run with expected output.
- **The red-team / review** — for security-relevant changes, `/vajra redteam`
  and `/vajra review` outputs belong in the evidence.

## Wiring

1. **Plan Mode** — the plan's `## Verification` block lists the commands; running
   `/vajra verify-work` against them closes the plan (`engine/plan-mode.md`).
2. **Campaigns** — attach the walkthrough path to the campaign's final
   checkpoint so the evidence is part of the persistent record.
3. **Stop-hook (optional, operator-installed)** — teams that want this enforced
   can add a Stop hook that refuses to end a session marked "complete" unless a
   fresh walkthrough exists. Vajra does not auto-install it (settings.json is the
   immutable core); the protocol + script are provided so you can.

## The discipline (say this, then do it)

- Never write "done / fixed / passing" without having just run the check and seen
  it pass — restate the captured result in the final message.
- If a step was skipped or a test failed, say so plainly with the output.
- A green walkthrough is the unit of "done." No walkthrough, not done.
