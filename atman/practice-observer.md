# Practice Observer — Track Every Skill Execution

Like game film. Every play recorded. After the game, review the tape.

## What to Record

After EVERY task where a skill was invoked (via Vajra routing or direct `/skill` command):

```jsonl
{"ts":"2026-04-09T12:00:00Z","skill":"code-review","task":"review auth middleware","outcome":"success","toolCalls":8,"errors":[],"routing":"T3-keyword","userReaction":"accepted","sessionId":"abc123"}
```

Append to `~/.claude/vajra/practice-log.jsonl` — one JSON line per skill use.

## Outcome Classification

- **success**: Task completed, user continued without correction
- **failure**: Task errored, agent couldn't complete, or user said "no/wrong/stop"
- **partial**: Task completed but user made significant corrections
- **rejected**: User explicitly rejected the output

## When to Observe

After every completed task, before responding to the next user message:
1. Determine which skill(s) were used
2. Count tool calls made
3. Check for any tool errors in the session
4. Assess user reaction (accepted = no correction within 2 turns)
5. Append to practice log

## Failure Pattern Detection

After recording, check the practice log for repeating failures:

```
Count entries WHERE skill = [this skill] AND outcome IN (failure, rejected) AND ts > [7 days ago]
```

If count ≥ 2 for similar task patterns (same first 3 words or same routing keyword):
- Flag for self-review
- Log: "Pattern detected: {skill} failing on {task-pattern} ({count} times)"

## Practice Log Maintenance

- **Max size**: 10,000 entries (prune oldest on overflow, keep all failures)
- **Format**: JSONL (one JSON object per line, append-only)
- **Integrity**: Each line HMAC-signed (appended as `"hmac":"..."` field)
- **Location**: `~/.claude/vajra/practice-log.jsonl`
