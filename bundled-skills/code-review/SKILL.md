---
name: code-review
description: "Code review skill — performs thorough code review checking correctness, security, performance, maintainability, and test coverage."
user-invocable: true
---

# Code Review

You are reviewing code. Be thorough but constructive. Focus on what matters.

## Review Checklist

### 1. Correctness
- Does the code do what it claims to do?
- Are edge cases handled?
- Are there off-by-one errors, null checks, or race conditions?

### 2. Security
- Any OWASP Top 10 vulnerabilities? (injection, XSS, CSRF, etc.)
- Secrets hardcoded? Input validation present? Auth checks in place?
- SQL injection, command injection, path traversal risks?

### 3. Performance
- Any O(n^2) or worse algorithms where O(n) is possible?
- Unnecessary memory allocations or copies?
- Missing caching for expensive operations?
- Database N+1 queries?

### 4. Maintainability
- Is the code readable? Would a new engineer understand it?
- Are names clear and consistent with codebase conventions?
- Is complexity justified or can it be simplified?
- Any premature abstractions or over-engineering?

### 5. Tests
- Are there tests? Do they cover the important cases?
- Are tests deterministic (no flaky tests)?
- Do tests actually test behavior, not implementation details?

### 6. Architecture
- Does this follow the established patterns in the codebase?
- Are responsibilities correctly separated?
- Any new dependencies? Are they justified?

## Output Format

```
## Code Review: [file/PR name]

### Summary
[1-2 sentence overall assessment]

### Critical Issues (must fix)
- [issue with file:line reference]

### Suggestions (should consider)
- [suggestion with rationale]

### Nits (optional)
- [minor style/preference notes]

### What's Good
- [positive observations — always include at least one]
```

## Rules
- Apply Zero Cognitive Bias Protocol — don't anchor on first impression
- Be specific: include file paths and line numbers
- Explain WHY something is an issue, not just WHAT
- Always include positive feedback — reviews are for growth, not punishment
- Severity matters: critical > suggestion > nit
