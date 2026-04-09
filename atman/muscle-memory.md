# Muscle Memory — Routing Gets Smarter From Practice

Like a basketball player who no longer thinks about dribbling — it's automatic.

## How Routing Learns

Every time a task is successfully routed via T4 (LLM classification, costs ~500 tokens), check if it could become a free route:

### T3 Keyword Learning
If the SAME skill has been routed to via T4 for the SAME keyword ≥3 times:
```
Example: "deploy" → T4 → ci-cd (3 times)
Proposal: Add "deploy" to tier3Keywords → ci-cd
```

### T1 Regex Learning
If a SPECIFIC pattern (more than a keyword) consistently routes to the same skill:
```
Example: "/deploy staging" → T4 → ci-cd (3 times)
Proposal: Add regex "^/deploy\\s+(staging|prod)" → ci-cd to tier1Patterns
```

## Proposal Process

1. After each T4 routing, check practice-log for pattern repetition
2. If threshold met (≥3 identical routings), generate a routing proposal
3. Write to `~/.claude/vajra/routing-proposals.json`:
```json
{
  "proposals": [
    {
      "id": "rp-001",
      "tier": "T3",
      "key": "deploy",
      "skill": "ci-cd",
      "evidence": ["trace-1", "trace-2", "trace-3"],
      "tokensSaved": 1500,
      "created": "2026-04-09T12:00:00Z",
      "status": "pending"
    }
  ]
}
```

4. During `/vajra dream`, apply approved proposals to `config/default.json`
5. Log: "Routing learned: 'deploy' → ci-cd (saves ~500 tokens/use)"

## Auto-Learn vs User-Confirm

- **T3 keywords**: Auto-learn (low risk, easily reversible)
- **T1 regex patterns**: User-confirm via `/vajra atman review` (regex can have unintended matches)

## Metrics

Track in `/vajra atman status`:
- Routing proposals pending / approved / rejected
- Token savings this month from learned routes
- T4 percentage trend (should decrease over time)
