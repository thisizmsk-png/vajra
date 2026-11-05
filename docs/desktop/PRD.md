# Vajra Desktop: product requirements

Status: draft for the two-model gate (authored by Opus 5.5, to be checked by GPT 6.1 Sol).
Date: 2026-11-05. Owner decisions are listed at the end.

Companion docs: [spec/requirements.md](spec/requirements.md) (stories and Given/When/Then criteria),
[spec/design.md](spec/design.md) (design, control channel, ADRs; Architecture pending the stack spike),
[spec/tasks.md](spec/tasks.md) (MVP task plan), [UX.md](UX.md), [DESIGN.md](DESIGN.md).

## 1. Summary

Vajra Desktop is a macOS app for watching and steering Vajra runs. Vajra stays the
engine: its hooks, saved workflows, agents, campaigns, task graph and knowledge ledger keep working
exactly as they do in the terminal, and stay the source of truth. The app reads their state, starts
Claude Code sessions, and adds the things a terminal can't do well: a crew tree you can hop through, a
chat with the lead agent about any worker, a run view with evidence per acceptance criterion, an
approvals inbox, and history.

One line: **each task handed back with a re-runnable proof for every acceptance criterion, so you check
evidence instead of reading the whole diff.**

### Positioning (final)

From `positioning.md` (vajra-5, FINAL, 2026-11-05). It re-ran 15 candidate reasons through "why does the
customer care?" against user evidence (vajra-4 pain points P1 to P23), the control-channel tracer (vajra-6)
and the final stack spike (vajra-2). One strong reason survives, plus one weak one. Memory is killed as a
claim.

> For Mac developers who run several Claude Code agents at once and can no longer review it all, Vajra
> Desktop is the open-source orchestrator that hands back each task with a re-runnable proof for every
> acceptance criterion, so you check evidence instead of reading 8,000-line diffs.

| Rank | Reason | Evidence and copy difficulty | Requirements |
|---|---|---|---|
| 1 | **Proof per acceptance criterion.** Criteria written before the code by a different agent, an independent Verifier, evidence bound to the exact tree (HEAD, whole-tree id, HMAC), a gate computed in code, fix rounds per failing criterion | Review can't keep up (P18, P12; indirect, since no pain point says "false done"). An engine contract, not a widget: a shallow copy takes about a week; that a faithful one takes months is our inference, not measured. octomux is one step away (one verify command per task), so this is the centre of the product, not a tab | POS-1.1 to POS-1.7 |
| 2 (weak) | **Earned project knowledge.** Every record has provenance and a trust tier; "verified" needs signed evidence; learned records wait for approval; secrets and injection are refused | P15 (agents forget decisions). Moderate to copy. Drops to a supporting point if POS-3.3 shows no effect | POS-3.1 to POS-3.4 |
| Killed (revivable) | Memory that stays bounded | P1 is the top pain, but the prototype failed the soak (+31.1 MiB in an hour against 20) and Orca already attributes memory (parity). Comes back only if POS-4.2, 4.4 and 4.6 pass on the real build, and is never a comparison with Runner | POS-4.1 to POS-4.7 (build budgets either way) |

**Table stakes.** These were killed as reasons, but missing any of them is a reason to leave, so they are
required:
- Claude Code-native, the app optional, runs as plain files (POS-2.x). Native agent view and others already
  do it (P23).
- Runs outlive the window (P2, P3, P6).
- Steer without typing into a terminal, parity-plus (POS-5.x; P8, P4).
- Worktree per task, the guard and approvals, local only with no telemetry, a usage line.

**60-second demo** (the memory beat is cut until POS-4.4 passes on the real build):

| Time | Shows | Gate |
|---|---|---|
| 0 to 20 s | A finished delivery: 5 criteria, 4 pass, 1 fails. Open the failing one: command, exit 1, output, diff, "verified by Verifier, written by Backend". Re-run it live; edit a file (even without committing) and the evidence turns stale | POS-1.1 to 1.3 |
| 20 to 35 s | Start `/vajra add CSV export` in a plain terminal; the open app picks it up live. Close the window; the run keeps going. Reopen; it reattaches | POS-2.1, 2.2 |
| 35 to 50 s | Message a worker from the Lead chat: it arrives through the hook, acked, labelled "from Lead". Stop blocks its next tool call | POS-5.1 to 5.3, and POS-5.5 for a `/vajra` delivery worker (until then the beat uses an Agent-tool subagent) |
| 50 to 60 s | A second run in the same repo shows "2 knowledge records used", each with its source run and tier, and a reject action | POS-3.1 |

