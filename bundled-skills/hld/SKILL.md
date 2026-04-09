---
name: hld
description: "High-Level Design skill — produces system architecture documents with component diagrams, data flow, API contracts, and trade-off analysis."
user-invocable: true
---

# High-Level Design (HLD)

ultrathink

You are designing a high-level system architecture. Follow this structure:

## Process

1. **Understand the requirement** — Read the spec/PRD/user story. Clarify scope.
2. **Identify components** — What are the major building blocks?
3. **Define interfaces** — How do components communicate? API contracts, data formats.
4. **Map data flow** — Input → Processing → Output for the primary use cases.
5. **Consider non-functional requirements** — Performance, scalability, security, observability.
6. **Document trade-offs** — Every design choice has alternatives. Document what you chose and why.
7. **Apply 5-Question Research Framework** for non-trivial design decisions.

## Output Format

```markdown
# HLD: [Feature/System Name]

## 1. Overview
[1-2 paragraph summary of what this system does and why]

## 2. Component Architecture
[Diagram showing major components and their relationships]

## 3. Data Flow
[Primary data flow paths with input/output at each stage]

## 4. API Contracts
[Key interfaces between components]

## 5. Non-Functional Requirements
| Requirement | Target | Approach |
|------------|--------|----------|

## 6. Trade-offs & Alternatives
| Decision | Chosen | Alternative | Rationale |
|----------|--------|-------------|-----------|

## 7. Open Questions
[Anything that needs further research or stakeholder input]
```

## Rules
- Apply Zero Cognitive Bias Protocol to all design decisions
- Every significant choice must have a documented alternative
- No premature optimization — design for correctness first
- Consider the 10x scale scenario for critical paths
