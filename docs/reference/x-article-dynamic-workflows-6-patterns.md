# Dynamic Workflows in Claude Code — 6 Patterns, 14 Steps

> Reference notes from X Article by Codez (@0xCodez), published Jun 3, 2026.
> Source: https://x.com/0xCodez/status/2062127385923776831
> Status: bookmarked on X (account @SaiKris59630524) — these notes retained locally on 2026-06-10.

## What Dynamic Workflows are

Shipped in Claude Code on May 28, 2026. Instead of one Claude planning and executing in a single context window, Claude writes a custom JavaScript harness for the task — a script with special functions that spawn and coordinate subagents, plus normal JS (Math, JSON, Array) to process data between them. Triggered by asking for a workflow directly or with the keyword `ultracode`. Interrupted workflows resume when the session resumes.

What it gives you over the default harness:
- **Per-agent isolation** — each subagent has its own context window and one focused goal
- **Per-agent model choice** — Opus for hard reasoning, Haiku for cheap exploration, Sonnet in between
- **Per-agent isolation level** — worktree (isolated git checkout) or remote (no checkout)

## The 3 failure modes workflows fix structurally

1. **Agentic laziness** — declaring done after partial progress (20 of 50 items "handled")
2. **Self-preferential bias** — Claude favoring its own output when asked to judge it
3. **Goal drift** — loss of fidelity to the original objective across many turns, especially after compaction

Mapping failure mode → pattern: drift → fan-out; self-preference → adversarial verification; open-ended → loop until done; hard-to-score → tournament.

## Static vs dynamic

Static harnesses (Agent SDK, `claude -p` chains) are written once, generic, conservative. A dynamic workflow is written *for this task* — it can read your code, check against your actual context, and run adversarial passes against its own emerging answer.

## Core API

- `agent(prompt, {model, schema, isolation})` — spawn a subagent
- `parallel([...])` — **barrier**: fans out, waits for ALL results before returning
- `pipeline(items, ...stages)` — **streaming**: each item flows through stages independently, no barrier

Decision rule: need all results before the next step? → parallel. Otherwise → pipeline (cheaper, faster).

## The 6 patterns

1. **Classify-and-act** — a classifier agent routes work by type/complexity first; spend the expensive model only where needed (e.g. classifier on cheap model decides if the explanation task goes to Sonnet or Opus).
2. **Fan-out-and-synthesize** — split into enumerable independent items (50 files, 200 endpoints), one agent each in parallel, then one synthesis agent merges structured outputs. Solves "too many things at once."
3. **Adversarial verification** — for each worker agent, a separate verifier agent checks output against a rubric. Verifier sees only the rubric + artifact, never who produced it. Use for claim-checking, code review, quality gates.
4. **Generate-and-filter** — generate many options (30 names, 5 designs), then filter/score/dedupe via verifiers. Makes Claude commit late instead of early.
5. **Tournament** — N agents attempt the same task with different approaches; judge results pairwise (comparative judgment beats absolute scoring, especially for taste). Bracket lives in deterministic loop code, not context. Right way to sort/rank 1,000+ items.
6. **Loop until done** — for unknown-size work, keep spawning agents until a stop condition holds (no new findings, zero errors). Pair with `/goal` for a hard completion requirement and `/loop` to run on a schedule.

## Composing patterns per use case

- **Migrations/refactors**: fan-out (agent per callsite in a worktree) → adversarial verify → loop until done (pattern Anthropic used to rewrite Bun from Zig to Rust)
- **Deep research**: fan-out searches → verify each claim independently → synthesize cited report
- **Deep verification of a draft**: extract claims → one verifier per claim → meta-verifier checks source quality
- **Sorting 1,000+ items**: tournament, never absolute scores
- **Rule adherence**: verifier per rule + skeptic persona reviewing the rules themselves
- **Root cause**: agents generate theories from disjoint evidence → panel of verifiers/refuters → loop until one survives
- **Triage at scale**: classify-and-act → dedupe → fix or escalate; pair with /loop
- **Taste/exploration**: generate-and-filter → tournament with rubric
- **Lightweight evals**: run candidate in worktree → graders against rubric → refine and re-grade

## Cost controls

- `/goal` = hard completion requirement (don't stop at soft "done")
- `/loop` = run the whole workflow on a recurring schedule
- Explicit token budgets in the prompt ("use 10k tokens") — without a cap, workflows balloon 5–10×
- Claude Code team: best practices still developing; workflows use more tokens — most ordinary coding tasks don't need a panel of 5 reviewers

## Security: quarantine pattern

Any workflow reading untrusted content (tickets, bug reports, scraped pages, third-party API output) must assume prompt injection. Reader agents that touch the untrusted content get NO high-privilege actions; separate actor agents (never exposed to the raw content) do the acting.

## Saving workflows

Press `s` in the workflow menu → saved to `~/.claude/workflows`. Reuse locally or ship inside a Skill (reference the JS file in SKILL.md). When packaging into a Skill, tell Claude to treat the workflow as a *template*, not a verbatim script, so it can adapt shape per task.

## Mistakes that waste tokens

- Using a workflow when a normal session would do
- No token budget
- Same agent doing work + verification (self-preference)
- Treating parallel() and pipeline() as interchangeable
- Skipping /goal on loop patterns
- Letting untrusted content reach the actor agent
- Sorting with absolute scores instead of tournaments
- Never saving working workflows
