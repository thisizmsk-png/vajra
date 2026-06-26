# Diagnosis & Measurement

## Diagnose before you spend a single marketing hour

"Attention but no adoption" (e.g. stars but low usage) is NOT one problem. Run three competing hypotheses and let data pick. Do not assume "it's a discovery problem":

1. **Discovery / distribution**, real visitors & clones are *low*. Nobody finds it. *Test:* GitHub Insights → Traffic (unique visitors, clones). Low → distribution problem → channels + listings + creator snowball.
2. **Activation / conversion**, visitors/clones are *high* but they don't stick. First-run friction. *Test:* discovery fine but issues/return-usage near-zero → fix time-to-first-success, the <2-minute first win, onboarding.
3. **Quality / saturation / ICP**, the likeliest killer for a solo builder in a crowded category. *Test:* can you answer "why you, not the 12 competitors?" in one sentence? If not, that's the blocker, no launch fixes it.

> The "stars-but-low-usage" pattern most often = **activation + saturation**, which a launch cannot fix. Beware the reflex "I need to launch more."

## Stars are a vanity metric, internalize this
- Star↔download correlation is only **0.14 to 0.47** (varies by ecosystem). Stars measure *awareness/notice*, not downloads or active use.
- ~**6M fake stars** on GitHub; a meaningful share of 50+ star repos touched fake-star campaigns. In AI tooling, lower-starred tools routinely out-use higher-starred ones.
- **Never optimize stars.** They rise from listings and reach while real installs stay flat, that false signal is a trap.
- Keep stars only as a cheap *awareness/credibility* KPI; **gate every "is anyone actually using this?" decision on usage signals, not stars.**

## What is actually measurable (the honest two-tier plan)

**Tier A, instrument these (cheap, real):**
- **Daily GitHub Action that dumps the Traffic API to CSV** (views, unique visitors, clones, unique cloners, top-10 referrers). The Traffic API only retains **14 days (views/referrers) / 30 days (clones)**, un-archived launch data is *permanently lost*. This is your only real launch-attribution signal (which post drove the spike).
- **star-history.com** for the star curve (awareness only).
- **Fork-to-star ratio** (healthy ~10-25%) as an organic-stars self-check.

**Tier B, forgery-resistant proxies for the unmeasurable middle** (install → active → contributor is largely unobservable: no install analytics for CC plugins/skills; git-clone installs don't phone home; clones are bot-inflated):
- Net-new **issues/discussions opened by distinct non-author accounts.**
- **Time-to-second-PR** per contributor.
- Inbound questions in Discord/issues.

**Tier C, if you truly need install/active counts:** the *only* path is **opt-in, disclosed first-run telemetry you build** (e.g. an anonymous `--version` ping with clear opt-out). Decide consciously; there is no passive path.

## Re-baseline success
A 500-star repo with **3 real external issue-filers + 1 returning contributor** is healthier than a 2,000-star repo with zero. Report success that way, or you'll optimize the corrupted metric.

## Run launches as experiments, honestly
You can A/B *referrer spikes + star deltas + "did anyone open a real issue in the following 14 days."* You **cannot** A/B "active users per launch", that data doesn't exist for most OSS dev tools. Don't pretend it does.
