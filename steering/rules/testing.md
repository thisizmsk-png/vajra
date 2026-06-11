# Steering: Testing Standards

- id: testing
- domain: testing
- blocking: false
- file-patterns: ["**/*test*", "**/*Test*", "test/**", "tests/**"]

1. **Frameworks**: JUnit 5 + Mockito (Java, avoid PowerMock); pytest (Python,
   no `unittest.TestCase` for new code); Jest/Vitest (TS). AssertJ over stock
   JUnit assertions.
2. **One concept per test** (multiple assertions for one concept is fine).
   Arrange–Act–Assert with whitespace separating sections.
3. **Descriptive names for behavior**: `should_chargeGateway_when_cardValid`,
   `test_process_order_with_invalid_id_raises_error`.
4. **Mock at boundaries, not internals** — mock the HTTP/SDK client, not the
   service method. Never call real DBs/HTTP/AWS/filesystem in unit tests
   (use LocalStack/Testcontainers for integration).
5. **Test edge cases**: null inputs, empty collections, boundary values,
   concurrent access. Use parametrization (`@pytest.mark.parametrize`).
6. **Assert content, not existence** — no coverage-only tests:
   - BAD: `assertNotNull(result)` / `toBeDefined()` / `assert result is not None`
     with no follow-up.
   - GOOD: assert specific field values and behavior.
7. **Exceptions**: `assertThrows` / `pytest.raises` — assert type AND message.
8. **Coverage**: minimum **80% line for new code; 90%+ for auth / payment /
   data-integrity**. Enforce in pipeline, not in review.
9. **CDK**: `Template.fromStack()` assertions — NOT snapshot tests (brittle on
   version bumps). Test IAM least-privilege, alarm thresholds, resource counts.
10. **Don't test**: third-party library internals, private implementation
    details, trivial getters. No `time.sleep`/flaky timing, no shared mutable
    global state between tests, no `xit`/`@Ignore` without a linked ticket.
