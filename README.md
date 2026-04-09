# Vajra — Agent Harness for Claude Code

Smart routing, campaign persistence, fleet mode, red-team testing, and 87 bundled skills for Claude Code.

## Quick Install

```bash
# Clone into your Claude Code skills directory
git clone https://github.com/thisizmsk-png/vajra.git ~/.claude/skills/vajra

# Run the install script
bash ~/.claude/skills/vajra/scripts/install.sh

# (Optional) Install bundled skills — copies 87 skills to ~/.claude/skills/
bash ~/.claude/skills/vajra/scripts/install-skills.sh
```

## What You Get

### Vajra Harness (core)
- **4-tier smart routing** — regex → campaign → keyword → LLM (80% of tasks at zero tokens)
- **Campaign persistence** — SQLite + HMAC integrity, checkpoint/resume across sessions
- **Tiered memory** — L1 index (always loaded) → L2 topics (on demand) → L3 archive (searchable)
- **Fleet mode** — parallel agents in git worktrees with HMAC-signed discovery relay
- **Red-team engine** — 24 attack scenarios, 6 security behaviors, HTML/JSON/SARIF reports
- **Lifecycle hooks** — PreToolUse blocks dangerous commands, SessionStart resumes campaigns
- **Explore-plan-act** — enforced read-only → discuss → execute phases
- **Cost tracking** — real-time token accounting with spend alerts
- **Supply chain integrity** — SHA-256 manifest on all files

### 87 Bundled Skills
Design, security, testing, architecture, business, data, and meta skills. Full list in `bundled-skills/`.

### Cortex Integration
If you use [Claude Cortex](https://github.com/thisizmsk-png/claude-cortex), Vajra auto-routes tasks to the right Mahabharat agent:

| Domain | Agent | Role |
|--------|-------|------|
| Backend | Bhima | Senior Backend Engineer |
| Frontend | Nakula | Senior Frontend Engineer |
| Security | Bhishma | Security Engineer |
| Red Team | Duryodhana | Red Team Lead |
| Testing | Vidura | QA Engineer |
| Architecture | Yudhishthira | CTO |
| Code Review | Arjuna | Principal SDE |
| Strategy | Krishna | CEO |

**Pre-built fleet crews:**
```
/vajra fleet security-audit       # Bhishma + Duryodhana + Shakuni + Vidura
/vajra fleet feature-build        # Arjuna + Bhima + Nakula + Vidura
/vajra fleet incident-response    # Ashwatthama + Hanuman + Bhima
/vajra fleet architecture-review  # Yudhishthira + Arjuna + Bhishma + Draupadi
```

## Commands

```
/vajra [task]         Smart-routed task execution
/vajra continue       Resume last campaign
/vajra status         Campaign status + cost
/vajra checkpoint     Save current state
/vajra rollback <id>  Restore to checkpoint
/vajra memory <query> Search tiered memory
/vajra dream          Consolidate memory
/vajra fleet <tasks>  Parallel agents in worktrees
/vajra redteam <tgt>  Security test a skill/agent
/vajra config         Edit settings
/vajra verify         Check file integrity
/vajra help           Command reference
```

## Requirements

- Claude Code CLI v2.1+
- Git
- Node.js 18+

## Security

- Encryption ON by default (AES-256-GCM) for campaign state and security reports
- HMAC-SHA256 integrity on all state files and inter-agent messages
- RE2 regex engine (linear-time, no ReDoS)
- Prompt injection sanitization on all context-loaded data
- Red-team self-test runs on read-only snapshots
- Supply chain manifest with SHA-256 checksums

## Architecture

Spec'd, reviewed by 3 parallel agents (threat model, architecture critique, security audit), all 15 findings addressed. Full spec at `docs/specs/vajra-spec.md`.

## License

MIT
