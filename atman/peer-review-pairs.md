# Peer Review Pairs — Skills That Coach Each Other

Like teammates who know each other's game — each pair has domain overlap that makes the review meaningful.

## Review Matrix

When a skill has a pending patch, the reviewer skill evaluates whether the patch improves the target.

### Engineering Pairs
| Skill Being Patched | Reviewed By | Why This Pair |
|---------------------|-------------|---------------|
| `code-review` | `systematic-debugging` | Debugging knows what code-review misses |
| `systematic-debugging` | `code-review` | Reviews catch patterns debuggers chase |
| `testing` | `property-based-testing` | PBT finds edge cases unit tests miss |
| `property-based-testing` | `testing` | Testing validates PBT strategies are practical |
| `hld` | `lld` | Low-level validates high-level is implementable |
| `lld` | `hld` | High-level validates low-level matches architecture |
| `api-design` | `sentry-security-review` | Security validates API attack surface |
| `database-design` | `performance-profiler` | Performance validates query patterns |
| `ci-cd` | `release-engineering` | Release validates pipeline correctness |
| `frontend-design` | `web-design-guidelines` | Guidelines validate design compliance |
| `react-best-practices` | `frontend-architecture` | Architecture validates component patterns |

### Security Pairs
| Skill Being Patched | Reviewed By | Why |
|---------------------|-------------|-----|
| `sentry-security-review` | `threat-modeling` | Threat models validate review coverage |
| `threat-modeling` | `sentry-security-review` | Reviews validate threat mitigations |
| `sentry-find-bugs` | `code-review` | Code review validates bug detection accuracy |
| `insecure-defaults` | `sentry-security-review` | Security validates default checks |

### Product Pairs
| Skill Being Patched | Reviewed By | Why |
|---------------------|-------------|-----|
| `product-spec` | `spec` | Spec validates PRD rigor |
| `spec` | `product-spec` | Product validates spec practicality |
| `brainstorming` | `grill-me` | Grilling stress-tests brainstorm output |

### Meta Pairs (Vajra reviewing itself)
| Skill Being Patched | Reviewed By | Why |
|---------------------|-------------|-----|
| `writing-plans` | `executing-plans` | Execution validates plan quality |
| `executing-plans` | `writing-plans` | Planning validates execution structure |
| `skill-creator` | `writing-skills` | Skill writing validates creator output |

## Review Protocol

When reviewing a peer's patch:

1. **Read the original skill** — understand what it does
2. **Read the failed traces** — understand what went wrong
3. **Read the proposed patch** — understand the fix
4. **Ask three questions:**
   - Does this fix the actual root cause, or just the symptom?
   - Does this introduce any contradiction with my own skill's instructions?
   - Would I follow these instructions if I were executing this skill?
5. **Verdict:** APPROVE (with confidence score 0-1), REJECT (with reason), or REVISE (with suggested changes)

## Unmatched Skills

Skills without a natural peer reviewer use the **generalist reviewer**: `code-review` for engineering skills, `spec` for process skills, `sentry-security-review` for security-adjacent skills.
