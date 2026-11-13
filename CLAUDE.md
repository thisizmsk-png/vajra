# Instructions for agents working in this repository

## Review comments

The guard that protects Vajra's own folders reads the text of a PR review, a review comment and a message, and blocks one that holds a path. Write so it never has to.

- Name a file in plain words, with no slash: "runproto.rs in the desktop core", "the Settings screen's stylesheet", "the changed-strings doc under docs design".
- Do not paste a path, a glob, a brace word or a two-dot range. Say "the commits since batch-3" instead of a range, and "every test file for the board" instead of a pattern.
- A branch name or a command inside a code block is for a person to run, not for a review. Describe what it does in words.
- If the guard still blocks a comment, reword it once with the folder described in words and post it again. Do not work around the block another way.
