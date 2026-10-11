---
name: negative-regex
description: Should not trigger the vajra skill
tags: [negative, routing]
runs: 3
max_turns: 4
allowed_tools: [Read, Glob, Grep, Skill]
---

Explain what the regex ^\d{3}-\d{4}$ matches, in one sentence.
