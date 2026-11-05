# Vajra Desktop: UX

Status: draft for owner review. Scope: the six MVP jobs (J1 start/resume, J2 crew, J3 orchestrator
chat and control, J4 run view, J5 approvals, J6 history and knowledge). macOS only. No mobile.
Visual tokens and components live in [DESIGN.md](DESIGN.md). Story IDs (US-x.y) come from the PRD.

The UI shows generic role names only (Lead, Architect, Backend, Frontend, Verifier, Security, and the
rest). Engine ids appear in one place: the "Engine id" field on a worker's Task tab.

## 1. Principles

1. **Customer experience first.** Fast to open (cold start to interactive at or under 1.0 s,
   provisional), never janky (no dropped frames while 10 workers stream), no surprises (nothing
   irreversible happens without a visible confirm, and the app itself never pushes, opens PRs or deploys).
2. **Keyboard-first.** Every action has a key or a palette entry. Mouse works everywhere, but nothing
   needs it.
3. **The human can always see what the Lead did.** Every control action (message, redirect, stop) shows
   up as an action card in chat, on the worker it touched, and in the control ledger, with a link to the
   matching audit.log line. The human can take the same action directly without asking the Lead.
4. **Engine state is the truth.** Screens render Vajra's campaign, task graph, evidence and ledger. The
   app never invents progress, percentages or estimates it cannot read from disk.
5. **Read-only by default.** The run session's terminal is read-only until the human picks "Take over".
   Worker streams are always read-only; workers are steered only through the control channel.
6. **Say what happened, with numbers.** "3 of 7 criteria pass", "Stopped Backend at 14:02", never
   "Something went wrong".

## 2. Information architecture

```
Vajra Desktop window
├── Sidebar (sections, Cmd-1..6)
│   ├── 1 Home ............ Start run form, Preflight, Resume list
│   ├── 2 Crew ............ Crew tree | Worker pane (Stream | Diff | Task | Worktree)
│   ├── 3 Chat ............ Orchestrator chat, full view (also a dock on 2, 4, 5)
│   ├── 4 Run ............. Phase timeline, Task graph, Criteria table
│   ├── 5 Approvals ....... Inbox rows -> Detail (plan, question, blocked task, permission prompt)
│   ├── 6 History ......... Campaigns -> Delivery report | Knowledge search
│   └── Settings (Cmd-,) .. Claude path, Theme, Notifications
├── Chat dock (right, toggle Cmd-J, focus Cmd-L)
├── Command palette (Cmd-K, overlay on every screen)
└── Status bar (run status, app memory, guard status, engine check)
```

Cross-links: a task graph node opens that worker in Crew. A criterion row opens its evidence file. An
action card opens the worker and the audit line. An inbox row for a blocked task opens the worker. A
history row opens its delivery report and, if the campaign is resumable, offers "Resume".

## 3. Layout frame and minimum window

Default 1280x800. Minimum 960x600 (the window cannot be made smaller).

```
+------+------------------------------------------------------------+-----------------+
| side | screen content                                             | chat dock       |
| bar  |                                                            | (optional)      |
| 220  | min 480                                                    | 320, min 280    |
+------+------------------------------------------------------------+-----------------+
| status bar 24 px                                                                    |
+-------------------------------------------------------------------------------------+
```

At widths under 1200 the sidebar collapses to 56 px (icons with tooltips and labels for VoiceOver).
At 960 the chat dock becomes an overlay sheet over the right of the content instead of a column.

## 4. Wireframes (1280x800)

Legend: `●` in progress, `◐` claimed, `◌` ready, `○` pending, `✓` done or pass, `■` blocked, `▲` needs you (approval or warning),
`◆` question, `✕` failed or fail. Every glyph is always followed by its text label.

### 4.1 Home / Start run + Resume (J1)

