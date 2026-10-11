# Muscle memory: learned routes

Routing learns from what the user actually ended up using, not from the
router's own guesses, so it cannot reinforce its own mistakes.

1. `scripts/vajra_route.py` logs each routed prompt to
   `~/.claude/vajra/routing-log.jsonl` (`ts, session_id, prompt_sha256, words, route, suggested`).
   `suggested` lists every skill the router showed, so a skill it hinted at 2nd
   or 3rd is never counted as a router miss or learned as a new route.
2. `atman.sh propose` labels each routed prompt with the first successful Skill
   call that followed it in the same session (from the verified practice log).
3. For each word, it measures precision on history: of the labeled prompts that
   contain the word, how many went to skill S. It proposes `word -> S` when the
   router missed S at least 3 times and precision is at least 0.8.
4. A human approves with `atman.sh approve <id>`. The route is written to
   `~/.claude/vajra/learned-routes.json`, which the router reads before its
   config patterns. `config/` is never edited.

Only single words matching `[a-z0-9][a-z0-9-]{1,31}` are learned; there are no
learned regexes. To undo a route, remove it from `learned-routes.json`.
