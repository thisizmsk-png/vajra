---
name: observability
description: Monitoring, logging, alerting, SLO design. Activate for monitoring, logging, alerting, metrics, dashboards, SLO, SLI, observability tasks.
user-invocable: true
---

# Observability Designer

## When to Activate
- User mentions: monitoring, logging, alerting, metrics, dashboards, SLO, SLI, observability, uptime, error rate
- Deploying a new service to production
- Debugging production issues or silent failures
- Designing health checks or status pages

## Three Pillars

### 1. Metrics (What happened?)
**Method selection by system type:**
| System Type | Framework | Key Metrics |
|------------|-----------|-------------|
| Request-driven (APIs) | RED | Rate, Errors, Duration |
| Resource-driven (infra) | USE | Utilization, Saturation, Errors |
| Queue/pipeline | LETS | Length, Error rate, Throughput, Staleness |
| Business | Custom | Revenue, conversion, churn, active users |

### 2. Logs (Why did it happen?)
**Structured logging rules:**
- JSON format (machine-parseable)
- Include: timestamp, level, message, request_id, user_id, service, duration_ms
- Levels: DEBUG (development), INFO (business events), WARN (recoverable), ERROR (needs attention), FATAL (system down)
- Never log: passwords, tokens, PII, credit cards
- Retention: 30 days hot, 90 days warm, 1 year cold

### 3. Traces (Where did it happen?)
- Distributed tracing across service boundaries
- Span: single operation with start/end time
- Trace: full request lifecycle across services
- Correlation ID: propagated across all services for a single request

## SLO Framework

### Step 1: Define SLIs (Service Level Indicators)
| SLI Type | Measurement |
|----------|------------|
| Availability | successful requests / total requests |
| Latency | % of requests < threshold (e.g., P99 < 500ms) |
| Throughput | requests/second capacity |
| Correctness | valid outputs / total outputs |
| Freshness | time since last successful update |

### Step 2: Set SLOs (Service Level Objectives)
- Availability: 99.9% = 43 min downtime/month
- Latency: P50 < 100ms, P99 < 500ms
- Error budget: 100% - SLO = budget for experimentation/risk

### Step 3: Alert on Burn Rate
- Don't alert on every error — alert when error budget burns too fast
- 1h window burning 2% budget = page immediately
- 6h window burning 5% budget = alert on-call
- 24h window burning 10% budget = ticket for next sprint

## Dashboard Design

**Rules:**
- 7±2 panels per dashboard (cognitive load limit)
- 80/20 ratio: 80% monitoring, 20% debugging detail
- Role-based: exec (business KPIs), eng (system health), on-call (error triage)
- Time range: default 24h, allow zoom to 7d/30d
- Every panel answers ONE question

**Layout:**
```
Row 1: Traffic + Error Rate + Latency (the big 3)
Row 2: Resource utilization (CPU, Memory, Disk, Network)
Row 3: Business metrics (conversions, revenue, queue depth)
Row 4: Dependencies (DB latency, API health, cache hit rate)
```

## Pipeline-Specific Observability
For batch pipelines like YouTube automation:

| Metric | What to Track |
|--------|--------------|
| Pipeline success rate | completed / attempted runs |
| Stage duration | time per stage (scraping, writing, rendering, upload) |
| Failure stage | which stage fails most often |
| Queue depth | pending topics, pending uploads |
| Resource usage | Ollama VRAM, FFmpeg CPU, disk space |
| Data freshness | time since last successful upload per channel |
| Alert: stale channel | no upload in >3 days for scheduled channel |

## Process

### Phase 1: Identify What Matters
1. What are the user-facing SLIs? (availability, latency, correctness)
2. What are the business KPIs? (uploads/day, views, revenue)
3. What are the operational indicators? (queue depth, error rate, resource usage)

### Phase 2: Instrument
1. Add structured logging at service boundaries
2. Add metrics for RED/USE/LETS as appropriate
3. Add health check endpoints
4. Add correlation IDs for request tracing

### Phase 3: Alert
1. Define SLOs and error budgets
2. Configure burn-rate alerts (not threshold alerts)
3. Every alert must have a runbook link
4. Test alerts in staging before production

## Guardrails
- NEVER alert on symptoms that auto-resolve (flapping alerts cause alert fatigue)
- NEVER monitor what you won't act on (unused dashboards are waste)
- ALWAYS start with the 3 golden signals (rate, errors, duration) before adding more
- Apply Zero Cognitive Bias — don't over-instrument because it feels productive