```
+-------------+--------------------------------------------------------------------------+
| Vajra       | Start a run                                                              |
|             |                                                                          |
| > Home   1  | Repository  [ ~/code/reports-app                         v ] [Choose...] |
|   Crew   2  | Ask         [ add CSV export to the reports page                       ] |
|   Chat   3  |             [                                                          ] |
|   Run    4  | Base        [ main (a1b2c3d)        v ]   Max rounds  ( 1 | 2 |[3])     |
|   Approvals5| > Advanced  Workers at once [ 4 ]  (1 to 16)                             |
|   History 6 |                                                   [ Start run  Cmd-Enter]|
|             |--------------------------------------------------------------------------|
|             | Preflight                                                   [Re-check]   |
|             | ✓ Pass  Claude Code found     /opt/homebrew/bin/claude  2.4.1            |
|             | ✓ Pass  vajra doctor          18 of 18 checks pass                       |
|             | ✓ Pass  Hooks registered      4 hooks in settings.json                   |
|             | ▲ Off   OS sandbox            Off. Recommended for unattended runs. [How]|
|             |--------------------------------------------------------------------------|
|             | Resume a campaign                                                        |
|             | deliver-20261104-1610  Add rate limiting    Execute, round 2   [Resume]  |
|             | deliver-20261103-0920  Fix login redirect   Verify, round 1    [Resume]  |
| Settings    |                                                                          |
+-------------+--------------------------------------------------------------------------+
| No run   App 92 MB   Guard on   Engine ok                                              |
+----------------------------------------------------------------------------------------+
```

