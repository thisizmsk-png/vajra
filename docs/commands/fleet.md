# /vajra fleet <crew> <brief>

Runs one saved crew workflow from `~/.claude/workflows/` (installed by
`scripts/install.sh`). Crews are defined in `config/default.json` under
`cortex.fleetCrews`.

| Crew | Workflow | Lead and members |
|---|---|---|
| `feature-build` | `vajra-feature-build` | arjuna; bhima, nakula, vidura |
| `security-audit` | `vajra-security-audit` | bhishma; shakuni, duryodhana, vidura |
| `incident-response` | `vajra-incident-response` | ashwatthama; hanuman, bhima, vidura |
| `architecture-review` | `vajra-architecture-review` | yudhishthira; arjuna, bhishma, draupadi |
| `sabha` | `vajra-sabha` | debate with a vote counted in code; yudhishthira synthesizes, duryodhana is the adversary and does not vote |

Steps:

1. If the word after `fleet` is not a crew name, the input is an ask, not a
   crew run (`/vajra fleet management dashboard`): follow `deliver.md` with the
   whole text as the ask, and mention the crew table in one line.
2. Launch the workflow with the brief as its args. The protocol is in
   `~/.claude/skills/vajra/fleet/coordinator.md`.
3. Handle the return `{status: done|need_input|rejected|error, node, artifacts, question?}`
   the same way `deliver.md` describes.

Writers run in worktrees and reviewers are read-only. A crew never pushes,
deploys or opens a PR; it returns a plan for the user to approve.
If you don't know which crew fits, use a bare `/vajra <ask>` and let
`vajra-deliver` plan.
