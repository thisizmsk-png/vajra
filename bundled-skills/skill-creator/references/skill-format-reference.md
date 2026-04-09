# Claude Code Skill Format — Complete Reference

## YAML Frontmatter (All Fields)

```yaml
---
# IDENTITY
name: string                    # Lowercase, hyphens only, max 64 chars. Default: directory name
description: string             # Max 1024 chars. Claude uses for auto-invocation. RECOMMENDED
argument-hint: string           # Autocomplete hint. E.g., "[url] [format]"

# INVOCATION CONTROL
disable-model-invocation: bool  # true = manual only (/name). Default: false
user-invocable: bool            # false = Claude-only, hidden from / menu. Default: true

# EXECUTION
context: string                 # "fork" for subagent isolation. Default: inline
agent: string                   # Subagent type: Explore, Plan, general-purpose, or custom
model: string                   # Model override: opus, sonnet, haiku
allowed-tools: string           # Comma-separated: "Read, Grep, Bash(git *)"

# LIFECYCLE
hooks: object                   # Skill-scoped hooks configuration
---
```

## Invocation Control Matrix

| Configuration | User Invokes | Claude Invokes | Context Loading |
|--------------|-------------|---------------|-----------------|
| Default | Yes | Yes | Description always; full on invoke |
| `disable-model-invocation: true` | Yes | No | Not in context; full on user invoke |
| `user-invocable: false` | No | Yes | Description always; full on Claude invoke |

## Directory Structure

```
my-skill/
├── SKILL.md              # Required — YAML frontmatter + markdown instructions
├── references/           # Optional — Detailed docs (loaded on demand)
├── templates/            # Optional — Templates Claude fills in
├── examples/             # Optional — Sample outputs
├── scripts/              # Optional — Executable scripts
└── assets/               # Optional — Static resources
```

## Scope Hierarchy (Higher Priority Wins)

```
Enterprise > Personal > Project
```

| Scope | Path | Applies To |
|-------|------|------------|
| Enterprise | Managed settings | All org users |
| Personal | `~/.claude/skills/<name>/SKILL.md` | All your projects |
| Project | `.claude/skills/<name>/SKILL.md` | This project only |
| Plugin | `<plugin>/skills/<name>/SKILL.md` | Where enabled |

## String Substitutions

| Variable | Description |
|----------|-------------|
| `$ARGUMENTS` | All arguments passed |
| `$ARGUMENTS[N]` or `$N` | Nth argument (0-based) |
| `${CLAUDE_SESSION_ID}` | Current session ID |
| `${CLAUDE_SKILL_DIR}` | Skill's directory path |

## Dynamic Context Injection

`!`command`` runs shell commands BEFORE Claude sees the content. Output replaces placeholder.

```markdown
PR diff: !`gh pr diff`
Branch: !`git branch --show-current`
```

## Context Budget

- Descriptions loaded at 2% of context window (fallback: 16,000 chars)
- Full content loads only on invocation
- Override: `SLASH_COMMAND_TOOL_CHAR_BUDGET` env var
- Check with `/context` command

## Permission Control

```
Skill(commit)        # Allow exact skill
Skill(review-pr *)   # Allow prefix match
Skill(deploy *)      # Deny in deny rules
Skill                # Deny all skills
```

## Extended Thinking

Include "ultrathink" anywhere in SKILL.md content to enable extended thinking mode.

## Monorepo Auto-Discovery

Claude auto-discovers skills from nested `.claude/skills/` directories.
Editing files in `packages/frontend/` also searches `packages/frontend/.claude/skills/`.

## Agent Skills Open Standard

Skills follow the open standard at agentskills.io. Same SKILL.md works in:
- Claude Code (full features)
- Codex CLI (base format)
- ChatGPT (base format)

Claude-specific extensions (context: fork, agent:, allowed-tools:, !`commands`, hooks) are not portable.
