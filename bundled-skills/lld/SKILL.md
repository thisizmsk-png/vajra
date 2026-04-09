---
name: lld
description: "Low-Level Design skill — produces detailed implementation design with class diagrams, method signatures, algorithms, data structures, and error handling."
user-invocable: true
---

# Low-Level Design (LLD)

ultrathink

You are designing a detailed implementation. Follow this structure:

## Process

1. **Read the HLD** — Understand the high-level architecture and constraints.
2. **Define data structures** — Classes, models, schemas with fields and types.
3. **Design method signatures** — Public APIs, parameters, return types, errors.
4. **Specify algorithms** — Step-by-step logic for non-trivial operations.
5. **Plan error handling** — What can go wrong and how to handle each case.
6. **Define tests** — What tests verify this implementation is correct.

## Output Format

```markdown
# LLD: [Component/Module Name]

## 1. Data Structures
[Pydantic models, TypeScript interfaces, database schemas]

## 2. Method Signatures
[Public API with parameters, return types, and docstrings]

## 3. Algorithms
[Pseudocode or step-by-step for non-trivial logic]

## 4. Error Handling
| Error Case | Detection | Handling | User Impact |
|-----------|-----------|----------|-------------|

## 5. Dependencies
[External libraries, internal modules, API calls]

## 6. Test Plan
[Unit tests, integration tests, edge cases to cover]
```

## Rules
- Follow existing codebase patterns and conventions
- Apply Zero Cognitive Bias Protocol
- No over-engineering — minimum complexity for current requirements
- Every public method must have clear input/output contracts
- Error handling is not optional — plan for every failure mode
