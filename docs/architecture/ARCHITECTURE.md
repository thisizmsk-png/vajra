# Vajra Architecture

> Generated via `/graphify` knowledge graph analysis (168 nodes, 229 edges, 16 communities).
> Interactive graph: [graph.html](graph.html) | Raw data: [graph.json](graph.json) | Report: [GRAPH_REPORT.md](GRAPH_REPORT.md)

---

## System Overview

Vajra is an agent harness for Claude Code that adds orchestration, persistence, security, and self-improvement on top of the base CLI. It is not a standalone application — it runs as a Claude Code skill that intercepts user input, routes it to the right handler, and manages state across sessions.

```
User Input
    │
    ▼
┌──────────────────────────────────────────────────────────────┐
│  VAJRA HARNESS (SKILL.md entry point)                        │
│                                                              │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐     │
│  │  4-Tier      │   │  Campaign    │   │  Tiered      │     │
│  │  Router      │──▶│  Engine      │──▶│  Memory      │     │
│  │  (router.md) │   │  (engine.ts) │   │  (L1/L2/L3)  │     │
│  └──────┬───────┘   └──────┬───────┘   └──────┬───────┘     │
│         │                  │                   │             │
│  ┌──────▼───────┐   ┌──────▼───────┐   ┌──────▼───────┐     │
│  │  Cortex      │   │  Checkpoint  │   │  Dream       │     │
│  │  Bridge      │   │  / Resume    │   │  Consolidate │     │
│  └──────────────┘   └──────────────┘   └──────────────┘     │
│                                                              │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐     │
│  │  Fleet       │   │  Red-Team    │   │  Atman       │     │
│  │  Coordinator │   │  Engine      │   │  (Practice)  │     │
│  └──────────────┘   └──────────────┘   └──────────────┘     │
│                                                              │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐     │
│  │  Hooks       │   │  Sanitizer   │   │  Observatory │     │
│  │  (pre-tool)  │   │  (.ts)       │   │  (cost+audit)│     │
│  └──────────────┘   └──────────────┘   └──────────────┘     │
└──────────────────────────────────────────────────────────────┘
    │                                              │
    ▼                                              ▼
┌──────────────┐                        ┌──────────────────┐
│  87 Bundled  │                        │  17 Cortex       │
│  Skills      │                        │  Agents + Sepoys │
└──────────────┘                        └──────────────────┘
```

---

## Data Flow: Task Lifecycle

```
1. User types: "review the auth module for security issues"
                          │
2. ROUTING ───────────────┤
   T1: regex check        │ ✗ no match
   T2: campaign check     │ ✗ no active campaign
   T3: keyword "security" │ ✓ → sentry-security-review
                          │
3. AUTHORIZATION ─────────┤
   Check tool allowlist   │ ✓ Read,Grep,Glob allowed
                          │
4. CORTEX BRIDGE ─────────┤
   Domain: security       │ → Bhishma (Security Engineer, Opus)
                          │
5. PHASE ENFORCEMENT ─────┤
   Start in EXPLORE       │ read-only tools only
   → PLAN                 │ present findings, get approval
   → ACT                  │ apply fixes with full tools
                          │
6. CAMPAIGN (if multi-step)
   Create campaign        │ → SQLite + HMAC
   Checkpoint each step   │ → rollback-safe
   Resume next session    │ /vajra continue
                          │
7. OBSERVE (Atman) ───────┤
   Log: skill=security-review, outcome=success
   Check failure patterns │ none → no patch needed
   Check routing learning │ "security" already T3 → skip
                          │
8. COST TRACKING ─────────┤
   T3 routing: 0 tokens   │
   Task execution: ~2000  │ check against alertThreshold
```

---

## Module Reference

### Engine Layer (`engine/`)

