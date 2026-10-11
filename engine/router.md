# Vajra router

The router is code, not instructions. `hooks/user-prompt-submit.sh` runs
`python3 -I scripts/vajra_route.py` on every UserPromptSubmit event. It costs
0 model tokens and never blocks or fails a prompt.

## What it does

1. It acts only on prompts starting with `/vajra` (case-insensitive). With
   `routing.suggestOnAllPrompts: true` in `config/default.json` it also adds an
   advisory suggestion to other prompts that match a pattern.
2. If the first word is a subcommand (`plan`, `fleet`, `campaign`, ...), it
   points the model at `docs/commands/<doc>.md`. This is tier 0.
3. Otherwise the prompt is a bare ask. It tells the model to launch the
   `vajra-deliver` workflow, and lists up to `routing.maxSuggestions` crews,
   agents and skills for the plan to consider:
   - **tier 1:** learned routes from `~/.claude/vajra/learned-routes.json`
     (written only by `scripts/atman.sh approve`). These match words only,
     never regexes.
   - **tier 2:** `routing.patterns`, matched case-insensitively. Order is
     specificity: `security audit` comes before `security`, so the first match
     is the most specific one.
   - **tier 3:** no match. The workflow picks owners from the roster itself.
4. Every pattern names a domain, and `cortex.agentRouting[domain].agent`
   supplies the agent, so every tier suggests whom to delegate to. krishna is
   never a target.
5. If `scripts/vajra-campaign.sh active --brief` prints a line, the router
   appends it as data.
6. It appends one line to `~/.claude/vajra/routing-log.jsonl` (mode 600):
   `{ts, session_id, prompt_sha256, words (up to 20, lowercased), route: {tier, skill, agent, pattern}}`.
   The raw prompt is not stored. Atman reads this log to propose routes.

Skill and agent names from config or learned routes are echoed only when they
match `^[a-z0-9][a-z0-9_-]{0,63}$`, so a poisoned file can't inject text into
the context.

## Changing routes

Edit `config/default.json` by hand. Agent writes to `config/` are blocked by
the pre-tool hook. Put a new pattern above any broader pattern it overlaps
with, and run `bash tests/router/run-router.sh`.
