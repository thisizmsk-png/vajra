# Steering: Security (BLOCKING)

- id: security
- domain: security
- blocking: true
- file-patterns: ["**/*"]

A violation here blocks merge. Language-agnostic core followed by language notes.

## Always flag (blocking)
1. **Input validation at trust boundaries** — every external input (event body,
   headers, URLs, DB rows) MUST be validated with a schema (zod/joi/Pydantic/
   Bean Validation), allowlist approach. Types are compile-time only — `as`
   casts and type hints provide zero runtime safety.
2. **No injection** — parameterized queries only; never string-concatenate/
   f-string user input into SQL/NoSQL/commands.
3. **No dynamic code execution on untrusted data** — no `eval()`, `exec()`,
   `new Function()`, `setTimeout(string)`, `pickle.loads()`, `yaml.load()`
   (use `yaml.safe_load()`), Java deserialization of untrusted bytes
   (`ObjectInputStream`, Jackson `enableDefaultTyping` without a validator).
4. **No hardcoded secrets/credentials/API keys** — Secrets Manager / SSM /
   env vars resolved at runtime. Flag `AKIA*` and other key-shaped literals.
5. **Never log secrets, tokens, or PII** — not in errors, metrics dimensions,
   or exception messages.
6. **Least-privilege IAM** — no `*` in actions or resources without explicit
   justification; scoped resource ARNs; service roles, not stored credentials.
7. **Crypto** — `crypto.timingSafeEqual` / constant-time compare for tokens
   (never `===`); `SecureRandom`/`secrets` module, never `Random`.
8. **SSRF / path traversal / prototype pollution / ReDoS** — validate URLs
   (allowlist; reject `file://`, `localhost`, internal IPs); `path.resolve` and
   verify containment; validate object keys before merge; avoid nested-quantifier
   regex on user input (`(a+)+`), bound input length or use `re2`.
9. **No `shell=True` / shell interpolation** with user input.
10. **S3/network**: buckets encrypted + `BlockPublicAccess.BLOCK_ALL`; no
    `0.0.0.0/0` ingress on non-standard ports; `RemovalPolicy.RETAIN` on data.

## Category suppression
Do NOT raise input-validation/error-handling findings on declarative IaC (CDK),
test code, or trusted internal inputs with no evidence of bad values.
