---
name: incident-response
description: "Incident response skill — follows structured incident management: detect, triage, contain, recover, postmortem."
user-invocable: true
---

# Incident Response

You are managing an active incident or writing a postmortem. Stay calm. Follow the process.

## Active Incident Protocol

### 1. Detect & Triage
- What is the impact? (users affected, revenue impact, data risk)
- Severity: SEV1 (critical) / SEV2 (major) / SEV3 (minor)
- Who needs to know? (escalation chain)

### 2. Contain
- Stop the bleeding: rollback, feature flag, traffic redirect
- Prevent the blast radius from expanding
- Communicate: "We are aware and investigating"

### 3. Diagnose
- What changed? (recent deploys, config changes, traffic spikes)
- Where is it failing? (logs, metrics, traces)
- What is the root cause? (not the symptom)

### 4. Recover
- Implement the fix or rollback
- Verify recovery (metrics back to baseline)
- Communicate: "Resolved. Postmortem to follow."

### 5. Postmortem (within 24 hours)

```markdown
# Postmortem: [Incident Title]
**Date:** [date] | **Severity:** [SEV1/2/3] | **Duration:** [time]

## Summary
[1-2 sentence description]

## Impact
[Users affected, revenue impact, data impact]

## Timeline
[Minute-by-minute from detection to resolution]

## Root Cause
[The actual root cause, not the trigger]

## Action Items
| # | Action | Owner | Due Date | Status |
|---|--------|-------|----------|--------|

## Lessons Learned
[What we should do differently]
```

## Rules
- Blameless — find the system failure, not the person
- Communicate every 15 minutes during active incidents
- Fix first, investigate second (for SEV1)
- Every postmortem produces at least one action item
- Zero Cognitive Bias Protocol — don't anchor on the first theory
