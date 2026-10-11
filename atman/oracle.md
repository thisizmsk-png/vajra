# Oracle: second-opinion escalation

This is guidance for the model, not automation. Atman's deltas fix recurring
failures slowly; the Oracle helps with the current one.

## When to consult it

1. **Repeated failure.** `bash ~/.claude/skills/vajra/scripts/atman.sh status`
   lists skills and agents with 2 or more failures in the last 7 days. If the
   current skill or agent is listed, or you have failed the same step twice in
   this session, consult the Oracle before trying again.
2. **High-stakes step.** Schema or data migrations, security boundaries,
   money- or correctness-critical logic, irreversible actions.
3. **Real uncertainty** between approaches when being wrong is expensive.

Not for routine work.

## How

1. Brief tightly: the goal, the diff or approach in question, the error, and the
   decision to make. Not the whole repo.
2. Use the strongest available model with higher effort, read-only (for example
   an `arjuna` review, or `bhishma` for security).
3. Ask for the root cause and the smallest fix.
4. The worker or the user decides and applies. The Oracle does not edit.
