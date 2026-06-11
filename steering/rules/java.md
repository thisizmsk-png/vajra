# Steering: Java

- id: java
- domain: java
- blocking: false
- file-patterns: ["**/*.java"]

## Null safety & correctness (blocking subset)
- Guard against NPEs: validate public/protected params (`Objects.requireNonNull`,
  `Validate.notBlank`); check `Map.get`/`Optional.get`/`JsonNode` before use.
- **Return `Optional<T>`** for nullable APIs; **never** Optional as a field,
  parameter, or collection element. `optional.orElseThrow(...)`, not `.get()`.
- **Collections never null** — return `List.of()`. Validate inputs at method entry.

## Classes, types, immutability
- One reason to change per class; minimize accessibility (default `private`);
  `final` fields by default; favor composition over inheritance; make classes
  `final` unless designed for extension; prefer interfaces to abstract classes;
  declare by interface (`List<Foo>` not `ArrayList<Foo>`); no raw types.
- **Records** (16+) for value objects/DTOs/keys (not JPA entities). Sealed
  interfaces (17+) + pattern-matching switch (21+) for finite domains. `var`
  only when the RHS makes the type obvious. Constructor injection, `private
  final` fields — no field injection, no `new` for service deps.
- No `float`/`double` for money (`BigDecimal`); `java.time`, never `Date`/
  `Calendar`/`SimpleDateFormat`; `BigDecimal`/immutable collections at boundaries.
- Override `equals`/`hashCode`/`toString` together; refer to objects by interface.

## Exceptions
- For exceptional conditions only, never control flow. Specific types. Checked
  only when the caller can recover; service layer throws unchecked. Translate at
  boundaries (don't leak `SQLException` to a controller). Never catch
  `Throwable`; `Exception` only at process boundaries. Never swallow;
  try-with-resources for `AutoCloseable`; never `return` from `finally`; don't
  log-and-rethrow. Re-interrupt on `InterruptedException`; classify
  `ExecutionException` cause. No `throws Exception` on public APIs.

## Concurrency
- Prefer immutability; `java.util.concurrent` over raw threads; protect shared
  mutable state (`synchronized`/`ConcurrentHashMap`/`AtomicReference`/`volatile`);
  bounded named thread pools; `future.get(timeout)`; no `parallelStream()` in
  Lambda handlers; document thread safety in Javadoc.

## Performance
- Profile first (JFR/async-profiler/JMH). No N+1 (batch); pre-compile regex as
  `static final`; reusable `final ObjectMapper`; no String concat in loops
  (`StringBuilder`); avoid hot-path allocation; SDK clients in static init.

## Anti-patterns (fix on sight)
`Util/Manager/Helper` god class → split by verb+noun · `catch (Exception e) {
log.error(e.getMessage()) }` → narrow type, include throwable, rethrow/handle ·
magic numbers → constants · boolean flag param → two methods · `null` from a
collection → empty · `Optional` field/param → return-only · `System.out` →
logger · `new Date()`/`SimpleDateFormat` → `java.time` · raw types →
parameterize · hand-rolled retry → library · shared mutable statics → instance/
`ConcurrentHashMap` · Lombok `@Data` on domain types → records / `@Getter +
@RequiredArgsConstructor` · hand-written DTO mappers (>~5) → MapStruct · `.toList()`
returned as `List` (looks mutable, isn't) → `ImmutableList` return type or Javadoc.

## Review checklist (one pass)
one reason to change? every possible field `final`? `null` that should be empty/
Optional? raw types / `System.out` / `new Date()` / `Random` for security?
caught `Exception`/`Throwable` / empty catch / swallowed cause? try-with-
resources missing? method >30 lines / >4 params / boolean flag? magic constants?
same-intent duplication? tests behavioral, `assertThrows`, no mocked statics?
logger string-concat / secret logged / throwable passed last? hand-rolled retry /
money as double? records/sealed/pattern-matching would simplify? abstraction with
one impl+caller? Lombok `@Data` on immutables? — all pass → approve.
