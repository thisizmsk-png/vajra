# Vajra Desktop: product requirements

Status: draft for the two-model gate (authored by Opus 5.5, to be checked by GPT 6.1 Sol).
Date: 2026-11-05. Owner decisions are listed at the end.

Companion docs: [spec/requirements.md](spec/requirements.md) (stories and Given/When/Then criteria),
[spec/design.md](spec/design.md) (design, control channel, ADRs; Architecture pending the stack spike),
[spec/tasks.md](spec/tasks.md) (MVP task plan), [UX.md](UX.md), [DESIGN.md](DESIGN.md).

## 1. Summary

Vajra Desktop is a fast, low-memory macOS app for watching and steering Vajra runs. Vajra stays the
engine: its hooks, saved workflows, agents, campaigns, task graph and knowledge ledger keep working
exactly as they do in the terminal, and stay the source of truth. The app reads their state, starts
Claude Code sessions, and adds the things a terminal can't do well: a crew tree you can hop through, a
chat with the lead agent about any worker, a run view with evidence per acceptance criterion, an
approvals inbox, and history.

One line: **a crew you can see and talk to, with proof for every "done", in a fraction of the memory.**

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
- **Resuming is manual.** `/vajra continue` works, but finding which campaign is stuck, and why, means
  reading `progress.md` files by hand.

The desktop orchestrators that exist (section 7) are mostly fleet dashboards: many independent agents,
a kanban board, a PR at the end. None shows evidence per acceptance criterion, none runs verify/fix rounds,
and the two installed on this Mac are Electron apps (bundle inspection), one using about 1.6 GB summed RSS
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
| J1 Start and resume | Start a `/vajra` run (repo, ask, base branch, max rounds) and resume a campaign (`/vajra continue`), after a preflight that shows whether Claude Code, Vajra's hooks and the sandbox are in place | US-1.1 to US-1.3 |
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
2. **Performance and memory.** Idle and active memory clearly lower than the competitors we measure.
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
| octomux (ShreyPaharia/octomux) | Kanban of the fleet | macOS, Linux; needs Node 18+ and tmux | Claude Code (others unverified) | Yes (unverified detail) | One inbox for permission prompts | In-app diff review | ? | TypeScript web UI over tmux | MIT | 25 stars, last push 2026-09-18 |
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
  each only when scrollback is full"); no published measurement. GPUI renders without a webview, so Runner
  is the competitor to beat on memory, and the spike must measure it if it can be installed.

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
| Verify and fix rounds | Up to 3 rounds; a failing criterion goes back to its owning task; the timeline shows each round | Not found as a product concept |
| The guard | Every tool call passes Vajra's fail-closed guard; protected actions come back as plans; the app adds no bypass | Permission prompts are passed through or answered in-app |
| A crew you can talk to | Ask the Lead about any worker; it messages, redirects or stops workers through a documented, audited channel, never by typing into a terminal | AO has a main agent you chat with (mechanism not confirmed); Runner injects into PTYs without audit; Traycer and ai-maestro have agent-to-agent messaging |
| Memory | Budgets set from measurement and checked in CI against installed competitors | No competitor publishes measured RSS. AO, Orca, wmux and Vicoa are Electron; Pragma is Tauri; Runner and Ghostex are native (Ghostex embeds Chromium) |

Pragma shares our likely stack (Tauri v2 + Rust) and Runner is native GPUI, so "lower memory" can't rest on
the stack choice alone. It has to come from the design: no embedded browser, no database server, workers
read from transcripts instead of one PTY each, bounded buffers, and virtualized views. And it has to be
measured, not claimed.

### Deliberately not built

A fleet kanban, issue-tracker intake, PR, CI and merge automation, mobile apps and push, multi-machine
mesh, cloud sync and shared boards, an embedded browser or IDE, fan-out of one prompt to competing
attempts, agent breadth beyond Claude Code, and account or usage switching. Each belongs to a tool above,
mostly to Agent Orchestrator, which the owner already runs.

No competitor's name, branding, UI, copy or assets are used in Vajra Desktop.

## 8. Non-goals (deliberately not built)

- **A fleet kanban or issue-tracker intake.** Vajra plans its own task graph from one ask.
- **PR, CI and merge lifecycle.** Agent Orchestrator does this well and the owner already uses it. Vajra
  Desktop stops at "approved plan handed to Claude Code".
