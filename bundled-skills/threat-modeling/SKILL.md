---
name: threat-modeling
description: "Threat modeling skill — conducts STRIDE/DREAD analysis, identifies attack surfaces, and produces security recommendations."
user-invocable: true
---

# Threat Modeling

ultrathink

You are conducting a threat model for a system or feature.

## STRIDE Analysis

For each component/data flow, evaluate:

| Threat | Question |
|--------|----------|
| **S**poofing | Can an attacker impersonate a user or component? |
| **T**ampering | Can data be modified in transit or at rest? |
| **R**epudiation | Can actions be denied without evidence? |
| **I**nformation Disclosure | Can sensitive data leak? |
| **D**enial of Service | Can the system be made unavailable? |
| **E**levation of Privilege | Can an attacker gain unauthorized access? |

## DREAD Risk Scoring

For each identified threat:

| Factor | Score (1-10) |
|--------|-------------|
| **D**amage potential | How bad if exploited? |
| **R**eproducibility | How easy to reproduce? |
| **E**xploitability | How easy to exploit? |
| **A**ffected users | How many users impacted? |
| **D**iscoverability | How easy to discover? |

**Risk = Average of DREAD scores**

## Output Format

```markdown
# Threat Model: [System/Feature]

## Attack Surface
[Identified entry points, data flows, trust boundaries]

## Threats
| # | Threat | STRIDE | DREAD Score | Mitigation |
|---|--------|--------|-------------|------------|

## Recommendations
[Prioritized security recommendations]

## Residual Risk
[Risks accepted and why]
```

## Rules
- Apply Zero Cognitive Bias Protocol — don't assume "nobody would do that"
- Think like the attacker (Shakuni mindset)
- Consider both technical and social engineering vectors
- Prioritize by DREAD score, not by ease of fix
