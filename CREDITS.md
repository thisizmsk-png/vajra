# Credits & Third-Party Attributions

Vajra bundles a number of third-party Claude Code skills (installed via `scripts/install-skills.sh` from `bundled-skills/`) and builds on prior work. This file credits the original authors and records the licenses.

> Auto-compiled from a local scan on 2026-06-25. **Verify each entry against the skill's own LICENSE/README before publishing or redistributing.** This is attribution, not legal review.

> **Before you ship:** each bundled skill ships its own LICENSE file, keep those intact. Confirm every source actually grants redistribution rights. Prefer permissive (MIT / Apache-2.0) sources. For anything ambiguous, **link to it instead of bundling it.**

---

## Bundled skills, by source

### Anthropic, Apache License 2.0 (official Claude skills)
Keep each skill's `LICENSE.txt` (and any `NOTICE`) intact when shipping.
- `canvas-design`, `frontend-design`, `theme-factory`, `web-artifacts-builder`, `webapp-testing`, `ui-styling`
- Source: Anthropic Claude skills (verify the exact upstream repo).

### ClaudeKit, MIT
- `banner-design`, `brand`, `design`, `design-system`, `slides`, `ui-styling`
- Author: claudekit (per SKILL.md frontmatter). Verify repo + license per skill. (Note: `ui-styling` shows both a claudekit/MIT frontmatter and an Apache `LICENSE.txt`, reconcile which applies before shipping.)

### Vercel, MIT
- `composition-patterns`, `react-best-practices`, `web-design-guidelines`
- Author: Vercel (per SKILL.md frontmatter), from Vercel's engineering guidelines.

### Sentry
- `sentry-code-simplifier` (based on Anthropic's code-simplifier agent), `sentry-gha-security-review` (cites StepSecurity research), `sentry-security-review`, `sentry-skill-scanner`
- Author: Sentry. Verify license per skill.

### OWASP, CC BY-SA 4.0 ⚠️ share-alike
- `sentry-security-review` is based on the **OWASP Cheat Sheet Series** (CC BY-SA 4.0). Share-alike requires attribution **and** that derivative works be released under the same license. Confirm compliance, or replace, before redistributing.

### Wikipedia / WikiProject AI Cleanup, CC BY-SA ⚠️ share-alike
- `humanizer` is based on Wikipedia's "Signs of AI writing," maintained by WikiProject AI Cleanup. CC BY-SA: attribution + share-alike.

### Andrej Karpathy
- `autoresearch` applies Karpathy's autoresearch approach. https://github.com/karpathy/autoresearch

---

## Tools referenced (NOT bundled, no redistribution)
- **agent-reach**, MIT, by Neo Reid (@Panniantong). Used locally for research, not shipped in Vajra. https://github.com/Panniantong/agent-reach
- The upstream tools agent-reach routes to (yt-dlp, Jina Reader, Exa, mcporter, feedparser, etc.) are each their own projects under their own licenses.

## Vajra's own work
- The Vajra harness (routing, campaign engine, verify gate, OS sandbox, Ralph loop, Atman, Cortex fleet) and the `devtool-growth` skill are original.
- The Mahabharata agent names (Arjuna, Bhima, Karna, Krishna, etc.) are original creative naming.

---

## Checklist before publishing
1. Verify each bundled skill's actual LICENSE + author against its own files (this list is a scan, not a legal review).
2. Keep every per-skill `LICENSE`/`NOTICE` file inside `bundled-skills/<skill>/`.
3. CC BY-SA items (`humanizer`, OWASP-derived `sentry-security-review`): comply with attribution + share-alike, or remove/replace.
4. Apache-2.0 items (Anthropic skills): preserve LICENSE + NOTICE, state changes if you modified them.
5. Anything whose redistribution rights you can't confirm: **link, don't bundle.**
6. Not legal advice. When in doubt, read the original repo's license.
