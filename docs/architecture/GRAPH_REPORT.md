# Graph Report - .  (2026-04-09)

## Corpus Check
- Corpus is ~22,617 words - fits in a single context window. You may not need a graph.

## Summary
- 168 nodes · 229 edges · 16 communities detected
- Extraction: 90% EXTRACTED · 10% INFERRED · 0% AMBIGUOUS · INFERRED: 22 edges (avg confidence: 0.87)
- Token cost: 0 input · 0 output

## God Nodes (most connected - your core abstractions)
1. `Vajra Agent Harness` - 12 edges
2. `getDb()` - 10 edges
3. `Vajra SKILL.md Entrypoint` - 10 edges
4. `4-Tier Router Instructions` - 8 edges
5. `Red Team Orchestrator` - 8 edges
6. `computeHmac()` - 7 edges
7. `Fleet Coordinator Protocol` - 7 edges
8. `Atman Practice Loop (Self-Improvement)` - 7 edges
9. `verifyHmac()` - 6 edges
10. `createCheckpoint()` - 6 edges

## Surprising Connections (you probably didn't know these)
- `4-Tier Smart Routing` --conceptually_related_to--> `4-Tier Router Instructions`  [INFERRED]
  README.md → engine/router.md
- `Tiered Memory (L1/L2/L3)` --conceptually_related_to--> `Tiered Memory Architecture (L1/L2/L3)`  [INFERRED]
  README.md → memory/tiered-memory.md
- `Red-Team Engine` --conceptually_related_to--> `Red Team Orchestrator`  [INFERRED]
  README.md → redteam/orchestrator.md
- `Cortex Fleet Crew Execution` --references--> `Fleet Coordinator Protocol`  [INFERRED]
  engine/cortex-bridge.md → fleet/coordinator.md
- `Campaign Persistence (SQLite + HMAC)` --conceptually_related_to--> `Checkpoint / Resume Reference`  [INFERRED]
  README.md → engine/checkpoint.md

## Hyperedges (group relationships)
- **Fleet Security Audit Crew** — readme_fleet_crews, coordinator_fleet_protocol, cortex_bridge_crew_execution, discovery_relay_protocol [EXTRACTED 1.00]
- **Atman Continuous Improvement Loop** — atman_practice_loop, practice_observer, self_review_process, peer_review_pairs, atman_promote_phase, muscle_memory_routing_learning [EXTRACTED 1.00]
- **Red Team Testing Pipeline** — redteam_orchestrator, redteam_security_contracts, redteam_attack_scenarios, redteam_scorer, redteam_reporting [EXTRACTED 1.00]
- **Tiered Memory System** — tiered_memory_architecture, tiered_memory_l1_index, tiered_memory_l2_topics, tiered_memory_l3_archive, dream_consolidation_protocol, tiered_memory_sanitizer [EXTRACTED 1.00]
- **4-Tier Smart Routing Cascade** — router_four_tier, router_tier1_regex, router_tier2_campaign, router_tier3_keyword, router_tier4_llm, router_authorization_gate [EXTRACTED 1.00]
- **Security and Integrity System** — skill_state_integrity, discovery_relay_hmac_signing, checkpoint_hmac_verification, checkpoint_timing_safe, practice_observer_log_maintenance, redteam_report_encryption, readme_supply_chain_integrity [INFERRED 0.85]
- **Security Audit Fleet Crew** — bhishma_security_engineer, duryodhana_red_team, shakuni_offensive_security, vidura_qa_engineer [EXTRACTED 1.00]
- **Feature Build Fleet Crew** — arjuna_principal_sde, bhima_senior_backend, nakula_senior_frontend, vidura_qa_engineer [EXTRACTED 1.00]
- **Incident Response Fleet Crew** — ashwatthama_sre_incident, hanuman_devops_sre, bhima_senior_backend [EXTRACTED 1.00]
- **Architecture Review Fleet Crew** — yudhishthira_cto, arjuna_principal_sde, bhishma_security_engineer, draupadi_product_manager [EXTRACTED 1.00]

## Communities

### Community 0 - "Explore-Plan-Act Workflow"
Cohesion: 0.11
Nodes (19): Act Phase (Full Access), Explore Phase (Read-Only), Explore-Plan-Act Phase Controller, Rationale: Enforce Phases to Prevent Premature Changes, Plan Phase (Discussion Only), 87 Bundled Skills, Campaign Persistence (SQLite + HMAC), Claude Cortex Integration (+11 more)

### Community 1 - "Campaign State Engine"
Cohesion: 0.29
Nodes (16): computeHmac(), createCampaign(), createCheckpoint(), ensureDir(), getActiveCampaign(), getDb(), getHmacKey(), listCampaigns() (+8 more)

### Community 2 - "Memory & Dream Consolidation"
Cohesion: 0.13
Nodes (18): Atman Peer Review Phase (during dream), Dream Archive (L3 Pruned Content), Dream Consolidation Protocol, Dream Consolidation Steps (Dedup/Prune/Rewrite), Routing Proposals (routing-proposals.json), Engineering Peer Review Pairs, Meta Peer Review Pairs (Vajra self-review), Peer Review Pairs Matrix (+10 more)

### Community 3 - "Cortex Bridge & Routing"
Cohesion: 0.13
Nodes (18): Atman Muscle Memory (Routing Learning), Cortex Bridge (Vajra ↔ Claude Cortex), Cortex Fleet Crew Execution, Domain → Cortex Agent Map, Rationale: Vajra Routing Priority Over skill-detect.sh, Cortex skill-detect.sh Fallback, Rationale: Auto-Learn T3 vs User-Confirm T1, Muscle Memory — Routing Learns from Practice (+10 more)

