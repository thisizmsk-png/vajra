# Steering: TypeScript

- id: typescript
- domain: typescript
- blocking: false
- file-patterns: ["**/*.ts", "**/*.tsx", "!**/*.test.ts"]

## tsconfig (non-negotiable)
`strict: true` plus `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`
(may relax for React with a documented reason), `noFallthroughCasesInSwitch`,
`noImplicitReturns`, `noPropertyAccessFromIndexSignature`,
`forceConsistentCasingInFileNames`. Never weaken `strict`; wrap a
strict-incompatible lib in a typed adapter instead.

## Types
- **Never `any`** — `unknown` + type guard, or a specific type. No `@ts-ignore`
  — use `@ts-expect-error` with explanation or fix the type.
- **Explicit return types** on exported functions; internal helpers may infer.
- `const` over `let`; never `var`. Strict equality `===`/`!==` only.
- **Discriminated unions** over class hierarchies for finite domains; rely on
  `noFallthroughCasesInSwitch` for exhaustiveness. `satisfies` for literals
  without widening. `readonly` for immutability. Branded types for domain IDs.

## Null safety
- Optional chaining `?.`; nullish coalescing `??` (NOT `||`, which treats `0`,
  `''`, `false` as falsy). Type guards for runtime narrowing. Return `[]` from
  collection-returning functions, never `null`.

## Async
- `async/await`, never mixed with `.then()` in one function. **No floating
  promises** — `await`, `return`, or `void p.catch(...)`. `Promise.all` for
  parallel; `Promise.allSettled` when partial failure is OK. ESLint:
  `no-floating-promises`, `no-misused-promises`, `await-thenable`.

## Anti-patterns (fix on sight)
`any` → `unknown`+guard · `==` → `===` · floating promise → await/void · mixed
.then+await → pick one · empty catch → log+rethrow · `||` default → `??` ·
SDK client in handler → module-level · `@aws-sdk/*` in deps → devDeps ·
snapshot tests for logic → assertions · barrel `index.ts` re-exports → direct
imports · `null` for collections → `[]` · string IDs everywhere → branded types ·
`new Date()` for storage → `Date.now()`/ISO.

## Review checklist (one pass)
strict on? any/@ts-ignore? exported fns typed? floating/mixed promises? empty
catch? `||` that should be `??`? console.log/secrets logged? fn >50 lines / >4
params / boolean flag? hardcoded account/region/ARN/secret? SDK client in
handler? `@aws-sdk/*` in deps? input validated (zod)? mutable module state in
Lambda? tests behavioral / no logic-snapshots? abstraction with one impl+caller?
CDK name has disambiguator/region? `RemovalPolicy.DESTROY` on prod data? CR
scoped? — all pass → approve; else quote the rule + smallest fix.
