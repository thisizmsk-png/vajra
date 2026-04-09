---
name: testing
description: "Testing skill — designs test strategies, writes test plans, and implements unit/integration/E2E tests with proper coverage."
user-invocable: true
---

# Testing

You are designing and implementing tests. Tests are the safety net that enables
velocity — not a tax on development.

## Test Strategy Framework

### 1. What to Test
- **Happy paths** — Primary use cases work correctly
- **Edge cases** — Boundary values, empty inputs, max values
- **Error paths** — Invalid input, network failures, timeouts
- **Security paths** — Unauthorized access, injection, tampering

### 2. Test Pyramid
```
        /  E2E  \        ← Few: critical user flows
       / Integration \    ← Moderate: component interactions
      /    Unit Tests   \ ← Many: individual functions/methods
```

### 3. Test Quality Criteria
- **Deterministic** — Same input always produces same result. No flaky tests.
- **Fast** — Unit tests < 100ms each. Total suite < 30s for unit tests.
- **Independent** — Tests don't depend on each other or shared mutable state.
- **Readable** — Test name describes the scenario. Arrange-Act-Assert structure.
- **Meaningful** — Tests verify behavior, not implementation details.

## Test Plan Template

```markdown
# Test Plan: [Feature/Component]

## Unit Tests
| Test | Input | Expected | Edge Case? |
|------|-------|----------|------------|

## Integration Tests
| Test | Components | Scenario |
|------|-----------|----------|

## E2E Tests (if applicable)
| Test | User Flow | Acceptance Criteria |
|------|-----------|-------------------|
```

## Rules
- No test should depend on external services (mock at boundaries)
- Aim for >90% coverage on new code
- Test names should read like documentation
- Apply Zero Cognitive Bias Protocol — don't skip tests because "it obviously works"
- One assertion per test concept (multiple assertions OK if testing one logical thing)
