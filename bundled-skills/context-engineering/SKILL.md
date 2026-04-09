---
name: context-engineering
description: Design information architecture for AI agents — CLAUDE.md, skills, memory, MCP, subagents. Activate for context design, agent setup, skill authoring, memory design, MCP configuration tasks.
user-invocable: true
---

# Context Engineering

> Context engineering is the new coding. The human's job is to think clearly and specify precisely.

## When to Activate
- Setting up a new project's AI context (CLAUDE.md, skills, agents)
- Designing memory systems or skill architectures
- Optimizing token usage across skills and subagents
- Configuring MCP servers or hooks

## The Context Stack
```
Layer 0: System prompt (~50 instructions, immutable)
Layer 1: ~/.claude/ global (IDENTITY, SOUL, USER, MEMORY, skills, agents)
Layer 2: ./CLAUDE.md project-level (architecture, commands, conventions)
Layer 3: .claude/skills/ (loaded on demand by description match)
Layer 4: .claude/agents/ (invoked by name for subagent work)
Layer 5: MCP tools (loaded on demand via Tool Search)
Layer 6: Session context (conversation, tool results, plan files)
```

## CLAUDE.md Design Principles
- **Budget**: ~150 instructions max before compliance drops. System prompt uses ~50. You get ~100.
- **Line test**: "Would Claude make a mistake without this line?" No → delete it.
- **Progressive disclosure**: Reference `@docs/detailed.md` instead of embedding everything
- **What goes where:**

| Content | Where | Why |
|---------|-------|-----|
| Personal preferences | `~/.claude/CLAUDE.md` | Apply to ALL projects |
| Project architecture | `./CLAUDE.md` | Shared via git |
| Domain knowledge | `.claude/skills/*/SKILL.md` | Loaded only when relevant |
| Role definitions | `.claude/agents/*.md` | Invoked by name |
| Secrets, local overrides | `.claude/settings.local.json` | Gitignored |

## Skill Architecture (DBS Framework)
1. **Direction** (SKILL.md) — When to activate, process phases, guardrails
2. **Blueprints** (references/) — Detailed patterns, examples, templates
3. **Solutions** (scripts/) — Executable tools, validators, formatters

**Quality bar for a skill:**
- Must have executable structure (phases/checklists, not essays)
- Must be concrete enough to implement without interpretation
- Must justify being a skill vs a one-off prompt
- Must integrate zero-bias where decisions are made
- Description field < 250 chars (this is how auto-detection works)

## Subagent Design
```markdown
# .claude/agents/my-agent.md
---
name: my-agent
model: sonnet        # opus for complex, sonnet for focused, haiku for simple
allowed-tools: [Read, Grep, Glob]  # principle of least privilege
---
[Role definition, scope, guardrails]
```

**Routing rules:**
- Main session (Opus): complex reasoning, user interaction, architectural decisions
- Subagents (Sonnet): focused implementation, research, file processing
- Subagents (Haiku): simple scanning, formatting, categorization
- Parallel dispatch: 3+ independent tasks with no shared state

## Token Optimization
- Skills load by description match (keep descriptions precise, not broad)
- MCP tools load via Tool Search (~95% savings vs loading all definitions)
- Subagents isolate context (verbose research doesn't pollute main session)
- `@path/import` syntax loads docs on demand, not on every session
- Inline MCP in subagent frontmatter avoids loading schemas in main context

## Memory Design
```
~/.claude/memory/
├── decisions/     # Why we chose X over Y (audit trail)
├── projects/      # Project status snapshots
├── people/        # Collaborator profiles
├── daily/         # Session journals
└── topics/        # Deep knowledge on specific subjects
```
- Keep MEMORY.md < 100 lines (index only, not content)
- Be ruthless about removing stale entries
- When you correct Claude, update the relevant memory immediately

## Guardrails
- NEVER stuff everything into CLAUDE.md — use progressive disclosure
- NEVER create a skill for something a one-off prompt handles
- ALWAYS test new skills with a dry run before committing
- Token cost of context is real — every skill loaded is tokens stolen from actual work
