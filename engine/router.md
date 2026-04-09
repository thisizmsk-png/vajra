# Vajra Router — 4-Tier Routing Instructions

When you receive user input, execute these tiers IN ORDER. Stop at the first tier that produces a match. Never skip the Authorization Gate.

Config location: `~/.claude/skills/vajra/config/default.json`

---

## Tier 1 — Regex Pattern Match

Cost: 0 tokens.

Test the raw input string against each pattern below (RE2 syntax, case-sensitive). Evaluate in order; use the FIRST match.

| Pattern | Target Skill | Description |
|---|---|---|
| `^/test\b` | testing | Route test commands to testing skill |
| `^/review\b` | code-review | Route review commands to code-review |
| `^/debug\b` | systematic-debugging | Route debug to systematic-debugging |
| `^/security\b` | sentry-security-review | Route security to security review |
| `^/spec\b` | spec | Route spec to specification skill |
| `^/plan\b` | writing-plans | Route plan to writing-plans |
| `^fix\s+#\d+` | systematic-debugging | Route "fix #123" to debugging |
| `^refactor\b` | sentry-code-simplifier | Route refactor to simplifier |

On match: log `T1 match: [pattern] -> [skill]` and proceed to the Authorization Gate. Do NOT continue to Tier 2.

If no pattern matches, proceed to Tier 2.

---

## Tier 2 — Campaign Resume

Cost: 0 tokens.

Check `~/.claude/vajra/vajra.db` for active campaigns. Read any campaign whose status is `active` or `paused`.

For each active campaign:
1. Compare the user's input against the campaign's context, goal, and recent checkpoint summaries.
2. If the input clearly relates to an active campaign (references its goal, continues its last step, or mentions its artifacts), resume that campaign and route to its assigned skill.
3. If multiple campaigns could match, prefer the most recently updated one.

On match: route to the campaign's skill and restore campaign state. Proceed to the Authorization Gate.

If no active campaign matches, proceed to Tier 3.

---

## Tier 3 — Keyword Lookup

Cost: 0 tokens.

Tokenize the input into lowercase words (split on whitespace and punctuation). Check each word against the keyword map below. Use the FIRST significant match (skip stop words, articles, pronouns).

| Keyword | Target Skill |
|---|---|
| test | testing |
| review | code-review |
| debug | systematic-debugging |
| security | sentry-security-review |
| spec | spec |
| plan | writing-plans |
| deploy | ci-cd |
| monitor | observability |
| design | frontend-design |
| architecture | hld |
| database | database-design |
| api | api-design |
| performance | performance-profiler |
| accessibility | audit |
| brand | brand |
| slides | slides |
| diagram | architecture-diagrams |
| threat | threat-modeling |
| compliance | compliance-checklist |
| release | release-engineering |

If multiple keywords match different skills, use the first one that appears in the input (left-to-right). Do not combine skills.

On match: route to the mapped skill. Proceed to the Authorization Gate.

If no keyword matches, proceed to Tier 4.

---

## Tier 4 — LLM Classification

Cost: ~500 tokens.

Tiers 1-3 all missed. You must now classify the input yourself:

1. List all installed skills available in this session.
2. Determine which skill best fits the user's intent. Consider:
   - The primary action requested (build, fix, analyze, create, etc.)
   - The domain (frontend, backend, security, data, etc.)
   - Whether the task is single-step or multi-step.
3. If the task requires multiple steps across multiple skills, create a new campaign:
   - Write initial campaign state to `~/.claude/vajra/campaigns/`
   - Record the campaign in `vajra.db` with status `active`.
   - Route to the first skill in the campaign plan.
4. If the task is single-step, route directly to the best-fit skill.

If you genuinely cannot determine a skill, ask the user to clarify before routing.

Proceed to the Authorization Gate.

---

## Authorization Gate

Execute this after EVERY routing decision, regardless of which tier matched. Never skip this step.

1. Read the target skill's declared tool requirements.
2. Compare against the allowed tool list in `fleet.defaultToolAllowlist`: `["Read", "Glob", "Grep", "Bash", "Write", "Edit"]`.
3. If the skill requires tools outside the allowlist, DENY the route. Inform the user which tools are blocked and why.
4. If the skill's tools are all permitted, APPROVE and execute the skill.

Do not silently downgrade a skill's capabilities. Either fully authorize or fully deny.

---

## Routing Log

After every routing decision (approved or denied), record the following entry:

```
{
  "timestamp": "<ISO 8601>",
  "input_summary": "<first 80 chars of input>",
  "tier_used": <1|2|3|4>,
  "target_skill": "<skill name>",
  "tokens_consumed": <0 for T1-T3, estimated for T4>
}
```

If a campaign is active, append this entry to the campaign's state file. If no campaign is active, log to the current session context.

This log is mandatory. It enables cost tracking and routing audits.
