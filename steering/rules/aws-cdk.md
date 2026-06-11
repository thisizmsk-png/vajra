# Steering: AWS CDK / Infrastructure (BLOCKING)

- id: aws-cdk
- domain: aws-cdk
- blocking: true
- file-patterns: ["lib/**/*.ts", "**/cdk/**"]

## Code quality (blocking)
1. **No hardcoded account IDs, regions, ARNs, partitions** — use
   `Stack.of(this).account` / `.region` / `formatArn()`. Stage-aware values
   (account IDs, bucket names, role ARNs, env names) come from a centralized
   StageConfig; add to StageConfig first, ship that CR, then consume.
2. **L2 constructs over `CfnResource`**; CDK grant methods
   (`bucket.grantRead(role)`) over hand-written PolicyStatements (which often
   miss `ListBucket`); `cdk.Tags.of()` and `cdk.Aspects` for cross-cutting.
3. **No `as` assertions / `any`** without justification — use `unknown` + narrow.
4. **Extract constructs** when a stack file exceeds 500 lines; functions > 50
   lines → decompose.

## Naming & dependencies
5. **Disambiguator + region in resource names** to prevent dev/alpha and
   multi-region conflicts (e.g. `error-alarm-${disambiguator}-${region}`).
6. **No cross-stack resource passing via props** — creates implicit
   export/import that's near-impossible to remove. Use `CfnOutput` +
   `Fn.importValue`.
7. **Multi-region/stage awareness** — handle every region (don't add an NA-only
   config and forget EU); no hardcoded `us-east-1`; `default` case in
   StackSuffix switches.

## Safety (blocking)
8. **`RemovalPolicy.RETAIN`** for stateful resources (DDB, S3) in production;
   flag `RemovalPolicy.DESTROY` on production data without justification.
9. **Dev-stack safety flags** (e.g. `devStackConfig.ts` booleans) MUST be
   `false` before merge — `true` creates expensive shared-account resources.
10. **Lambda construct invariants** — keep the `description: Timestamp: ${new
    Date().toISOString()}` change-detector line; don't silently change the
    runtime version.
11. **Codegen sync** — if a GraphQL schema / generated types pair exists, a
    schema change must include the regenerated output in the same CR.
