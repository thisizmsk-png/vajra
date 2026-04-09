# Audit Trail

## What to Log

Every significant event in a Vajra session should be logged:

1. **Routing decisions**: tier used, input summary, target skill, tokens consumed
2. **Campaign state changes**: create, checkpoint, resume, rollback, complete
3. **Memory operations**: write, read, dream consolidation
4. **Hook executions**: which hook, result (pass/block), duration
5. **Cost alerts**: threshold reached, user decision
6. **Integrity events**: HMAC verification pass/fail, manifest check pass/fail

## Log Format

Append entries to the campaign's audit log (stored in SQLite alongside campaign state):

```json
{
  "timestamp": "ISO8601",
  "event": "routing|campaign|memory|hook|cost|integrity",
  "action": "specific action name",
  "details": {},
  "result": "success|blocked|failed"
}
```

## Retention

- Active campaign: all logs retained
- Completed campaign: logs retained with campaign archive
- Logs are part of campaign state — subject to same HMAC integrity and encryption
