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

### Positioning requirements (POS)

From `positioning.md` (vajra-5, first pass, 2026-11-05). They make the three surviving reasons testable. POS
ids keep vajra-5's numbering. Where a POS item met the existing design, the resolution is in its last
sentence. POS-1.4, 1.6 and 3.3 are measurements: they gate claims, not the build.

**POS-R1 Evidence per criterion is the centre of the app** (reason 1)
- POS-1.1 GIVEN a finished delivery with N criteria, WHEN I open it from the run list, THEN the evidence view
  is the default view of that run and, within one click, each criterion shows a status (pass, fail, not run,
  stale, unverified), the check command, its exit code, an output excerpt linking to the full log, the diff,
  and the Verifier role next to the author role. If the two are the same role, the row says so.
- POS-1.2 GIVEN a criterion with recorded evidence, WHEN I choose Re-run and confirm the exact command shown,
  THEN the app runs it through `scripts/vajra-verify-work.sh` in a fresh worktree detached at the evidence
  commit (the task's own worktree is usually cleaned up), with the command passed as one argument, and shows
  whether the new result matches the recorded one. The recorded command came from an agent, so it is
  untrusted: it never runs without this confirm, and the Lead and MCP tools cannot start a re-run (NFR-SEC-9).
- POS-1.3 GIVEN evidence whose HMAC fails, or whose `git.head` differs from the branch head now, WHEN the
  delivery is shown, THEN the criterion shows "unverified" or "stale", never pass. The check goes through
  engine change E3 (`vajra_tasks.py check-evidence --head`); the app never reads the key.
- POS-1.4 (measurement) GIVEN a seeded benchmark of at least 20 tasks with planted defects where the author
  claims done, WHEN each runs through `/vajra`, THEN the report states how many false "done" claims were
  caught and how many slipped through. No catch-rate claim ships without this number.
- POS-1.5 GIVEN a criterion still failing after the last fix round, WHEN the delivery ends, THEN the run shows
  "not done" with the failing criteria listed first, and no aggregate green badge appears anywhere.
- POS-1.6 (measurement) GIVEN 5 or more developers reviewing the same delivery once as transcript plus diff
  and once in the evidence view, WHEN they decide merge or reject, THEN time to decision and correctness are
  recorded for each.

**POS-R2 Claude Code-native; the app is optional; files are the source of truth** (reason 2)
- POS-2.1 (MVP scope per ADR-9) GIVEN a run started from the app, WHEN I close its last window, THEN the run
  keeps going, a menu bar item shows the number of active runs, and reopening the window shows the run with
  no lost events.
- POS-2.1a GIVEN active runs, WHEN I press Cmd-Q, THEN a dialog asks "Quit and stop N runs?" with Keep
  running as the default; choosing Keep running closes the window and the runs continue.
- POS-2.1b GIVEN a run that was running when the app quit or crashed, WHEN the app next starts, THEN it is
  offered for resume (`claude --resume <session_id>` plus `/vajra continue`). A session I stopped or quit on
  purpose is not offered, and nothing resumes without a click (VD-5).
- POS-2.1c GIVEN an update is ready and runs are active, WHEN it would install, THEN it waits until no run is
  active, and exactly one app process exists afterwards (VD-6).
- Deferred to v1.1 (ADR-9): runs that keep going through a quit or SIGKILL of the app (VD-4).
- POS-2.2 GIVEN `/vajra <ask>` started in a plain terminal, WHEN the app is open, THEN the run appears live
  within the event latency budget (NFR-PERF-9), with the crew, run view and inbox. The app can't take over
  that terminal; the control channel works if the control hook is installed.
- POS-2.3 GIVEN the app is uninstalled, WHEN I list `~/.claude/vajra/`, THEN every campaign, evidence file
  and ledger record is still there, readable with the CLI. The app's own files (run index, control ledger,
  report copies) are plain JSONL and markdown in its data directory. This strengthens Q3 in the PRD
  (persist the delivery report in the campaign, engine change E4).
- POS-2.4 GIVEN my Claude Code settings (permissions, hooks, sandbox), WHEN the app starts any session, THEN
  that session uses exactly those settings. The app never passes a flag that widens permissions or disables
  the sandbox (no `--dangerously-skip-permissions`, no permissive `--permission-mode`); the Lead chat adds
  only the app's control tools (design.md section 5.2).

**POS-R3 Knowledge is earned and visible** (reason 3, weakest)
- POS-3.1 GIVEN a run that used ledger records, WHEN I open it, THEN it lists each record with its tier and
  source (a run id or "you"). Approve, reject and forget stay Vajra's human gates: the app opens a visible
  terminal pane with the `vajra_kb.py` command filled in and I run it myself. The app never sends the
  keystroke (SECURITY.md: a process that owns a PTY can pass a TTY check, so that has to stay a human act).
- POS-3.2 GIVEN the knowledge write gate refused a record during a run, WHEN I open the run, THEN the refusal
  (reason, no refused text) shows in the run's activity, read from the ledger's `audit.jsonl`.
- POS-3.3 (measurement) GIVEN the same class of task run twice in one repo, WHEN run with and without the
  ledger, THEN fix rounds and tokens per delivery are reported. Reason 3 drops to a supporting point if
  there is no improvement.

