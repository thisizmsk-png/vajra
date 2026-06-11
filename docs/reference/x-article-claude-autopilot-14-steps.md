# Claude Code on Autopilot — /loop, Routines, and the Full Automation Stack (14 Steps)

> Reference notes from X Article by Codez (@0xCodez), published Jun 5, 2026.
> Source: https://x.com/0xCodez/status/2062926176117469477
> Status: bookmarked on X (account @SaiKris59630524) — these notes retained locally on 2026-06-10.

Three layers of automation, verified against code.claude.com/docs as of June 2026: `/loop` (March), Auto Mode (Mar 24), Cloud Routines (Apr 14, research preview). Arc: get the manual loop right → make it persist across restarts (Desktop tasks) → push to the cloud (Routines).

## Tier 1 — /loop (session-scoped)

1. **`/loop <interval> <prompt>`** — calls native tools CronCreate / CronList / CronDelete. Intervals: m/h/d, minimum 1 minute. Natural language works without /loop ("every weekday at 7am, summarize overnight commits"). Local timezone, not UTC. Any slash command can go inside a /loop.
2. **Real cron expressions** — standard 5-field cron (min hour dom month dow). Supports `*`, single values, steps `*/15`, ranges, lists. NOT supported: L, W, ?, name aliases (MON/JAN). Dom+dow both constrained = matches if either matches (vixie-cron semantics). Useful patterns: `*/5 9-17 * * 1-5` (every 5 min business hours), `0 7 * * 1-5` (weekday 7am), `30 2 * * *` (nightly 2:30am), `0 */6 * * *` (every 6h).
3. **Hard constraints** — (a) 7-day auto-expire: every recurring task deletes itself 7 days after creation; (b) 50 tasks max per session; (c) no catch-up firing — a missed interval fires once when idle, not once per miss; (d) session-scoped: closing the terminal cancels everything. `CLAUDE_CODE_DISABLE_CRON=1` disables cron tools entirely (CI/shared servers).
4. **Pair /loop with /goal** — /goal makes the stop condition explicit and enforced, fixing agentic laziness inside recurring tasks. Use for flaky-test debugging, long migrations, inbox-style triage.

## Tier 2 — Desktop scheduled tasks (machine-scoped)

5. **Desktop scheduler** — Schedule → New task → New local task. Name, prompt, frequency, permissions, working folder. Each fire = fresh session, no shared context. Survives restarts, but machine must be awake (missed runs: one catch-up for the most recent miss within 7 days). "Keep computer awake" setting helps; lid close still sleeps. macOS + Windows only — Linux uses /loop or system cron with `claude -p`.
6. **Token budgets and rate limits** — every fire is a full session counting toward usage limits (a 5-min loop for 24h = 288 sessions). Habits: explicit token budgets in the prompt ("max 5k tokens, save partial progress and exit cleanly"); match model to task (Sonnet for most automation, Haiku for cheap exploration — model picker defaults per panel, set explicitly per task); pick the plan for the volume (Pro tightens fast; Max = 5× headroom).
7. **Permissions for unattended runs** — settings.json allow/deny lists; `.claudeignore` to block credentials/env files; audit logging reviewed each morning. Auto-approve test: cheap to undo → approve (draft PR comment); expensive to undo → never (force-push). Deny examples: `rm -rf*`, `git push*`, `*--force*`, `curl*`, `Edit(.env*)`, `Edit(secrets/*)`.
8. **Auto Mode** — AI-classified permissions, the alternative to `--dangerously-skip-permissions`. Anthropic measured users approve 93% of prompts; Auto Mode automates the 93%, keeps humans on the 7%. Three tiers (Permissive/Balanced/Restrictive), server-side prompt-injection probe on tool outputs + per-action classifier, full audit trail. As of May 2026: Max/Team/Enterprise/API only (not Pro, Bedrock, Vertex, Foundry). Research preview; joins the Shift+Tab mode cycle. Pair with .claudeignore + audit logs.
9. **Picking the scheduler** — one question: where does this need to run and who must be awake? /loop to prototype → Desktop tasks for daily use → Routines when hardware shouldn't matter.

## Tier 3 — Cloud Routines (hardware-independent)

10. **Routines** — saved Claude Code configs (prompt, repos, connectors, permissions, environment, triggers) running on Anthropic cloud. All paid plans. Create at claude.ai/code/routines or `/schedule` in CLI (CLI = schedule triggers only; API/GitHub triggers via web). Prompt must be fully self-contained — no follow-up questions, ambiguity = coin flip per run. Default environment "Trusted" (package registries OK, arbitrary outbound blocked). Default branch guard: can only push to `claude/`-prefixed branches — keep it on.
11. **Schedule-triggered** — hourly/daily/weekdays/weekly/one-shot. High-payoff patterns: morning briefing (metrics + issues digest to Slack), weekday PR review pass, weekly docs-drift detection with draft PR. Local timezone, runs may stagger a few minutes.
12. **API-triggered** — unique HTTP endpoint + bearer token; POST from alerting/CI/webhooks; optional JSON body becomes one-shot context appended to the prompt. Endpoint: `POST https://api.anthropic.com/v1/claude_code/routines/$ROUTINE_ID/fire` with beta header `experimental-cc-routine-2026-04-01`. Token shown ONCE at creation — store in a secret store immediately. Response includes session id + live-watch URL. Unlocks: CI failure → investigate + fix PR; PagerDuty → triage; Stripe webhook → dashboard update.
13. **GitHub-triggered** — via Claude GitHub App webhook. Events: PR, push, issue, check run, workflow run, discussion, release, merge queue. Each event = independent session. PR filters: author, title, body, base/head branch, labels, draft, merged, from-fork. Uses: PR open → first-pass review; issue create → triage + label; workflow failure → root-cause + draft fix PR; release → draft notes.
14. **Compose the stack** — Skills inside Routines ("use the morning-briefing skill"); Dynamic Workflows inside Routines for parallel-subagent jobs; routines chaining routines (one's output is another's trigger); per-routine permissions as a security boundary. Full stack: Skills = recipes, Workflows = orchestration, Routines = triggers, Auto Mode = permission classifier, audit logs = morning review. Adopt incrementally: /loop → Desktop → Routines → Skills → Workflows.

## Habits that waste money

- Never trying /loop at all
- Forgetting the 7-day auto-expire on a critical loop
- Using /loop for things that should outlast the terminal (promote to Tier 2/3)
- Letting automations default to Opus when Sonnet would do
- No /goal on loops that should run until actually done
- Blanket `--dangerously-skip-permissions` instead of Auto Mode / allow lists
- Vague routine prompts (autonomous runs can't ask follow-ups)
- No audit logs
- Disabling the claude/ branch prefix without a real review process
- Heavy automation on a Pro plan (rate limits)

## Relevance to youtube-automation-framework

The youtube-automation-framework's `youtube-48h-optimizer` scheduled task and round-robin pipeline map to Tier 2/3: candidates for promotion to cloud Routines (schedule-triggered daily pipeline run; API-triggered re-run on upload failure) so runs don't depend on the laptop being awake.