| File | Purpose | Key Concepts |
|------|---------|--------------|
| `engine.ts` | SQLite state machine | `createCampaign()`, `saveCampaign()`, `createCheckpoint()`, `rollbackToCheckpoint()`, HMAC-SHA256 integrity on all state |
| `router.md` | 4-tier routing cascade | T1 regex (RE2, 0 tokens) → T2 campaign resume → T3 keyword → T4 LLM (~500 tokens). Authorization gate after every route. |
| `checkpoint.md` | Save/restore protocol | When to checkpoint, how to resume, HMAC verification, error handling for tampering |
| `explore-plan-act.md` | Phase enforcement | EXPLORE (read-only) → PLAN (discussion) → ACT (full tools). Prevents premature code changes. |
| `cortex-bridge.md` | Agent persona routing | Maps 16 domains → Cortex agents. Loads persona from `~/.claude/agents/{name}.md`, restricts tools per agent. |
| `sanitizer.ts` | Prompt injection defense | Strips 22 injection patterns, escapes control chars, wraps loaded content in `<untrusted-data>` tags. Config-driven via `security.sanitization` in default.json. |

### Fleet Layer (`fleet/`)

| File | Purpose | Key Concepts |
|------|---------|--------------|
| `fleet-coordinator.sh` | Process supervisor (~300 LOC bash) | Spawns parallel Claude Code processes in git worktrees. PID tracking, exit code collection, trap-based cleanup. |
| `coordinator.md` | Fleet protocol instructions | Parse tasks → create worktrees → spawn agents → collect discoveries → merge report |
| `discovery-relay.md` | Inter-agent messaging | JSON format with HMAC-SHA256 signing. Types: finding, blocker, insight. Always wrapped in `<untrusted-data>` even after verification. |

### Memory Layer (`memory/`)

| File | Purpose | Key Concepts |
|------|---------|--------------|
| `tiered-memory.md` | 3-tier architecture | L1 index (always loaded, ≤200 lines) → L2 topics (on demand) → L3 archive (search only). All content sanitized before context injection. |
| `dream-consolidation.md` | Memory compaction | Triggered when L1 > 200 lines or via `/vajra dream`. Dedup → resolve contradictions → prune stale → rewrite → archive pruned content to L3. |

### Red-Team Layer (`redteam/`)

| File | Purpose | Key Concepts |
|------|---------|--------------|
| `orchestrator.md` | Test coordinator | Creates read-only snapshot, runs 24 attack scenarios across 6 security behaviors, uses mutation verification (fingerprinting). |
| `scorer.md` | Vulnerability scoring | Promptfoo-compatible scoring. Maps behaviors to pass/fail criteria. |
| `reporting.md` | Report generation | HTML (interactive), JSON (machine-readable), SARIF 2.1.0 (GitHub Code Scanning). Reports encrypted with AES-256-GCM. |
| `scenarios/` | 6 scenario files | prompt-injection, sandbox-escape, tool-bypass, session-boundary, config-drift, protocol-security. 4 attacks each = 24 total. |

### Atman Layer (`atman/`) — Self-Improvement

| File | Purpose | Key Concepts |
|------|---------|--------------|
| `ATMAN.md` | Practice loop overview | 5 phases: Observe → Self-Review → Peer Review → Promote → Muscle Memory. Safety: max 5 patches/dream, no self-approve, auto-rollback on regression. |
| `practice-observer.md` | Execution logging | Appends to `practice-log.jsonl` after every skill use. Outcome classification: success/failure/partial/rejected. HMAC on each line. |
| `self-review.md` | Failure analysis | Triggered on ≥2 failures of same skill. Reads SKILL.md + failure traces, generates targeted patch (≤10 lines net new). Anti-bloat rules. |
| `peer-review-pairs.md` | Cross-skill review | 20+ domain pairs (code-review ↔ systematic-debugging, hld ↔ lld, etc.). 3-question protocol: root cause? contradictions? followable? |
| `muscle-memory.md` | Routing optimization | T4→T3 after 3 identical routings (auto). T4→T1 regex proposals (user confirm). Tracks token savings. |

