# Channels, where to post, mechanics & relaunch ethics

> Channel ROI ranking lives in SKILL.md. This file holds the *named venues* and *mechanics*.
> The attested **boom / launch-day playbook** (channel verdicts, launch runbook, honest budget, cringe traps) is the final section of this file.

## Claude Code / AI-agent ecosystem venues (named, 2026)

**Highest leverage, in-app + curated (do FIRST, install-intent, ban-proof, compounding):**
- **Anthropic's gated `/plugin` directory**, `anthropics/claude-plugins-official` + `claude-plugins-community`; submit via `clau.de/plugin-directory-submission` (verify live). Passing the automated security screen → Verified badge. Being 1 of ~100 in-app entries >> entry #16,671 in a registry.
- **Awesome-lists (PR yourself in):** `hesreallyhim/awesome-claude-code` (canonical, ~47k★, updated daily, top priority), `travisvn/awesome-claude-skills`, `VoltAgent/awesome-agent-skills` (cross-agent), `ComposioHQ/awesome-claude-skills`, `rohitg00/awesome-claude-code-toolkit`, `jqueryscript/awesome-claude-code`, `LangGPT/awesome-claude-code` (large Chinese audience).
- **Directories (auto-scrape / submit):** cultofclaude.com, codeguilds.dev, claudemarketplaces.com (→crossaitools; skills now gated at 500+ installs; keep repo clean, editorial sweeps), Vercel `skills.sh`, mcpfind.org. For MCP: official MCP registry (`server.json`, ~1hr) then *manually claim* Glama + PulseMCP (publishing does NOT auto-propagate to Smithery, that needs a separate Dockerfile+deploy).

**Communities (sustained presence, value-first):** r/ClaudeCode (high-intent, post here first), r/ClaudeAI (~545k, broad/noisy), r/AgentsOfAI, Anthropic/Claude Code Discord, Claude Code Pirates (skool), thehiveindex vibe-coding directory.

**Newsletters (free paths only):** Ben's Bites community board (news.bensbites.com), AI Coding Daily (substack), smaller AI-tooling newsletters via give-first outreach. **NOT TLDR** (paid-only, $5 to 15k/slot; no free editorial intake).

## Hacker News / Show HN mechanics
- **It's a lottery:** median Show HN = 2 points; ~2.3% reach front page; ~1.4 stars/upvote (median launch ≈ 3 stars); HN score explains only ~8% of star variance; 92% of impact within 48h.
- **One shot per repo, ever.** A repo that already had its front-page moment must not be re-Show-HN'd → duplicate → **silent permanent shadowban** risk (the ring detector marks your future submissions `[dead]`, invisible to others, visible to you). Each new HN post = a genuinely NEW artifact/URL.
- **Win before posting:** plain title ("Show HN: <tool>, <one concrete capability>", never "another <category>"), tested first comment, Tue to Thu 08:00 to 10:00 PT (or Sun 18:00 to 21:00 PT), 5 to 10 genuine contacts to *engage* (comments, NOT a vote ring) in the first hour to survive `/newest`. Never delete-and-repost; email hn@ for the second-chance pool instead.
- **Account hygiene:** build karma >250 with substantive comments first, or new-account posts get auto-killed.

## Product Hunt mechanics
- Relaunch rule: wait **≥6 months** between launches of the *same* product page AND a relaunch needs a *significant update* (new app / full redesign, not a new UI or pricing). The verb is "ask," not "require."
- The cooldown governs the same page, not posting frequency, you can launch *distinct surfaces* (harness / a skill pack / a CLI / a hosted variant) as separate first-time listings.
- For a cold solo builder the real bottleneck isn't cadence, it's a **warmed upvote list** you don't have. Treat each PH listing as a durable SEO/discovery artifact; only "launch" it with a warm list.

## Reddit mechanics
- Read each sub's sidebar rules BEFORE posting (self-promo allowed? weekly thread only? required flair? AutoMod karma/account-age gate?). Post from an aged account with real prior comment history.
- **Text (self) posts, not bare links**, lead with value, link LAST or in the first comment. Reply to every comment in the first 1 to 2 hours (comment velocity is the ranking signal).
- **~1 self-post per 9 genuine contributions.** Never the same link from a second account, never ask for upvotes (sitewide+IP permaban), never cross-post identical links to many subs in a day.

