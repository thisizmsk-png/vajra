---
name: negative-rename
description: Should not trigger the vajra skill
tags: [negative, routing]
runs: 3
max_turns: 4
allowed_tools: [Read, Glob, Grep, Skill]
---

Rename the variable cnt to count in this snippet: let cnt = 0; cnt++;