### Observatory (`observatory/`)

| File | Purpose | Key Concepts |
|------|---------|--------------|
| `cost-tracker.md` | Token accounting | Reads rates from config, tracks per-operation cost, spend alerts at configurable threshold. |
| `audit-trail.md` | Event logging | JSON events: routing, campaign, memory, hook, cost, integrity. Retention policy. |

### Hooks (`hooks/`)

| File | Purpose | Key Concepts |
|------|---------|--------------|
| `pre-tool-use.sh` | Tool blocking | Blocks dangerous Bash commands (rm -rf, mkfs, dd, etc.). Fail-closed on parse failure. Sanitized audit log. |
| `session-start.sh` | Campaign resume | Checks for active campaigns on session start, offers to resume. |

### Config (`config/`)

| File | Purpose | Key Concepts |
|------|---------|--------------|
| `default.json` | Central configuration | Routing rules (T1 patterns, T3 keywords), RE2 engine, campaign paths, memory settings, cost rates, security config (22 strip patterns, AES-256-GCM, HMAC-SHA256), fleet settings (max 6 agents, tool allowlist), Cortex integration (16 agent routings, 4 fleet crews). |
| `security-contracts.json` | Security behavior specs | Defines the 6 security behaviors tested by the red-team engine. |

---

## Cortex Agent Hierarchy

```
                    Krishna (CEO, God, Opus)
                   /          |            \
        Yudhishthira     Draupadi      Duryodhana
        (CTO, Opus)      (PM, Opus)    (Red Team, Opus)
       /    |    \          |    \           |
   Arjuna  Bhishma  Hanuman  Drona  Dhrishtadyumna
   (SDE)   (Sec)    (SRE)   (PO)    (PgM)
   / | \     |        |
Bhima Nakula Vidura  Shakuni
(BE)  (FE)   (QA)   (Pentest)
  |    |      |
Sepoy Sepoy  Sepoy
(BE)  (FE)   (QA)

[Opus agents] = strategic decisions, architecture, security
[Sonnet agents] = execution, operations, junior tasks
```

### Pre-Built Fleet Crews

| Crew | Lead | Members | Use Case |
|------|------|---------|----------|
| Security Audit | Bhishma | Duryodhana, Shakuni, Vidura | `/vajra fleet security-audit` |
| Feature Build | Arjuna | Bhima, Nakula, Vidura | `/vajra fleet feature-build` |
| Incident Response | Ashwatthama | Hanuman, Bhima | `/vajra fleet incident-response` |
| Architecture Review | Yudhishthira | Arjuna, Bhishma, Draupadi | `/vajra fleet architecture-review` |

---

## Security Architecture

### Defense in Depth

```
Layer 1: Input Sanitization
  └─ 22 regex strip patterns for prompt injection
  └─ Control character escaping
  └─ <untrusted-data> wrapping for all loaded content

Layer 2: Tool Authorization
  └─ Pre-tool hook blocks dangerous commands (fail-closed)
  └─ Per-agent tool allowlists in fleet mode
  └─ Authorization gate after every routing decision

Layer 3: State Integrity
  └─ HMAC-SHA256 on all campaign state (hex key, timing-safe compare)
  └─ HMAC on fleet discovery relay messages
  └─ HMAC on practice log entries (append-only)

Layer 4: Isolation
  └─ Fleet agents in separate git worktrees
  └─ Red-team tests on read-only snapshots
  └─ Explore phase enforces read-only tools

Layer 5: Observability
  └─ Audit trail logs all routing, campaign, memory, hook events
  └─ Cost tracking with spend alerts
  └─ Atman practice log tracks every skill execution

Layer 6: Supply Chain
  └─ SHA-256 manifest on all files
  └─ /vajra verify checks integrity
```

### Key Security Files

