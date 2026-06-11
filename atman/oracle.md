# Vajra Oracle — Second-Opinion Escalation

Atman's peer review is *horizontal* (skills review each other). The Oracle is
*vertical*: when a task is stuck or high-stakes, escalate to a stronger model for
a focused second opinion **before** the run fails — the pattern Amp's "Oracle"
and Devin's self-correction use to get outsized debugging/review gains.

## When to summon the Oracle

Summon automatically on any of:
1. **Repeated failure** — Atman has flagged a skill/agent with ≥2 failures in the
   last 7 days (`~/.claude/vajra/flags/*.json`, written by the post-tool hook).
   Consult the Oracle *before* the next attempt rather than looping blindly.
2. **High-stakes step** — the step touches a schema/data migration, a security
   boundary, money/correctness-critical logic, or an irreversible action.
3. **Genuine uncertainty** — the agent is choosing between approaches with no
   clear winner and the cost of being wrong is high.

Do NOT summon for routine work — the Oracle is expensive (stronger model + more
reasoning). Most tasks never need it.

## How it works

The Oracle is a focused consult sub-agent, not a worker:

1. **Brief it tightly** — give it only the failing context: the goal, the diff or
   the approach under question, the error/trace, and the specific decision. Not
   the whole repo. (Just-in-time context — it should not need to explore.)
2. **Stronger config** — run it on the strongest available model with higher
   reasoning effort. It reasons, it does not edit.
3. **Ask for a verdict + the smallest fix** — "What's the root cause? What's the
   minimal change? What did the worker miss?" — same discipline as the review
   pipeline (concrete, quote the evidence, smallest fix).
4. **The worker decides** — the Oracle advises; the worker (or user) applies. The
   Oracle never ships code itself.

## Wiring

- **Atman failure signal** — the post-tool hook already writes a flag at the 2nd
  failure of a skill/agent. That flag is the trigger: on the next routing to that
  skill, summon the Oracle first.
- **Plan Mode** — a step tagged high-risk in `data/plans/{slug}.plan.md` summons
  the Oracle during planning, before it reaches `act`.
- **Cortex** — any agent can escalate to the Oracle; it composes with the agent's
  steering rules (the Oracle checks the work against the same rules).

## Why vertical escalation matters

A worker that fails twice and keeps trying the same shape burns tokens and drifts.
A worker that *stops, summons a stronger reasoner with the failing context, and
gets a root-cause* converges. The Oracle is the structural answer to "the agent
got stuck and dug deeper" — escalate before the run dies, not after.
