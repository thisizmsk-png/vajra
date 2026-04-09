# Vajra — Agent Harness for Claude Code

Smart routing, campaign persistence, fleet mode, red-team testing, self-improvement, and 87 bundled skills for Claude Code.

## Quick Install

```bash
git clone https://github.com/thisizmsk-png/vajra.git ~/.claude/skills/vajra
bash ~/.claude/skills/vajra/scripts/install.sh

# (Optional) Install bundled skills + 17 Cortex agents
bash ~/.claude/skills/vajra/scripts/install-skills.sh
```

## How It Works

```
User Input → 4-Tier Router → Skill Execution → Atman Observation
                 │                   │                  │
            T1: regex (0 tok)   Cortex agent       Log outcome
            T2: campaign (0)    assigned per        Check for
            T3: keyword (0)     domain              failure
            T4: LLM (~500)                          patterns
                 │                   │                  │
            Authorization       Phase enforced     Routing learns
            Gate                (explore→plan→act)  shortcuts
```

### The Routing Cascade

Every user input goes through 4 tiers, stopping at the first match:

| Tier | Method | Cost | Example |
|------|--------|------|---------|
| T1 | Regex pattern match (RE2) | 0 tokens | `/test auth` → testing |
| T2 | Active campaign resume | 0 tokens | continues where you left off |
| T3 | Keyword lookup | 0 tokens | "deploy staging" → ci-cd |
| T4 | LLM classification | ~500 tokens | complex/ambiguous tasks |

**Target: 80% of tasks route at T1-T3 (zero tokens).** Atman's muscle memory continuously moves T4 routes down to T3/T1.

### The Campaign Lifecycle

Multi-step tasks persist across sessions:

```
/vajra "migrate database to PostgreSQL"
  → Creates campaign (SQLite + HMAC-SHA256)
  → Step 1: Explore schema ──── checkpoint ✓
  → Step 2: Write migration ─── checkpoint ✓
  → [session ends]

/vajra continue
  → Verifies HMAC integrity
  → Resumes at Step 3: Test migration
  → Step 4: Apply ──────────── checkpoint ✓
  → Campaign complete
```

### The Atman Practice Loop

Skills improve through use — like a basketball player practicing daily:

```
Every shot tracked     →  practice-log.jsonl (HMAC-signed, append-only)
Review game tape       →  Self-review on ≥2 failures of same skill
Teammates review       →  Peer review by domain-paired skills
Muscle memory builds   →  Routing learns shortcuts from repetition
Bad patterns corrected →  Auto-rollback if patched skill regresses
```

## Architecture

Full architecture doc with data flow diagrams, module reference, security layers, and agent hierarchy:
**[docs/architecture/ARCHITECTURE.md](docs/architecture/ARCHITECTURE.md)**

Interactive knowledge graph (168 nodes, 229 edges, 16 communities):
**[docs/architecture/graph.html](docs/architecture/graph.html)** — open in browser

Graph analysis report:
**[docs/architecture/GRAPH_REPORT.md](docs/architecture/GRAPH_REPORT.md)**

### Directory Structure

```
vajra/
├── SKILL.md                  ← Entry point (Claude Code loads this)
├── config/default.json       ← All config: routing, security, fleet, cortex
│
├── engine/                   ← Core systems
│   ├── engine.ts             ← SQLite state machine + HMAC integrity
│   ├── router.md             ← 4-tier routing cascade
│   ├── sanitizer.ts          ← Prompt injection defense (22 patterns)
│   ├── checkpoint.md         ← Save/resume across sessions
│   ├── explore-plan-act.md   ← Phase enforcement (read-only → discuss → execute)
│   └── cortex-bridge.md      ← Maps 16 domains → Cortex agents
│
├── fleet/                    ← Parallel agent orchestration
│   ├── fleet-coordinator.sh  ← Process supervisor (worktrees, PID tracking)
│   ├── coordinator.md        ← Fleet protocol
│   └── discovery-relay.md    ← HMAC-signed inter-agent messaging
│
├── memory/                   ← Tiered knowledge persistence
│   ├── tiered-memory.md      ← L1 index → L2 topics → L3 archive
│   └── dream-consolidation.md ← Memory compaction protocol
│
├── redteam/                  ← Security self-testing
│   ├── orchestrator.md       ← Test coordinator (read-only snapshots)
│   ├── scorer.md             ← Promptfoo-compatible scoring
│   ├── reporting.md          ← HTML/JSON/SARIF output
│   └── scenarios/            ← 6 files × 4 attacks = 24 scenarios
│
├── atman/                    ← Self-improvement loop
│   ├── ATMAN.md              ← Practice loop overview
│   ├── practice-observer.md  ← Log every skill execution
│   ├── self-review.md        ← Analyze failure patterns
│   ├── peer-review-pairs.md  ← 20+ cross-skill review pairs
│   └── muscle-memory.md      ← Routing learns from repetition
│
├── hooks/                    ← Claude Code lifecycle hooks
│   ├── pre-tool-use.sh       ← Block dangerous commands (fail-closed)
│   └── session-start.sh      ← Auto-resume campaigns
│
├── observatory/              ← Monitoring
│   ├── cost-tracker.md       ← Token accounting + spend alerts
│   └── audit-trail.md        ← Event logging
│
├── bundled-agents/           ← 17 Mahabharat-themed agents + sepoys
├── bundled-skills/           ← 87 skills (design, security, testing, etc.)
├── scripts/                  ← Install scripts, manifest generator
├── docs/architecture/        ← Architecture docs + graphify outputs
└── manifest.json             ← SHA-256 supply chain checksums
```