| Concern | File | Mechanism |
|---------|------|-----------|
| Prompt injection | `engine/sanitizer.ts` | Strip + escape + wrap |
| Dangerous commands | `hooks/pre-tool-use.sh` | Fail-closed blocklist |
| State tampering | `engine/engine.ts` | HMAC-SHA256, `timingSafeEqual` |
| Inter-agent trust | `fleet/discovery-relay.md` | HMAC signing + `<untrusted-data>` |
| Self-modification safety | `atman/ATMAN.md` | Max 5 patches, peer review, auto-rollback |
| Regex DoS | `config/default.json` | RE2 engine (linear time) |

---

## File System Layout

```
~/.claude/skills/vajra/          ← Skill installation (this repo)
├── SKILL.md                     ← Entry point (loaded by Claude Code)
├── config/default.json          ← All configuration
├── engine/                      ← Core: router, state machine, sanitizer
├── fleet/                       ← Parallel agent coordination
├── memory/                      ← Tiered memory docs
├── redteam/                     ← Security testing engine
├── atman/                       ← Self-improvement loop
├── hooks/                       ← Claude Code lifecycle hooks
├── observatory/                 ← Cost tracking + audit trail
├── bundled-agents/              ← 17 Cortex agents + 4 sepoy templates
├── bundled-skills/              ← 87 skills (design, security, testing, etc.)
├── scripts/                     ← Install scripts, manifest generator
├── docs/architecture/           ← This doc + graphify outputs
└── manifest.json                ← SHA-256 supply chain checksums

~/.claude/vajra/                 ← Runtime state (created on first use)
├── vajra.db                     ← SQLite campaign database
├── .hmac-key                    ← HMAC secret (chmod 600)
├── campaigns/                   ← Campaign state files
├── memory/
│   ├── index.md                 ← L1 (always loaded, ≤200 lines)
│   ├── topics/                  ← L2 (on demand)
│   └── archive/                 ← L3 (search only)
├── practice-log.jsonl           ← Atman execution traces
├── patches/                     ← Pending skill patches
├── routing-proposals.json       ← Learned routing shortcuts
├── relay/                       ← Fleet discovery messages (ephemeral)
└── worktrees/                   ← Fleet git worktrees (ephemeral)
```

---

## Knowledge Graph Communities (from graphify)

The graphify analysis identified **16 communities** in the codebase. The major ones:

| # | Community | Cohesion | Size | Description |
|---|-----------|----------|------|-------------|
| 0 | Explore-Plan-Act Workflow | 0.11 | 19 | Phase enforcement + top-level features |
| 1 | Campaign State Engine | 0.29 | 16 | SQLite, HMAC, all `engine.ts` functions |
| 2 | Memory & Dream Consolidation | 0.13 | 18 | L1/L2/L3 tiers + dream protocol + peer review |
| 3 | Cortex Bridge & Routing | 0.13 | 18 | Agent mapping + routing learning |
| 4 | Cortex Agent Hierarchy | 0.16 | 18 | All 17 agents + model assignments |
| 5 | Red-Team Security Engine | 0.14 | 16 | Scenarios, scorer, reports |
| 6 | Atman Practice Loop | 0.15 | 15 | Observe → self-review → promote |
| 7 | Checkpoint & Resume | 0.18 | 12 | Save/restore + persona loading |
| 8 | Fleet Coordinator | 0.22 | 11 | Worktrees, discovery relay, HMAC |
| 9 | SARIF Converter | 0.54 | 7 | `sarif-convert.js` functions |
| 10 | Observatory | 0.29 | 8 | Cost tracking + audit trail |
| 11 | Sanitizer | 1.00 | 2 | `sanitize()` + `loadConfig()` |

**God nodes** (most connected): Vajra Harness (12 edges), getDb() (10), SKILL.md Entrypoint (10), 4-Tier Router (8), Red-Team Orchestrator (8).

See [GRAPH_REPORT.md](GRAPH_REPORT.md) for the full analysis and [graph.html](graph.html) for the interactive visualization.
