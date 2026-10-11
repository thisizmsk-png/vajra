# Vajra architecture (v0.6)

Vajra is a Claude Code skill plus a few hooks, scripts and saved workflows.
Wherever Claude Code has a native primitive, Vajra uses it rather than asking
the model to act one out.

```
 user prompt
     |
     v
 UserPromptSubmit hook ── scripts/vajra_route.py ──> one context line (0 tokens)
     |                     reads config/default.json,      + ~/.claude/vajra/routing-log.jsonl
     |                     ~/.claude/vajra/learned-routes.json,
     |                     scripts/vajra-campaign.sh active --brief,
     |                     scripts/vajra_kb.py search (top 3 hits; remember -> add --source user)
     v
 SKILL.md (/vajra)
     |-- bare ask ---------> workflow vajra-deliver  (~/.claude/workflows/)
     |                         understand (kb search) > plan (vajra_tasks.py graph) > execute in waves
     |                         (worktrees, <= 4 in flight) > verify > fix x3 > deliver (+ kb write-back)
     |-- fleet <crew> -----> workflow vajra-<crew>
     |-- plan -------------> native plan mode + engine/plan-mode.md template
     |-- campaign ---------> scripts/vajra-campaign.sh
     |-- verify-work ------> scripts/vajra-verify-work.sh
     |-- remember, kb -----> scripts/vajra_kb.py
     |-- ralph, cost, replay, repomap, review, redteam, atman, doctor -> docs/commands/<cmd>.md
     v
 specialists: the Agent tool (subagent_type) or a workflow agentType -> ~/.claude/agents/<name>.md

 every tool call:   PreToolUse  -> hooks/pre-tool-use.sh   (exit 2 blocks; fails closed)
                    PostToolUse / PostToolUseFailure -> hooks/post-tool-use.sh -> practice-log.jsonl
 session:           SessionStart -> hooks/session-start.sh (manifest signature, active campaign,
                                                       vajra_kb.py prime)
                    PreCompact   -> hooks/pre-compact.sh   (git snapshot ref)
```

## Components

| Area | Files | Notes |
|---|---|---|
| Entry point | `SKILL.md`, `docs/commands/*.md` | About 80 lines; each subcommand loads its own doc on demand. |
| Router | `scripts/vajra_route.py`, `hooks/user-prompt-submit.sh`, `engine/router.md` | Tier 0 subcommand, tier 1 learned route, tier 2 config pattern (specificity order), tier 3 no match. Every route resolves an agent through `cortex.agentRouting`. It never blocks a prompt. |
| Config | `config/default.json`, `config/steering.json`, `config/pricing.json`, `config/sandbox.json` | Edited by hand only; the pre-tool hook blocks agent writes. |
| Workflows | `workflows/vajra-*.js` | `deliver`, `feature-build`, `security-audit`, `incident-response`, `architecture-review`, `sabha`. They return `{status, node, artifacts, question?}` and never perform protected actions. |
| Knowledge | `scripts/vajra_kb.py`, `docs/commands/kb.md` | `~/.claude/vajra/kb/<project-id>/`: HMAC-chained `events.jsonl` (source of truth), `kb.sqlite` FTS index, `INDEX.md`, `pending/`. Write gate, trust tiers, dedup, supersede and tombstones. Read back by SessionStart (`prime`) and the router (`search`); written by deliver on verified success. |
| Task graph | `scripts/vajra_tasks.py` | `~/.claude/vajra/campaigns/<slug>/tasks.json`. Validation (cycles, unknown agents), atomic claims, waves of non-overlapping file globs. Only the script changes a status. |
| Campaigns | `scripts/vajra-campaign.sh` | `~/.claude/vajra/campaigns/<slug>/{campaign.json, features.json, progress.md, checkpoints/}`, signed with openssl HMAC by the script. |
| Verification | `scripts/vajra-verify-work.sh`, `engine/verification.md` | At least one command; diff against `HEAD` plus untracked files. |
| Long runs | `scripts/vajra-ralph.sh`, `engine/ralph-loop.md` | A fresh `claude -p` per iteration, with permission mode, turn cap and stall detection. |
| Guard | `hooks/pre-tool-use.sh` | Inspects only the command and path fields, compares canonical paths case-insensitively. A quote- and heredoc-aware awk pre-pass drops inert heredoc bodies and splits no quoted reader text, so only real targets are judged; commands of 1 KB or more are normalized in one linear awk pass. Caps: 512 KB raw, 32 KB after inert heredoc text, 2,500 words after splitting on shell operators; over a cap the call is blocked at once with a reason. Measured on an M-series Mac: a typical command 20 ms; a 4,093-char `python3 -c "(a),[b] …"` 0.8 s; a 4 KB python heredoc or `node -e` 0.5 s; a 512 KB inert `cat` heredoc 0.65 s; a 31 KB `python3 -c` (token cap) and anything over 512 KB are blocked in under 0.2 s. `tests/hooks/run-guard-text.sh` fails any of these cases over 3 s (the hook timeout is 10 s, and a timeout also blocks). The key-name masking in quoted `echo`/`printf` text never applies to a redirect target or to anything after `>`, `>>`, `<` or `|`. |
| Integrity | `scripts/generate-manifest.sh`, `hooks/session-start.sh` | SHA-256 manifest signed with the user's own key in `~/.claude/vajra/manifest.sig`. |
| Learning | `hooks/post-tool-use.sh`, `scripts/atman.sh`, `atman/`, `evals/` | Signals from real failures, proposals in `~/.claude/vajra/proposals/`, human approve or reject. |
| Cost | `scripts/vajra-cost.sh` | Per-transcript ledger, deduplicated by message id. |
| Review | `steering/`, `config/steering.json` | Rule packs selected by file pattern, confidence-gated. |
| Sanitizer | `engine/sanitizer.ts` | Reference implementation (tested in CI, not called at runtime) of stripping injection markers and wrapping content in `<untrusted-data>`; at runtime the wrapping is an instruction in `SKILL.md`. |
| Memory | `memory/tiered-memory.md` | User preferences stay in Claude Code's native auto memory; project knowledge is the ledger above. |

