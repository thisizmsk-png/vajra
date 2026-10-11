# /vajra help

Print this reference to the user, then stop. This applies only to `/vajra`,
`/vajra help` or `/vajra help <subcommand>`. If more follows (`/vajra help me
add dark mode`), it is an ask: follow `deliver.md` instead.

```
Vajra, an agent harness for Claude Code

  /vajra <ask>                      deliver it end to end (vajra-deliver workflow)
  /vajra plan <task>                plan first (native plan mode + Vajra template)
  /vajra fleet <crew> <brief>       run a crew workflow:
                                      feature-build | security-audit |
                                      incident-response | architecture-review | sabha
  /vajra campaign list|show|...     long-running work across sessions
  /vajra continue | status          resume or inspect the active campaign
  /vajra verify-work <slug> "<cmd>" prove work is done (exit non-zero on failure)
  /vajra ralph <progress.md>        fresh-context loop against a progress file
  /vajra review [target]            steering-rule review of the current diff
  /vajra cost [transcript]          per-model tokens, USD and cache-hit rate
  /vajra replay [session]           replay and integrity-check a session log
  /vajra repomap                    relevance-ranked map of the repo
  /vajra redteam [target]           red-team scenarios against a skill
  /vajra atman status|review|...    learning proposals (you approve each one)
  /vajra doctor                     check that hooks and workflows are wired
```

Words like help, status, continue, doctor and verify are subcommands only on
their own. `/vajra status page for our API` is an ask and gets delivered.

Memory: Vajra uses Claude Code's native auto memory, so there is no
`/vajra memory` or `/vajra dream`. Routing and spend settings live in
`~/.claude/skills/vajra/config/`. Edit them by hand; they are protected from
agent writes.
