# Cortex Bridge — Vajra ↔ Claude Cortex Integration

## Overview

This bridge connects Vajra's orchestration engine with Claude Cortex's 17 Mahabharat-named agents. Vajra handles routing, persistence, and fleet coordination. Cortex provides the specialized agent personas and domain expertise.

## How Routing Uses Cortex Agents

When Vajra's Tier 4 (LLM classification) determines a task's domain, it checks `config.cortex.agentRouting` to assign the right Cortex agent:

### Agent Assignment Rules

1. **Read the task domain** from Tier 4 classification (backend, frontend, security, testing, etc.)
2. **Look up `cortex.agentRouting[domain]`** to get the agent name, model, and skill set
3. **Load the agent's persona** from `~/.claude/agents/{agent}.md`
4. **Adopt the agent's role** for the duration of this task/campaign node
5. **Restrict available skills** to the agent's designated skill list

### Domain → Agent Map

| Domain | Agent | Role | Model |
|--------|-------|------|-------|
| Backend work | **Bhima** | Senior Backend Engineer | Opus |
| Frontend work | **Nakula** | Senior Frontend Engineer | Opus |
| Security audit | **Bhishma** | Security Engineer | Opus |
| Red team | **Duryodhana** | Red Team Lead | Opus |
| Penetration test | **Shakuni** | Offensive Security | Sonnet |
| Testing/QA | **Vidura** | QA Engineer | Sonnet |
| DevOps/CI/CD | **Hanuman** | DevOps/SRE | Sonnet |
| Incidents | **Ashwatthama** | SRE Incident Response | Sonnet |
| Product spec | **Draupadi** | Product Manager | Opus |
| Architecture | **Yudhishthira** | CTO | Opus |
| Code review | **Arjuna** | Principal SDE | Opus |
| Data/analytics | **Sahadeva** | Data Scientist | Opus |
| Quant work | **Karna** | Quant Engineer | Opus |
| Junior tasks | **Abhimanyu** | Junior Engineer | Sonnet |
| Strategy | **Krishna** | CEO | Opus |
| Program mgmt | **Dhrishtadyumna** | Program Manager | Sonnet |

## How Fleet Mode Uses Cortex Crews

When `/vajra fleet` is invoked, check if a pre-defined crew matches the task type:

### Pre-Built Crews (from `cortex.fleetCrews`)

**Security Audit Crew:**
- Lead: Bhishma (threat model)
- Agents: Duryodhana (red team), Shakuni (pentest), Vidura (QA validation)
- Use: `/vajra fleet security-audit`

**Feature Build Crew:**
- Lead: Arjuna (principal SDE)
- Agents: Bhima (backend), Nakula (frontend), Vidura (tests)
- Use: `/vajra fleet feature-build`

**Incident Response Crew:**
- Lead: Ashwatthama (SRE)
- Agents: Hanuman (DevOps), Bhima (backend debug)
- Use: `/vajra fleet incident-response`

**Architecture Review Crew:**
- Lead: Yudhishthira (CTO)
- Agents: Arjuna (code), Bhishma (security), Draupadi (product)
- Use: `/vajra fleet architecture-review`

### How Crew Execution Works

1. Parse crew name from `/vajra fleet {crew-name}` or auto-detect from task
2. Load crew definition from `cortex.fleetCrews`
3. For each agent in the crew:
   a. Read persona from `~/.claude/agents/{agent}.md`
   b. Spawn in isolated worktree with the agent's context as system prompt
   c. Restrict tools to agent's skill set
4. Lead agent coordinates — receives discoveries from all agents
5. Merge results when all complete

## Cortex skill-detect.sh Integration

When `cortex.enabled = true`, Vajra's routing takes priority over Cortex's `skill-detect.sh` hook:

- If Vajra routes at Tier 1-3 (zero tokens): skip skill-detect.sh, Vajra already knows
- If Vajra routes at Tier 4 (LLM): Vajra's classification is more accurate than keyword grep
- skill-detect.sh becomes a **fallback** for prompts that bypass `/vajra` entirely

## Agent Persona Loading

When loading a Cortex agent persona into context:

1. Read `~/.claude/agents/{agent}.md`
2. Verify the file exists under `~/.claude/agents/` (operator-controlled path — do NOT sanitize these files). Agent personas are authored by the operator and intentionally contain identity statements like "You are Bhima". Sanitizing them would strip the persona directives and break agent role adoption.
3. Extract frontmatter: name, role, model, allowed-tools
4. Inject persona into the subagent's system context
5. Log agent assignment in campaign audit trail

**Important:** Only user-supplied content (memory files, user messages, discovery relay content) goes through the sanitizer. Operator-controlled files under `~/.claude/agents/` are trusted and loaded verbatim.
