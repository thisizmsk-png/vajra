# Changed strings (owner approval)

M3 kit (K2). Every visible string the kit owns lives in `app/ui/strings.ts`.

| String | Where | Note |
|---|---|---|
| Dismiss | Toast button | plain verb |
| Cancel | AlertDialog default cancel label | plain verb |
| Pending, Ready, Claimed, In progress, Done, Blocked, Question, Failed, Pass, Fail, Not run, Needs approval | StatusDot labels | from DESIGN.md "Status vocabulary" |

Screens add their own strings in M4.

M4.0 shell.

| String | Where | Note |
|---|---|---|
| Home, Run, Crew, Chat, Inbox, History, Settings | sidebar | one word per screen |
| Main | nav landmark label | screen readers only |
| This screen is not built yet. | placeholder screens | removed as each screen lands |
| This screen hit a problem / The rest of the app is fine. You can try again or go to another screen. / Try again | error boundary | no stack, no error text |

S1 Home.

| String | Where | Note |
|---|---|---|
| Checks / Check again | preflight section | |
| Start a run, Repository folder, What should the crew do?, Base branch or commit, Fix rounds, Workers at once (1 to 16), Advanced, Start run / Starting | start form | plain labels; hints say what the value is |
| Campaigns, Resume / Resuming, "N of M passing", No campaigns yet. Start a run and it appears here. | campaign list | |
| Started outside this app. Continue it from your terminal. | campaign row without a recorded session | no Resume button for it |
| Enter the repository folder., Describe what the crew should do., Enter a branch or commit., Fix rounds must be 1, 2 or 3., Use a number from 1 to 16. | inline field errors | each names the field |
| Could not run the checks. / Could not read the campaigns. | load failures | followed by the core's error text |

S6 History and Settings.

| String | Where | Note |
|---|---|---|
| History, Past campaigns, Campaign / Goal / Status / Passing / Updated, No campaigns yet., Unreadable | campaign table | |
| Search what the crew learned, Search, Searches the knowledge ledger of the repository below., Repository folder, Searching, No results. | knowledge search | |
| Knowledge search is unavailable (exit code N). | search failure | names the exit code |
| Enter something to search for. / Enter the repository folder. | inline errors | |
| Settings, Checks, Check again, Could not run the checks. | settings | the app has no preferences yet |

S2a Crew tree.

| String | Where | Note |
|---|---|---|
| Crew, Details, Status, Kind, Engine id | Crew screen | engine id appears only in the details of a selected node |
| No run is active. / Start a run from Home, or open a past run from History. | empty state | |
| Could not read the crew. | load failure | followed by the core's error text |
| Unknown (no signal) | node status | shown for a node nothing reports on, never Idle or Done (AC-2.1.5) |
| Stopped by you / Message pending / Active now | node overlays | appended to the status after " · " |

S2b Stream (worker output).