## Cross-channel atomization (the engine that fits 3 to 5 hrs/week)
Write once, cut into 6: one launch thread → LinkedIn post → Reddit body → blog/Dev.to article outline; one demo GIF → tweet 3 → README hero → LinkedIn native video → Reddit embed; one feature ship → a standalone tweet + a 20s clip; one real user quote → a social-proof tweet + a LinkedIn post. Create-once-distribute-everywhere beats 50 original posts a week. Atomize to **3 to 4 channels MAX, not 10** (cornerstone → dev.to canonical + Ben's Bites board + one X thread + one LinkedIn post). Skip the carousel/TIL/Hashnode/Medium busywork.

---

## Boom / launch-day playbook (attested + critic-corrected)

**Reality.** "Boom" is a lottery, not a deliverable. Median Show HN = 2 upvotes; front page ~10% even with perfect timing; HN explains ~8% of star outcomes. You control exactly two things: the quality/novelty of **one** artifact, and how many cheap independent shots you fire. A win can even pay in the wrong currency (a star spike with zero activated users).

**Diagnose before you assume a leak.** Do NOT reflexively conclude "stars-but-low-usage = activation problem." If stars are vanity (they are), 500 of them implies *nothing* about funnel health, low usage is equally consistent with niche TAM, discovery, retention, or drive-by stars. **First hour = pull the clone-to-issue ratio + traffic referrers (free, GitHub Insights)** to find where the drop actually is. Only then pick the fix.

**Channel verdicts (ROI-ranked):**
| Channel | Verdict | Why |
|---|---|---|
| In-app plugin directory + first-run activation | **WORTH_IT (highest)** | The only controllable, compounding discovery surface. Fix activation FIRST so a spike → clones, not stars. *(Rank on controllability, don't anchor on unverified traffic numbers.)* |
| Demo GIF (VHS `.tape` in repo) | **WORTH_IT** | 6 to 12s, <5MB, verify-gate catching drift w/ cost visible. The universal asset every channel reuses; frame-0 = the screenshottable still. |
| README top-fold + custom OG image (1280×640) | **WORTH_IT** | The conversion page every share funnels into. ~2 to 3h one-time, multiplies all upstream effort. |
| Hacker News (Show HN) | CONDITIONAL | ONE deliberate ticket per genuinely-new artifact. A pulse, not growth. Never re-spam a repo. |
| YouTube creator outreach | CONDITIONAL | A pickup is the cheapest single-shot lever, but expect ~zero from 20 to 30 emails. Target 5k to 250k *review/teaching* channels; manual emails only (no sequencer). |
| X launch thread + capped replies | CONDITIONAL | Keep the mechanics (link-in-reply, native MP4, contrarian hook, Wed 9am). **KILL the 60%/15-to-20-replies-a-day grind, it's a shadowban trap.** Cap ~5 bespoke replies/day. |
| Content atomization (cornerstone + dev.to + Ben's Bites) | CONDITIONAL | ONE "rising-term" cornerstone optimized for **AEO** (AI-engine citation, not Google rank). Ben's Bites community board = best 10-min bet. |
| Shareability garnish (click-to-tweet, star-history) | CONDITIONAL (low) | 5-min, zero-risk. Do once, forget. asciinema cast is the one with real value for a "verified" tool. |
| LinkedIn | **TRAP** | Wrong graph (B2B/recruiter); the GitHub click is the most-punished action. Only value = passive job-search brand insurance. No cadence, no pods, no "comment LOOP" bait. |
| Short-form vertical (TikTok/Reels) + Product Hunt | **TRAP** | Low-intent consumer traffic / reciprocal-upvote theater for a GitHub-gated CLI. Only survivor: one watermark-free YouTube Short exported from a GIF you already made. |

**The one move.** Fix first-run activation (instrument clone→install→first-verified-run; cut to <5 min) **and** submit to the in-app plugin directory, *in that order*, BEFORE any launch hour. Everything else is a lottery ticket; this is the only controllable, compounding move aimed at the real bottleneck. The single verify-gate-catching-drift GIF is the artifact that makes both land.

**Launch-day runbook (only after the pre-launch GATES are met):**
- **Gates (not nice-to-haves):** activation fixed+instrumented; directory submission in flight; README+OG+GIF live. If not done, DON'T launch, you'll spend a ticket on a leaking funnel.
- **T-2wk:** build the 60 to 90s "drift caught on camera" clip (outreach payload + HN/X media). Start 20 to 30 manual creator emails.
- **T-1wk:** publish the cornerstone on your domain + dev.to (canonical); submit Ben's Bites board. Confirm HN karma is non-zero (zero-karma accounts auto-removed).
- **Launch (Sun ~7pm ET):** post Show HN, repo-linked, plain title, immediate TL;DR top comment, **no upvote asks.** 7 to 9pm = the golden window: answer every comment <10 min. 9pm: post the X thread (a *separate* ticket, not coordinated voting). Next morning: if dead (the median outcome), don't panic-edit or repost.
- **Ongoing engine (~30 to 45 min/wk):** re-fire the artifact on each real capability win; ~5 bespoke X replies/day; atomize only on a genuine win; track clones/activation/issues, **never stars.**

**Honest budget (the critic's correction, important).** The pre-launch gates are ~15 to 20h of *serial* work = **4 to 6 weeks at 3 to 5 hrs/wk before one ticket fires.** Cap marketing at ≤~1.5 of your weekly hours until activation telemetry shows the funnel converts, otherwise every hour is negative ROI by this playbook's own logic. Add a per-tactic **kill-criterion** tied to clones/activations, not stars.

**Cringe / ban traps (instant-disqualify):** the X reply-grind (60% / 15-to-20/day / templated) = the canonical 2026 reply-deboost signature; LinkedIn "comment LOOP and I'll send the repo" = NLP engagement-bait suppression; LinkedIn friend-comment pods = ~97%-detected, 60 to 90 day shadowban; Reddit same-link cross-posting from a low-karma account = documented permaban; per-run CLI star-nag = dark-pattern devs hate; cold-DM "pls check it out" = prohibited + silently dropped; over-produced zero-typo synthetic demo = reads fake on a tool whose pitch is honesty; editing your HN title post-launch = resets timestamp, kills rank.
