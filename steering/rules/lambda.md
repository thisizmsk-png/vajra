# Steering: AWS Lambda Patterns (BLOCKING)

- id: lambda
- domain: lambda
- blocking: true
- file-patterns: ["**/handlers/**", "**/lambda/**", "**/*Handler*"]

## Cold start & container reuse (blocking)
1. **Initialize SDK clients at MODULE level**, outside the handler — reused
   across invocations. A client created inside the handler pays a cold-start
   penalty on every call.
   - GOOD: `const ddb = new DynamoDBClient({}); export const handler = async (e) => { await ddb.send(...) }`
   - BAD: handler body does `const ddb = new DynamoDBClient({})`.
2. **No mutable module-level state** that persists between invocations —
   singleton-scoped mutable state leaks data from a previous invocation into the
   next. (`let cache = {}`, `lateinit var` on singletons → flag.)
3. **Node: `@aws-sdk/*` in devDependencies only** — externalized by the runtime;
   in `dependencies` it balloons bundles to 50+ MB. Python: reuse boto3 clients
   module-level and pass `region_name` explicitly.
4. **Set `callbackWaitsForEmptyEventLoop = false`** for callback-style handlers.

## Observability (blocking)
5. **Use Powertools (Logger, Tracer, Metrics)** at module level via middleware
   (middy / decorators). No `console.log` / `print()` in production Lambda code.
6. **Catch blocks MUST log** with context — a silent catch makes degradation
   invisible. Top-level handler must not swallow: log traceback AND return 500.

## Correctness
7. **Validate input on entry** (zod/Pydantic) before processing; validate env
   config at startup (module level).
8. **Never mix async/await with callback** in one handler — silent swallowed errors.
9. **SQS/EventBridge handlers must be idempotent** (Lambda retries) — return
   `batchItemFailures` for partial batch failure.
10. **Timeouts/memory**: default 3s is too short; API Gateway ≤ 29s; memory
    scales CPU; remote/cache calls need a timeout + fallback (don't let a cache
    miss throw and fail the request).
11. **esbuild**: handlers self-contained; native modules via layers; assets
    copied at build time, not imported.