## Cortex Integration

If you use [Claude Cortex](https://github.com/thisizmsk-png/claude-cortex), Vajra auto-routes tasks to the right Mahabharat agent:

```
                    Krishna (CEO)
                   /      |       \
          Yudhishthira  Draupadi  Duryodhana
          (CTO)         (PM)      (Red Team)
         /    |    \      |
     Arjuna Bhishma Hanuman  Drona
     (SDE)  (Sec)   (SRE)   (PO)
     / | \    |
  Bhima Nakula Vidura  Shakuni
  (BE)  (FE)   (QA)   (Pentest)
```

**Pre-built fleet crews:**
```
/vajra fleet security-audit       # Bhishma + Duryodhana + Shakuni + Vidura
/vajra fleet feature-build        # Arjuna + Bhima + Nakula + Vidura
/vajra fleet incident-response    # Ashwatthama + Hanuman + Bhima
/vajra fleet architecture-review  # Yudhishthira + Arjuna + Bhishma + Draupadi
```

## Commands

```
/vajra [task]              Smart-routed task execution
/vajra continue            Resume last campaign
/vajra status              Campaign status + cost
/vajra checkpoint          Save current state
/vajra rollback <id>       Restore to checkpoint
/vajra memory <query>      Search tiered memory
/vajra dream               Consolidate memory + apply approved patches
/vajra fleet <tasks>       Parallel agents in worktrees
/vajra redteam <tgt>       Security test a skill/agent
/vajra config              Edit settings
/vajra verify              Check file integrity
/vajra atman status        Practice stats + improvement metrics
/vajra atman review        Approve/reject pending skill patches
/vajra atman log           Evolution audit trail
/vajra atman rollback <id> Revert a specific skill patch
/vajra help                Command reference
```

## Security

Six defense layers — see [ARCHITECTURE.md](docs/architecture/ARCHITECTURE.md#security-architecture) for details:

1. **Input sanitization** — 22 injection patterns stripped, control chars escaped, `<untrusted-data>` wrapping
2. **Tool authorization** — Pre-tool hook (fail-closed), per-agent allowlists, authorization gate
3. **State integrity** — HMAC-SHA256 on campaigns, discoveries, practice log. Timing-safe comparison.
4. **Isolation** — Fleet in git worktrees, red-team on read-only snapshots, explore phase read-only
5. **Observability** — Audit trail, cost tracking, practice log
6. **Supply chain** — SHA-256 manifest on all files, `/vajra verify`

## Requirements

- Claude Code CLI v2.1+
- Git
- Node.js 18+

## What Makes Vajra Different

| Feature | Vajra | Other Harnesses |
|---------|-------|-----------------|
| Self-improvement | Atman practice loop (skills evolve) | Static instructions |
| Routing cost | 80% at 0 tokens (T1-T3) | Every task costs tokens |
| Campaign persistence | SQLite + HMAC across sessions | Ephemeral context |
| Fleet mode | Parallel agents in worktrees | Sequential only |
| Security testing | 24 built-in red-team scenarios | Manual only |
| Agent hierarchy | 17 specialized agents + sepoys | Generic agents |

## License

MIT
