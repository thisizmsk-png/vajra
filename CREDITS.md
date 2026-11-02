# Credits & Third-Party Attributions

Vajra pins third-party Claude Code skills in `config/skills.json`, bundles a few (installed via `scripts/install-skills.sh` from `bundled-skills/`) and builds on prior work. This file credits the original authors and records the licenses.

> Auto-compiled from a local scan on 2026-06-25. **Verify each entry against the skill's own LICENSE/README before publishing or redistributing.** This is attribution, not legal review.

> **Before you ship:** each bundled skill ships its own LICENSE file, keep those intact. Confirm every source actually grants redistribution rights. Prefer permissive (MIT / Apache-2.0) sources. For anything ambiguous, **link to it instead of bundling it.**

---

## Pinned skills, not redistributed

Vajra no longer ships most third-party skills. `config/skills.json` names each one with its
repo, commit, license and tree hash, and `scripts/vajra_skills.py add <name>` fetches it from
upstream on demand. Their licenses apply to the upstream copies:

- Anthropic, Apache-2.0: `canvas-design`, `frontend-design`, `theme-factory`, `web-artifacts-builder`, `webapp-testing`
- ClaudeKit, MIT: `banner-design`, `brand`, `design`, `design-system`, `slides`; `ui-styling` states both MIT and Apache-2.0
- Vercel, MIT in frontmatter, no repo license file: `composition-patterns`, `react-best-practices`, `web-design-guidelines`
- Trail of Bits, CC BY-SA 4.0: `differential-review`, `insecure-defaults`, `property-based-testing`
- Sentry: `sentry-code-simplifier`, `sentry-find-bugs`, `sentry-gha-security-review`; `sentry-security-review` builds on the OWASP Cheat Sheet Series, CC BY-SA 4.0
- DietrichGebert, MIT: `ponytail`, `ponytail-review`, `ponytail-audit`, `ponytail-debt`, `ponytail-gain`, `ponytail-help`
- Every other pin: see its `repo` and `license` in `config/skills.json`

## Third-party skills still bundled

- `systematic-debugging`, `test-driven-development`, `verification-before-completion`, from `obra/superpowers`, MIT. Bundled so they work offline on a fresh machine.
- `sentry-skill-scanner`, by Sentry. Bundled because the skill-pins Action runs it, and its text fails the load-time command check.
- `ui-ux-pro-max`. Its upstream path contains a symlink, which the installer refuses.
- `brainstorming`. Four files differ from its pin and have not been reviewed.

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
3. CC BY-SA items: pinned only, so nothing is redistributed. Keep it that way.
4. Apache-2.0 items (Anthropic skills): preserve LICENSE + NOTICE, state changes if you modified them.
5. Anything whose redistribution rights you can't confirm: **link, don't bundle.**
6. Not legal advice. When in doubt, read the original repo's license.
