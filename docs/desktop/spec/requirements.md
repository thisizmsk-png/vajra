# Vajra Desktop: Requirements (spec Phase 1)

Status: draft, returned as `needs_decision`. Phase 1 of 3 (Requirements, Design, Tasks). Owner: Product Owner role. Date: 2026-11-05.
Sources: `docs/commands/deliver.md`, `docs/commands/campaign.md`, the shared brief. Terms: "Lead", "Backend", "Verifier" etc. are the UI names of engine agents (mapping lives in design.md only).

## 0. Open assumptions (owner to confirm or correct)

| # | Assumption | State |
|---|---|---|
| A1 | MVP is macOS only (Apple Silicon first, Intel if it costs nothing). No Windows or Linux. | open |
| A2 | The UI shows generic role names (Lead, Architect, Backend, ...). Engine ids (krishna, bhima, ...) appear only in the worker inspector's "engine id" field. Renaming agents inside the Vajra engine is a separate owner decision. | open |
| A3 | Workers are subagents inside ONE Claude Code run session. They have no PTY. Their live stream is rendered from Claude Code transcript files (session jsonl plus subagent transcripts), tailed read-only. Transcript layout and stability are to be verified in the stack spike. | open |
| A4 | The Lead chat is a separate Claude Code session (Lead agent, read tools plus the app's control tools), so chat never reaches workflow agents (deliver.md, "Messages sent while it runs"). | open |
| A5 | Control messages reach a worker through a new Vajra hook script (`hooks/desktop-control.sh`, not the guard) reading an app-owned queue. The queue directory is guard-protected from agents. Feasibility of agent identification from the hook payload is to be verified in the spike. | open |
| A6 | All numeric budgets (Section 5) are PROVISIONAL until the stack spike replaces them with measured numbers. | open |
| A7 | Stack (Tauri v2 + Rust core vs Electron) is undecided; this document is stack-agnostic. | open |
| A8 | Delivery reports are workflow result text, not files. The app stores its own copy; whether Vajra should persist the report in the campaign is an open question for the Vajra owner. | open |
| A9 | Light theme is derived from the dark brand palette (no light art exists). Design concern, not a requirement here. | open |

## 1. Problem statement

**Who.** A developer running Vajra deliveries (`/vajra <ask>`), typically 4 to 16 parallel workers (`maxInFlight` 1..16, default 4).

**Pain.**
- A run is opaque: one terminal, one scrolling log. There is no way to hop between workers, see one worker's diff, or ask "what is Backend stuck on?"
- Steering a worker means typing into the run session. deliver.md states Claude Code relays the most recent user message to every workflow agent started afterwards, so typing there can leak to all workers.
- Approvals (`pending_approval` plans, `need_input` questions, blocked tasks) and per-criterion evidence are buried in report text.
- Existing fleet dashboards (Agent Orchestrator desktop, Orca) are Electron apps with high memory (one snapshot: Agent Orchestrator, 13 processes, about 1.6 GB RSS) and show sessions, not per-criterion evidence.

**Cost of inaction.** Users keep one terminal and read the end report; stuck workers are found late; people paste steering text into the run session and pollute the run.

**Why now.** A public Mac launch is planned, and the Vajra engine (hooks, signed evidence, campaigns) is stable enough to build a view on top of it.

## 2. Personas

- **Dev (primary).** Runs Vajra on their own repos, 4 to 16 workers, comfortable with a terminal, wants to watch, ask, steer, approve without losing the audit trail.
- **Reviewer-approver (secondary).** Often the same person in a different mood: returns to a finished or paused run and needs the report, per-criterion evidence and pending approvals in under a minute, mostly by keyboard.

## 3. User stories and acceptance criteria

Jobs: J1 start/resume, J2 crew, J3 Lead chat and control, J4 run view, J5 approvals, J6 history and knowledge. Persona "Dev" unless stated.

### J1 Start and resume

**US-1.1** As a Dev, I want to start a run by choosing a repo, typing an ask, a base branch and max rounds, so that I launch `vajra-deliver` without composing JSON by hand.
- AC-1.1.1 GIVEN a git repo is chosen and preflight passes, WHEN I fill ask (non-empty), base branch and max rounds (1..3) and press "Start run", THEN the app launches `claude --session-id <uuid>` in that repo with the vajra-deliver launch, args include `run_id` as UTC `YYYYMMDD-HHMM` and a `session_id` matching `^[A-Za-z0-9-]{8,}$`, and the Run view opens within 1 s.
- AC-1.1.2 GIVEN the ask field is empty or max rounds is outside 1..3, WHEN I press "Start run", THEN the button is disabled or an inline error names the field, and no process is spawned.
- AC-1.1.3 GIVEN any run start, WHEN it completes or fails, THEN no file under `~/.claude` was created or modified by the app itself; changes there come only from Claude Code and Vajra's own scripts (verified by a filesystem-event diff over `~/.claude` attributing writes by process).
- AC-1.1.4 GIVEN the advanced section, WHEN I set maxInFlight outside 1..16, THEN the value is rejected with an inline message; a valid value is passed as `maxInFlight`.

**US-1.2** As a Dev, I want to resume a campaign, so that I continue unfinished work (`/vajra continue`).
- AC-1.2.1 GIVEN the campaign list shows campaigns found under `~/.claude/vajra/campaigns/`, WHEN I choose one and press "Resume", THEN the app runs `vajra-campaign.sh verify <slug>` first and, on exit 0, launches the run session with `campaign: <slug>` and a fresh `run_id`.
- AC-1.2.2 GIVEN `vajra-campaign.sh verify <slug>` exits 3, WHEN I press "Resume", THEN resume is blocked, no run session starts, and the app shows "Integrity check failed" with the restore command (`vajra-campaign.sh resume <slug> --from-checkpoint N`, N being the latest checkpoint listed).
- AC-1.2.3 GIVEN a resumable campaign, WHEN I open the resume dialog, THEN it shows slug, goal, features passing as N/M, and the last integration branch recorded in progress; an optional "base" field overrides it.

**US-1.3** As a Dev, I want a preflight check, so that I know why a run cannot start before it fails.
- AC-1.3.1 GIVEN the app opens, WHEN preflight runs, THEN rows show: `claude` on PATH (with version), `vajra doctor` PASS or FAIL, hooks registered (yes/no), sandbox state (on/off/unknown), git, python3, jq, openssl present.
- AC-1.3.2 GIVEN any required row fails, WHEN I look at the Start screen, THEN "Start run" is disabled and the failing row shows one fix hint (a command or link).
- AC-1.3.3 GIVEN preflight, WHEN it executes, THEN it only runs read-only commands and never reads `~/.claude/vajra/.hmac-key` (verified: no open of that path in the process trace).
- AC-1.3.4 GIVEN `~/.claude/workflows/vajra-deliver.js` is missing, WHEN preflight runs, THEN the row says so and shows `bash ~/.claude/skills/vajra/scripts/install.sh` followed by `/vajra doctor`.

### J2 Crew

**US-2.1** As a Dev, I want a crew tree with status, so that I see who is doing what at a glance.
- AC-2.1.1 GIVEN a run is active, WHEN I open Crew view, THEN the tree shows Lead -> Run -> agents (one node per task owner per round) -> sub-workers (verifier and reviewer passes, fix-round reruns), each node with role name, a status dot AND a text status (pending, ready, claimed, in_progress, done, blocked, question, failed).
- AC-2.1.2 GIVEN `tasks.json` status of a task changes, WHEN the file changes, THEN the node's status updates within 2 s without user action.
- AC-2.1.3 GIVEN an agent id not in the mapping, WHEN it is rendered, THEN it shows the id title-cased, and no mahabharat name appears anywhere in the UI except the inspector's "engine id" field.

**US-2.2** As a Dev, I want a worker's live stream, so that I can read what it is doing.
- AC-2.2.1 GIVEN a worker node is selected, WHEN the Stream tab is shown, THEN lines from its transcript appear within 1 s of being written, rendered as plain text (no HTML, no escape sequences executed).
- AC-2.2.2 GIVEN a stream exceeds its ring buffer, WHEN more lines arrive, THEN the oldest lines spill to disk and remain reachable by scrolling, and app memory per attached stream stays within NFR-PERF-2.
- AC-2.2.3 GIVEN a worker is not selected, WHEN it produces output, THEN no UI tab is rendered for it but its buffer keeps receiving data and the node shows an "activity" indicator.
- AC-2.2.4 GIVEN the run session PTY is shown, WHEN I type in it without pressing "Take over", THEN no input is sent; "Take over" is an explicit button and the pane shows "You are controlling the run session" while active.

**US-2.3** As a Dev, I want a worker's diff, worktree and current task, so that I can judge its progress.
- AC-2.3.1 GIVEN a worker with a worktree, WHEN I open the Diff tab, THEN it shows `git diff` of that worktree against its base, read-only, with no edit control.
- AC-2.3.2 GIVEN a worker, WHEN I look at its header, THEN it shows role name, status, current task id and title, worktree path and branch, each copyable.
- AC-2.3.3 GIVEN the Task tab, WHEN opened, THEN it shows owner, deps, files, acceptance, attempts and last report from `tasks.json`; the inspector shows the engine id.

**US-2.4** As a Dev, I want to hop between workers by keyboard and palette, so that I never need the mouse.
- AC-2.4.1 GIVEN Crew view, WHEN I press Cmd-K and type a role or task name, THEN the palette lists matching workers and Enter focuses that worker's pane.
- AC-2.4.2 GIVEN focus is in the tree, WHEN I press Up/Down/Left/Right, THEN focus moves between nodes and the Worker pane follows within 100 ms.
- AC-2.4.3 GIVEN any worker, WHEN I use only the keyboard, THEN I can switch tabs (Stream, Diff, Task, Worktree) and return focus to the tree.

### J3 Lead chat and control

**US-3.1** As a Dev, I want to ask the Lead about a worker, so that I get an answer without reading its stream.
- AC-3.1.1 GIVEN the chat panel, WHEN I ask "what is Backend stuck on?", THEN the Lead calls `list_workers` / `inspect_worker` and its reply cites the worker, task id and the last transcript evidence; each call appears as an action card.
- AC-3.1.2 GIVEN the Lead chat session, WHEN it runs, THEN it is a separate Claude Code session from the run session; a chat message never appears in any worker's context (verified: absent from every worker transcript).
- AC-3.1.3 GIVEN a read-only question, WHEN the Lead answers, THEN no entry is added to any worker queue.

**US-3.2** As a Dev, I want to message or redirect a worker (for example "use the kit Button" to Frontend), so that I steer it without typing into the run session.
- AC-3.2.1 GIVEN a worker is running, WHEN I send a message to it (via Lead or the worker's "Message" control), THEN nothing is written to any PTY (verified: zero bytes to the run PTY from the control path).
- AC-3.2.2 GIVEN a queued message, WHEN the worker makes its next tool call, THEN the message appears in that worker's next tool-call context as additionalContext wrapped in `<untrusted-data source="human-via-desktop">` (`source="human"` if typed directly by the human), and the queue entry is acknowledged exactly once.
- AC-3.2.3 GIVEN a message was sent, WHEN I check the four places, THEN it appears (a) as an action card in the chat, (b) on the worker's activity feed, (c) as a line in the control ledger, (d) in Vajra's `audit.log` as an ALLOW or BLOCK line for the control tool.
- AC-3.2.4 GIVEN a message mentions "approve" or "push", WHEN delivered, THEN no protected action is unlocked; an attempted push still needs Claude Code's permission prompt (control messages are never approval).
- AC-3.2.5 GIVEN the worker finishes before its next tool call, WHEN the message is unconsumed, THEN the card shows "not delivered" and the message is not applied later to a different worker.

**US-3.3** As a Dev, I want to stop a worker, so that I can halt a wrong direction.
- AC-3.3.1 GIVEN a worker, WHEN I press "Stop", THEN a confirm dialog names the worker and task; Cancel sends nothing.
- AC-3.3.2 GIVEN I confirm, WHEN the worker makes its next tool call, THEN the call is blocked with reason "stopped by the human" and the worker node shows "stopped by the human".
- AC-3.3.3 GIVEN Lead asks to stop a worker, WHEN requested, THEN the same human confirm dialog appears; the Lead cannot confirm on my behalf.

**US-3.4** As a Dev, I want every control action visible and doable directly, so that I trust the Lead.
- AC-3.4.1 GIVEN any control action (list, inspect, message, redirect, stop), WHEN executed by the Lead or by me, THEN it appears in the activity feed and in the control ledger (append-only JSONL) with an action id; message, redirect and stop also appear in `audit.log`: the hook's delivery or stop line carries the same action id, and a Lead tool call is logged by the guard under its MCP tool name (design.md D5: the guard's own lines carry no id, and reads I make in the UI are ledger-only).
- AC-3.4.2 GIVEN the Lead is unavailable or its session has ended, WHEN I use the worker's Message, Redirect and Stop controls, THEN each works without the Lead.
- AC-3.4.3 GIVEN a reconciliation script compares ledger action ids to `audit.log`, WHEN run after a session, THEN zero ledger entries lack an audit line and zero control audit lines lack a ledger entry.
- AC-3.4.4 GIVEN the control queue directory, WHEN a worker tries to write to it, THEN the guard blocks it and `audit.log` records BLOCK.

### J4 Run view

**US-4.1** As a Dev, I want a phase timeline, so that I know where the run is.
- AC-4.1.1 GIVEN a run, WHEN the Run view opens, THEN it shows phases Understand, Plan, Execute, Verify, Fix (round N of max), Deliver with exactly one marked current and a text label for each state.
- AC-4.1.2 GIVEN the run ends, WHEN the result is `done`, `need_input`, `rejected` or `error`, THEN the timeline shows that status and the node reached.

**US-4.2** As a Dev, I want a task graph with owners, so that I see dependencies and waves.
- AC-4.2.1 GIVEN `tasks.json`, WHEN the graph renders, THEN each node shows task id, owner role name and status text, and edges follow `deps`.
- AC-4.2.2 GIVEN a graph of 200 tasks, WHEN scrolled or zoomed, THEN the list is virtualized and meets NFR-PERF-3.
- AC-4.2.3 GIVEN I select a node, WHEN I press Enter, THEN focus jumps to that worker in Crew view.

**US-4.3** As a Dev, I want acceptance criteria with pass or fail evidence, so that I trust "done".
- AC-4.3.1 GIVEN a criterion, WHEN it shows "pass", THEN the row lists the command, exit code 0, evidence file path and commit, taken from signed `evidence.json` or the Verifier result.
- AC-4.3.2 GIVEN a criterion with no signed evidence and no Verifier result, WHEN rendered, THEN it shows "no evidence" and never "pass".
- AC-4.3.3 GIVEN evidence whose HMAC fails verification (checked through Vajra's script, never by reading the key), WHEN rendered, THEN the row shows "unverified evidence" and is not counted as pass.
- AC-4.3.4 GIVEN a criterion failed, WHEN I expand it, THEN it shows the command, nonzero exit code and Verifier notes.

### J5 Approvals

**US-5.1** As a Reviewer-approver, I want one inbox of things waiting on me, so that nothing stalls silently.
- AC-5.1.1 GIVEN items exist, WHEN I open the inbox, THEN it lists `pending_approval` plans, `need_input` questions, tasks in blocked or question, and sessions waiting on a Claude Code permission prompt, each with source, age and a link.
- AC-5.1.2 GIVEN a session waits on a permission prompt, WHEN I choose its item, THEN the app focuses that terminal; it does not answer the prompt.
- AC-5.1.3 GIVEN a new item arrives, WHEN the app is in the background, THEN the Dock badge count updates within 2 s.

**US-5.2** As a Reviewer-approver, I want to approve a plan, so that execution proceeds under my control.
- AC-5.2.1 GIVEN a pending plan, WHEN I open it, THEN the exact plan text (commands, rollback) is shown unabridged before the Approve button is enabled.
- AC-5.2.2 GIVEN I press Approve, WHEN submitted, THEN the app submits a visible human turn "Execute approved plan <id>" to the run session (the turn is visible in that terminal) and does not itself run git push, open a PR or deploy.
- AC-5.2.3 GIVEN the plan includes push, PR or deploy, WHEN the run session proceeds, THEN Claude Code's own permission prompt still appears for each protected command.
- AC-5.2.4 GIVEN approval, WHEN it happens, THEN a ledger entry and `audit.log` line record who, plan id and time.

**US-5.3** As a Reviewer-approver, I want to reject a plan with a reason, so that the record shows why.
- AC-5.3.1 GIVEN a pending plan, WHEN I press Reject, THEN a reason (non-empty) is required.
- AC-5.3.2 GIVEN a reason, WHEN confirmed, THEN the app runs `vajra-campaign.sh progress "<reason>"` for that campaign, with the reason passed as one argument (no shell interpolation).
- AC-5.3.3 GIVEN rejection, WHEN done, THEN the item leaves the inbox and appears in History with state "rejected".

### J6 History and knowledge

**US-6.1** As a Reviewer-approver, I want campaign history and delivery reports, so that I can revisit past runs.
- AC-6.1.1 GIVEN campaigns exist, WHEN I open History, THEN a table lists slug, goal, status, features passing N/M and date, sortable by keyboard.
- AC-6.1.2 GIVEN a delivered run, WHEN I open it, THEN the stored report (markdown) renders as text, showing branch, commit and evidence per criterion.
- AC-6.1.3 GIVEN a campaign whose signature fails verification, WHEN listed, THEN it shows "integrity check failed" and is excluded from resume.

**US-6.2** As a Dev, I want to search the knowledge ledger, so that I reuse past decisions.
- AC-6.2.1 GIVEN a query, WHEN submitted, THEN the app runs `vajra_kb.py search "<query>"` with the query as one argument and lists hits.
- AC-6.2.2 GIVEN search exits 0 with no output, WHEN shown, THEN the UI says "no results"; a non-zero exit says "knowledge search unavailable" with the exit code.
- AC-6.2.3 GIVEN search, WHEN it runs, THEN the app never writes to the ledger (no `add` calls).

## 4. Scope

**In scope.** The six jobs above, macOS, reading Vajra state, launching Claude Code sessions, a control channel for the Lead and the human.

**Out of scope (non-goals).**
- Mobile app and push to phone; multi-machine; cloud sync; team sharing.
- Kanban board; issue-tracker intake.
- PR/CI lifecycle and merging (Agent Orchestrator already does this; Vajra Desktop complements it).
- Non-Claude agents; editing code in-app.
- Replacing Vajra hooks, workflows, agents or campaigns.
- Writing under `~/.claude` (except through Claude Code and Vajra's own scripts).
- A plugin API; Windows and Linux; auto-approve of any kind.

**Dependencies.** Claude Code CLI; Vajra installed with hooks registered; git; python3; jq; openssl. New Vajra pieces: `hooks/desktop-control.sh`, extended `hooks.json` matcher (strengthens the guard's audit coverage, does not loosen it).

**Assumptions.** See Section 0.

## 5. Non-functional requirements

All performance numbers are PROVISIONAL (A6) until the stack spike. Each is measured on an M-series Mac.

### Performance
- NFR-PERF-1 PROVISIONAL: idle RSS (app, no sessions), summed over all app processes, <= 150 MB.
- NFR-PERF-2 PROVISIONAL: per attached agent stream <= 15 MB RSS; ring buffer 2 MB per stream, then spill to disk.
- NFR-PERF-3 PROVISIONAL: with 10 agents streaming 200 lines/s each, UI frame time p95 <= 16.7 ms and p99 <= 33 ms; lists and terminals virtualized.
- NFR-PERF-4 PROVISIONAL: cold start to interactive <= 1.0 s.
- NFR-PERF-5: the event stream uses bounded queues; a slow consumer is dropped to a snapshot; a PTY reader is never blocked by UI or IPC (test: stall the consumer, PTY read latency unchanged).
- NFR-PERF-6 PROVISIONAL: 1 h soak under NFR-PERF-3 load, RSS growth <= 5%.
- NFR-PERF-7: measured against at least two installed competitors (Agent Orchestrator desktop and Orca), same machine, same method, recorded in a benchmark note; the app is lower on idle RSS and on RSS per attached stream.
- NFR-PERF-8 PROVISIONAL: the control hook (`hooks/desktop-control.sh`) adds <= 10 ms p95 per tool call when the app has no queue for that session.

### Security
- NFR-SEC-1: respect the guard (`hooks/pre-tool-use.sh`); never weaken it or the sandbox.
- NFR-SEC-2: never read `~/.claude/vajra/.hmac-key`; signature checks go through `vajra-campaign.sh verify`.
- NFR-SEC-3: all agent output is untrusted; rendered as text; no HTML or escape-sequence execution.
- NFR-SEC-4: no shell interpolation of agent or user text; arguments are passed as argv.
- NFR-SEC-5: IPC is owner-only (Unix socket mode 0600); no network listener.
- NFR-SEC-6: filesystem reads follow an explicit path allowlist; no recursive watch on `~/.claude/vajra`.
- NFR-SEC-7: release builds are signed and notarized; updates are signed; dependencies are pinned and checked (lockfile audit, SBOM).
- NFR-SEC-8: control messages are never approval for protected actions.

### Accessibility
- NFR-A11Y-1: WCAG 2.2 AA (contrast, target size, focus visible, no color-only status).
- NFR-A11Y-2: every job J1..J6 completes keyboard-only.
- NFR-A11Y-3: VoiceOver reads tree nodes, status, action cards and approvals with correct roles and names.

### UX
- NFR-UX-1: no UI action blocks the interface for more than 100 ms (long work is async).
- NFR-UX-2: visible feedback within 100 ms of any action; a skeleton appears only after 300 ms.
- NFR-UX-3: no surprises: every background action (launch, control, write-back call) is visible in the activity feed.

### Observability
- NFR-OBS-1: an app log with rotation (size and count bounded) and no agent text or secrets.
- NFR-OBS-2: the control ledger is append-only JSONL, one line per control action, with action id.

## 6. Success metrics

| Metric | Target |
|---|---|
| Time to find what a worker is stuck on (usability test, 5+ participants) | under 10 s |
| Hidden control actions (ledger vs `audit.log` reconciliation, AC-3.4.3) | 0 |
| Provisional budgets (NFR-PERF-1..7) | all met, or budget revised with measured evidence |
| Crash-free sessions in beta | >= 99.5% |
| Vajra user completes J1..J6 keyboard-only | yes, in usability test |
| Mahabharat names outside the inspector engine id field | 0 (automated string scan of UI) |

## 7. Traceability

| Story | Criteria | NFRs |
|---|---|---|
| US-1.1 | AC-1.1.1 .. 1.1.4 | SEC-1, SEC-4, UX-2, UX-3 |
| US-1.2 | AC-1.2.1 .. 1.2.3 | SEC-2, SEC-4, UX-2 |
| US-1.3 | AC-1.3.1 .. 1.3.4 | SEC-2, UX-1, A11Y-1 |
| US-2.1 | AC-2.1.1 .. 2.1.3 | PERF-3, A11Y-1, A11Y-3 |
| US-2.2 | AC-2.2.1 .. 2.2.4 | PERF-2, PERF-3, PERF-5, PERF-6, SEC-3 |
| US-2.3 | AC-2.3.1 .. 2.3.3 | PERF-3, SEC-3, SEC-6 |
| US-2.4 | AC-2.4.1 .. 2.4.3 | A11Y-2, UX-1, UX-2 |
| US-3.1 | AC-3.1.1 .. 3.1.3 | SEC-3, SEC-5, OBS-2 |
| US-3.2 | AC-3.2.1 .. 3.2.5 | SEC-1, SEC-3, SEC-4, SEC-8, OBS-2 |
| US-3.3 | AC-3.3.1 .. 3.3.3 | SEC-8, UX-3, OBS-2 |
| US-3.4 | AC-3.4.1 .. 3.4.4 | SEC-1, SEC-5, OBS-1, OBS-2, UX-3 |
| US-4.1 | AC-4.1.1 .. 4.1.2 | A11Y-1, UX-2 |
| US-4.2 | AC-4.2.1 .. 4.2.3 | PERF-3, A11Y-2 |
| US-4.3 | AC-4.3.1 .. 4.3.4 | SEC-2, SEC-3 |
| US-5.1 | AC-5.1.1 .. 5.1.3 | A11Y-3, UX-3 |
| US-5.2 | AC-5.2.1 .. 5.2.4 | SEC-1, SEC-8, OBS-2, UX-3 |
| US-5.3 | AC-5.3.1 .. 5.3.3 | SEC-4, OBS-2 |
| US-6.1 | AC-6.1.1 .. 6.1.3 | SEC-2, A11Y-2 |
| US-6.2 | AC-6.2.1 .. 6.2.3 | SEC-4, SEC-6 |

Cross-cutting: NFR-PERF-1, 4, 7, NFR-SEC-5, 7 and NFR-OBS-1 apply to the whole app and are verified in the benchmark and release checklists, not per story.

## 7a. Pain points from user research (pending)

Filled in once the competitor pain-point research (`pain-points.md`) lands. One row per pain point: the
accepted ones point to a requirement and an acceptance id (new ids continue the existing series, for
example AC-2.2.5 or NFR-UX-4); the rest say "won't do" with the reason.

| Pain point (source) | Decision | Requirement | Acceptance id |
|---|---|---|---|
| (pending) | | | |

## 7b. Review findings (pending)

Rows from the architecture review, the pre-mortem, the STRIDE threat model and the ponytail review of this
spec, in the same shape: finding, decision, requirement, acceptance id.

| Finding (review) | Decision | Requirement | Acceptance id |
|---|---|---|---|
| (pending) | | | |

## 8. Decisions requested from the owner

1. Confirm A1..A9 (especially A3 transcript streams and A5 hook identification, which the spike must prove).
2. Decide whether Vajra should persist delivery reports in the campaign (A8).
3. Decide whether renaming engine agents is wanted later (A2).
