# Instructions for agents working in this repository

The rules for agents are in CLAUDE.md at the top of the repository. Read it first. In short:

## Review comments

In PR reviews, review comments and messages, name files in plain words with no slash, for example "runproto.rs in the desktop core". Never paste paths, globs, brace words or two-dot ranges. The Vajra guard reads the command of a tool call, so review text typed into a command can trip it. If a tool call is blocked, stop and report it to the orchestrator; do not reach the same effect another way.