### Community 4 - "Cortex Agent Hierarchy"
Cohesion: 0.16
Nodes (18): Abhimanyu — Junior Engineer, Arjuna — Principal SDE, Ashwatthama — SRE Incident Response, Bhima — Senior Backend Engineer, Bhishma — Security Engineer, Draupadi — Product Manager, Duryodhana — Red Team Lead, Hanuman — DevOps / SRE (+10 more)

### Community 5 - "Red-Team Security Engine"
Cohesion: 0.14
Nodes (16): Attack Scenarios (24 Scenarios), Mutation Verification (post-test fingerprint), Red Team Orchestrator, Report Encryption (AES-256-GCM), Report Formats (HTML/JSON/SARIF), Red Team Report Generation, SARIF 2.1.0 Output (GitHub Code Scanning), Red Team Scorer (+8 more)

### Community 6 - "Atman Practice Loop"
Cohesion: 0.15
Nodes (15): Rationale: Auto-Rollback on Regression, Atman Observe Phase, Atman Practice Loop (Self-Improvement), Atman Promote Phase (patch application), Atman Safety Rules, Atman Self-Review Trigger (≥2 failures), Practice Observer, Failure Pattern Detection (+7 more)

### Community 7 - "Checkpoint & Resume"
Cohesion: 0.18
Nodes (12): Checkpoint Engine API (engine.ts), Checkpoint HMAC Verification, Checkpoint / Resume Reference, HMAC Timing-Safe Comparison (crypto.timingSafeEqual), Agent Persona Loading Protocol, Authorization Gate, Campaign Management Protocol, Cost Tracking Protocol (SKILL.md) (+4 more)

### Community 8 - "Fleet Coordinator"
Cohesion: 0.22
Nodes (11): Fleet Coordinator Protocol, fleet-coordinator.sh Script, Coordinator HMAC Relay Verification, Agent Tool Allowlists (Fleet), Worktree Isolation, Discovery HMAC Signing, Discovery Message Format (JSON Schema), Fleet Discovery Relay Protocol (+3 more)

### Community 9 - "SARIF Converter"
Cohesion: 0.54
Nodes (7): buildResult(), buildRule(), convertToSarif(), main(), readInput(), toRuleId(), toSarifLevel()

### Community 10 - "Observatory (Cost & Audit)"
Cohesion: 0.29
Nodes (8): Audit Trail, Audit Event Types (routing/campaign/memory/hook/cost/integrity), Audit Log Format (JSON events), Audit Log Retention Policy, Cost Tracker, Spend Alerts (configurable threshold), Routing Tier Cost Summary (T1-T4), Router Routing Log (audit)

### Community 11 - "Prompt Injection Sanitizer"
Cohesion: 1.0
Nodes (2): loadConfig(), sanitize()

### Community 12 - "Product Owner (Drona)"
Cohesion: 1.0
Nodes (1): Drona — Product Owner

### Community 13 - "Quant Engineer (Karna)"
Cohesion: 1.0
Nodes (1): Karna — Quant Engineer

### Community 14 - "Data Scientist (Sahadeva)"
Cohesion: 1.0
Nodes (1): Sahadeva — Data Scientist

### Community 15 - "Program Manager (Dhrishtadyumna)"
Cohesion: 1.0
Nodes (1): Dhrishtadyumna — Program Manager

## Knowledge Gaps
- **56 isolated node(s):** `Lifecycle Hooks`, `87 Bundled Skills`, `Pre-Built Fleet Crews`, `Direct Command Routing Table`, `Authorization Gate` (+51 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Product Owner (Drona)`** (1 nodes): `Drona — Product Owner`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Quant Engineer (Karna)`** (1 nodes): `Karna — Quant Engineer`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Data Scientist (Sahadeva)`** (1 nodes): `Sahadeva — Data Scientist`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Program Manager (Dhrishtadyumna)`** (1 nodes): `Dhrishtadyumna — Program Manager`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Vajra Agent Harness` connect `Explore-Plan-Act Workflow` to `Fleet Coordinator`, `Checkpoint & Resume`?**
  _High betweenness centrality (0.287) - this node is a cross-community bridge._
- **Why does `Vajra SKILL.md Entrypoint` connect `Checkpoint & Resume` to `Explore-Plan-Act Workflow`, `Memory & Dream Consolidation`, `Cortex Bridge & Routing`, `Atman Practice Loop`?**
  _High betweenness centrality (0.199) - this node is a cross-community bridge._
- **Why does `Atman Practice Loop (Self-Improvement)` connect `Atman Practice Loop` to `Memory & Dream Consolidation`, `Cortex Bridge & Routing`?**
  _High betweenness centrality (0.135) - this node is a cross-community bridge._
- **What connects `Lifecycle Hooks`, `87 Bundled Skills`, `Pre-Built Fleet Crews` to the rest of the system?**
  _56 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Explore-Plan-Act Workflow` be split into smaller, more focused modules?**
  _Cohesion score 0.11 - nodes in this community are weakly interconnected._
- **Should `Memory & Dream Consolidation` be split into smaller, more focused modules?**
  _Cohesion score 0.13 - nodes in this community are weakly interconnected._
- **Should `Cortex Bridge & Routing` be split into smaller, more focused modules?**
  _Cohesion score 0.13 - nodes in this community are weakly interconnected._