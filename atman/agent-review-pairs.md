# Agent Review Pairs — Agents That Coach Each Other

Like a team where seniors review juniors, and peers cross-check each other's domain.

## Review Matrix

Based on the Cortex reporting chain — each reviewer has domain overlap with the agent being reviewed.

### Leadership Reviews (upward knowledge)
| Agent Being Patched | Reviewed By | Why This Pair |
|---------------------|-------------|---------------|
| Arjuna (Principal SDE) | Yudhishthira (CTO) | CTO validates engineering decisions |
| Bhishma (Security) | Yudhishthira (CTO) | CTO validates security posture |
| Draupadi (PM) | Krishna (CEO) | CEO validates product direction |
| Duryodhana (Red Team) | Krishna (CEO) | CEO validates adversarial scope |
| Hanuman (DevOps/SRE) | Yudhishthira (CTO) | CTO validates infra decisions |

### Peer Reviews (cross-domain expertise)
| Agent Being Patched | Reviewed By | Why This Pair |
|---------------------|-------------|---------------|
| Bhima (Backend) | Arjuna (Principal SDE) | Principal validates backend patterns |
| Nakula (Frontend) | Arjuna (Principal SDE) | Principal validates frontend patterns |
| Vidura (QA) | Arjuna (Principal SDE) | Principal validates test strategy |
| Shakuni (Pentest) | Bhishma (Security) | Security validates offensive approach |
| Ashwatthama (SRE) | Hanuman (DevOps) | DevOps validates incident response |
| Sahadeva (Data) | Yudhishthira (CTO) | CTO validates data architecture |
| Karna (Quant) | Yudhishthira (CTO) | CTO validates quant strategy |
| Drona (PO) | Draupadi (PM) | PM validates product ownership |
| Dhrishtadyumna (PgM) | Draupadi (PM) | PM validates program management |
| Abhimanyu (Junior) | Arjuna (Principal SDE) | Principal mentors junior |

### Cross-Functional Reviews (adversarial checks)
| Agent Being Patched | Reviewed By | Why This Pair |
|---------------------|-------------|---------------|
| Arjuna (Principal SDE) | Duryodhana (Red Team) | Red team stress-tests engineering |
| Bhishma (Security) | Duryodhana (Red Team) | Red team validates security claims |
| Draupadi (PM) | Vidura (QA) | QA validates product feasibility |
| Yudhishthira (CTO) | Duryodhana (Red Team) | Red team challenges architecture |

### Sepoy Reviews (by their masters)
| Agent Being Patched | Reviewed By | Why This Pair |
|---------------------|-------------|---------------|
| Sepoy Backend | Bhima (Backend) | Master reviews apprentice |
| Sepoy Frontend | Nakula (Frontend) | Master reviews apprentice |
| Sepoy Test | Vidura (QA) | Master reviews apprentice |

## What Agents Review

Unlike skills (which have SKILL.md instructions), agents have persona files with:
- **Role definition** — is the role description accurate and useful?
- **Escalation paths** — are the "escalate to" rules correct?
- **Tool restrictions** — does the allowed-tools list match the domain?
- **Decision boundaries** — are "NEVER do X" and "ALWAYS do Y" rules right?
- **Domain knowledge** — are the domain-specific instructions complete?

## Review Protocol

When reviewing an agent's patch:

1. **Read the agent's persona** from `bundled-agents/{name}.md`
2. **Read the failure traces** — what went wrong when this agent was active?
3. **Read the proposed patch** — what's being changed in the persona?
4. **Ask three questions:**
   - Does this fix address why the agent failed, not just what it did wrong?
   - Does this change conflict with the agent's reporting chain or escalation rules?
   - Would I trust this agent more with this change applied?
5. **Verdict:** APPROVE / REJECT / REVISE

## Fallback Reviewer

If no specific reviewer is mapped:
- **Engineering agents** → Arjuna (Principal SDE)
- **Security agents** → Bhishma (Security)
- **Product agents** → Draupadi (PM)
- **All agents** → Yudhishthira (CTO) as final fallback
