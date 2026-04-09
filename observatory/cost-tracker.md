# Cost Tracker

## When to Track

Track token usage after every operation that consumes tokens:
- Tier 4 routing classification (~500 tokens)
- Subagent spawns
- Tool calls that use LLM (not file reads/writes)

## How to Track

1. Read cost rates from `config/default.json` → `cost.ratesPer1MTokens`
2. After each LLM operation, estimate tokens consumed (input + output)
3. Calculate cost: `(tokens / 1_000_000) * rate`
4. Update campaign state's `cost` field:
   - Increment `totalTokens`, `inputTokens`, `outputTokens`
   - Recalculate `estimatedUSD`

## Spend Alerts

If `cost.alertThreshold` is set (not null):
- After each cost update, check if `estimatedUSD >= alertThreshold`
- If exceeded, pause and ask: "Spend alert: ~$X.XX consumed (threshold: $Y.YY). Continue? [y/n]"
- Log the alert in campaign state

## Routing Cost Summary

| Tier | Typical Cost |
|------|-------------|
| T1 (Regex) | 0 tokens |
| T2 (Campaign) | 0 tokens |
| T3 (Keyword) | 0 tokens |
| T4 (LLM) | ~500 tokens |

Track which tier was used for each routing decision to demonstrate the cost savings of tiers 1-3.
