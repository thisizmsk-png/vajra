---
name: pricing-strategy
description: SaaS pricing design, monetization strategy, revenue architecture. Activate for pricing, monetization, revenue model, subscription tier tasks.
user-invocable: true
---

# Pricing Strategy

## When to Activate
- User mentions: pricing, monetization, revenue model, subscription, tiers, freemium, paywall, MRR, ARR, unit economics
- Designing pricing for a new product or feature
- Evaluating or changing existing pricing

## The Three-Axis Framework
**Order matters. Most founders skip to price point — that's backwards.**

### Axis 1: Packaging (What do they get?)
- **Free tier**: What creates habit without giving away the product?
- **Paid tiers**: Good-Better-Best (3 tiers max for clarity)
- **Add-ons**: What's valuable enough to pay extra for?

**Decision framework:**
| Feature | Free? | Paid? | Add-on? |
|---------|-------|-------|---------|
| Creates habit, low cost to serve | Yes | — | — |
| Core value, differentiates from free | — | Yes | — |
| Expensive to serve, niche need | — | — | Yes |

### Axis 2: Value Metric (What do they pay per?)
The unit that scales with the customer's success:
- Per seat (Slack, Notion) — simple but creates perverse incentive to limit users
- Per usage (AWS, Twilio) — aligns cost with value but unpredictable bills
- Per outcome (Stripe 2.9%) — perfect alignment but hard to measure
- Flat rate (Basecamp) — simple but leaves money on the table

**For AI/Agent products (2026 shift):**
- Per-task or per-agent-run (not per-seat — AI replaces seats)
- Hybrid: base subscription + usage overage
- Outcome-based: % of value created (e.g., revenue generated, time saved)

### Axis 3: Price Point (How much?)
- **Van Westendorp**: Survey 4 questions (too cheap, cheap, expensive, too expensive)
- **Competitor anchoring**: Position relative to alternatives (cheaper? premium? different axis?)
- **Cost-plus floor**: Never price below 5x your marginal cost
- **Willingness-to-pay ceiling**: What's 10% of the value you create?

**Zero-Bias Check:**
- Am I anchoring to a competitor's price instead of my own value?
- Am I pricing based on cost (floor) instead of value (ceiling)?
- Am I afraid to charge more? (Most indie founders underprice by 2-5x)
- Am I copying a pricing model because it's popular, not because it fits?

## Revenue Architecture

### Key Metrics
| Metric | Formula | Healthy Range |
|--------|---------|--------------|
| MRR | Sum of all monthly subscriptions | Growing month-over-month |
| ARPA | MRR / total customers | Higher = better economics |
| LTV | ARPA / monthly churn rate | > 3x CAC |
| CAC | Total acquisition cost / new customers | < 1/3 LTV |
| Payback period | CAC / (ARPA × gross margin) | < 12 months |
| Net revenue retention | (MRR + expansion - contraction - churn) / starting MRR | > 100% |
| Gross margin | (Revenue - COGS) / Revenue | > 70% for SaaS |

### Solo Founder Benchmarks
| Stage | MRR Target | ARPA Sweet Spot | Customer Count |
|-------|-----------|----------------|---------------|
| Validation | $1K | $50-$100 | 10-20 |
| Traction | $5K | $100-$200 | 25-50 |
| Growth | $10K+ | $200+ | 50-100 |
| Scale | $50K+ | $500+ | 100+ |

### Pricing for Indian Market
- US customers: $99-$199/mo (standard SaaS pricing)
- India/SEA customers: $19-$49/mo (PPP-adjusted)
- Use Stripe for US, Lemon Squeezy for global tax compliance
- Consider annual plans at 2 months free (improves cash flow)

## Process

### Phase 1: Research
1. List 5 closest competitors and their pricing
2. Identify your unique value (what do you do that they don't?)
3. Survey or interview 10 potential users on willingness to pay
4. Calculate your marginal cost per customer

### Phase 2: Design
1. Define packaging (3 tiers)
2. Select value metric (must scale with customer success)
3. Set price points (floor = 5x cost, ceiling = 10% of value created)
4. Design upgrade path (how does free become paid?)

### Phase 3: Validate
1. Launch with pricing visible (don't hide it)
2. Track: conversion rate, tier distribution, upgrade rate
3. If >40% choose the highest tier → you're underpriced
4. If <5% convert from free → free tier gives away too much

### Phase 4: Iterate
- Raise prices for new customers every quarter until conversion drops
- Grandfather existing customers (or give 90-day notice)
- Test annual vs monthly split

## Guardrails
- NEVER price without understanding the customer's alternative (what they do today)
- NEVER copy a competitor's pricing without understanding their cost structure
- ALWAYS design for expansion revenue (usage growth, seat growth, tier upgrades)
- Apply Zero Cognitive Bias Protocol — especially anchoring bias
