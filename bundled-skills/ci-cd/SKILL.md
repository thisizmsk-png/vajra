---
name: ci-cd
description: "CI/CD skill — designs and maintains build, test, deploy, and rollback pipelines with infrastructure as code."
user-invocable: true
---

# CI/CD Pipeline Design

You are designing or maintaining CI/CD pipelines. Automate everything.
Manual steps are bugs in the deployment process.

## Pipeline Stages

### 1. Build
- Compile/transpile source code
- Install dependencies (locked versions)
- Generate artifacts (Docker images, bundles, packages)

### 2. Test
- Unit tests (must pass, >90% coverage)
- Integration tests (API, database, service interactions)
- Security scans (dependency audit, SAST)
- Linting and formatting checks

### 3. Deploy
- **Staging:** Automatic deploy on merge to main
- **Production:** Manual approval gate (or canary)
- **Rollback:** One-click rollback to previous version
- **Feature flags:** Decouple deploy from release

### 4. Monitor
- Health checks after deploy
- Error rate monitoring (compare to baseline)
- Performance metrics (latency, throughput)
- Automatic rollback if error rate exceeds threshold

## Deployment Strategies

| Strategy | Risk | Speed | Use When |
|----------|------|-------|----------|
| **Blue/Green** | Low | Medium | Database-compatible changes |
| **Canary** | Low | Slow | Risky changes, large user base |
| **Rolling** | Medium | Medium | Stateless services |
| **Recreate** | High | Fast | Development/staging only |

## Rules
- Infrastructure as code — all pipeline configs version-controlled
- No manual steps in production deployment
- Every deploy must be rollbackable
- Secrets never in code — use secret management (vault, env vars)
- Zero Cognitive Bias Protocol — don't skip stages because "it's a small change"
- Monitor after every deploy, not just the big ones
