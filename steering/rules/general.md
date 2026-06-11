# Steering: General Code Quality & Anti-Patterns

- id: general
- domain: general
- blocking: false
- file-patterns: ["**/*"]

## Structure
- **Single Responsibility** — one reason to change per class/module. If you need
  "and" to describe it, split it.
- **Methods/functions** ≤ ~50 lines, ≤ 4 parameters. More params → introduce a
  parameter object / dataclass / options object. No boolean flag arguments —
  split into two named functions or use an enum.
- **Guard clauses** to reduce nesting (max ~3 levels); return early.
- **No God objects/modules** (>300 lines or >10 methods / >500-line files) —
  split by responsibility.
- **No duplication of intent** (5+ identical lines for the same purpose) —
  extract a named helper. A few repeated lines for *different* intents are fine.
- **Delete dead code** — commented-out code, debug prints, abstractions with
  exactly one implementation and one caller.

## Naming
- Reveal intent without a comment. `elapsedDays`, not `d`. Name length tracks
  scope length (`i` in a tight loop; full words for module exports).
- No vague `Manager`/`Helper`/`Util`/`Data` god names. No Hungarian/type-encoding.
- Booleans: `is`/`has`/`should`/`can`. Constants `UPPER_SNAKE_CASE`. Acronyms as
  words (`XmlHttpRequest`, `orderId`).
- TODO in code → open a ticket and reference it; never leave bare TODOs.

## Comments
- Explain WHY, not WHAT. `// returns the id` is noise. State contracts (format,
  range, side effects, thread safety, null behavior). Link, don't duplicate.

## Universal anti-patterns (fix on sight)
| Anti-pattern | Fix |
|---|---|
| Empty/swallowing catch | Log with context + rethrow, or handle fully |
| Generic `catch (Exception/Throwable)` in business code | Catch the narrowest type; generic only at process boundaries |
| Magic numbers/strings | Named constants / enums / config |
| Returning `null` for a collection | Return empty (`[]` / `List.of()`) |
| Hardcoded endpoints/keys/secrets/regions | Config / Secrets Manager / `Stack.of(this)` |
| Hand-rolled retry loops | A retry library w/ backoff (Resilience4j, tenacity, Powertools) |
| `print`/`System.out.println`/`console.log` in prod | Structured logger |
| Money in `float`/`double` | `BigDecimal` / integer minor units |
| `new Date()`/`SimpleDateFormat` for logic | `java.time` / `Date.now()` + ISO strings |
| Shared mutable statics | Instance state / `ConcurrentHashMap` |
| Framework-style extensibility, one caller | Delete the abstraction |
| Commented-out code | Delete; git remembers |

## Backward-compatibility & deployment safety
- API/serialization changes additive only; new fields optional with defaults;
  deprecate before removal; gate behavioral changes behind feature flags with a
  safe fallback. DB schema changes additive; removals/type changes need migration.
- Keep a feature CR reasonably scoped (~code/tests/config balance); don't mix
  unrelated concerns. Refactors/migrations may legitimately be larger.
