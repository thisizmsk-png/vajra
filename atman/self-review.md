# Skill and agent deltas

Atman proposes one-bullet changes ("deltas") to a skill's `SKILL.md` or an
agent's `~/.claude/agents/<name>.md`. It never edits those files.

- **add:** a failure cluster (same tool and error hash) of at least 3 failures in
  at least 2 sessions while the skill or agent was active. The bullet is a fixed
  template filled with the tool, counts and the error signature.
- **modify:** after an approved `add`, failures in the cluster continue (3+) but
  at a lower rate. The bullet is strengthened.
- **retire:** after an approved `add`, the failure rate did not drop over 20+
  calls. The bullet is removed.

Approving writes `~/.claude/vajra/patches/<id>.patch`. Review it, edit the
bullet if you want better wording, then apply it with `git apply`.

When you review a delta, check:
- Does the evidence (failures, sessions, calls) justify a standing instruction?
- Is the bullet specific enough to change behavior? Rewrite it if not.
- Would it contradict or duplicate something already in the file?
