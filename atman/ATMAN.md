---
name: atman
description: Vajra's learning loop. Records real tool outcomes, turns repeated patterns into proposals, and changes nothing until a human approves.
---

# Atman: the learning loop (v0.5)

Atman learns from what actually happened, proposes small changes with their
evidence, and leaves every decision to a human. Code: `atman/atman.py`, run via
`hooks/post-tool-use.sh` and `scripts/atman.sh`. Tests: `tests/atman/run-*.sh`.

## 1. Observe (automatic)

`hooks/post-tool-use.sh` runs on `PostToolUse` and `PostToolUseFailure`. It
appends one line per tool call to `~/.claude/vajra/practice-log.jsonl`:

```json
{"ts":"…","session_id":"…","event":"PostToolUseFailure","tool":"Bash","skill":null,
 "agent":"vidura","outcome":"failure","error_sha256":"…","seq":41,"prevHmac":"…","hmac":"…"}
```

- `skill` is `tool_input.skill` for Skill calls; `agent` is the payload's `agent_type`.
- `outcome` is `failure` for `PostToolUseFailure`, or when `tool_response` reports
  `is_error`, `success: false` or a non-zero exit code.
- `error_sha256` hashes the error after collapsing paths, hex strings and numbers,
  so the same failure clusters. No raw error text, command or prompt is stored.
- Bash calls of `scripts/vajra-verify-work.sh` add `"signal":"verify-work"`.
- Names are reduced to `[A-Za-z0-9._:-]` before they are logged or used in a path.
- Each line's HMAC (key `~/.claude/vajra/.hmac-key`) covers the previous line's
  HMAC. Appends take a lock, so parallel hooks keep the chain intact.
- The hook also writes the per-session event log read by `scripts/vajra-replay.sh`.
- It never blocks and always exits 0.

## 2. Verify before learning

Atman learns only from lines whose HMAC verifies **and** that link to the previous
accepted line. Forged, edited, replayed or reordered lines are skipped and
counted (`atman.sh status`, `atman.sh verify`). With no valid key it learns
nothing. This is tamper evidence, not tamper proofing: anyone who can read the
key can re-sign the log. The pre-tool hook keeps the key and the logs out of the
model's reach.

## 3. Propose (`atman.sh propose`, or `atman.sh dream`)

Signals: tool failures, failing verify-work runs, and the router's
`~/.claude/vajra/routing-log.jsonl`. At most 5 new proposals per run, written to
`~/.claude/vajra/proposals/<id>.json`. Nothing else is written.

- **Route proposals.** A routed prompt is labeled with the first successful
  Skill call that follows it in the same session (within 30 minutes, before the
  next routed prompt). A word is proposed for a skill when the router missed that
  skill at least 3 times and at least 80% of labeled prompts with the word went
  to it. Stopwords and words already learned are skipped. Evidence: counts,
  precision and prompt hashes.
- **Skill and agent deltas.** Each call is attributed to the skill active in its
  thread and to its agent. A failure cluster (same tool, same error hash) with at
  least 3 failures across at least 2 sessions yields an `add` proposal: one
  bullet, with evidence counts. After an `add` is approved, Atman compares the
  cluster's failure rate over the next 20+ calls: no improvement proposes
  `retire`; partial improvement with continued failures proposes `modify`.
- Rejected proposals keep their id, so the same proposal is not raised again.

## 4. Decide (human only)

```
bash ~/.claude/skills/vajra/scripts/atman.sh review
bash ~/.claude/skills/vajra/scripts/atman.sh approve <id>
bash ~/.claude/skills/vajra/scripts/atman.sh reject <id> [reason]
```

`approve` and `reject` need an interactive terminal (or
`VAJRA_ATMAN_ASSUME_HUMAN=1`, which is meant for tests); from the model's Bash
tool they exit 3 and print the command for the user to run.

- An approved route goes to `~/.claude/vajra/learned-routes.json`
  (`{"version":1,"routes":[{"word","skill","agent","precision","support","proposal","approved_at"}]}`),
  which `scripts/vajra_route.py` reads first.
- An approved skill or agent delta is written as
  `~/.claude/vajra/patches/<id>.patch`, a unified diff against the skill's
  `SKILL.md` or `~/.claude/agents/<name>.md`. Bullets are tagged
  `<!-- atman:<id> -->` so they can be retired later. You apply it yourself:
  `git -C <dir> apply <patch>` (the command is in the patch header).
- Every decision is appended to `~/.claude/vajra/atman-history.jsonl`
  (`atman.sh log`).

Atman never writes to `config/`, the Vajra skill core, skills or agents.

## 5. Maintain

`atman.sh dream` prunes the practice log to the newest 10,000 lines in their
original order, moves older lines to `~/.claude/vajra/practice-archive/`, and
writes a signed anchor so the remaining chain still verifies. Then it proposes.

## Commands

| Command | Effect |
|---|---|
| `status` | integrity, last-7-day failures, flagged skills/agents, proposal counts |
| `propose` | build proposals from verified signals |
| `review` | pending proposals with evidence |
| `approve <id>` / `reject <id>` | human decision |
| `log` | approval and rejection history |
| `pause` / `resume` | stop or restart proposals; the practice log keeps recording |
| `observe` | failure clusters per skill/agent (read-only) |
| `dream` | prune + propose |
| `verify` | check the practice-log chain (exit 1 on any rejected line) |

## Evals

`evals/` holds seed cases in `claude plugin eval` format: should-trigger and
should-not-trigger cases for the vajra skill and routing cases graded by an LLM
judge. Run them with `claude plugin eval ~/.claude/skills/vajra` (it runs real
model calls). `tests/atman/run-evals-lint.sh` checks their structure offline.

## Not built

No automatic promotion, no peer review between skills, no automatic rollback,
and no model-written instructions: bullets come from fixed templates filled with
counts and hashes. Gated, human-approved deltas are the design choice, because
self-written skill changes show little average gain unless they are gated.