**Ideal first user** (inference, to confirm): a senior developer already paying for Claude Code, running 3
to 8 sessions in parallel, on an Apple Silicon Mac, in a TypeScript or Python repo with a real test suite
(evidence is only as good as the checks). Existing Vajra CLI users are the first cohort.

**Banned claims at launch** (no measurement unlocks them without an owner decision): "lighter than Runner",
"lightest", "lowest memory", any total-RAM number, "faster startup". The only load comparison (Tauri
prototype 805.3 MiB against Runner 1,138 MiB at 10 workers, both 5-minute figures) is unequal load and
supports no claim.

**Claims we may not make until measured.** Any memory wording (POS-4.4 and POS-4.6 on the real build).
"Never janky with 10 agents" (load test, H1). Any catch rate for false "done" claims (POS-1.4). Any time
saved in review (POS-1.6). "Runs survive the app quitting" (not in the MVP, ADR-9; v1.1). "Gets better every
run" (POS-3.3). Steering for `/vajra` deliveries (POS-5.5; until then the claim covers Agent-tool subagents
only). "Safe to run unattended" or "secure" (security gate and PoCs; the README's limits stand).
"Production-ready" (a live `/vajra` run validated end to end). Any comparison that names a competitor needs a
dated, reproducible measurement with its method.

## 2. Problem

A `/vajra <ask>` run plans tasks, runs up to 4 (max 16) workers in parallel git worktrees, verifies every
acceptance criterion with a real command, runs up to 3 fix rounds and delivers a local branch with a
report (`docs/commands/deliver.md`). Today all of that is one terminal:

- **Opaque.** You see the main session scroll. You can't open what the backend worker is doing right now,
  its worktree, or its diff, without digging through `.claude/worktrees/` and transcript files.
- **No safe way to steer.** `deliver.md` ("Messages sent while it runs") records that a message typed into
  the run session is relayed to every workflow agent started afterwards. There is no way to tell one worker
  something, or stop one, without affecting the run.
- **Evidence and approvals are buried.** Pass/fail evidence per criterion and the push/PR plans waiting for
  approval live in a markdown report and signed JSON files. Nothing lists what is waiting for you.
- **Resuming is manual.** `/vajra continue` works in the CLI, but finding which campaign is stuck, and why, means
  reading `progress.md` files by hand.

The desktop orchestrators that exist (section 7) are mostly fleet dashboards: many independent agents,
a kanban board, a PR at the end. None we found shows evidence per acceptance criterion (octomux and Agent Orchestrator do loop verify
or CI fixes, per task), and the two installed on this Mac are Electron apps (bundle inspection), one using about 1.6 GB summed RSS
across 13 processes while running sessions (one `ps` snapshot; summed RSS over-counts shared pages).

**Why now:** we plan to launch a Mac app publicly, and Vajra's engine is stable enough (v0.6, signed
campaigns, a task graph, a verified knowledge ledger) to put a product over it.

## 3. Users

- **Primary: the Vajra developer.** Runs `/vajra` deliveries on their own repos, often several workers at
  once, sometimes for an hour or more. Lives in a terminal and an editor; keyboard first; cares about
  battery and memory because the app sits next to an IDE, a browser and several `claude` processes.
- **Secondary: the reviewer of a delivery.** Opens a finished campaign to decide whether to approve the
  push and PR. Needs the evidence, the diff and the plan, not the chatter.

## 4. MVP jobs

| Job | The human can... | Stories |
|---|---|---|
| J1 Start and resume | Start a `/vajra` run (repo, ask, base branch, max rounds) and resume a campaign by its slug (never `/vajra continue`, which follows the global `.active` pointer), after a preflight that shows whether Claude Code, Vajra's hooks and the sandbox are in place | US-1.1 to US-1.3 |
| J2 Crew view | See a tree of Lead, the run, its agents and their sub-workers; hop to any of them; see its live stream, status, current task, worktree and diff | US-2.1 to US-2.4 |
| J3 Orchestrator chat | Ask the Lead about any worker ("what is Backend stuck on?"), and have it message, redirect or stop a worker through a documented control channel. Every action is visible in the UI and the audit log; the human can take the same actions directly | US-3.1 to US-3.4 |
| J4 Run view | Follow the phase timeline (route, recall, understand, plan, execute, verify, fix, deliver), the task graph with owners, and every acceptance criterion with pass/fail evidence | US-4.1 to US-4.3 |
| J5 Approvals inbox | See every plan waiting for them (push, PR, deploy) and questions from the run; approve or reject | US-5.1 to US-5.3 |
| J6 History and knowledge | Browse past campaigns and delivery reports; search the knowledge ledger | US-6.1, US-6.2 |

