# Steering: Core Principles (shared doctrine)

- id: principles
- domain: general
- blocking: false
- file-patterns: ["**/*"]

Distilled from the production code-review rule packs and their SKILL.md companions
(Effective Java/Python, Clean Code, PEP 8/484, Lambda Powertools, production
incident learnings). Optimize for correctness and maintainability first. Never
over-engineer.

## The order of priorities (non-negotiable, in order)
1. **Correctness** — the code does what it claims; types/tests prove it.
2. **Readability** — code is read 10× more than written; optimize for the next engineer.
3. **Simplicity** — fewest moving parts; delete code whenever possible; never solve a problem you don't have yet.
4. **Type safety** — the type system is the first line of defense; use it fully.
5. **Testability** — if the design makes testing painful, the design is wrong.
6. **Operability** — logged, measured, alarmed, rollback-safe.
7. **Performance** — only after the above, and only with a profile in hand.

When a rule and simplicity disagree, side with simplicity and explain the
deviation in the PR/CR description.

## Forbidden by default
- Cleverness that saves lines but hides intent.
- Abstractions introduced "for future flexibility."
- Design patterns applied because they exist.
- Framework-style extensibility with one call site.
- `any` / `Any` / `# type: ignore` / `@ts-ignore` without an explicit justification.

## When to deviate
- You have a measured reason (benchmark, user-visible metric, hard constraint).
- You document the deviation in the PR/CR description.
- The deviation is the smallest one that solves the problem.
- A follow-up ticket exists if it introduces tech debt.
Otherwise, default to the rules. Consistency with the codebase beats personal preference.

## How an agent uses this
Consult the relevant domain rule file, then write the **minimum code** that
satisfies the rule. Skip rules that don't apply. When reviewing, run the
[review pipeline](../review-pipeline.md): flag only what actually hurts
correctness, readability, performance, or security; quote the rule; offer the
smallest fix.