| String | Where | Note |
|---|---|---|
| Stream, Worker output, read only | Crew details | the log region label |
| ~~Load earlier output / Loading~~ | button, removed (#45 UI half) | scrolling near the window edge now fetches the rows around the view, so there is no button |
| Jump to latest | button | drops the paged history and re-reads the newest lines |
| Showing earlier output. Jump to latest to follow new lines. | notice | reworded in #45 UI half (was "Showing older output. Newer lines are held back until you jump to latest."); shown while the view is not following the newest lines |
| Some output in this range is no longer available. | notice, new in #45 UI half | shown when the core reports a hole in the window (output that fell off the buffer) |
| No output yet. / Could not read the output. | empty / failure | failure is followed by the core's error text |
| 1 line was skipped / N lines were skipped | marker row | shown where output fell off the core's buffer |

Graphite shell, Home and Crew (PR #39).

| String | Where | Note |
|---|---|---|
| Vajra | sidebar top | the app name |
| ⌘1 to ⌘7 | sidebar links | each link shows its key on hover and on focus; screen readers get `aria-keyshortcuts` |
| Start a run | Home heading | was "Home"; the sidebar still says Home |
| The crew builds in its own worktrees and reports back. This app never pushes. | Home, under the heading | new |
| N of M checks pass / All checks | Home checks line | new; replaces the "Checks" heading, opens when a check fails |
| Check again | Home action bar | moved from the checks section |
| ⌘↵ | Start run button | the key shown beside the label; not part of the button's name |
| No repository chosen | Home action bar | shown until a folder is entered; then the folder's name |
| Actions on Home / Actions for the selection | action bar names | screen readers only |
| Stopped by you / Message pending / Active now | node overlays | the separator changes from " · " to ", " (no middle dots) |

Command palette (PR #39).

| String | Where | Note |
|---|---|---|
| Search or run | sidebar button, with ⌘K | opens the palette; a search icon before it |
| Search commands | palette field | placeholder and the field's name |
| Go to Home … Go to Settings | palette rows | every screen but the current one, with its ⌘ key |
| Go to worker NAME | palette rows on Crew | the worker's status beside it |
| Check again | palette row on Home | the same action as the action bar |
| N commands / 1 command / No command matches "Q". | palette foot | "1 command" was "1 commands" |

History, Settings and the report on Graphite (PR #39).

| String | Where | Note |
|---|---|---|
| N campaigns / 1 campaign | History head | new, beside the title |
| Report / Verify | History row buttons | the visible word; the button's name still reads "Report SLUG" and "Verify SLUG" |
| Report, Close the report | History report pane | new; the pane opens beside the table instead of under it |
| Signature verified. | report pane | now a success badge, same words |
| Actions on History / Actions on Settings | action bar names | screen readers only |
| The app has no preferences yet. These checks decide whether a run can start. | Settings, under the heading | new |
| Check again | Settings action bar and palette | moved from the Checks section head |

Empty, loading and error states (PR #39).

| String | Where | Note |
|---|---|---|
| The app's core did not answer. It may still be starting; try again in a moment, and if it keeps happening, quit and reopen Vajra. | under any "Could not ..." title | new; shown when the core gives no answer at all (before, "unexpected error"). A core error still shows its own code and detail |
| Try again | every load failure | new; reloads that part only |
| Loading | loading rows | screen readers only |
| Go to Home / Open History | Crew with no run | new buttons under "No run is active." |
| Every run you start, here or from the terminal, is listed here with its report. | History with no campaigns | new, under "No campaigns yet.", with Go to Home |
| Open the crew for this run | Run view right after a start | new; the Run view is not built, so it points at the crew |
| The run is under way. Its timeline is not built yet; the crew shows each worker as it goes. | same | new |

Accessibility pass (PR #39).

| String | Where | Note |
|---|---|---|
| Skip to the screen | first Tab stop | new; out of sight until it has focus |
| SCREEN - Vajra | window title | was "Vajra Desktop" on every screen |

Click crawl fixes (PR #39).

| String | Where | Note |
|---|---|---|
| Checked at TIME | under the checks on Home and Settings | new; a re-check that finds nothing new used to look like a dead click |
| Tried again at TIME | in every "could not load" box, after Try again | new; a retry that fails the same way used to look like a dead click |

Polish pass (stacked on PR #39).

| String | Where | Note |
|---|---|---|
| Active / Done / Stopped / Failed / Abandoned | History status badges | were the raw lower-case states |
| N/M (with "N of M" for screen readers) | History, Passing | beside a small progress bar |
| 2 days ago (relative time) | History, Updated | was the stored timestamp; the exact time is the tooltip |
| Terminal | Home campaign rows | short tag; replaces the visible "Started outside this app. Continue it from your terminal.", which stays as the tooltip and for screen readers |
| Workers; N of M done | Crew details for the Lead, a run or a task | new; per-status counts of the workers under the selection |
| (Goal column removed) | History | the goal now leads the Campaign cell, with the slug under it |

Shell B frame (T4, wave 1).

| String | Where | Note |
|---|---|---|
| Director-General | view tab, project column card, palette | new; the UI name for the project's orchestrator (owner, 2026-11-08). Code ids stay `orchestrator`/`orch` |
| Leading run ID / No run active | Director-General card | new; honest until the core's orchestrator contract (C3) lands |
| N of M | Director-General card meter | criteria passing; "N of M criteria passing" for screen readers |
| 1 needs you / N need you | project tab pill | new |
| Stopped / Failed | project tab | the newest campaign's state |
| Unsorted | project tab, last | new; tooltip "Runs started outside the app, or before it recorded their folder. Nothing is moved until you sort them." |
| Add project | title bar + button | tooltip "Start a run in another folder; the folder becomes a project" |
| Search or jump | title bar ⌘K field | was "Search or run" in the sidebar |
| Board / Review | view tabs | Review is the old Run screen; `#/run` still opens it |
| Workers; N of M active | project column | new |
| Workers appear once a run starts | project column, no run | new |
| Sessions; Search NAME sessions | project column | new; the search shows past four sessions |
| Today / Yesterday / Fri 6 Nov / Earlier; "1 done", "1 stopped" | session day groups | new |
| Backlog / Running / Review / Done; Needs you; Only what needs you | Board | new |
| Read-only: cards move when the work and its checks do | Board | new |
| Plan to approve / Question; N of M criteria | Board cards | new |
| No campaign in this project yet; Start a run from the Director-General view; its tasks appear here. | Board, empty | new |
| Campaign, Criteria, Workers, Folder, Worktree, Branch; N more in Review | Inspector | new |
| No runs active / 1 run active / N runs active | status bar | new |
| Phone not connected | status bar | new; opens Settings > Mobile |
| Providers / Models and capacity / Mobile / Notifications / Appearance / Keyboard / Diagnostics, each with a one-line lede | Settings sections | new; only Providers (the real checks) and Keyboard are built, the rest say "Not built yet" |
| Show or hide the project column / inspector; Go to project 1 to 9 | Settings > Keyboard, palette | new |
| VIEW, PROJECT - Vajra | window title | was "SCREEN - Vajra" |
| Project column / Inspector | status-bar and view-tab toggles (accessible name) | one constant name; aria-pressed carries the state. "Show/Hide …" stays in the tooltip and the palette commands (#86 review) |
| (removed) "This screen is not built yet.", "Main" nav label, "Vajra" brand | old sidebar | the static nav sidebar is gone |
| Could not read which folder each run used, so this project's runs cannot be listed. | Home inside a project, campaign list | new; shown with Try again when resume targets fail, instead of an empty project (#86 review) |

"Lead" stays for now (orchestrator, 2026-11-08): the run-scoped "Lead" in the Crew tree, Review and the palette keeps
its name until FG-02 replaces the run Lead with the per-project orchestrator. Those surfaces then become
"Director-General" too, so nothing is renamed twice. Every user-facing label of the per-project orchestrator says
"Director-General"; code and the contract keep `orchestrator`.

Terminal dock (standalone component, `app/ui/src/terminal/`).

| String | Where | Note |
|---|---|---|
| Terminal, Terminals, New terminal, Hide terminal, Terminal here | dock bar and the "Terminal here" action | new |
| The shell ended. / The shell ended with code N. / The shell was stopped. | banner over an ended terminal | new |
| Start a new shell | button on an ended terminal | new |
| No terminals are open. A terminal runs your own shell in this project folder, as you. It is not sandboxed and agents cannot see it. | empty state | new; states plainly that it is outside the guard |
| This folder is not a project or one of its worktrees, so a terminal cannot start there. (and the other open errors) | error state | new; one line per core error code |
| The shell is not reading input right now. What you typed was dropped. | transient note | new |
