# Vajra Steering — Machine-Checkable Engineering Rules

Steering brings the **AutoSDE / CodingAgent rule-pack model** into Vajra: a set
of scoped, blocking-aware engineering rules plus a confidence-gated review
pipeline that Cortex agents enforce. It is how Vajra turns "review my code" from
a vibe into a checklist of concrete, quotable rules.

## What's here

```
steering/
├── README.md                ← you are here
├── review-pipeline.md       ← the 5-pass review protocol (generate→dedup→
│                              confidence→guideline→refine) + discard criteria
└── rules/
    ├── principles.md        ← priority order (correctness→…→performance), doctrine
    ├── general.md           ← structure, naming, universal anti-patterns
    ├── security.md          ← BLOCKING: injection, secrets, IAM, crypto, SSRF…
    ├── testing.md           ← frameworks, coverage (80%/90% auth-payment), edge cases
    ├── lambda.md            ← BLOCKING: cold-start, container reuse, Powertools
    ├── aws-cdk.md           ← BLOCKING: no hardcoded ARNs, L2/grants, RETAIN
    ├── typescript.md        ← strict tsconfig, no any, async hygiene
    ├── python.md            ← mypy strict, from-exc chaining, no mutable defaults
    └── java.md              ← Optional, records/sealed, exceptions, concurrency
```

`config/steering.json` is the machine-readable index: which rule file applies to
which `filePatterns`, whether it's `blocking`, the review `confidenceThreshold`,
the `discardCriteria`, the `categorySuppressions`, and the **agent → rules
mapping** (`agentSteering`).

## How it wires into Vajra & Cortex

- **`/vajra review <target>`** runs `steering/review-pipeline.md`: it selects the
  rule files whose `filePatterns` match the changed files, reviews the diff, and
  emits few, high-confidence, rule-quoted findings (blocking vs advisory).
- **Cortex agents** load only their assigned rule sets (`agentSteering` in
  `config/steering.json`) — Arjuna/Bhima/Vidura/Bhishma each carry the domains
  they own, so a backend review pulls lambda+cdk+security, a QA review pulls
  testing, etc. See `engine/cortex-bridge.md`.
- **The red-team** (`/vajra redteam`) uses the same confidence/discard discipline
  so its findings are concrete and non-speculative.
- **Atman** can propose new steering rules from recurring review findings — but
  rule files live under the immutable core, so changes go through staging +
  review, never silent self-edit.

## Why a rule-pack model (vs prose guidance)

A reviewer with an explicit, scoped, confidence-gated ruleset:
- flags the **same** issues every time (consistency beats preference),
- stays **quiet** where rules don't apply (category suppressions — no
  input-validation nags on CDK or tests),
- and produces **actionable** comments (quote the rule, offer the smallest fix),
which is exactly the failure mode generic "review my code" agents fall into.

## Adding or editing rules

1. Edit/add a `rules/*.md` file (keep the `id/domain/blocking/file-patterns`
   header).
2. Register it in `config/steering.json` `rules[]` and, if agent-specific, in
   `agentSteering`.
3. Re-run `scripts/generate-manifest.sh` (rule files are integrity-tracked).
Keep rules concrete: a rule that can't be checked against a diff is documentation,
not steering. Never over-engineer — delete a rule that stops earning its place.