**POS-R4 The memory claim is earned or dropped** (pending the stack spike)
- POS-4.1 GIVEN 10 stub agents streaming at the spike's rate, WHEN measured by the spike's written method,
  THEN the app's own RSS (claude processes excluded, and also reported with them included) is within
  NFR-PERF-1 and NFR-PERF-2 and is reported next to at least two installed competitors (Orca, Agent
  Orchestrator). The word "lighter" is allowed only against competitors it actually beats.

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


### Performance

Gates from the stack spike (`vajra-2/stack-spike.md`), reviewed by both models (Opus review: `vajra-3/stack-spike-review.md`). Definition: app RSS = app, WebKit helpers and app-owned PTY shells, summed; `claude` and agent processes reported separately; `footprint` reported next to RSS. "Spike" is the measured value on 2026-11-05 (Apple M3 Max, busy machine, summed RSS); the gate holds until a quiet-machine rerun replaces it (H1).

- NFR-PERF-1: Idle app RSS (no run attached): ceiling 200 MiB, preferred 190 MiB; aspirational 171 MiB (15% under Runner). Spike: 220.2 MiB (Tauri spike, 1 PTY); **not met**.
- NFR-PERF-2: Each attached worker stream adds at most 8 MiB app RSS after warm-up; retained tail at most 256 KiB per worker, then spill to disk with an explicit gap marker. Spike: 7.8 MiB amortized (not a marginal measurement); marginal curve 0, 1, 2, 10 still to run.
- NFR-PERF-3: 10 workers streaming: app RSS at most 400 MiB; rAF p95 at most 20 ms and p99 at most 34 ms at 60 Hz; renderer work p95 at most 8 ms per frame; worker switch to paint at most 100 ms p95. Spike: 328.9 MiB at 60 min; rAF p95 20.0 ms; renderer work and switch latency not measured.
- NFR-PERF-4: Cold start to usable crew view p95 at most 800 ms (process-cold, warm filesystem; cold boot reported separately). Spike: 394.3 ms median of 3 (not a p95).
- NFR-PERF-5: Bounded byte queues at every hop; a slow UI can't grow backend memory; at most one in-flight UI snapshot of at most 64 KiB; slow disk pauses the source or fails visibly; a PTY or file reader is never blocked by the UI. Spike: met in the spike's design; the spike's blocking writes must not be reused (O6).
- NFR-PERF-6: Soak: 1 h with 10 live workers, GUI RSS growth from minute 5 to 60 at most 20 MiB and no sustained trend above 1 MiB/min; then an 8 h soak before release. Spike: 31.1 MiB growth (WebContent); **not met**.
- NFR-PERF-7: Every gate is run serially on a quiet machine against at least two installed competitors with the same fixture, and published with its method; "lighter" is claimed only against apps beaten on that method. Spike: Electron, Orca beaten; Runner beats us at idle.
- NFR-PERF-8: The control hook adds at most 10 ms p95 per tool call when the app has no queue for that session. Spike: not measured.
- NFR-PERF-9: A run started outside the app appears within 2 s; new transcript or campaign events reach the UI within 250 ms p95; control receipt within 100 ms p95. Spike: not measured.
- NFR-PERF-10: Compressed installer at most 20 MiB; installed app at most 50 MiB (arm64 and universal measured separately). Spike: 3.0 MiB DMG (unsigned, arm64).

### Security
- NFR-SEC-1: respect the guard (`hooks/pre-tool-use.sh`); never weaken it or the sandbox.
- NFR-SEC-2: never read `~/.claude/vajra/.hmac-key`; signature checks go through `vajra-campaign.sh verify`.
- NFR-SEC-3: all agent output is untrusted; rendered as text; no HTML or escape-sequence execution.
- NFR-SEC-4: no shell interpolation of agent or user text; arguments are passed as argv.
- NFR-SEC-5: IPC is owner-only (Unix socket mode 0600); no network listener.
- NFR-SEC-6: filesystem reads follow an explicit path allowlist; no recursive watch on `~/.claude/vajra`.
- NFR-SEC-7: release builds are signed and notarized; updates are signed; dependencies are pinned and checked (lockfile audit, SBOM).
- NFR-SEC-9: the app never runs a command that came from agent output (a recorded check, a plan step) without the human confirming the exact text; the Lead and MCP tools can't trigger one.
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

### Positioning traceability

| Reason | POS ids | Stories it shapes | Tasks |
|---|---|---|---|
| 1 Proof per criterion | POS-1.1, 1.2, 1.3, 1.5 | US-4.3, US-6.1 | S4, E3, C4 |
| 1 (measurement) | POS-1.4, 1.6 | none (claim gates) | H7 |
| 2 Claude Code-native | POS-2.1, 2.2, 2.3, 2.4 | US-1.1, US-1.2, US-2.1 | G1, C3, C5, C4 |
| 3 Earned knowledge | POS-3.1, 3.2 | US-6.2 | S6, C4 |
| 3 (measurement) | POS-3.3 | none (claim gate) | H7 |
| (4) Memory, pending | POS-4.1 | none | G1, H1 |

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