## Runtime state (`~/.claude/vajra/`)

| Path | Written by | Read by |
|---|---|---|
| `.hmac-key` (mode 600) | `install.sh` | campaign script, manifest signing, practice log; never the model |
| `manifest.sig` | `generate-manifest.sh` / `install.sh` | `session-start.sh`, `vajra-doctor.sh` |
| `routing-log.jsonl` | router | Atman |
| `practice-log.jsonl` | `post-tool-use.sh` | Atman |
| `learned-routes.json` | `atman.sh approve` | router |
| `proposals/`, `patches/` | `atman.sh` | the user |
| `campaigns/` | `vajra-campaign.sh`, `vajra_tasks.py` (`tasks.json` and its `.tasks.lock`; the guard blocks direct writes to both) | the campaign and task scripts, the deliver workflow |
| `kb/<project-id>/` | `vajra_kb.py` only (the guard blocks direct writes) | SessionStart, the router, the deliver workflow, `atman.sh review` |

## Atman (learning)

Design and details: `atman/ATMAN.md`.

1. **Observe.** `hooks/post-tool-use.sh` appends an HMAC-chained line per tool
   call (outcome, error hash, skill, agent; no raw text) to `practice-log.jsonl`.
2. **Verify.** Only lines whose HMAC verifies and links to the previous accepted
   line are used.
3. **Propose.** `atman.sh propose` turns repeated failures and the routing log
   into proposals with evidence counts: routing words that consistently lead
   to a skill (with their precision), and delta bullets to add, modify or
   retire in a skill or agent.
4. **Decide.** The user runs `atman.sh approve <id>` or `reject <id>` in their
   own terminal; both refuse to run from the model. An approved route is
   written to `learned-routes.json`, and an approved skill or agent change
   becomes a patch in `patches/` to apply with git.
5. **Evaluate.** Seed cases in `evals/` (routing should-trigger and
   should-not-trigger) check changes before they are approved.

## Agents

The 20 mahabharat agents in `bundled-agents/` (17 masters, 3 sepoys) are
installed to `~/.claude/agents/`. The domain-to-agent map lives in
`config/default.json` under `cortex.agentRouting`. For example, duryodhana runs
non-security pre-mortems, bhishma owns security review, karna owns backtests
and drona owns acceptance criteria. krishna is a main-session orchestrator
(`claude --agent krishna`) and is never a routing target.

## Removed in v0.5

- The SQLite campaign engine (`engine/engine.ts`) and `better-sqlite3`. It was
  never called; campaigns are the signed files above.
- The model-read tier 3 keyword and tier 4 LLM-classification routing, and the
  prose "authorization gate". Claude Code's permissions and the agents' tool
  allowlists do that job.
- Vajra's own L1/L2/L3 memory and dream consolidation, replaced by native auto
  memory.
- `fleet/fleet-coordinator.sh`, replaced by the crew workflows.
