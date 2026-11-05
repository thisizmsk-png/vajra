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
- AC-1.1.5 (P24, billing.md) GIVEN 10 `claude` sessions requested together, WHEN they run, THEN each is the stock binary handling its own login; the app never reads, copies, refreshes or shares tokens (no access by the app's own process to `~/.claude/.credentials*`, the keychain item, `CLAUDE_CODE_OAUTH_TOKEN` or `~/.codex/auth.json`, checked by an fs and process trace; the stock `claude` and `codex` children, including `codex login status`, are the named exceptions), and sessions start at least 2 s apart, a mitigation for refresh races at start; later refreshes stay the binary's own concern.
- AC-1.1.6 (billing.md) GIVEN any agent row, WHEN it is shown, THEN it reads its billing source as "plan login" or "API key" (Claude Code: `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` or `apiKeyHelper` present in that session's environment or settings; Codex: `codex login status`), API key wins when both apply, and the app learns only presence, never a value: settings files are parsed for key names and values are dropped at parse time. Test: a canary key value in the environment and in a settings `env` block never appears in app logs, IPC payloads or a heap dump.
- AC-1.1.7 (billing.md) GIVEN any launch, WHEN argv and the child environment are inspected, THEN the binary is the user's stock `claude` or `codex`, run in a PTY, with no `--bare`, no `-p`, no Agent SDK and no `ANTHROPIC_BASE_URL` added by the app; a stray API key inherited from my shell is shown in preflight before the first start, and it is passed to agent children only after I acknowledge it once for that repo (that acknowledgement is API-key mode); otherwise it is removed from the child environment.

**US-1.2** As a Dev, I want to resume a campaign, so that I continue unfinished work (by campaign slug through `/vajra-deliver {"campaign":"<slug>"}`, never `/vajra continue`, ADR-9).
- AC-1.2.1 GIVEN the campaign list shows campaigns found under `~/.claude/vajra/campaigns/`, WHEN I choose one and press "Resume", THEN the app runs `vajra-campaign.sh verify <slug>` first and, on exit 0, launches the run session with `campaign: <slug>` and a fresh `run_id`.
- AC-1.2.2 GIVEN `vajra-campaign.sh verify <slug>` exits 3, WHEN I press "Resume", THEN resume is blocked, no run session starts, and the app shows "Integrity check failed" with the restore command (`vajra-campaign.sh resume <slug> --from-checkpoint N`, N being the latest checkpoint listed).
- AC-1.2.3 GIVEN a resumable campaign, WHEN I open the resume dialog, THEN it shows slug, goal, features passing as N/M, and the last integration branch recorded in progress; an optional "base" field overrides it.

**US-1.3** As a Dev, I want a preflight check, so that I know why a run cannot start before it fails.
- AC-1.3.1 GIVEN the app opens, WHEN preflight runs, THEN rows show: `claude` on PATH (with version), `vajra doctor` PASS or FAIL, hooks registered (yes/no), sandbox state (on/off/unknown), git, python3, jq, openssl present.
- AC-1.3.2 GIVEN any required row fails, WHEN I look at the Start screen, THEN "Start run" is disabled and the failing row shows one fix hint (a command or link).
- AC-1.3.3 GIVEN preflight, WHEN it executes, THEN it only runs read-only commands and never reads `~/.claude/vajra/.hmac-key` (verified: no open of that path in the process trace).
- AC-1.3.4 GIVEN `~/.claude/workflows/vajra-deliver.js` is missing, WHEN preflight runs, THEN the row says so and shows `bash ~/.claude/skills/vajra/scripts/install.sh` followed by `/vajra doctor`.
- AC-1.3.5 (VD-27) GIVEN the control hook is not registered, WHEN preflight runs, THEN it names the missing piece and shows the exact command to run in my own terminal; the app never edits `~/.claude/settings.json` or any agent config itself (a file hash before and after is identical), and control actions show "control hook not installed" instead of failing silently.
- AC-1.3.6 (VD-12, VD-31) GIVEN a fresh install with the network blocked, WHEN I start a run and use every screen, THEN everything works and no login or account screen appears; with the network open for an hour under a network monitor, the only outbound connection the app makes is the update check (Claude Code's own traffic excluded).
- AC-1.3.7 (VD-28) GIVEN a Mac with Claude Code and Vajra installed, WHEN I open the signed DMG and launch the app, THEN I reach Home with a passing preflight in under 2 minutes, the app needs no Node or Python of its own, and it writes nothing outside `~/Library/Application Support/<app>/` and the repos I pick.

**US-1.4** (owner decision, 2026-11-05) As a Dev leaving Conductor, I want my repos, worktrees and sessions found, so that I keep working in Vajra Desktop without redoing setup.
- AC-1.4.1 GIVEN Conductor data on this Mac, WHEN I choose "Import from Conductor", THEN the app lists the repos, worktrees (from `git worktree list` in each repo) and Claude Code sessions (from transcripts whose `cwd` is one of those worktrees) it found, each with its path, branch and last activity, and selects none by default.
- AC-1.4.2 GIVEN the discovery list, WHEN I confirm items one by one (or select several and confirm once, with the list shown), THEN only the confirmed items are added to the app's own `runs.jsonl` and repo list; nothing else changes.
- AC-1.4.3 GIVEN any import, WHEN it finishes or I cancel, THEN no file or worktree is deleted, moved, renamed or written outside the app data dir, and no git command other than read-only ones ran (fs and process trace).
- AC-1.4.4 GIVEN Conductor's on-disk layout is unknown or changed, WHEN discovery runs, THEN it shows "Conductor data not recognised" and offers to add a repo by path, instead of guessing.

**US-1.5** (owner decision, 2026-11-05) As a Dev, I want to run Codex CLI agents next to Claude Code ones, so that I'm not locked into one vendor.
- AC-1.5.1 GIVEN the agent-adapter interface (spawn, stream, PTY attach, inspect, message, stop, resume, plus capability flags), WHEN the Claude Code and Codex CLI adapters run the shared conformance suite, THEN every flag set to true passes its test and every flag set to false is hidden in the UI.
- AC-1.5.2 GIVEN a Codex CLI worker, WHEN I open its row, THEN only the actions its flags support appear (no greyed-out Message or Stop that can't work), and the inspector names the adapter and its flags.
- AC-1.5.3 GIVEN an unsupported action requested through the Lead or MCP, WHEN it reaches the control service, THEN it returns `unsupported_by_adapter` and nothing is queued.

### J2 Crew

**US-2.1** As a Dev, I want a crew tree with status, so that I see who is doing what at a glance.
- AC-2.1.1 GIVEN a run is active, WHEN I open Crew view, THEN the tree shows Lead -> Run -> agents (one node per task owner per round) -> sub-workers (verifier and reviewer passes, fix-round reruns), each node with role name, a status dot AND a text status (pending, ready, claimed, in_progress, done, blocked, question, failed).
- AC-2.1.2 GIVEN `tasks.json` status of a task changes, WHEN the file changes, THEN the node's status updates within 2 s without user action.
- AC-2.1.3 GIVEN an agent id not in the mapping, WHEN it is rendered, THEN it shows the id title-cased, and no mahabharat name appears anywhere in the UI except the inspector's "engine id" field.
- AC-2.1.4 (VD-1) GIVEN a run with 10 workers, WHEN the crew view is open, THEN the status bar shows the app's own memory and the run node shows the memory of its `claude` process tree, labelled separately; the app never shows one combined number. (Workers are subagents inside one `claude` process, so per-worker memory is not available and is not invented.)
- AC-2.1.5 (VD-8) GIVEN a worker the app has no signal source for (no transcript found, no task-graph entry, no hook event yet) while its run's `claude` process is alive, WHEN its row renders, THEN its status reads "Unknown (no signal)", never Idle, Done, Exited or Thinking.
- AC-2.1.6 (VD-11) GIVEN the run's transcript records a usage-limit stop, WHEN the run node renders, THEN it shows "Rate-limited until HH:MM" (or "Rate-limited" when no time is given), not Idle or Stuck, and the inbox lists it once as "Paused by usage limit". Detection is to verify in the M2 smoke run; until then the state is Unknown.
- AC-2.1.7 (P4 stall detector, 11 sources) GIVEN a worker whose transcript is being tailed, WHEN it produces no new output and no tool call for N seconds (default 90, settable 30 to 600) while its process is alive and it isn't waiting on a permission prompt, THEN its row reads "Stalled 3m 12s" with its last output line (sanitized, one line) and the time of its last tool call, never "Thinking"; any new record clears it at once, and a stall longer than 5 minutes also appears in the needs-you inbox. Status comes only from `tasks.json`, transcript records and hook events, never from parsing terminal text.
- AC-2.1.8 (VD-19, P11, 9 sources) GIVEN worktrees left by runs, WHEN I open the worktree list, THEN each shows how its session ended (done, crashed or abandoned), its age and disk size, and retention follows the end state, not age alone: done is removed by the engine's cleanup as today; crashed is kept for resume and flagged after 7 days; abandoned (no live session, not resumable, untouched for 14 days) is offered for cleanup. Nothing is removed without my confirm, and a worktree with tracked changes or unpushed commits is never offered for removal.

**US-2.2** As a Dev, I want a worker's live stream, so that I can read what it is doing.
- AC-2.2.1 GIVEN a worker node is selected, WHEN the Stream tab is shown, THEN lines from its transcript appear within 1 s of being written, rendered as plain text (no HTML, no escape sequences executed).
- AC-2.2.2 GIVEN a stream exceeds its ring buffer, WHEN more lines arrive, THEN the oldest lines spill to disk and remain reachable by scrolling, and app memory per attached stream stays within NFR-PERF-2.
- AC-2.2.3 GIVEN a worker is not selected, WHEN it produces output, THEN no UI tab is rendered for it but its buffer keeps receiving data and the node shows an "activity" indicator.
- AC-2.2.4 GIVEN the run session PTY is shown, WHEN I type in it without pressing "Take over", THEN no input is sent; "Take over" is an explicit button and the pane shows "You are controlling the run session" while active.
- AC-2.2.5 (VD-7) GIVEN I took over the run terminal, WHEN I paste more than 64 KiB or a binary blob, THEN the app asks before sending, sends it in bounded chunks, and the UI stays within NFR-PERF-3.
- AC-2.2.6 (VD-23) GIVEN a stream or terminal receiving output, WHEN I select text and press Cmd-C, THEN the selection holds (the view does not snap to the bottom) and the copy works.

**US-2.3** As a Dev, I want a worker's diff, worktree and current task, so that I can judge its progress.
- AC-2.3.1 GIVEN a worker with a worktree, WHEN I open the Diff tab, THEN it shows `git diff` of that worktree against its base, read-only, with no edit control.
- AC-2.3.2 GIVEN a worker, WHEN I look at its header, THEN it shows role name, status, current task id and title, worktree path and branch, each copyable.
- AC-2.3.3 GIVEN the Task tab, WHEN opened, THEN it shows owner, deps, files, acceptance, attempts and last report from `tasks.json`; the inspector shows the engine id.

**US-2.4** As a Dev, I want to hop between workers by keyboard and palette, so that I never need the mouse.
- AC-2.4.1 GIVEN Crew view, WHEN I press Cmd-K and type a role or task name, THEN the palette lists matching workers and Enter focuses that worker's pane.
- AC-2.4.2 GIVEN focus is in the tree, WHEN I press Up/Down/Left/Right, THEN focus moves between nodes and the Worker pane follows within 100 ms.
- AC-2.4.3 GIVEN any worker, WHEN I use only the keyboard, THEN I can switch tabs (Stream, Diff, Task, Worktree) and return focus to the tree.
- AC-2.4.4 (VD-3) GIVEN 10 workers with one visible, WHEN the other 9 have been hidden for 60 s, THEN they hold no rendered view or terminal grid (only their bounded tail), and reopening one paints within 100 ms p95.

### J3 Lead chat and control

**US-3.1** As a Dev, I want to ask the Lead about a worker, so that I get an answer without reading its stream.
- AC-3.1.1 GIVEN the chat panel, WHEN I ask "what is Backend stuck on?", THEN the Lead calls `list_workers` / `inspect_worker` and its reply cites the worker, task id and the last transcript evidence; each call appears as an action card.
- AC-3.1.2 GIVEN the Lead chat session, WHEN it runs, THEN it is a separate Claude Code session from the run session; a chat message never appears in any worker's context (verified: absent from every worker transcript).
- AC-3.1.3 GIVEN a read-only question, WHEN the Lead answers, THEN no entry is added to any worker queue.
- AC-3.1.4 (STRIDE E4) GIVEN the Lead chat session, WHEN it starts, THEN its tool list is exactly the read tools plus the five control tools; a test fails on any other tool (for example Bash, Write, Agent or Workflow).

**US-3.2** As a Dev, I want to message or redirect a worker (for example "use the kit Button" to Frontend), so that I steer it without typing into the run session.
- AC-3.2.1 GIVEN a worker is running, WHEN I send a message to it (via Lead or the worker's "Message" control), THEN nothing is written to any PTY (verified: zero bytes to the run PTY from the control path).
- AC-3.2.2 GIVEN a queued message for an Execute-phase task worker, WHEN that worker makes its next tool call, THEN the message appears in its context as `<vajra-steer nonce=N source="human"|"lead-via-desktop">`, the nonce appears in no app file except `control/<sid>/nonce`, and the worker's next action reflects it (L4 live check). Messages to verify, review or merge steps are refused with "control disabled for the verification gate".
- AC-3.2.3 GIVEN a message was sent, WHEN I check the four places, THEN it appears (a) as an action card in the chat, (b) on the worker's activity feed, (c) as a line in the control ledger, (d) in Vajra's `audit.log` as an ALLOW or BLOCK line for the control tool.
- AC-3.2.4 GIVEN a message mentions "approve" or "push", WHEN delivered, THEN no protected action is unlocked; an attempted push still needs Claude Code's permission prompt (control messages are never approval).
- AC-3.2.5 GIVEN the worker finishes before its next tool call, WHEN the message is unconsumed, THEN the card shows "not delivered" and the message is not applied later to a different worker.
- AC-3.2.6 (VD-26) GIVEN the control hook is installed and no message is queued, WHEN a session makes 100 tool calls, THEN the hook adds no text to the transcript; with messages queued, only the delivered steer text is added.
- AC-3.2.7 (GPT-2) GIVEN a run session starts or resumes, WHEN the Control service creates its nonce, THEN it is at least 128 bits from the OS CSPRNG, written to `control/<sid>/nonce` with mode 0600, owner-only and no-follow, rotated on every resume, and absent from queue entries, the ledger, acks, `audit.log`, `runs.jsonl`, app logs, stderr and MCP results (tests `nonce_is_os_csprng_128`, `nonce_file_0600_nofollow_excl`, `rotate_on_resume`, `no_nonce_in_outputs`).
- AC-3.2.8 (GPT-2) GIVEN the nonce file is missing, not 0600, owned by another user or a symlink, WHEN a message is queued, THEN nothing is delivered and the action shows "not delivered: control channel not ready" (test `nonce_source_only_file`).
- AC-3.2.9 (GPT-2) GIVEN a tool result contains a `vajra-steer` tag (any nonce), WHEN PostToolUse runs, THEN the worker gets a warning that the tag came from tool output, no steer is delivered from it, and `audit.log` records a WARN (test `forged_tag_in_tool_response`).
- AC-3.2.10 (GPT-2) GIVEN message text that tries to close the tag or carries attribute-like text, WHEN it is delivered, THEN it is escaped so the delivered tag stays one tag with enum `kind` and `source` and a ULID `id`; malformed entries are refused (tests `text_cannot_close_tag`, `bad_attrs_refused`).
- AC-3.2.11 (GPT-2) GIVEN a worker tool input that contains the nonce, WHEN PreToolUse runs, THEN the call is blocked and the reason does not contain the nonce (test `pretool_blocks_nonce_echo`).

**US-3.3** As a Dev, I want to stop a worker, so that I can halt a wrong direction.
- AC-3.3.1 GIVEN a worker, WHEN I press "Stop", THEN a confirm dialog names the worker and task; Cancel sends nothing.
- AC-3.3.2 GIVEN I confirm, WHEN the worker makes its next tool call, THEN the call is blocked with reason "stopped by the human" and the worker node shows "stopped by the human".
- AC-3.3.3 GIVEN Lead asks to stop a worker, WHEN requested, THEN the same human confirm dialog appears; the Lead cannot confirm on my behalf.
- AC-3.3.4 (GPT-1, F2) GIVEN a stopped worker, WHEN it calls `SubagentHandback`, THEN that call goes through and the parent gets a "stopped" report; any other tool name (including prefixes or look-alikes) is blocked.
- AC-3.3.5 (GPT-1, F2) GIVEN a stopped worker that handed back, WHEN its parent resumes the same agent id with `SendMessage`, THEN its next tool call is still blocked, and no queued message is delivered to it, on handback or after.
- AC-3.3.6 (GPT-1) GIVEN a stopped worker, WHEN I choose Resume worker and confirm, THEN the stop clears (ledger record with actor and time), and the same agent id continues; nothing else (transcript end, run end, the Lead) clears a stop.
- AC-3.3.7 (GPT-1) GIVEN a stopped worker reports `done`, WHEN the run view renders, THEN the task shows "stopped by the human", not done.

**US-3.4** As a Dev, I want every control action visible and doable directly, so that I trust the Lead.
- AC-3.4.1 GIVEN any control action (list, inspect, message, redirect, stop), WHEN executed by the Lead or by me, THEN it appears in the activity feed and in the control ledger (append-only JSONL) with an action id; message, redirect and stop also appear in `audit.log`: the hook's delivery or stop line carries the same action id, and a Lead tool call is logged by the guard under its MCP tool name (design.md D5: the guard's own lines carry no id, and reads I make in the UI are ledger-only).
- AC-3.4.2 GIVEN the Lead is unavailable or its session has ended, WHEN I use the worker's Message, Redirect and Stop controls, THEN each works without the Lead.
- AC-3.4.3 GIVEN a reconciliation script compares ledger action ids to `audit.log`, WHEN run after a session, THEN zero ledger entries lack an audit line and zero control audit lines lack a ledger entry.
- AC-3.4.4 GIVEN the control queue directory, WHEN a worker tries to write to it, THEN the guard blocks it and `audit.log` records BLOCK.
- AC-3.4.5 (STRIDE R1) GIVEN any human action (approve, reject, take over, re-run, resume worker, clear stop), WHEN it happens, THEN the control ledger has one record with the actor, the exact target and the time.
- AC-3.4.6 (GPT-3) GIVEN a worker tool call (payload has `agent_id`), WHEN it reads `VajraDesktop/control/<sid>/nonce` or any path under `~/.claude/projects/` through Read, Grep, Glob, LS, read-only Bash (`cat`, `head`, `python3 -c`), a symlink or a recursive Grep over an ancestor, THEN the guard blocks it and `audit.log` records BLOCK; the same reads from the main session or the Lead chat (no `agent_id`) are allowed.
- AC-3.4.7 (GPT-3) GIVEN a worker, WHEN it tries to write anywhere under `~/Library/Application Support/VajraDesktop/` with Write, Edit, `mv`, `rm`, `cp`, `tee` or a redirect (path with a space), THEN the guard blocks it.
- AC-3.4.8 (GPT-3, Q10) GIVEN preflight, WHEN it runs, THEN it shows whether the sandbox `denyWrite` for the control dir is active; when it isn't, the control row reads "stop only (sandbox off)", and message and redirect, from the UI,
  the Lead or MCP, return `control_disabled: sandbox_off` with nothing queued, while stop still works (Q10
  working assumption; test with the sandbox off and on). Every existing guard test still passes (CI).
- AC-3.4.9 (Q10) GIVEN the OS sandbox on with Vajra's profile applied, WHEN an agent writes to `VajraDesktop/control/` through a path the guard doesn't see (a fixture that bypasses the guard, run only in the test harness), THEN the OS denies the write and the queue is unchanged; with the sandbox off, the same fixture shows the guard as the only layer and control is stop-only (AC-3.4.8).
- AC-3.4.10 (Q10, VD-27) GIVEN first-run setup or `enable-sandbox.sh`, WHEN the user enables the sandbox, THEN the exact settings change (including the `denyWrite` entry) is shown and applied only after the user approves it, and `vajra-doctor.sh` then reports the rule as active.

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
- AC-5.1.4 (VD-9) GIVEN 3 items waiting on me, WHEN I open the inbox (Cmd-5), THEN they are listed oldest first with how long each has waited and the last 3 lines of context, and Enter opens the item's worker or plan.
- AC-5.1.5 (VD-24) GIVEN 5 workers finish turns and 1 waits for a permission prompt while the app is in the background, WHEN notifications fire, THEN exactly 1 notification appears (the permission wait); worker turn ends never notify; a finished or stuck run notifies once.
- AC-5.1.6 (STRIDE S2) GIVEN agent output that looks like an inbox item or an app notice, WHEN rendered, THEN it appears only inside that worker's stream as text; inbox items come only from the workflow result, task status or a permission wait.

**US-5.2** As a Reviewer-approver, I want to approve a plan, so that execution proceeds under my control.
- AC-5.2.1 GIVEN a pending plan, WHEN I open it, THEN the exact plan text (commands, rollback) is shown unabridged before the Approve button is enabled.
- AC-5.2.2 GIVEN I press Approve, WHEN submitted, THEN the app submits a visible human turn "Execute approved plan <id>" to the run session (the turn is visible in that terminal) and does not itself run git push, open a PR or deploy.
- AC-5.2.3 GIVEN the plan includes push, PR or deploy, WHEN the run session proceeds, THEN Claude Code's own permission prompt still appears for each protected command.
- AC-5.2.4 GIVEN approval, WHEN it happens, THEN a ledger entry and `audit.log` line record who, plan id and time.
- AC-5.2.5 (PM-5) GIVEN a pending plan whose run session has ended, WHEN I press Approve, THEN the app starts `claude --resume <sid> 'Execute approved plan <id>'` as argv in the repo (nothing typed into a PTY) and Claude Code's permission prompts appear there. The Cmd-Q dialog counts sessions holding pending approvals. For runs started in a plain terminal, Approve shows the exact turn to copy into that terminal.

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

From `positioning.md` (vajra-5, FINAL, 2026-11-05). They make the surviving reasons testable (1 proof per
criterion, 2 earned knowledge, weak), keep the table stakes (POS-2, POS-5) and gate the killed memory claim
(POS-4). POS
ids keep vajra-5's numbering. Where a POS item met the existing design, the resolution is in its last
sentence. POS-1.4, 1.6, 3.3 and 4.2 to 4.5 are measurements: they gate claims, not the build.

**POS-R1 Evidence per criterion is the centre of the app** (reason 1)
- POS-1.1 GIVEN a finished delivery with N criteria, WHEN I open it from the run list, THEN the evidence view
  is the default view of that run and, within one click, each criterion shows a status (pass, fail, not run,
  stale, unverified), the check command, its exit code, an output excerpt linking to the full log, the diff,
  and the Verifier role next to the author role. If the two are the same role, the row says so.
- POS-1.2 GIVEN a criterion with recorded evidence, WHEN I choose Re-run and confirm the full text shown monospaced and untruncated (STRIDE E2),
  THEN in a fresh worktree detached at the evidence commit (the task's own worktree is usually cleaned up)
  the app first rebuilds the verified tree: it applies the evidence's `diff.patch` (`git apply --binary`,
  untracked files included) and checks that the whole-tree id, computed as in POS-1.3, equals the signed
  `git.worktreeTree`; if the patch is missing, doesn't apply, or the id differs, Re-run is refused with "can't
  rebuild the verified tree" and nothing runs. Then it runs the delivery's recorded setup commands, in their recorded order, each as one `bash -c`
  argument, exactly as the Deliver write-back does (`workflows/vajra-deliver.js:787` and `:816`), then runs
  the criterion command through `scripts/vajra-verify-work.sh` as one argument, and shows whether the new
  result matches the recorded one. The confirm lists every setup command and the criterion command, in run
  order. The bundle and the setup list come only from the delivery's archived, signed copy (engine change E7,
  design 3.2): never the report's truncated `(setup)` lines, and never the verifier's unfiltered
  `setup_cmds`. A setup failure is reported as "setup failed at command k", not as a criterion failure.
  Without an archive the row shows "evidence not archived", and verification and Re-run are refused. Test: a fixture criterion that only passes
  after its setup (`npm ci` or a `.venv` build) matches its recorded pass on Re-run, and fails with the
  setup commands removed. A second test: signed evidence taken on a dirty tree (a tracked edit plus an
  untracked file) reproduces on Re-run, and with one byte of `diff.patch` changed it is refused. Also tested (E7): a completed delivery whose
  worktrees cleanup already removed still verifies and re-runs from its archive; a setup command longer than
  300 characters is shown and replayed byte for byte; a setup entry that write-back filtered out (a path
  outside the repo) is neither shown nor run. The recorded command came from an agent, so it is
  untrusted: it never runs without this confirm, and the Lead and MCP tools cannot start a re-run (NFR-SEC-9).
  Re-run runs as the user, outside Claude Code's guard and sandbox, so: the child gets an environment
  allowlist of exactly `PATH`, `HOME`, `USER`, `SHELL`, `LANG`, `LC_ALL`, `TERM` and `TMPDIR` (no API keys,
  tokens, `SSH_AUTH_SOCK` or cloud credentials; setup that needs a registry token such as `NPM_TOKEN` fails on
  Re-run by design, and the result says which variable was withheld if the output names it); the confirm renders control, bidi and zero-width characters
  visibly (for example `<U+202E>`) and says "these run as you, outside the sandbox, and may run package
  install scripts". Tests: a setup command with a bidi override shows it in the confirm; a check that prints
  `env` shows none of the denied variables.
- POS-1.3 (GPT-4) Two separate answers per criterion, never merged into one green:
  - **Proof at the recorded commit:** "verified" when E3 (`vajra_tasks.py check-evidence --head <recorded
    head>`) is green, and "unverified" when the HMAC or any green rule fails. The app never reads the key.
    This stays visible as history.
  - **Current:** "current" only when the evidence's `git.worktreeTree` equals the whole-tree id of the
    checked-out integration worktree now, computed the same way (`git add -A` into a throwaway
    `GIT_INDEX_FILE`, then `git write-tree`). That covers tracked edits, staged changes and untracked files,
    with ignored files excluded. Otherwise "stale", or "no worktree to compare" when that worktree is gone.
  - GIVEN a passing criterion, WHEN I (a) edit a tracked file without committing, (b) add an untracked file,
    or (c) move the branch to another commit while the evidence bytes are unchanged, THEN within 2 s of the
    file event the criterion shows "stale", never a current pass.
- POS-1.4 (measurement) GIVEN a seeded benchmark of at least 20 tasks with planted defects where the author
  claims done, WHEN each runs through `/vajra`, THEN the report states how many false "done" claims were
  caught and how many slipped through. No catch-rate claim ships without this number.
- POS-1.5 GIVEN a criterion still failing after the last fix round, WHEN the delivery ends, THEN the run shows
  "not done" with the failing criteria listed first, and no aggregate green badge appears anywhere.
- POS-1.6 (measurement) GIVEN 5 or more developers reviewing the same delivery once as transcript plus diff
  and once in the evidence view, WHEN they decide merge or reject, THEN time to decision and correctness are
  recorded for each.
- POS-1.7 GIVEN a running delivery, WHEN I look at a worker in the crew tree, THEN its status reads in terms of
  the work (task id, criterion, fix round "2 of 3"), not only process state; VD-8 still governs "unknown".

**POS-R2 Claude Code-native; the app is optional; files are the source of truth** (table stakes: killed as
a reason at why 4, since native agent view already does it, P23)
- POS-2.1 (MVP scope per ADR-9) GIVEN a run started from the app, WHEN I close its last window, THEN the run
  keeps going, a menu bar item shows the number of active runs, and reopening the window shows the run with
  no lost events.
- POS-2.1a GIVEN active runs, WHEN I press Cmd-Q, THEN a dialog asks "Quit and stop N runs?" with Keep
  running as the default; choosing Keep running closes the window and the runs continue.
- POS-2.1b GIVEN a run that was running when the app quit or crashed, WHEN the app next starts, THEN it is
  offered for resume by its own campaign slug: argv `claude --resume <session_id>
  '/vajra-deliver {"campaign":"<slug>","run_id":"<UTC now>"}'` with the slug from `runs.jsonl` (ADR-9
  decision 5), never `/vajra continue`, which follows the global `.active` pointer. A session I stopped or
  quit on purpose is not offered, and nothing resumes without a click (VD-5). Test: the two-campaign fixture
  of POS-2.1e, with argv asserted.
- POS-2.1c GIVEN an update is ready and runs are active, WHEN it would install, THEN it waits until no run is
  active, and exactly one app process exists afterwards (VD-6).
- POS-2.1d (PM-6) GIVEN the last window closes, WHEN 60 s pass, THEN no WebContent process exists and app RSS
  is within 10 MiB of the core alone; reopening shows every run with no lost events.
- POS-2.1e (PM-1) GIVEN runs A and B were stopped by one quit and Vajra's `.active` points at B, WHEN I resume
  A, THEN campaign A resumes and B is untouched; resuming a campaign already live in any session is refused.
- POS-2.1f (PM-8) GIVEN the app is killed at 50 random points during a 2-worker fixture run, WHEN it relaunches,
  THEN the startup cleanup barrier (design ADR-9 6b') runs before any resume is offered and ends within
  20 s; no process in the runs' recorded process groups survives it; any escaped descendant (cwd inside a
  run worktree) is listed and blocks that campaign's resume until I confirm its kill or choose "Keep and resume anyway"; and every campaign then
  either verifies or offers its checkpoint restore (`vajra-campaign.sh resume`). The fixture includes a
  child that calls `setsid` to leave the group.
- Deferred to v1.1 (ADR-9): runs that keep going through a quit or SIGKILL of the app (VD-4).
- POS-2.2 GIVEN `/vajra <ask>` started in a plain terminal, WHEN the app is open, THEN the run appears live
  within the event latency budget (NFR-PERF-9), with the crew, run view and inbox. The app can't take over
  that terminal. Workers in it can be stopped but not messaged, because the app did not create its nonce (A1 M4).
- POS-2.3 GIVEN the app is uninstalled, WHEN I list `~/.claude/vajra/`, THEN every campaign, evidence file
  and ledger record is still there, readable with the CLI. The app's own files (run index, control ledger,
  report copies) are plain JSONL and markdown in its data directory. This strengthens Q3 in the PRD
  (persist the delivery report in the campaign, engine change E4).
- POS-2.4 GIVEN my Claude Code settings (permissions, hooks, sandbox), WHEN the app starts any session, THEN
  that session uses exactly those settings. The app never passes a flag that widens permissions or disables
  the sandbox (no `--dangerously-skip-permissions`, no permissive `--permission-mode`); the Lead chat adds
  only the app's control tools (design.md section 5.2).

**POS-R3 Knowledge is earned and visible** (reason 2, weak)
- POS-3.1 GIVEN a run that used ledger records, WHEN I open it, THEN it lists each record with its tier and
  source (a run id or "you"). Approve, reject and forget stay Vajra's human gates: the app shows the exact
  `vajra_kb.py` command with a Copy button to run in my own terminal, and never runs, types or sends it into
  any PTY (SECURITY.md: a process that owns a PTY can pass a TTY check).
- POS-3.2 GIVEN the knowledge write gate refused a record during a run, WHEN I open the run, THEN the refusal
  (reason, no refused text) shows in the run's activity, read from the ledger's `audit.jsonl`.
- POS-3.3 (measurement) GIVEN the same class of task run twice in one repo, WHEN run with and without the
  ledger, THEN fix rounds and tokens per delivery are reported. Reason 2 drops to a supporting point if
  there is no improvement.
- POS-3.4 GIVEN a Claude Code session started under Vajra with 100 tool calls, WHEN the transcript is
  inspected, THEN Vajra's injected text (router line, kb prime, steer tags) totals at most 1 KB, or the budget
  is set explicitly in NFR-PERF (VD-26; AC-3.2.6 covers the control hook's share).

**POS-R4 Memory revival gates** (killed as a reason; build budgets regardless, and they only unlock wording)
- POS-4.1 GIVEN 10 stub agents streaming at the spike's rate, WHEN measured by the spike's written method,
  THEN the app's own RSS (claude processes excluded, and also reported with them included) is within
  NFR-PERF-1 and NFR-PERF-2 and is reported next to at least two installed competitors (Orca, Agent
  Orchestrator).
- POS-4.2 (measurement) GIVEN Runner and Orca installed locally, WHEN each runs 10 sessions and its whole
  process tree (Runner's daemon included, agent processes excluded) is sampled as RSS and `phys_footprint`,
  3 or more runs each, THEN the medians and the method are published (NFR-PERF-7).
- POS-4.3 (measurement) GIVEN 10 real `claude` agents, WHEN total RSS is measured, THEN the app's share of
  the total is reported. It sets the attribution story, not a "lighter" claim.
- POS-4.4 GIVEN the 10-agent load on the real build for 1 hour (8 hours before launch), WHEN RSS is sampled
  every minute, THEN app-only GUI RSS grows at most 20 MiB from minute 5 to 60 with no sustained trend above
  1 MiB/min, and the 8-hour repeat stays flat (NFR-PERF-6; the prototype measured +31.1).
- POS-4.5 (measurement) GIVEN the real build and Runner, each with 10 visible live terminals at the same
  scrollback limit, WHEN the 1-hour load runs 3 or more times, THEN the medians are recorded. Any "lighter
  than Runner" wording is banned at launch (PRD section 1) and needs at least 10% below Runner's median plus
  an owner decision.
- POS-4.6 GIVEN the real build idle (window open, no run, 60 s settle), WHEN RSS is summed over its process
  tree, THEN it is at most 200 MiB (NFR-PERF-1). "On par with native at idle" is allowed only if this passes.
- POS-4.7 GIVEN 10 agents, one of them using several GB, WHEN I open the memory panel, THEN each run's agent
  process-tree RSS and the app's own RSS are shown separately and the largest consumer is named (VD-1,
  AC-2.1.4).

**POS-R5 Steer without typing** (parity-plus, from the tracer bullet; the claim is limited by POS-5.5)
- POS-5.1 GIVEN a worker in a run and a message from me via the Lead, WHEN it is delivered, THEN it arrives
  through the hook channel as a `<vajra-steer nonce=N>` tag, an ack is logged, the worker's pane shows "from
  Lead", and nothing but the human or the agent wrote to the worker's PTY (AC-3.2.1, AC-3.2.2, VD-16).
- POS-5.2 GIVEN a steer tag with a wrong or missing nonce (for example text an agent echoed from a web page),
  WHEN the worker sees it, THEN no steer is delivered from it and the event is logged (AC-3.2.9, AC-3.2.11).
- POS-5.3 GIVEN I press Stop on a worker, WHEN it attempts its next tool call, THEN PreToolUse blocks it with
  "stopped by the human", `SubagentHandback` still goes through and the parent receives a "stopped" report
  (AC-3.3.2, AC-3.3.5).
- POS-5.4 GIVEN a stopped worker, WHEN I choose Resume worker, THEN the stop clears (ledgered) and the same
  agent id continues (AC-3.3.6).
- POS-5.5 GIVEN a `/vajra` delivery (task workers spawned by the Workflow tool, not the Agent tool), WHEN
  POS-5.1 to 5.4 are run, THEN all pass (G3b). Until then every steering claim is limited to Agent-tool
  subagents.

## 4. Scope

**In scope.** The six jobs above, macOS, reading Vajra state, launching Claude Code sessions (and Codex CLI through the adapter interface, US-1.5), read-only Conductor import (US-1.4), a control channel for the Lead and the human.

**Out of scope (non-goals).**
- Mobile app and push to phone; multi-machine; cloud sync; team sharing.
- Kanban board; issue-tracker intake.
- PR/CI lifecycle and merging (Agent Orchestrator already does this; Vajra Desktop complements it).
- Agents beyond Claude Code and Codex CLI (Gemini CLI and OpenCode post-MVP); a plugin system; editing code in-app.
- Replacing Vajra hooks, workflows, agents or campaigns.
- Writing under `~/.claude` (except through Claude Code and Vajra's own scripts).
- A plugin API; Windows and Linux; auto-approve of any kind.

**Dependencies.** Claude Code CLI; Vajra installed with hooks registered; git; python3; jq; openssl. New Vajra pieces: `hooks/desktop-control.sh`, extended `hooks.json` matcher (strengthens the guard's audit coverage, does not loosen it).

**Assumptions.** See Section 0.

## 5. Non-functional requirements


### Performance

One canonical table (GPT-5). Source: `vajra-2/stack-spike.md` (Tauri v2 prototype, Apple M3 Max, 2026-11-05, summed RSS on a busy machine, single runs). The owner authorized provisional Tauri MVP work; that is **not** performance approval, which both models withhold until NFR-PERF-1 and NFR-PERF-6 pass on the real build. App RSS means the app, its WebKit helpers and app-owned PTY shells; `claude` and agent processes are reported separately.

Not inferred from the spike: marginal per-worker cost, render duration (rAF intervals are pacing), cold-boot start, physical footprint. Each is measured by H0 and H1 on the real build.

| ID | What | Target | Observed (prototype) | Status |
|---|---|---|---|---|
| NFR-PERF-1 | Idle app RSS, window open, no run, 60 s settle | <= 200 MiB (preferred 190) | 220.2 MiB | **Fails** |
| NFR-PERF-2 | App RSS per worker after warm-up: hidden worker <= 4 MiB, selected-view surcharge <= 8 MiB; retained tail <= 256 KiB per worker, then spill with a gap marker | as stated (provisional) | 7.8 MiB amortized (idle to 10 workers), not a marginal cost | **Unmeasured** (marginal curve 0, 1, 2, 10) |
| NFR-PERF-3 | 10 workers streaming: app-only RSS | <= 400 MiB | 328.9 MiB at 60 min | Passes on the prototype |
| NFR-PERF-3a | Frame pacing at 60 Hz: rAF interval p95 / p99 | <= 20 / 34 ms | rAF p95 20.0 ms (intervals, not render time) | At the limit |
| NFR-PERF-3b | Renderer CPU work per frame p95; worker switch to paint p95 | <= 8 ms; <= 100 ms | not measured | **Unmeasured** |
| NFR-PERF-4 | Start to usable crew view p95, process-cold and warm filesystem; true cold boot reported separately | <= 800 ms | 394.3 ms median of 3 trials (not a p95, not cold boot) | **Unmeasured** as a p95 |
| NFR-PERF-5 | Bounded byte queues at every hop; one in-flight UI snapshot <= 64 KiB; a slow UI can't grow backend memory; slow disk pauses or fails visibly; a PTY or file reader is never blocked | as stated | met by the spike's design; its blocking writes must not be reused (review O6) | Design passes; real build untested |
| NFR-PERF-6 | GUI RSS growth, minute 5 to 60, 10 live workers; no sustained trend > 1 MiB/min; then 8 h, and 72 h with 20 window cycles (PM-6) | <= 20 MiB | 31.1 MiB (WebContent) | **Fails** |
| NFR-PERF-7 | Comparison method: serial runs on a quiet machine, same fixture, RSS and `phys_footprint`, method published; "lighter" only against apps beaten by that method | as stated | Runner 201.3 idle; Orca 1,061.5 idle; Agent Orchestrator **uncontrolled** (it hosted the measurement); single busy-machine runs | **Unmeasured** (repeat runs, footprint) |
| NFR-PERF-8 | Control hook cost per tool call when the app has no queue for the session | <= 10 ms p95 | not measured | **Unmeasured** |
| NFR-PERF-9 | A run started outside the app appears; events reach the UI; control receipt | <= 2 s; <= 250 ms p95; <= 100 ms p95 | not measured | **Unmeasured** |
| NFR-PERF-10 | Compressed installer; installed app (arm64 and universal separately) | <= 20 MiB; <= 50 MiB | 3.0 MiB DMG (unsigned, arm64) | Passes on the prototype |
| NFR-PERF-11 | Key press to glyph in a taken-over terminal or the Lead chat while 10 workers stream | <= 33 ms p95 | not measured | **Unmeasured** |

IDs keep their earlier meaning, so references elsewhere still hold.

### Security
- NFR-SEC-1: respect the guard (`hooks/pre-tool-use.sh`); never weaken it or the sandbox.
- NFR-SEC-2: never read `~/.claude/vajra/.hmac-key`; signature checks go through `vajra-campaign.sh verify`.
- NFR-SEC-3: all agent output is untrusted; rendered as text; no HTML or escape-sequence execution.
- NFR-SEC-4: no shell interpolation of agent or user text; arguments are passed as argv.
- NFR-SEC-5: IPC is owner-only (Unix socket mode 0600); no network listener.
- NFR-SEC-6: filesystem reads follow an explicit path allowlist; every path is canonicalized and symlink escapes are refused; reads are size-capped; no recursive watch on `~/.claude/vajra`.
- NFR-SEC-7: release builds are signed and notarized; updates are signed; dependencies are pinned and installed with lifecycle scripts off; `build.rs` and native dependencies are reviewed; `cargo-deny` and `npm audit` run in CI; each release has an SBOM.
- NFR-SEC-9: the app never runs a command that came from agent output (a recorded check, its recorded setup commands, a plan step) without the human confirming the exact text; the Lead and MCP tools can't trigger one.
- NFR-SEC-10 (STRIDE S1): the control socket checks the peer uid on every connection and binds each connection to the session it was started for; a connection that names another session is refused.
- NFR-SEC-11 (STRIDE I1): spill files, logs and report copies are 0600 in the app data dir, kept at most 14 days or 1 GB (oldest first), and never uploaded.
- NFR-SEC-12 (STRIDE I2): the webview loads only bundled assets under a strict CSP, can't navigate away or open windows, and opens links only after a human click, in the default browser.
- NFR-SEC-13 (STRIDE D1): parsers cap line length (1 MiB), JSON depth (64) and record size; an over-cap record shows as "truncated" and never stalls the reader.
- NFR-SEC-14 (STRIDE E1): the Tauri command surface is a fixed, documented list; no command takes an executable path, a shell string or an arbitrary filesystem path; each validates its arguments and the calling window.
- NFR-SEC-15 (STRIDE E3): updates are signed with an offline key held by the owner; a bad signature or a downgrade is refused, shown, and leaves the installed app unchanged.
- NFR-SEC-16 (owner decision, billing.md): the app never reads, stores, proxies or offers credentials or a claude.ai login (no access to Claude Code's credential files, the keychain item, `CLAUDE_CODE_OAUTH_TOKEN` or `~/.codex/auth.json`); it launches only the stock binaries under the user's own login, and never pays for, resells or proxies model usage.
- NFR-SEC-8: control messages are never approval for protected actions.

Steering claim limit (GPT-2): proven for Agent-tool subagents (tracer bullet). "Any worker in a /vajra run" is claimed only after G3b passes inside `vajra-deliver` (POS-5.5).

### Compatibility
- NFR-COMPAT-1 (PM-4): preflight records `claude --version`. Outside the tested range the control row reads
  "untested Claude Code X, control unverified". A message shows "delivered" only when the worker's transcript
  contains its steer id; a hook ack alone shows "handed to hook". The G3b tracer is rerun on each new Claude
  Code minor version before the range widens.

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
| US-1.1 | AC-1.1.1 .. 1.1.7 | SEC-1, SEC-4, UX-2, UX-3 |
| US-1.2 | AC-1.2.1 .. 1.2.3 | SEC-2, SEC-4, UX-2 |
| US-1.3 | AC-1.3.1 .. 1.3.7 | SEC-2, UX-1, A11Y-1 |
| US-1.4 | AC-1.4.1 .. 1.4.4 | SEC-2, SEC-6 |
| US-1.5 | AC-1.5.1 .. 1.5.3 | SEC-5, SEC-16 |
| US-2.1 | AC-2.1.1 .. 2.1.8 | PERF-3, A11Y-1, A11Y-3 |
| US-2.2 | AC-2.2.1 .. 2.2.6 | PERF-2, PERF-3, PERF-5, PERF-6, SEC-3 |
| US-2.3 | AC-2.3.1 .. 2.3.3 | PERF-3, SEC-3, SEC-6 |
| US-2.4 | AC-2.4.1 .. 2.4.4 | A11Y-2, UX-1, UX-2 |
| US-3.1 | AC-3.1.1 .. 3.1.3 | SEC-3, SEC-5, OBS-2 |
| US-3.2 | AC-3.2.1 .. 3.2.11 | SEC-1, SEC-3, SEC-4, SEC-8, OBS-2 |
| US-3.3 | AC-3.3.1 .. 3.3.7 | SEC-8, UX-3, OBS-2 |
| US-3.4 | AC-3.4.1 .. 3.4.10 | SEC-1, SEC-5, OBS-1, OBS-2, UX-3 |
| US-4.1 | AC-4.1.1 .. 4.1.2 | A11Y-1, UX-2 |
| US-4.2 | AC-4.2.1 .. 4.2.3 | PERF-3, A11Y-2 |
| US-4.3 | AC-4.3.1 .. 4.3.4 | SEC-2, SEC-3 |
| US-5.1 | AC-5.1.1 .. 5.1.5 | A11Y-3, UX-3 |
| US-5.2 | AC-5.2.1 .. 5.2.5 | SEC-1, SEC-8, OBS-2, UX-3 |
| US-5.3 | AC-5.3.1 .. 5.3.3 | SEC-4, OBS-2 |
| US-6.1 | AC-6.1.1 .. 6.1.3 | SEC-2, A11Y-2 |
| US-6.2 | AC-6.2.1 .. 6.2.3 | SEC-4, SEC-6 |

Cross-cutting: NFR-PERF-1, 4, 7, NFR-SEC-5, 7 and NFR-OBS-1 apply to the whole app and are verified in the benchmark and release checklists, not per story.

### Positioning traceability

| Reason | POS ids | Stories it shapes | Tasks |
|---|---|---|---|
| 1 Proof per criterion | POS-1.1, 1.2, 1.3, 1.5, 1.7 | US-2.1, US-4.3, US-6.1 | S2, S4, E3, C4 |
| 1 (measurement) | POS-1.4, 1.6 | none (claim gates) | H7 |
| 2 Earned knowledge (weak) | POS-3.1, 3.2, 3.4 | US-6.2, US-3.2 | S6, C4, E1 |
| 2 (measurement) | POS-3.3 | none (claim gate) | H7 |
| Table stakes: Claude Code-native | POS-2.1, 2.2, 2.3, 2.4 | US-1.1, US-1.2, US-2.1 | G1, C3, C5, C4 |
| Table stakes: steer without typing | POS-5.1 to 5.5 | US-3.2, US-3.3 | G3b, E1, E5, C6 |
| Killed: memory (revival gates) | POS-4.1, 4.4, 4.6, 4.7 build budgets; POS-4.2, 4.3, 4.5 measurements | US-2.1 | H1, C5 |

## 7a. Pain points from user research

Source: `vajra-4/pain-points.md` (23 pain points from GitHub issues, HN, X, V2EX and Product Hunt, 2026-11-05). Ponytail applied: 27 rows MVP, 5 deferred, 7 won't do (P11 moved to MVP by the vajra-4 top-up) (P21 moved to MVP by owner decision) (P24 added from `vajra-4/billing.md`). Every MVP row names the task that delivers it. New criteria from this table carry their VD id in brackets in section 3.

| Pain point | Requirement (vajra-4 id) | Decision | Where in this spec | Task |
|---|---|---|---|---|
| P1 memory bloat | VD-1 own vs agent memory | MVP | AC-2.1.4; NFR-PERF-1 to 3 define app RSS without agents | C5, S2 |
| P1 | VD-2 bounded buffers | MVP (already) | AC-2.2.2, NFR-PERF-2, NFR-PERF-5 | C2 |
| P1 | VD-3 hidden panes free their view | MVP | AC-2.4.4 | K3 |
| P2 sessions die | VD-4 daemon outlives the UI | Deferred v1.1 (ADR-9) | MVP keeps runs alive while the window is closed: POS-2.1, 2.1a | none |
| P2 | VD-5 honest resume | MVP | POS-2.1b | C3 |
| P3 crashes, bad updates | VD-6 one process generation | MVP | POS-2.1c (no daemon, updates wait for idle) | C3 |
| P3 | VD-7 paste and output guard | MVP | AC-2.2.5; escape sanitizing NFR-SEC-3 | K3 |
| P4 opaque status (11 sources) | VD-8 status from signals, else Unknown; stall detector | MVP | AC-2.1.5, AC-2.1.7 | C5 |
| P4 | VD-9 needs-you inbox | MVP (already US-5.1) | AC-5.1.1 to 5.1.4 | S5 |
| P4 | VD-10 ask the Lead, answer cites evidence | MVP (already US-3.1) | AC-3.1.1 to 3.1.3 | S3 |
| P5 rate limits, cost | VD-11 rate-limited state | MVP (detection to verify) | AC-2.1.6. Per-run cost stays in `/vajra cost`: deferred | C5 |
| P5 | Automatic account failover | Won't do | Credential rotation across accounts; terms and secrets risk (agree with vajra-4) | none |
| P6 tool disappears | VD-12 no cloud, no account, open files | MVP | AC-1.3.6, POS-2.3 | C1, H7 |
| P6 | VD-13 run the real interactive CLI | MVP | POS-2.4; the app never uses `-p` for runs or proxies auth | C3 |
| P7 agents collide | VD-14 conflicts routed to the owner | Won't do in the app | Vajra's deliver workflow owns the integration merge and the fix loop; a failing merge already returns to the owning task | none |
| P7 | VD-15 per-worker ports and env | Deferred (engine) | Belongs in the workflow's worktree setup, not the app | none |
| P7 | Semantic conflict detection | Won't do | Research-grade (agree with vajra-4) | none |
| P8 typing into terminals | VD-16 documented control channel | MVP (already) | US-3.2 to 3.4, ADR-2; Lead text labelled `lead-via-desktop` (STRIDE F3) | E1, C6 |
| P9 too many windows | VD-17 crew tree | MVP (already) | US-2.1, US-2.4. Split view of up to 4 panes: deferred (one worker pane plus the chat dock covers the MVP) | S2 |
| P10 multi-repo | VD-18 multi-repo crew | Deferred | A Vajra run is one repo; needs engine support first | none |
| P11 worktree bloat (9 sources) | VD-19 worktree lifecycle keyed to how the session ended | MVP (vajra-4 top-up) | AC-2.1.8; `vajra-worktree-clean.sh` still does the removal | C4, S2 |
| P11 | VD-20 worktrees optional | Won't do | Vajra runs every writer in its own worktree by design; a shared checkout breaks parallel waves | none |
| P12 permission fatigue vs YOLO | VD-21 pass the user's permission modes, never bypass | MVP | POS-2.4 (argv test) | C3 |
| P12 | VD-22 batch approvals for the crew | Won't do | It would answer Claude Code's permission prompt on the human's behalf; protected actions stay one human approval each (SKILL.md rule 2) | none |
| P13 terminal fidelity | VD-23 proven terminal core, latency gate | MVP | xterm.js (G2); NFR-PERF-11; AC-2.2.6 | K3, H1 |
| P14 notification noise | VD-24 notify on needs-you and finished runs only | MVP | AC-5.1.5. The ntfy webhook: deferred | S5 |
| P15 lost context | VD-25 context meter and handoff | Deferred | Vajra's knowledge ledger and campaigns carry context across runs (POS-R3) | none |
| P15 | VD-26 zero-cost hooks | MVP | AC-3.2.6, NFR-PERF-8 | E1 |
| P16 silent config edits | VD-27 never edit config without consent | MVP | AC-1.3.5, AC-1.1.3; engine changes install through Vajra's own installer, run by the human | S1 |
| P16 | VD-28 two-minute first run | MVP (app part) | AC-1.3.7. Vajra itself still needs python3, jq and openssl | S1 |
| P17 feature bloat | VD-29 small surface by rule | MVP | The six jobs plus Settings (UX.md); H2 click crawl finds no other screen | H2 |
| P18 diff review bottleneck | VD-30 hunk comments sent to the worker | Deferred v1.1 | Evidence per criterion (POS-R1) is the MVP answer; workers have usually finished by review time | none |
| P18 | Submit GitHub PR reviews | Won't do | `gh` and the browser do it (agree) | none |
| P19 telemetry, login | VD-31 no telemetry by default | MVP | AC-1.3.6 | C1 |
| P20 Electron complaints | VD-32 stack by measurement | MVP (done) | G2, NFR-PERF-7 | G2 |
| P21 lock-in | VD-33 agent-agnostic launch | MVP, narrowed (owner decision, 2026-11-05) | One adapter interface; Claude Code full, Codex CLI with honest capability flags; Gemini CLI and OpenCode post-MVP; no plugin system: US-1.5 | C7 |
| P24 parallel sessions race on one OAuth login (Runner, Paseo; billing.md) | Never touch tokens; let the stock binary refresh | MVP | AC-1.1.5 | C7 |
| P22 mobile breaks | Mobile app, remote hosts | Won't do (v1) | Agree with vajra-4 | none |
| P23 why not just Claude Code | VD-34 win where native tools are weak | MVP (product rule) | Adapted: compete on evidence per criterion, the audited control channel and memory, not on mixed vendors. Every feature cites a job, a P-row or a POS reason (PRD section 8) | PRD rule |

## 7b. Review findings

Sources: architecture review workflow run 20261105-1745 (Architect ADR at commit 6b0cedb on a local branch, three votes: two revise, one approve; its pre-mortem step returned no output, so a Critic pre-mortem was run separately: PM-1 to PM-8 below, also in the PRD); STRIDE (`vajra-3/stride.md`); ponytail review.

| Finding (review) | Decision | Requirement or design change | Acceptance id |
|---|---|---|---|
| B1 Steering needs the nonce tag; plain wrapper is ignored (arch review, tracer) | Fixed | design 5.3, 5.4 step 5.2, engine change E5 | AC-3.2.2 |
| B2 / F2 Stop blocks the final output; stop file cleared too early; resume via SendMessage | Fixed | design 5.3, 5.4 step 4, ADR-8 | AC-3.3.4 to 3.3.7 |
| B3 Control not proven through the Workflow tool | Fixed: validation gate G3b after E1 and E5, before C6 (no cycle) | tasks.md G3b; fallback stop-only (ADR-2) | G3b done-when |
| S1 voter: a stopped worker could still report done | Fixed | design 5.4 step 4 (only status blocked passes) | AC-3.3.4 |
| S2 voter: the verification gate was steerable | Fixed | design 5.4 step 5b, E5 (task briefs only) | AC-3.2.2 |
| S3 voter / M3: queue protection rests on guard regexes | Spec fixed, owner decision | design 5.4 E2 scope; sandbox denyWrite approved (Q10, E8) | E2 tests |
| F3 Lead text labelled human | Fixed | source from actor: human or lead-via-desktop | AC-3.2.2 |
| H1 design section 9 had stale budgets | Fixed | pointer to NFR-PERF-1 to 11; 256 KiB tail | NFR-PERF-2 |
| M1 D1 to D3 and 5.9 still say to verify | Fixed | tasks.md G3; design 5.9 note | G3 |
| M4 terminal-started runs promised steering | Fixed | POS-2.2 narrowed to stop only | POS-2.2 |
| M5 windowless menu bar app may be throttled by App Nap | Added | ADR-9: hold an activity assertion while runs are active; 30 min closed-window test in C3 | POS-2.1 |
| M6 control hook should check the dir before jq | Fixed | design 5.4 step 2 | NFR-PERF-8 |
| L1 app directory name | Fixed | `VajraDesktop` everywhere | none |
| L3 privacy prompts for repos under ~/Documents | Added | C3 test for the TCC prompt | AC-1.1.x |
| F1 forged steer with a leaked nonce (steering threat model) | Mitigated, residual stated | design 5.3 trust model; nonce read blocks (E2); nonce-echo block | AC-3.2.9, AC-3.2.11, AC-3.4.6 |
| F2 stop dodged through handback then SendMessage | Fixed | stop cleared only by a human | AC-3.3.5, AC-3.3.6 |
| F4 the tracer hook logs full payloads | Fixed | the shipped E1 logs ids and tool names only; tracer scripts never ship | AC-3.2.7 (`no_nonce_in_outputs`) |
| STRIDE S1 socket spoofing | Adopted | NFR-SEC-10 | NFR-SEC-10 |
| STRIDE S2 forged inbox item | Adopted | AC-5.1.6 | AC-5.1.6 |
| STRIDE T1 agents edit app data | Adopted | E2 protects the whole app data dir | E2 tests |
| STRIDE T2 symlink escape | Adopted | NFR-SEC-6 amended | NFR-SEC-6 |
| STRIDE R1 human actions unrecorded | Adopted | AC-3.4.5 | AC-3.4.5 |
| STRIDE I1 secrets in spill files | Adopted | NFR-SEC-11 | NFR-SEC-11 |
| STRIDE I2 remote content | Adopted | NFR-SEC-12 | NFR-SEC-12 |
| STRIDE I3 clipboard escapes | Adopted | NFR-SEC-3 plus test | NFR-SEC-3 |
| STRIDE D1 parser floods | Adopted | NFR-SEC-13 | NFR-SEC-13 |
| STRIDE D2 control write blocks | Adopted | NFR-PERF-5 plus test | NFR-PERF-5 |
| STRIDE E1 broad Tauri commands | Adopted | NFR-SEC-14 | NFR-SEC-14 |
| STRIDE E2 re-run of reflowed command | Adopted | POS-1.2 amended | POS-1.2 |
| STRIDE E3 malicious update | Adopted | NFR-SEC-15 | NFR-SEC-15 |
| STRIDE E4 Lead tools widen | Adopted | AC-3.1.4 | AC-3.1.4 |
| STRIDE E5 supply chain | Adopted | NFR-SEC-7 amended | NFR-SEC-7 |
| PM-1 resume continued the wrong campaign (global `.active`) | Fixed | ADR-9 decision 5 resumes by slug | POS-2.1e |
| PM-2 no task owns the live end-to-end checks | Fixed | new task H8, owner-run live acceptance | H8 done-when |
| PM-3 performance gates close after the work they gate | Fixed | new task H0 (perf CI from C1); end-of-M3 budget rule; ADR-9 and design 5 time-boxed review | NFR-PERF-1, NFR-PERF-6 |
| PM-4 a Claude Code update breaks steering silently | Adopted | NFR-COMPAT-1 | NFR-COMPAT-1 |
| PM-5 approvals stranded after the session ends | Adopted | AC-5.2.5 | AC-5.2.5 |
| PM-6 the menu bar app turns a slow leak into a multi-day one | Adopted | ADR-9 6c; 72 h soak; Install now after 72 h | POS-2.1d, NFR-PERF-6 |
| PM-7 transcripts are a single point of failure | Adopted | E4 moves into M1; new E6 records the worktree in the claim so control matches on tasks.json | E6 done-when |
| PM-8 a crash makes resume unsafe | Adopted | ADR-9 6b (process group); fault test | POS-2.1f |
| Ponytail: resume typed into the PTY | Fixed (6db4529) | ADR-9 decision 5 uses argv | POS-2.1b |
| Ponytail: kb approvals prefilled in an app PTY | Fixed (6db4529) | copy to own terminal | POS-3.1 |

### 7c. GPT 6.1 Sol spec review (`vajra-7/spec-review.md`, REQUEST_CHANGES) and PR #2 review rounds

| Finding | Decision | Commit | Acceptance ids |
|---|---|---|---|
| GPT-1 Stop blocks handback and can be bypassed after completion | Fixed: stop exempts exactly `SubagentHandback`, is sticky across `SendMessage` resume, and only a human clears it | 916f4b9 | AC-3.3.4 to AC-3.3.6 |
| GPT-2 Steering uses the wrapper the tracer rejected; no nonce trust model | Fixed: `<vajra-steer nonce=N kind source id>`, nonce at least 128 bits, Execute-phase task workers only, trust model and residual risk stated | a8c02fc | AC-3.2.2, AC-3.2.7 to AC-3.2.11 |
| GPT-3 Queue write protection lacks guard read blocks and the sandbox boundary | Fixed: worker reads of the control dir and transcripts blocked, whole data dir write-protected, same-user limitation stated, sandbox `denyWrite` is Q10 | b251b96 | AC-3.4.6 to AC-3.4.8 |
| GPT-4 Freshness can't detect the demo's file edit | Fixed: "stale" compares the signed whole-tree id with the current one; "unverified" comes from E3 | 5d8a9b8 | POS-1.3 |
| GPT-5 Budget tables contradict the spike | Fixed: one canonical NFR-PERF table with observed values and status; the PRD points to it | d641f83 | NFR-PERF-1 to NFR-PERF-11 |
| GPT-6 Research coverage and MVP exclusions blank | Fixed: every MVP pain point names its task; F1, F2 and F4 added to 7b; OAuth refresh race P24 | e9c071d | AC-1.1.5 |
| GPT-7 Positioning still pass 1, with a contradicted competitor claim | Fixed: final positioning, memory killed as a claim, banned claims, "none runs verify/fix rounds" removed, copy time labelled inference | 870b3ce | POS-1.7, POS-3.4, POS-4.2 to POS-4.7, POS-5.1 to POS-5.5 |
| GPT-8 (optional) Separate the shipping slice | Adopted: two slices, with the evidence loop in the private beta | 074a894 | none (ordering only) |
| PR: Re-run must replay setup commands | Fixed | 5adfbcd | POS-1.2 |
| PR: G3b/E5 circular prerequisite | Fixed: G3b after E1 and E5, before C6 | 0e0928a | G3b done-when |
| PR: campaign-specific resume in POS-2.1b | Fixed | f7d84c1, 148c9f0 | POS-2.1b, POS-2.1e |
| PR: crash cleanup before "no process survives" | Fixed: startup cleanup barrier (ADR-9 6b') | f7d84c1 | POS-2.1f |
| PR: materialize the evidence tree before Re-run | Fixed: apply `diff.patch`, require the signed tree id, else refuse | f7d84c1 | POS-1.2 |
| PR: sandbox-off messaging policy inconsistent | Fixed: sandbox off means stop-only (kept after Q10 was approved, for users who leave the sandbox off) | f7d84c1 | AC-3.4.8 |
| PR: preserve the verification bundle before cleanup | Fixed: engine change E7 archives it with a signed manifest; E3 `--archived` | 6fa24c1 | POS-1.2 |
| PR: lossless executed setup list | Fixed: `setup.json` holds the exact executed `setupCmds`; report lines and raw `setup_cmds` are never sources | 6fa24c1 | POS-1.2 |

## 8. Decisions requested from the owner

1. Confirm A1..A9 (especially A3 transcript streams and A5 hook identification, which the spike must prove).
2. Decide whether Vajra should persist delivery reports in the campaign (A8).
3. Decide whether renaming engine agents is wanted later (A2).
4. Q10: decided 2026-11-05 (approved; E8, AC-3.4.9, AC-3.4.10).
5. Q13: decided 2026-11-05 (E7 approved, under vidura review).
6. Q12 (launch blocker): whether to ask Anthropic in writing about consumer-plan use by a local orchestrator (PRD section 11).

Decided 2026-11-05: Q11 approved (E5 for task briefs only and the wider E2, under Security review, every existing guard test passing); the agent-adapter interface with Codex CLI (US-1.5); Conductor import (US-1.4); the billing hard rules (NFR-SEC-16, AC-1.1.6, AC-1.1.7).
