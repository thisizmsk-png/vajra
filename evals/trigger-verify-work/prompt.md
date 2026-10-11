---
name: trigger-verify-work
description: Should trigger the vajra skill
tags: [trigger, routing]
runs: 3
max_turns: 8
allowed_tools: [Read, Glob, Grep, Skill]
---

Before you tell me it's done, run vajra's verify-work proof-of-work check on my last change.
