# Vajra Fleet Mode — Crews Are Workflows

Fleet mode runs a **crew** of the Mahabharat agents on one task. In v0.5 a crew is a
saved Claude Code **dynamic workflow** (`workflows/vajra-<crew>.js`), not a shell
coordinator. The workflow runtime owns the orchestration: it spawns each agent with
the Agent tool, runs independent work in parallel, isolates writers in git worktrees,
journals every step for deterministic resume, and validates each agent's output
against a schema. The retired `fleet-coordinator.sh` (bash-3.2 crashes, deleted agent
work on exit, was a blind HMAC signing oracle) is gone; nothing launches a separate
agent process or a relay directory any more.

## Launching a crew

```
/vajra fleet <crew> "<task>"
```

`<crew>` is one of the workflows installed under `~/.claude/workflows/`:

| Crew | Workflow | What it does |
|------|----------|--------------|
| `feature-build` | `vajra-feature-build` | arjuna plans; bhima, nakula and the sepoys build in worktrees; vidura verifies; arjuna and (for security) bhishma review. Returns one local branch + report. |
| `security-audit` | `vajra-security-audit` | bhishma threat-models; bhishma and shakuni review the same scope independently; vidura confirms the top findings; bhishma issues a gate. Read-only; returns a verdict + remediation plan. |
| `incident-response` | `vajra-incident-response` | ashwatthama triages; bhima and hanuman investigate read-only; a reversible mitigation plan goes to the human; bhima fixes the root cause with a regression test vidura verifies; blameless postmortem. |
| `architecture-review` | `vajra-architecture-review` | arjuna drafts an ADR; bhishma, draupadi and hanuman vote read-only; duryodhana runs a pre-mortem; yudhishthira rules. |
| `sabha` | `vajra-sabha` | a debate: independent voters, a non-voting adversary (duryodhana), the majority counted in code, at most one rebuttal round, and a yudhishthira synthesis that answers every adversary point. |

The crew → workflow map is `config/default.json` → `cortex.fleetCrews`. Each entry
records the `workflow` name plus the `lead` and `agents` for documentation; the
counts and voting are enforced inside the workflow script, not read from config.

To launch, the main session calls the Workflow tool with the crew's workflow name and
the task as `args` (a string, or `{"ask"/"feature"/"scope"/"subject": "…"}`). The
run shows up in `/workflows`; its result is the workflow's return object
(`{status, node, artifacts, …}`). A bare `/vajra <ask>` with no crew launches
`vajra-deliver`, the autopilot, which plans its own crew.

## What a crew never does

Every crew workflow is bounded by the same rules (see `engine/cortex-bridge.md` and
each script's header):

- **No protected actions.** Push, PR, deploy, migrations, destructive data ops,
  secrets, spend and external messages are never performed by a workflow. They come
  back as a plan for the human to approve and run. A deploy request produces a deploy
  plan, not a deploy.
- **Writers in worktrees, reviewers read-only.** Only agents that change files get
  `isolation: worktree`; reviewers and verifiers read the git tree and run read-only
  commands. Worktrees are isolation for parallel edits, **not** a security boundary —
  the OS sandbox is (see `SECURITY.md`).
- **Evidence over assertion.** A criterion counts as met only when vidura ran a real
  command against the integrated tree and it passed; the gate is computed in code from
  that evidence, never from an agent's self-report.
- **Bounded loops.** Fix/revise loops stop after a fixed number of rounds and return
  `need_input` rather than looping forever.

## Resuming and stopping

Because the runtime journals each agent's result, a stopped or paused crew resumes
from `/workflows` (`p`) or by relaunching the same workflow — the unchanged prefix of
agent calls returns cached results. Stop a run with `x` in `/workflows`. A long
`vajra-deliver` run also records a campaign (`vajra-campaign.sh`), so you can resume
the *ask* across sessions with `/vajra continue`, independent of the workflow runtime.

## Verifying the wiring

`scripts/vajra-doctor.sh` checks that the workflows are installed in
`~/.claude/workflows/`. The crews are tested offline with the stub harness in
`tests/workflows/` (no model calls), which asserts the orchestration: schemas,
worktree/read-only roles, bounded loops, the protected-action rule, and the vote math.