- **Mobile apps, push to phone, multi-machine, cloud sync, team sharing.**
- **Non-Claude agents** (Codex, Gemini CLI and others). Vajra is a Claude Code harness.
- **An editor or in-app code edits**, a plugin API, an embedded browser.
- **Replacing any Vajra primitive**: hooks, workflows, agents, campaigns, the guard, the ledger.
- **Auto-approve** of any protected action, ever.
- **Windows and Linux** in the MVP.

## 9. Success metrics

| Metric | Target | How measured |
|---|---|---|
| Find what a worker is stuck on | Median under 10 s from opening the app | Usability test, 5 Vajra users, scripted stuck run |
| Hidden control actions | 0 | Reconcile the control ledger against `audit.log` and the UI action feed after the 10-agent load test |
| Memory vs competitors | Idle and per-agent RSS below every measured competitor (Agent Orchestrator desktop, Orca, at least) | Spike harness, same machine, same 10-agent fixture |
| Budgets | All NFR-PERF budgets met | CI perf job + 1 h soak |
| Keyboard-only | J1 to J6 completable without a pointer | Scripted keyboard walkthrough |
| Crash-free sessions | At least 99.5% in beta | Local crash reports, opt-in |
| Evidence honesty | 0 criteria shown as pass without evidence | Fixture campaigns with missing and failing evidence |

## 10. Performance and memory budgets

Provisional until the stack spike reports measured numbers; the final table goes in
[spec/design.md](spec/design.md) and replaces these. IDs from [spec/requirements.md](spec/requirements.md).

| ID | Budget (provisional) |
|---|---|
| NFR-PERF-1 | Idle RSS, app with no session attached: at most 150 MB summed over all app processes |
| NFR-PERF-2 | Each attached agent stream adds at most 15 MB; ring buffer 2 MB per stream, then spill to disk |
| NFR-PERF-3 | UI frame time p95 at most 16.7 ms, p99 at most 33 ms with 10 agents streaming 200 lines/s each; lists and terminals virtualized |
| NFR-PERF-4 | Cold start to interactive at most 1.0 s on an M-series Mac |
| NFR-PERF-5 | Bounded queues; a slow UI consumer drops to a snapshot; a PTY or file reader is never blocked |
| NFR-PERF-6 | 1 h soak under the NFR-PERF-3 load: RSS growth at most 5% after warm-up, no leaked handles |
| NFR-PERF-7 | Lower idle RSS and RSS per stream than every measured competitor, same machine and method |
| NFR-PERF-8 | The control hook adds at most 10 ms p95 per tool call when the app is idle |

Measured against at least two competitors installed locally (Agent Orchestrator desktop 0.14.10-nightly
and Orca 1.4.249 are installed on the reference Mac).

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

Threat model and tests: [spec/design.md](spec/design.md#8-security-considerations-summary-the-security-role-reviews-later), red-team tasks in
[spec/tasks.md](spec/tasks.md).

## 12. Release plan (MVP)

1. Stack spike and two-model approval (in progress).
2. Build the MVP from [spec/tasks.md](spec/tasks.md) with a `/vajra` run.
3. Red team: pre-mortem, security gate, exploit PoCs as failing tests, regressions and a11y, load and
   soak, flamegraphs.
4. Private beta on macOS (signed and notarized), then public launch. Each step needs the owner's yes.

## 13. Open questions and owner decisions

The owner hasn't answered these yet. Until they do, each **working assumption** below is in effect
and the spec is written to it. Changing one later is a doc edit plus the tasks named in the row.

| # | Question | Working assumption (in effect until the owner decides) |
|---|---|---|
| Q1 | Product name for the public Mac app ("Vajra Desktop" is the working name) | Keep "Vajra" |
| Q2 | Rename the agents inside the engine too, or keep engine ids and map in the app only? | Map in the app only |
| Q3 | Should Vajra persist the delivery report into the campaign (`report.md`)? It is only text in the workflow result today | Yes, a small engine change |
| Q4 | Distribution: direct download (Developer ID, notarized) vs Mac App Store (sandboxed; spawning `claude` in PTYs is hard there) | Direct download |
| Q5 | Minimum macOS version and Intel support | macOS 14+, Apple Silicon first |
| Q6 | Pricing and license for the app (Vajra itself is MIT) | Owner to decide before launch |
| Q7 | Telemetry: none, or opt-in crash reports only | Opt-in crash reports only |
| Q8 | Claude Code facts the design depends on (transcript layout for subagents, hook payload agent fields, `additionalContext` for subagent tool calls) | Verify in the spike; fallbacks in design.md |
