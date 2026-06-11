# Vajra Steering Review Pipeline

A high-signal code-review protocol, modeled on a proven 5-pass
engine. Used by `/vajra review <target>`, by Cortex review agents (Arjuna,
Vidura, Bhishma), and inside the red-team's findings pass.

The goal is **few, correct, actionable comments** — not volume. A reviewer that
emits 30 nitpicks is worse than one that emits the 3 that matter.

## Inputs

- The diff (changed files only — you do NOT see the whole file unless a rule's
  `tools: ReadFile` allows pulling surrounding context).
- The steering rules whose `filePatterns` match the changed files
  (`config/steering.json` → `rules[]`).
- `review.confidenceThreshold` (default 8) and `review.discardCriteria`.

## The 5 Passes

### Pass 1 — Generate
Review the diff against the matched steering rules. For each concrete issue,
draft a comment: **file:line · the rule it violates · the smallest fix**. Pull
surrounding context only when a rule needs it (e.g. "is this SDK client created
inside the handler?" needs the enclosing function).

### Pass 2 — Deduplicate
Merge comments that say the same thing across files/lines. One comment per
distinct issue. Collapse a repeated pattern into a single comment that names all
occurrences.

### Pass 3 — Confidence gate
Score each comment 1–10 on: correctness, rule-compliance, actionability, impact.
**Discard anything below `confidenceThreshold` (8).** Also discard any comment
matching `review.discardCriteria`:
- incorrect / misunderstands the code
- nitpick / pure style with no correctness-security-maintenance impact
- speculative ("might…") without direct evidence in the diff
- praises or explains an already-correct change
- assumes code is missing (you only see the changed portion)

### Pass 4 — Guideline compliance (category suppressions)
Drop comments that violate `review.categorySuppressions`:
- **input-validation**: not for declarative IaC (CDK), test code, or trusted
  internal input with no evidence of bad values.
- **error-handling**: not for declarative IaC, test code, or framework-managed
  flows.
Respect each rule's `blocking` flag: a violation of a `blocking: true` rule
(security, lambda, aws-cdk here) is a **merge blocker**; others are advisory.

### Pass 5 — Refine
Every surviving comment must have: clear reasoning (WHY, not WHAT), the quoted
rule, and a concrete code example of the fix. Cap at
`review.maxCommentsPerPass` (5) — keep the highest-impact ones.

## Output

```
## Review — <target>
Blocking: <n>   Advisory: <m>   (confidence ≥ 8)

### [BLOCKING] <rule-id> — file:line
<why it matters, concrete consequence>
Rule: "<quoted rule>"
Fix:
  <smallest code change>

### [advisory] ...
```
If every matched rule passes, output `Approve — N rules checked, 0 findings`.

## Core review stance (from the rule-pack doctrine)
- Only flag issues that still need fixing; never explain or praise correct code.
- No speculation — only concrete, observable issues in the diff.
- Assume the author did due diligence unless there's clear evidence otherwise.
- Only comment on real bugs, security holes, or significant maintenance risk.
- When a rule and simplicity disagree, side with simplicity and say why.
- One pass is enough. Quote the rule, offer the smallest fix.