Names: the app shows generic role names (Lead, Architect, Backend, Frontend, Verifier, Security, ...),
not the engine's agent ids. The engine id is visible only in a worker's inspector. Mapping table:
[spec/design.md](spec/design.md#44-generic-name-mapping-engine-id---ui-name).

## 5. Principles (in priority order, from the owner)

1. **Customer experience first.** Fast, responsive, never janky, no surprises. Nothing happens to a worker
   that the human can't see.
2. **Performance and memory.** Idle and active memory clearly lower than the competitors we measure (a
   build budget, NFR-PERF; not a marketing claim, see the banned claims in section 1).
   Bounded buffers everywhere. Rust core is the default if the spike's evidence supports it.
3. **Multi-agent orchestration UX.** Hop between any worker; talk to the Lead about any worker; steer
   through a control channel, never by typing into another process's terminal behind the human's back.
4. **Red team before shipping.** Load, soak and profiling for performance; IPC, injection, filesystem,
   secrets, update channel and supply chain for security.

## 6. How it sits on Vajra

```
 Vajra Desktop (macOS)                                Vajra engine (unchanged)
 ┌───────────────────────────────┐   starts      ┌──────────────────────────────────┐
 │ Run session PTY (read-only    │──────────────▶│ claude: /vajra <ask>             │
 │   until "Take over")          │               │   hooks: route, guard, post-tool │
 │ Lead chat session PTY         │──────────────▶│   workflow vajra-deliver         │
 │ Crew tree, run view, inbox    │◀── reads ─────│     workers = subagents in       │
 │ Control service + MCP tools   │── queue ──┐   │     worktrees                    │
 └───────────────────────────────┘           │   │ ~/.claude/vajra/campaigns/<slug> │
                                             └──▶│ hooks/desktop-control.sh (new)   │
                                                 │   delivers messages at the next  │
                                                 │   tool call; logs to audit.log   │
                                                 └──────────────────────────────────┘
```

- **Reads, never writes, under `~/.claude/`.** Mutations go through Vajra's own scripts
  (`vajra-campaign.sh`, `vajra_tasks.py`, `vajra_kb.py`) or through Claude Code.
- **Three kinds of session.** The run session runs the workflow. The Lead chat is a separate Claude Code
  session, so chatting never leaks into workflow agents. Workers are the workflow's subagents; their
  streams come from Claude Code's transcripts (to verify in the spike).
- **One documented control channel.** The Lead (or the human) calls `message_worker`, `redirect_worker` or
  `stop_worker`; a new Vajra hook delivers the message at that worker's next tool call, or blocks it on
  stop. Each action is recorded in the app's control ledger and in Vajra's `audit.log`.
- **Approvals stay human.** The app never pushes, opens a PR or deploys. Approving submits a visible
  human turn to the run session, and Claude Code's own permission prompt still fires.

Details and ADRs: [spec/design.md](spec/design.md).

## 7. Competitive landscape

Researched 2026-11-05 from each project's README and GitHub metadata (`gh api`), and for the two apps
installed on the reference Mac, read-only bundle inspection (`ls Contents/Frameworks`, `Info.plist`). No
app was decompiled and no proprietary code was read. The REA skill's MCP tools were not registered in this
session, so this is plain inspection. "(unverified)" marks a claim taken from marketing copy or not
confirmed from a primary source; "?" means not found.

| Tool | Model | Platforms | Agents | Worktree per task | Approval inbox | Diff review | Mobile / push | Stack | License | Status (2026-11-05) |
|---|---|---|---|---|---|---|---|---|---|---|
| Runner (yicheng47/runner) | Crew in split terminal panes; the `lead` flag is now "a retained storage field ignored by current code", so peers in practice | macOS arm64, Windows x64, Linux beta; no Intel Mac | Claude Code, Codex, Copilot CLI, pi (README) | None found in README or architecture doc | Human-question cards (`runner ask`) | No diff viewer; a reviewer agent | Desktop notifications; remote control in private beta | Native Rust: Zed's GPUI + `alacritty_terminal`, a `runnerd` daemon, Sparkle updater | MIT | 460 stars, active, alpha |
| Agent Orchestrator (OrchestratorInc/agent-orchestrator) | Fleet dashboard plus a per-project main agent that plans and spawns workers | Desktop (macOS, Windows, Linux), iPhone, Android | "35+" (claimed, unverified) | Yes, one per worker | Yes, in chat ("approve the request") | Files view; PR and CI loop | iPhone and Android apps | **Electron** desktop (bundle: Electron Framework, Squirrel), Go core, bundled daemon | Apache-2.0 | 14.1k stars, v0.14.10-nightly installed |
| Vibe Kanban (BloopAI/vibe-kanban) | Kanban issues, then workspaces with a branch, terminal and dev server | Local web UI via `npx`, self-host via Docker | Claude Code, Codex, Gemini CLI, Copilot, Amp, Cursor, OpenCode, Droid, CCR, Qwen Code | Yes | ? | Yes, inline comments sent to the agent | ? | Rust backend + TypeScript web UI | Apache-2.0 | 28.6k stars; README: "Vibe Kanban is sunsetting" |
| octomux (ShreyPaharia/octomux) | Kanban of the fleet; orchestrator tree view; a verify command looped until exit 0 | macOS, Linux; needs Node 18+ and tmux | Claude Code (others unverified) | Yes | One inbox for permission prompts | In-app diff review | ? | **Electron** + React + xterm (package manifest) | MIT | 25 stars, last push 2026-09-18 |
| Pragma (pragma-sh/pragma) | Agent board from prompt to review to PR; one prompt fanned out to several attempts | macOS (arm64, Intel), Windows, Linux; companion iOS, Android, web | Claude Code, Codex, OpenCode, Cursor and more | Yes | ? | Yes, plus staging, commit, push, PRs | Companion apps over your own tunnel | **Tauri v2 + Rust** host, OS webview | AGPL-3.0 | 25 stars, early |
| Orca (stablyai/orca) | Worktree-per-agent IDE; fan one prompt across agents and merge the winner | macOS, Windows, Linux; iOS and Android companion | Codex, Claude Code, OpenCode, Pi and more | Yes | ? | Yes, annotate diff lines and send back | Mobile companion | **Electron** (bundle: Electron Framework, Squirrel), TypeScript; embedded browser and computer use | MIT | 116k stars, v1.4.249 installed |
| Vicoa (vicoa-ai/vicoa) | Team of agents driven from desktop, web or phone | Electron desktop, web, Android | claude, codex, hermes, pi and more | Yes, parallel worktrees (README list) | Reply and approve remotely (unverified) | ? | Mobile apps, push notifications | Electron + Python daemon, PostgreSQL | AGPL-3.0 | 531 stars |
| wmux (openwong2kim/wmux) | Terminal multiplexer with a fleet list (needs you, ready to review, running, idle) | Windows, macOS, Linux | Claude Code, Codex, Gemini and more | Worktree fan-out | Spawn requests wait for approval | Git page with diff and CI logs | ? | Electron + xterm.js, TypeScript | MIT | 448 stars |
| Ghostex (maddada/Ghostex) | Terminal workspace for agents | macOS (others unverified) | Many (unverified) | ? | ? | Annotations sent back to the agent | Mobile apps | Native Rust (GPUI, Ghostty terminals) but embeds CEF and code-server | MIT | 1.6k stars |
| Traycer (traycerai/traycer) | Parallel sessions, agent-to-agent loops, shareable boards | macOS (arm64, Intel), Windows, Linux | Bring your own agent | ? | ? | ? | Cross-device sync (unverified) | TypeScript (+ Swift); framework unverified | MIT | 1.65k stars |
| ai-maestro (23blocks-OS/ai-maestro) | Multi-machine dashboard, signed agent-to-agent messaging, memory search | macOS, Linux, Windows (WSL2) | "Any agent" | ? | ? | ? | Push via its messaging protocol (unverified) | TypeScript web dashboard at `localhost:23000`, CozoDB | MIT | 845 stars |

### Runner, studied closest

Runner is the nearest to our model (a crew of CLI agents, each in a real terminal) and the strongest native
competitor. From its README and `docs/arch/arch.md`:

- **Coordination is a file bus.** One append-only NDJSON event log per mission, appended with `flock` from
  each agent's PTY, tailed by a router. Close to Vajra's own campaign files, which validates file-first state.
- **The router types into agents' terminals.** It writes into a session's PTY through the same serialized
  writer the human types through, and the architecture doc says these injections deliberately emit no audit
  event. This is exactly what the owner's rules forbid for us (ADR-2 in design.md).
- **The lead is gone.** The lead flag is ignored by current code, so the crew is peers who can each ask the
  human. Vajra keeps a real lead (the Lead agent routes and never approves).
- **No worktree isolation** found in its docs, so agents share a checkout; Vajra gives every writer its own
  worktree and merges one integration branch.
- **Memory:** the only claim is a design target (a handful of missions, about 10 live sessions, "tens of MB
  each only when scrollback is full"); no published measurement. GPUI renders without a webview. Runner is
  measured by POS-4.2 for our own budgets; we don't compare ourselves with it in marketing.

### Agent Orchestrator: complement, don't duplicate

The owner already uses Agent Orchestrator for fleets of independent tasks across projects, with a main
agent per project, phone apps and the task to PR to CI to merge lifecycle. Vajra Desktop does none of that.
It goes deep on one thing AO treats as a black box: what happens inside a single Vajra delivery (criteria,
task graph, verify and fix rounds, evidence, campaign checkpoints, the knowledge ledger). The two fit
together: an AO worker can run `/vajra <ask>`, and Vajra Desktop is where you watch and steer that run and
approve its plan; AO then carries the approved branch through PR and CI.

### Where Vajra Desktop differs

| | Vajra Desktop | Others |
|---|---|---|
| Evidence per acceptance criterion | Each criterion shows the command, exit code, signed evidence file and commit; no evidence means "no evidence", never a pass | None found. octomux loops one `--verify` command until exit 0; Runner uses a reviewer agent; AO, Orca and Pragma gate on CI and PRs |
| Verify and fix rounds | Up to 3 rounds, per failing criterion: only that criterion's task reruns, and the report names what is still failing | Parity on the loop itself: octomux loops one verify command until exit 0; AO sends failed CI back to the owning worker. Only the per-criterion part is ours |
| The guard | Every tool call passes Vajra's fail-closed guard; protected actions come back as plans; the app adds no bypass | Permission prompts are passed through or answered in-app |
| A crew you can talk to | Ask the Lead about any worker; it messages, redirects or stops workers through a documented, audited channel, never by typing into a terminal | AO has a main agent you chat with (mechanism not confirmed); Runner injects into PTYs without audit; Traycer and ai-maestro have agent-to-agent messaging |
| Memory | Bounded and attributed: the app and each run's agents shown separately (AC-2.1.4, POS-4.7); budgets in NFR-PERF; no comparative claim | Parity on attribution: Orca has a Resource Manager. Measured RSS: Runner 201.3 MiB and Orca 1,061.5 MiB idle (single busy-machine runs) |

Memory is a budget, not a positioning claim. Pragma shares our stack (Tauri v2 + Rust) and Runner is native
GPUI, so a low footprint can't rest on the stack choice alone. It has to come from the design: no embedded browser, no database server, workers
read from transcripts instead of one PTY each, bounded buffers, and virtualized views. And it has to be
measured, not claimed.

### Deliberately not built

A fleet kanban, issue-tracker intake, PR, CI and merge automation, mobile apps and push, multi-machine
mesh, cloud sync and shared boards, an embedded browser or IDE, fan-out of one prompt to competing
attempts, agent breadth beyond Claude Code and Codex CLI (Gemini CLI and OpenCode come post-MVP, through the same
adapter interface), and account or usage switching. Each belongs to a tool above,
mostly to Agent Orchestrator, which the owner already runs.

No competitor's name, branding, UI, copy or assets are used in Vajra Desktop.

### Pre-mortem: six months after launch, why did it fail?

From `vajra-4/pain-points.md` (the Critic's protocol, run inline by vajra-4; an independent Critic pass is
part of review stage A1). Ranked by impact times likelihood.

| # | Failure | Mitigation in this spec | Detection signal |
|---|---|---|---|
| 1 | The platform absorbed us: Claude Code's own agent view and Desktop ship crew views, and we read as "a Claude wrapper" | Compete where Claude Code is weak: evidence per criterion, the audited control channel, Vajra's verify gate. Cut any feature it ships natively (VD-34) | Each Claude Code release note checked against our feature list |
| 2 | Provider terms or billing stop subscription use through third-party apps | The app runs the stock interactive `claude` under the user's own login and never proxies auth or uses `-p` for runs (POS-2.4, VD-13) | Provider terms changes |
| 3 | "Low memory" is disproved on day one because agents dominate RAM | App and agent memory always shown separately (AC-2.1.4); no claim until NFR-PERF gates are met, and never a total-RAM number | Launch reviews quoting Activity Monitor |
| 4 | Terminal feel worse than a real terminal, so power users go back to tmux | xterm.js core, NFR-PERF-11, AC-2.2.6 | "Went back to tmux" in feedback |
| 5 | Wrong status kills trust | Unknown instead of guessing (AC-2.1.5) | False-idle rate on fixtures |
| 6 | Workers ignore control messages, so steering is hollow | Proven by the tracer bullet with a nonce-tagged steer; L1-L7 live-model checks (STRIDE) before launch | Steer obedience rate in L4 |
| 7 | An update breaks live sessions | One process, updates wait until no run is active (POS-2.1c) | N to N+1 upgrade test with a live run |
| 8 | We copy the 500-feature surface | Six jobs only (VD-29); every feature cites a job, P-row or POS reason | Screen count in the H2 crawl |
| 9 | No distribution among many clones | The launch story is the evidence demo (PRD 60-second demo), backed by measurements | Installs from launch posts |
| 10 | The two-model gate makes us too slow | Gate only irreversible stack and design decisions; time-box each round | Days per gate round |
| 11 | Worktree disk bloat | Vajra's cleanup already removes clean worktrees; disk display deferred | Disk-full reports |
| 12 | Security: permission bypass by default, agent output steering the orchestrator | POS-2.4 argv test, NFR-SEC-3 and NFR-SEC-9, steering threat model (F1-F4) | Security gate H4, PoCs H5 |

### Pre-mortem, second pass (Critic, 2026-11-05)

Eight failure modes the first table missed, found after ADR-9 and the A1 fixes. All are adopted in the
spec (requirements 7b, PM-1 to PM-8).

| # | Failure | Mitigation | Signal |
|---|---|---|---|
| 13 | Resume continues the wrong campaign: `/vajra continue` follows Vajra's one global `.active` pointer, which every new run rewrites | Resume by slug as argv (ADR-9); POS-2.1e | Two sessions on one slug |
| 14 | No task owns the live end-to-end checks, so beta users are the first to run real control, resume and approve | H8 owner-run live acceptance blocks launch | RC tagged with no dated live run |
| 15 | Performance gates close after the work they gate | H0 perf CI from C1; end-of-M3 budget rule | First real idle number arrives late |
| 16 | A Claude Code update breaks steering silently | NFR-COMPAT-1: tested version range; "delivered" only when the transcript shows the steer | Acks with no steer in the next turn |
| 17 | Approvals stranded after the session ends | AC-5.2.5: approve resumes the session as argv | Inbox items with exited sessions |
| 18 | The menu bar app turns a slow leak into a multi-day one | Webview destroyed when the window closes; 72 h soak; Install now after 72 h | RSS by uptime |
| 19 | Transcript format change breaks crew, control and reports at once | E4 and E6 into M1: control matches on `tasks.json` | "Format not recognized" rate |
| 20 | A crash leaves orphan processes or a stale signature | Process group SIGTERM on quit; resume refused while it lives; fault test POS-2.1f | Orphans after relaunch |

## 8. Non-goals (deliberately not built)

- **A fleet kanban or issue-tracker intake.** Vajra plans its own task graph from one ask.
- **PR, CI and merge lifecycle.** Agent Orchestrator does this well and the owner already uses it. Vajra
  Desktop stops at "approved plan handed to Claude Code".
- **Mobile apps, push to phone, multi-machine, cloud sync, team sharing.**
- **Agents beyond Claude Code and Codex CLI in the MVP.** One small agent-adapter interface in the core
  (owner decision, 2026-11-05): Claude Code is the full adapter, Codex CLI the second with honest capability
  flags. Gemini CLI and OpenCode come after the MVP. No plugin system.
- **An editor or in-app code edits**, a plugin API, an embedded browser.
- **Replacing any Vajra primitive**: hooks, workflows, agents, campaigns, the guard, the ledger.
- **Auto-approve** of any protected action, ever, including batch approval of permission prompts across a crew (VD-22).
- **Features without a reason.** Every feature cites an MVP job, a pain point (requirements 7a) or a positioning reason.
- **Windows and Linux** in the MVP.

## 9. Success metrics

| Metric | Target | How measured |
|---|---|---|
| Find what a worker is stuck on | Median under 10 s from opening the app | Usability test, 5 Vajra users, scripted stuck run |
| Hidden control actions | 0 | Reconcile the control ledger against `audit.log` and the UI action feed after the 10-agent load test |
| Memory budgets (not a claim) | NFR-PERF-1 and NFR-PERF-6 pass on the real build; competitor medians and method published | H1 harness, same machine, same 10-agent fixture (POS-4.1, POS-4.2, POS-4.4, POS-4.6) |
| False "done" caught | Reported number, no target yet | POS-1.4 seeded benchmark, at least 20 tasks with planted defects |
| Review time | Reported, no target yet | POS-1.6 study, 5+ developers, transcript plus diff vs evidence view |
| Runs survive a closed window | 100% of runs continue when the window closes; resume offered after every quit or crash | POS-2.1 to POS-2.1b in CI (ADR-9; surviving quit and SIGKILL is v1.1) |
| Budgets | All NFR-PERF budgets met | CI perf job + 1 h soak |
| Keyboard-only | J1 to J6 completable without a pointer | Scripted keyboard walkthrough |
| Crash-free sessions | At least 99.5% in beta | Local crash reports, opt-in |
| Evidence honesty | 0 criteria shown as pass without evidence | Fixture campaigns with missing and failing evidence |

## 10. Performance and memory budgets

The canonical table (targets, prototype measurements and what is still unmeasured) is in [spec/requirements.md](spec/requirements.md#performance), not repeated here. The short version:

- **Idle fails:** 220.2 MiB against a 200 MiB ceiling.
- **Hourly growth fails:** 31.1 MiB against 20 MiB.
- **Unmeasured:** marginal cost per worker, render time, cold boot and physical footprint.
- **Like-for-like load:** at 10 workers the Tauri prototype measured 805.3 MiB and Runner 1,138 MiB, both
  5-minute figures. That comparison isn't matched (Runner renders 10 terminals), so it supports no claim.
- **Agent Orchestrator** was uncontrolled (it hosted the measurement).

The owner authorized provisional Tauri work. Performance approval is withheld by both models.

## 11. Security model

- **Respect Vajra's guard.** The app adds no bypass. The only guard change is additive: a new protected
  root for the control queue and logging of the app's control tools in `audit.log`.
- **Never read the HMAC key** (`~/.claude/vajra/.hmac-key`). Signature checks shell out to
  `vajra-campaign.sh verify`; evidence checks to Vajra's scripts.
- **Never weaken the sandbox.** The app shows the sandbox state and links to `scripts/enable-sandbox.sh`;
  it never edits settings.
- **All agent output is untrusted.** Rendered as text, never interpreted; never passed to a shell; ANSI
  sanitized; terminal hyperlinks and clipboard escape sequences off by default. Control messages are
  wrapped as untrusted data for the receiving worker and never count as approval.
- **Small IPC surface.** Owner-only Unix socket, peer uid check, no network listener.
- **Filesystem scope.** An explicit read allowlist under `~/.claude/` and the chosen repos; writes only to
  the app's data directory.
- **Signed, notarized updates; locked and audited dependencies.**

### Billing and terms: hard rules (owner decision, 2026-11-05)

From `vajra-4/billing.md` (not legal advice):
- The app launches the stock, unmodified `claude` and `codex` binaries in a PTY, under the user's own login.
- It never reads, stores, proxies or offers credentials or a claude.ai login. Sign-in happens only in the
  agent's own `/login` or `codex login`, which the user runs.
- No `--bare`, and no Agent SDK with plan credentials. Runs never use `-p` (VD-13).
- Each agent row shows its billing source: plan login or API key. When an API key is present it wins (Claude
  Code's precedence), and the row says so.
- An optional API-key mode for big crews: the user sets their own key in their own environment or
  `apiKeyHelper`. The app only detects that one is present; it never holds the value.
- No reselling, paying for or proxying model usage, and no `ANTHROPIC_BASE_URL` gateway.
- Plain-text "runs Claude Code" only, never in the product name or logo.

**Launch blocker: terms status is AMBIGUOUS.** Whether a third-party orchestrator spawning the user's own
`claude` sessions under a consumer plan is "ordinary, individual usage" isn't settled by Anthropic's
published terms. The owner decides whether to ask Anthropic in writing (Q12). Until there's an answer,
launch copy recommends API keys for large crews. The timeline so far:

| Date | Event |
|---|---|
| 2026-01-09 | OpenCode's Claude Max login stops working; its maintainer believes it was blocked |
| 2026-04-04 | Claude subscriptions stop covering usage in third-party tools (extra usage bundles or API keys instead) |
| 2026-05-13 | Anthropic announces that the Agent SDK, `claude -p` and SDK-based apps move to a dedicated monthly credit |
| 2026-06-15 | That change is paused "for now"; nothing changed |

Threat model and tests: [spec/design.md](spec/design.md#8-security-considerations-summary-the-security-role-reviews-later), red-team tasks in
[spec/tasks.md](spec/tasks.md).

## 12. Release plan (MVP)

1. Stack spike and two-model approval (stack approved; performance approval withheld).
2. Build the MVP from [spec/tasks.md](spec/tasks.md) with a `/vajra` run, in two slices (tasks.md,
   "Shipping slices"): slice 1 is the private beta, slice 2 must land before the public launch.
3. Red team: pre-mortem, security gate, exploit PoCs as failing tests, regressions and a11y, load and
   soak, flamegraphs.
4. Private beta on macOS (signed and notarized), then public launch. Each step needs the owner's yes.

## 13. Open questions and owner decisions

### Decision log

| Date | Decision | By | Where it lands |
|---|---|---|---|
| 2026-11-05 | Q3 approved: engine change E4 (persist the delivery report as `report.md` in the campaign) | owner, via orchestrator | tasks E4 (M1, vajra-6 engine work) |
| 2026-11-05 | Q10 approved: the sandbox `denyWrite` for the app data dir and control queue ships as the default in Vajra's sandbox profile, applied only when the user turns the OS sandbox on and approves the settings change | owner, via orchestrator | design 5.4, AC-3.4.8, AC-3.4.9, tasks E8 |
| 2026-11-05 | Q11 approved: E5 (steer nonce and one sentence in task briefs only), the wider E2, E4 and E6, all additive, each under bhishma or vidura review, with every existing guard test passing | owner, via orchestrator | tasks M1 |
| 2026-11-05 | H0 rule: if memory is over budget at the end of M3, the owner chooses between a written budget change, a stack change or a delay | owner, via orchestrator | tasks H0 |
| 2026-11-05 | Agent-adapter interface with Codex CLI; read-only Conductor import; billing hard rules | owner | ADR-10, ADR-11, section 11 |
| 2026-11-05 | Model routing: main components on Opus 5.5 (bhima, nakula, arjuna, bhishma); lighter items on Sonnet 5.5 (sepoys, abhimanyu); search and audit on Haiku; GPT 6.1 Sol reviews every PR | owner | tasks.md owners |

Conductor import is backed by the clearest live demand in the research: P6 (tool shutdown or rug-pull, 13
sources) includes displaced Conductor users asking in public for an alternative (`vajra-4/pain-points.md`).

### Open questions

The owner hasn't answered these yet. Until they do, each **working assumption** below is in effect
and the spec is written to it. Changing one later is a doc edit plus the tasks named in the row.

| # | Question | Working assumption (in effect until the owner decides) |
|---|---|---|
| Q1 | Product name for the public Mac app ("Vajra Desktop" is the working name) | Keep "Vajra" |
| Q2 | Rename the agents inside the engine too, or keep engine ids and map in the app only? | Map in the app only |
| Q3 | Should Vajra persist the delivery report into the campaign (`report.md`)? It is only text in the workflow result today | **Decided:** approved (E4, see the decision log) |
| Q4 | Distribution: direct download (Developer ID, notarized) vs Mac App Store (sandboxed; spawning `claude` in PTYs is hard there) | Direct download |
| Q5 | Minimum macOS version and Intel support | macOS 14+, Apple Silicon first |
| Q6 | Pricing and license for the app (Vajra itself is MIT) | Owner to decide before launch |
| Q7 | Telemetry: none, or opt-in crash reports only | Opt-in crash reports only |
| Q10 | Add a sandbox denyWrite for the app data dir in `config/sandbox.json` (A1 M3)? | **Decided** (see the decision log): shipped by default, applied when the user enables the sandbox; with the sandbox off, control stays stop-only |
| Q11 | Approve engine change E5 (steer nonce argument and one brief sentence in `vajra-deliver.js`) and the wider E2? | **Approved** (2026-11-05): E5 for task briefs only and the wider E2, both under Security review, with every existing guard test passing |
| Q13 | Approve engine change E7 (archive each delivery's signed verification bundle and executed setup list into the campaign before cleanup; E3 `--archived`)? Without it the app can show past proofs only as report text | Yes, additive; under Security review. **Blocks slice 1 (private beta)** |
| Q12 | Ask Anthropic in writing whether a local orchestrator spawning the user's own unmodified `claude` sessions under a consumer plan is permitted, and whether the Commercial Terms apply (section 11)? | **Launch blocker.** Until answered, launch copy recommends API keys for large crews |
| Q8 | Claude Code facts the design depends on (transcript layout for subagents, hook payload agent fields, `additionalContext` for subagent tool calls) | Verify in the spike; fallbacks in design.md |
