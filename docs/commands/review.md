# /vajra review [target]

A steering-rule review of the current git diff (or `target`).

`/vajra review and fix the flaky upload tests` (next word `and`, `&` or
`then`) is not a review: it is a bare ask, so follow `deliver.md`.

1. Pick the rule files in `~/.claude/skills/vajra/config/steering.json`
   (`rules[]`) whose `filePatterns` match the changed files.
2. Run the pipeline in `steering/review-pipeline.md`: generate, deduplicate,
   confidence gate (at least `review.confidenceThreshold`), category
   suppression, refine. Keep at most 5 high-impact comments.
3. Report each finding as **[BLOCKING]** (a rule with `blocking: true`) or
   **[advisory]**, quoting the rule and giving the smallest fix. If nothing is
   found: `Approve: N rules checked, 0 findings`.

Only concrete, observable issues: no speculation, nitpicks or praise. For a
deeper review, delegate to arjuna (or bhishma for security) with the matching
`agentSteering` rules in the brief.
