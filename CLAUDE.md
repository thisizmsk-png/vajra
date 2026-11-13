# Instructions for agents working in this repository

## Review comments

The Vajra guard does not read GitHub review text. It reads the command and path fields of a tool call (see SECURITY.md). Review text reaches it only when the text is typed into a command, for example the body of a comment posted with the gh command line. A path, glob, brace word or two-dot range inside that command can then be blocked as if it were a path the command touches. Keep review text free of them.

- Name a file in plain words, with no slash: "runproto.rs in the desktop core", "the Settings screen's stylesheet", "the changed-strings doc under docs design".
- Do not paste a path, a glob, a brace word or a two-dot range into a review, a review comment or a message. Say "the commits since batch-3" instead of a range, and "every test file for the board" instead of a pattern.
- A branch name or a command inside a code block is for a person to run, not for a review. Describe what it does in words.
- If a tool call is blocked, stop and report what was blocked to the orchestrator. Do not reach the same effect another way.