960x600: the form keeps one column; Preflight collapses to one summary line ("4 of 4 checks pass,
sandbox off") that expands on click; Resume list scrolls below.

### 4.2 Crew view (J2)

```
+------+-------------------------+--------------------------------------------+----------------+
| Home | Crew                 /  | Backend   ● In progress                    | Chat with Lead |
| Crew | v Lead          ● Live  | Task t3 Add CSV endpoint                   |                |
| Chat |  v Run 20261105-0930    | ~/.../.claude/worktrees/t3  vajra/2026..t3 | (dock, 4.3)    |
| Run  |    ✓ Acceptance  Done   | [Message] [Redirect] [Stop  Cmd-.]         |                |
| Appr |    ✓ Architect   Done   |--------------------------------------------|                |
| Hist |    ● Backend  In prog.  | [Stream] Diff  Task  Worktree   Cmd-F find |                |
|      |    ● Frontend In prog.  | 14:02:11 Read  src/api/reports.py          |                |
|      |    ◌ Verifier  Ready    | 14:02:13 Edit  src/api/reports.py +24 -3   |                |
|      |    v Fix round 1        | 14:02:20 Bash  pytest tests/test_csv.py    |                |
|      |      ✕ Verifier Failed  |   2 passed, 1 failed                       |                |
|      |      ○ Backend Pending  | ┌ Lead sent a message to Backend ────────┐ |                |
|      |                         | │ "Use the streaming writer" [audit line] │ |                |
|      |                         | └─────────────────────────────────────────┘ |                |
|      |                         | 14:02:31 Edit  src/api/csv.py +40          |                |
|      |                         |                    [Jump to latest  End]   |                |
+------+-------------------------+--------------------------------------------+----------------+
| Run 20261105-0930  Execute, round 1  3 workers   App 141 MB   Guard on   Engine ok          |
+---------------------------------------------------------------------------------------------+
```

Tabs: **Stream** (rendered transcript, read-only; the Run node shows the run session terminal,
read-only, with a "Take over" button), **Diff** (worktree vs base, file list left, unified diff right),
**Task** (title, acceptance, files, deps, attempts, engine id, last report), **Worktree** (path,
branch, head commit, dirty files, "Reveal in Finder", "Copy path").

960x600: chat dock closes (overlay via Cmd-J); tree pane narrows to 200 px with labels truncated
middle and full name in tooltip; header actions fold into a "Actions" menu button.

### 4.3 Orchestrator chat (J3), dock and full view

```
+------+------------------------------------------------------------------------------------+
| Home | Chat with Lead                                    Run 20261105-0930  [Pop in Cmd-J] |
| Crew |------------------------------------------------------------------------------------|
| Chat | You 14:05   What is Backend stuck on?                                              |
| Run  | Lead 14:05  Backend's last check failed: test_csv_header expects a BOM. It has     |
| Appr |             retried once. Evidence: pytest exit 1. [Open Backend]                  |
| Hist | You 14:06   Tell Backend to drop the BOM and match the spec.                       |
|      | ┌─────────────────────────────────────────────────────────────────────────────┐    |
|      | │ Lead sent a message to Backend                         14:06  Delivered     │    |
|      | │ "Drop the BOM; follow criterion C2 as written."                             │    |
|      | │ [View worker]  [View in audit]                                              │    |
|      | └─────────────────────────────────────────────────────────────────────────────┘    |
|      | ┌─────────────────────────────────────────────────────────────────────────────┐    |
|      | │ Lead wants to stop Frontend                            Waiting for you      │    |
|      | │ Reason: "duplicate of t3"   [Stop worker]  [Don't stop]                     │    |
|      | └─────────────────────────────────────────────────────────────────────────────┘    |
|      |------------------------------------------------------------------------------------|
|      | [ Ask the Lead, or @Backend to target a worker                     ] [Send Cmd-Ent] |
+------+------------------------------------------------------------------------------------+
| Run 20261105-0930  Execute, round 1   App 140 MB   Guard on   Engine ok                    |
+--------------------------------------------------------------------------------------------+
```

The Lead may message and redirect on its own; every such action renders a card with status
(Queued, Delivered, Acknowledged, Failed). Stop is always a request card the human must confirm.
The dock (320 px) shows the same thread with cards compacted to two lines.
960x600: the full view is unchanged; the dock becomes an overlay sheet 320 px wide.

### 4.4 Run view (J4): evidence first

Positioning reason 1 (PRD section 1): opening a run lands on its criteria, failing ones first (POS-1.1,
POS-1.5). The timeline and task graph sit below them and are one key away.

```
+------+-------------------------------------------------------------------------------------+
| Home | Run 20261105-0930  "add CSV export to the reports page"   Not done: 1 failing       |
| Crew | ✓ Route  ✓ Recall  ✓ Understand  ✓ Plan  ✓ Execute  ✓ Verify  ● Fix 1 of 3  ○ Deliver|
| Chat |-------------------------------------------------------------------------------------|
| Run  | Acceptance criteria                 3 pass, 1 fail, 1 stale, 1 not run              |
| Appr | ID  Criterion              Result       Command                 Exit  Checked by    |
| Hist | C2  CSV has header row     ✕ Fail       pytest tests/test_csv   1     Verifier      |
|      | C4  Keyboard reachable     ◐ Stale      npx playwright test a11y 0    Verifier      |
|      | C5  Docs updated           ○ Not run    -                       -     -             |
|      | C1  Export button visible  ✓ Pass       npm test -- export      0     Verifier      |
|      | C3  Streams > 10k rows     ✓ Pass       pytest -k large         0     Verifier      |
|      |   > C2 expanded: written by Backend, checked by Verifier. Exit 1. Output excerpt (3    |
|      |     lines, full log link). Diff (2 files). Evidence ev.json @ 9f3e2a. [Re-run...]      |
|      |-------------------------------------------------------------------------------------|
|      | Task graph                                   Wave 2 of 3, 2 of 4 workers busy       |
|      |  [t1 Schema  Architect ✓ Done] ──> [t3 CSV endpoint Backend ● In progress] ─┐      |
|      |  [t2 Button  Frontend  ✓ Done] ──> [t4 Export menu Frontend ● In progress] ─┼> [t5 |
+------+-------------------------------------------------------------------------------------+
| Run 20261105-0930  Execute, round 1   App 138 MB   Guard on   Engine ok                     |
+---------------------------------------------------------------------------------------------+
```

Status words: Pass, Fail, Not run, Stale (evidence is for an older head), Unverified (signature check
failed). Each has its own icon and text, never color alone. The header never shows a green "done" while any
criterion is not Pass. **Re-run...** opens a confirm dialog that shows the exact recorded command and where it
will run (a fresh worktree at the evidence commit); the result row shows "same as recorded" or "differs".
Graph nodes are focusable (arrow keys follow edges); Enter opens the worker in Crew. The criteria
table rows expand to show the full command, notes and the evidence path
`data/walkthroughs/deliver-<run_id>/evidence.json`. "Copy command" copies the literal command.
960x600: the timeline wraps to two rows; the graph becomes a list grouped by wave; the table hides
Evidence and Commit columns behind row expansion.

### 4.5 Approvals inbox (J5)

```
+------+-----------------------------------------+------------------------------------------+
| Home | Approvals                    3 waiting  | Push plan for run 20261105-0930          |
| Crew | > ▲ Push plan     run 0930   14:20      | Branch  vajra/20261105-0930/integration  |
| Chat |   ◆ Question      run 0930   14:12      | Commit  9f3e2a1                          |
| Run  |   ■ Blocked task  t4 Frontend 14:09     | Steps the run session will take:         |
| Appr |                                         |  1 git push origin vajra/2026...-0930/.. |
| Hist |                                         |  2 gh pr create --title "Add CSV export" |
|      |                                         | Claude Code will still ask you to allow  |
|      |                                         | each command.                            |
|      |                                         | Criteria  6 of 6 pass  [Open run]        |
|      |                                         |                                          |
|      |                                         | [Approve plan  A]   [Reject  R]          |
+------+-----------------------------------------+------------------------------------------+
| Run 20261105-0930  Waiting for approval   App 120 MB   Guard on   Engine ok              |
+-------------------------------------------------------------------------------------------+
```

Row kinds: push plan (pending_approval), question (need_input), blocked or question task, and a
session waiting on a Claude Code permission prompt (opens that terminal; no Approve button, because
the human answers the prompt in the terminal itself).
960x600: master-detail becomes a list; Enter opens the detail as a full pane with a Back button.

### 4.6 History + Knowledge search (J6)

```
+------+-----------------------------------------+------------------------------------------+
| Home | History   [Campaigns] Knowledge         | Delivery report  deliver-20261104-1610   |
| Crew | [ / Filter campaigns                  ] | Status  Done   Branch vajra/2026..1610/.. |
| Chat | deliver-20261105-0930  Running  today   | Criteria 5 of 5 pass   Fix rounds 1      |
| Run  | > deliver-20261104-1610  Done  Nov 4    |------------------------------------------|
| Appr |   deliver-20261103-0920  Stopped Nov 3  | ## What changed                          |
| Hist |   deliver-20261101-1200  Done  Nov 1    | (report markdown, rendered read-only)    |
|      |                                         | ## How to run it                         |
|      |                                         | pytest tests/test_rate_limit.py          |
|      |                                         |             [Copy branch] [Resume]       |
+------+-----------------------------------------+------------------------------------------+
```

```
| Hist | History   Campaigns [Knowledge]         | Decision  kb-0142   verified             |
|      | [ / csv export                     ]    | "Stream CSV with a generator, no BOM"    |
|      | 4 results for "csv export"              | Source  deliver-20261104-1610            |
|      | > Decision  Stream CSV, no BOM   Nov 4  | Session 7f2c...  Evidence ev.json        |
|      |   Lesson    Header test flaked   Nov 4  |                                          |
|      |   Delivery  Add CSV export       Nov 4  |                                          |
```

Search runs `vajra_kb.py search` (read only). Results show kind, title, date and verified or proposed.
960x600: same master-detail collapse as the inbox.

### 4.7 Settings (minimal)

```
| Settings | Claude Code path  [ /opt/homebrew/bin/claude ] [Choose...]  ✓ Found 2.4.1 |
|          | Theme             ( System | Dark | Light )                                |
|          | Notifications     [on] Needs approval  [on] Needs input                     |
|          |                   [on] Run done        [on] Run stuck (no event 10 min)     |
```

960x600: unchanged (single column, max width 640).

### 4.8 Command palette (global)

```
                +--------------------------------------------------------+
                | > fro                                                  |
                |  Go to worker  Frontend      ● In progress             |
                |  Go to worker  Frontend worker 1  ○ Pending            |
                |  Message Frontend...                                   |
                |  Stop Frontend...                               Cmd-.  |
                +--------------------------------------------------------+
```

Fuzzy over sections, workers, tasks, criteria, campaigns and actions. Actions that change state end
in "..." and open their confirm or form. 560 px wide, centered; at 960 it is 520 px.

## 5. Key flows

**F1 Start a run** (US-1.1, US-1.3)
1. Cmd-1 opens Home; focus lands on Ask.
2. Preflight runs on open (cached 60 s). Any FAIL disables Start run and names the fix.
3. Pick repository (recent list) and base branch; max rounds defaults to 3; Advanced holds workers at once.
4. Cmd-Enter starts. The app creates the run session and shows Crew with the Run node selected and its
   terminal read-only. The status bar shows "Run <id> Route".

**F2 Resume** (US-1.2)
1. Home, Resume list (or History row, "Resume"). Rows show phase and round from campaign.json.
2. The app runs `vajra-campaign.sh verify <slug>`; on exit 3 the row shows "Signature check failed"
   and Resume is disabled, with "Show details".
3. Resume launches the run session with the campaign slug; Crew opens on the restored tree.

**F3 Hop between workers** (US-2.1, US-2.4)
1. Cmd-2 focuses the crew tree. J/K or Up/Down move; Right/Left expand and collapse; Enter opens.
2. Cmd-K, type a role name, Enter. Or Cmd-[ / Cmd-] to go back and forward through hop history.
3. The worker pane keeps the last tab per worker and scroll position (stream pinned to latest unless
   the human scrolled up).

**F4 Ask the Lead about a worker** (US-3.1)
1. With a worker selected, Cmd-L focuses chat, prefilled with "@Backend ".
2. Type the question, Cmd-Enter. The Lead answers from inspect_worker; the answer links "Open Backend".

**F5 Message or redirect a worker** (US-3.2, US-3.4)
1. Via Lead: ask in chat ("tell Frontend to use the kit Button"); an action card appears.
2. Directly: worker header "Message" or "Redirect" (or palette). A sheet shows the target, the text and
   for Redirect the new instruction that replaces the current one. Cmd-Enter sends.
3. The card moves Queued, Delivered (hook consumed it), Acknowledged (worker's next turn). Each card
   has "View in audit" for the audit.log line and appears in the worker's Stream.

**F6 Stop a worker** (US-3.3, US-3.4)
1. Cmd-. (or header "Stop", or Lead's request card).
2. AlertDialog: "Stop Backend? Its next tool call is blocked and the task goes back to the run as
   stopped by you. Commits so far stay on vajra/<run_id>/t3." Buttons: Cancel (default focus), Stop worker.
3. Confirm; the card and the tree node show "Stopping", then "Stopped 14:22".

**F7 Approve a push plan** (US-5.1, US-5.2)
1. Notification or Cmd-5. Select the row; the detail shows the exact steps.
2. A, then the confirm dialog repeats the steps; Enter on "Approve plan".
3. The app opens the run session and submits a visible human turn "Execute approved plan <id>".
   Claude Code's permission prompt still fires for each protected command; the terminal is focused.

**F8 Reject with reason** (US-5.3)
1. R on the row opens a sheet with a required reason (min 1 character, placeholder "What should change").
2. Cmd-Enter records it via `vajra-campaign.sh progress`; the row moves to History as "Rejected: <reason>".

## 6. States

| Screen | Empty | Loading | Error | Engine missing / offline | Large data |
|---|---|---|---|---|---|
| Home | "No campaigns to resume." | Preflight rows show skeleton | Row names the failed check and the fix | Claude not found or doctor FAIL: Start disabled, failing checks listed with "Re-check" | Resume list virtualized |
| Crew | "No run yet. Start one from Home." with Cmd-1 hint | Tree skeleton | "Can't read transcript <path>: <reason>" on the node | Tree frozen, banner "Engine check failed; showing last known state" | Tree and stream virtualized; stream ring buffer 2 MB, older lines from disk on scroll |
| Chat | "Ask the Lead about any worker." | Typing row "Lead is reading Backend" | Card status Failed with reason and Retry | Input disabled: "Lead session stopped. Restart Lead" | Thread virtualized |
| Run | "No run selected." | Timeline and table skeleton | Criteria table: "Evidence file missing" per row | Last known state, timestamped | Graph switches to list over 40 nodes; table virtualized |
| Approvals | "Nothing waiting for you." | Row skeleton | "Plan could not be read" with path | Approve disabled; reason shown | Virtualized |
| History | "No campaigns yet." / "No results for "x"." | Row skeleton | "Knowledge search exited 2: <stderr line>" | Search disabled when vajra_kb.py missing | Virtualized, 50 per page from search |

Rules: show a skeleton only if loading passes 300 ms (no flash for fast loads); any wait over 1 s
shows text saying what it waits for. Never show placeholder rows that look like real data.

## 7. Keyboard map (macOS)

When a text field or terminal has focus, only Cmd chords are app shortcuts; single keys (J, K, A, R,
/) go to the field. Avoided system chords: Cmd-Q, Cmd-W, Cmd-H, Cmd-M, Cmd-Tab, Cmd-`, Cmd-Space,
Ctrl-arrows (Mission Control), Cmd-Shift-3/4/5 (screenshots). Ctrl-C in a taken-over terminal goes to
the process, never to the app.

| Keys | Action | Where |
|---|---|---|
| Cmd-K | Command palette | Global |
| Cmd-1 .. Cmd-6 | Home, Crew, Chat, Run, Approvals, History | Global |
| Cmd-, | Settings | Global (macOS standard) |
| J / K or Down / Up | Next / previous item | Crew tree, lists |
| Right / Left | Expand / collapse node | Crew tree |
| Enter | Open selected | Lists, tree, graph |
| Cmd-[ / Cmd-] | Back / forward in hop history | Global |
| Cmd-L | Focus chat (opens dock if closed) | Global |
| Cmd-J | Toggle chat dock | Global |
| Cmd-Enter | Send message / submit form | Chat, sheets, Start run |
| Cmd-. | Stop selected worker (opens confirm) | Crew, Chat |
| A / R | Approve / Reject selected (opens confirm or reason) | Approvals |
| / | Focus search or filter | History, lists |
| Cmd-F | Find in stream or diff | Worker pane |
| Ctrl-Tab / Ctrl-Shift-Tab | Next / previous worker pane tab | Worker pane |
| Esc | Close palette, sheet or dialog; clear filter; leave field | Global |
| Cmd-Shift-T | Take over / release run terminal (confirm) | Run node |

In the stop and approve dialogs, Esc and Cmd-. cancel (macOS convention) and the default button is
the safe one for Stop.

## 8. Notifications

macOS native notifications, only when the app is not frontmost. One per event, grouped per run, each
toggled in Settings. Clicking opens the related screen.

| Event | Text |
|---|---|
| Needs approval | "Run 0930 has a push plan waiting for you." |
| Needs input | "Run 0930 asked a question: <first 80 chars>" |
| Run done | "Run 0930 delivered: 6 of 6 criteria pass." (or "stopped: 4 of 6 pass") |
| Run stuck | "Run 0930 has had no activity for 10 minutes." |

No badges counting fake activity, no sounds by default, no mobile push in MVP.

## 9. Accessibility (WCAG 2.2 AA)

- Crew tree: `role="tree"`, nodes `role="treeitem"` with `aria-expanded`, `aria-level`,
  `aria-selected`; roving tabindex; type-ahead by role name.
- Status is never color alone: every status has shape, color and a text label; the label is in the
  accessible name ("Backend, in progress, task t3").
- Action cards are `role="article"` with a heading; status changes announce through one polite live
  region. Streams are not live regions (too noisy); "Jump to latest" announces count of new lines.
- Focus is always visible (2 px focus ring, offset 2 px) and never hidden by sticky headers or the dock
  (WCAG 2.4.11). Targets at least 24x24 px.
- Reduced motion: transitions become instant; nothing auto-scrolls with animation.
- VoiceOver labels on every icon button ("Stop worker", "Copy command"). Dialogs trap focus and return it.
- Contrast pairs are listed in DESIGN.md for both themes.
- Text scales with the system setting up to 200% without loss of content (panes reflow, tables scroll).

## 10. Copy rules

Sentence case. Plain verbs on buttons: "Start run", "Resume", "Approve plan", "Reject", "Stop worker",
"Take over". Say what happened with numbers and times. Name the file or command when something fails.
No em dashes, no exclamation marks, no "AI" or sparkle language, no mahabharat names. Questions from
the run are quoted verbatim.

Strings live in one file (`strings` module for the renderer, keyed by screen). Once screens exist,
every visible wording change is logged in `docs/design/changed-strings.md` for the owner's approval.
