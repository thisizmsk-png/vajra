---
name: product-spec
description: "Product specification skill — creates PRDs with problem statements, user stories, success metrics, and go-to-market considerations."
user-invocable: true
---

# Product Specification

ultrathink

You are writing a Product Requirements Document (PRD). Define the PROBLEM, not
the solution. Engineering defines solutions.

## PRD Structure

### 1. Problem Statement
- What specific user pain are we solving?
- Who experiences it? (persona with specifics)
- How do they solve it today? (current alternatives)
- Why is the current solution inadequate?

### 2. User Stories
```
As a [specific persona], I want [capability], so that [measurable benefit].
```

### 3. Success Metrics
- **North Star Metric:** The one number that matters
- **Input Metrics:** Leading indicators we can influence
- **Guardrail Metrics:** Things that must NOT get worse

### 4. Scope
- **In scope:** What we're building (MVP definition)
- **Out of scope:** What we're NOT building (equally important)
- **Future considerations:** What might come next (but not now)

### 5. Competitive Landscape (MANDATORY)
- **Direct competitors:** Who else solves this exact problem? Name them. (If "nobody" — verify with 3 searches, don't assume.)
- **Adjacent competitors:** Who solves a related problem that could expand here?
- **Your differentiation:** What do you do that they can't/won't?
- **Incumbent threat:** Can an existing big player add this in one sprint?
> NEVER skip this section. Draupadi learned this the hard way — missing a competitor poisons the entire PRD.

### 6. User Research Evidence
- What data supports this problem exists?
- What user interviews/surveys/analytics inform this?
- What competitors do about this?

### 6. Acceptance Criteria
GIVEN/WHEN/THEN for each user story.

## Rules
- Define problems, not solutions
- Every feature must connect to a success metric
- Apply Zero Cognitive Bias Protocol — validate assumptions with evidence
- Say "no" to scope creep — out of scope section is mandatory
