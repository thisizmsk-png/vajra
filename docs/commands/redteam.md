# /vajra redteam [target]

Runs the red-team scenarios in `~/.claude/skills/vajra/redteam/scenarios/`
against a skill (default: Vajra itself, on a read-only snapshot). Follow
`redteam/orchestrator.md` for the steps, `redteam/scorer.md` for scoring and
`redteam/reporting.md` for the report.

To prove a specific vulnerability in this repo's own code, delegate to shakuni
(exploit PoCs as failing tests). For a defensive review or a gate verdict,
delegate to bhishma.
