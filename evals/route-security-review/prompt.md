---
name: route-security-review
description: The router sends this /vajra ask to the right specialist
tags: [routing]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

/vajra review the login handler in this repo for SQL injection and auth bypass
