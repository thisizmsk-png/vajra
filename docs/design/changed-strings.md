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

Director-General conversation (FG-02, wave 2).

| String | Where | Note |
|---|---|---|
| Director-General; Conversation N; New conversation | conversation header | new; the generation number answers "what does it remember" (ADR-15 §2.5) |
| Waiting for a question / Answering a question / A run of this project is going | header state | the core's reason, sentence case |
| You / Director-General / Tools used / Problem | transcript turn labels | new; tool use folds into "Tools used" |
| Ask the Director-General; Ask about this project, or what to build next; Send; Stop answering | composer | new; ⌘↵ sends |
| No questions yet; Ask what is happening in this project, or what should be built next. It can propose a run; only you can start it. | empty conversation | new |
| The Director-General can talk once this project has a run started from this app. Start a run yourself; then you can ask it about the work. | project with no run | new; honest until the commands are keyed by project |
| The Director-General is still answering. Ask again when it finishes, or stop it. | `lead_busy` | new |
| Start a run?; Needs you; What the crew will be asked; Base; Fix rounds; From conversation N; Start run; Dismiss | run proposal card | new; the ask is shown in full, exactly as it will run |
| The Director-General proposes; nothing runs until you start it. | above proposals | new (ADR-15 D4) |
| Sent. Waiting for Vajra to confirm. / Run started. It shows on the board. / Dismissed. Nothing ran. / This proposal changed after you saw it. Nothing started; read it again. / No answer came back. Check the board before you try again. / Not done: REASON | proposal outcome | new; never says done before the core does (ADR-15 §3.2) |
| Start a run yourself | header link; the launcher's own view (`#/p/<id>/launch`) | the Director-General view was the launcher in T4 |

FG-08 settings (owner review).

All new settings copy lives in `S.preferences` in `app/ui/strings.ts`; section titles and ledes live in
`S.settings.sections`. Provider labels name integration targets only. The unavailable adapter label uses
parentheses, not an em dash. Notification delivery retains its existing true preference default; item text
starts off and OS permission remains separate.

| String | Where | Note |
|---|---|---|
| General / Default editor and shell for supported new opens. Active sessions keep their settings. | Settings section | new section |
| Choose global defaults or override them for this project. | Settings lede | saved preferences replace placeholder copy |
| Graphite, in dark or light. Follow the system or choose an appearance. | Appearance lede | system or explicit choice |
| Default agent and task capacity. Project overrides apply to new work; native capabilities and system limits still apply. | Models and capacity lede | new-launch scope |
| Preference scope / Global defaults / Saved preferences / Overrides for NAME. Changes apply only after Save. | scope and landmark | project name is dynamic |
| Global default / Project override / Unsaved change / Use global FIELD | fields | shows value source and inherit action |
| Save or discard changes before switching scope. | scope | scope selector disabled while dirty |
| Reading preferences… / Reload preferences / Save preferences / Discard changes / Reset this scope / Saved. | lifecycle/actions | reset touches selected scope only |
| Theme / Follow system / Dark / Light | Appearance | saved choices |
| Default editor / System default / Editor CLI / Terminal shell / Login shell / /bin/sh / /bin/bash / /bin/zsh | General | bounded choices; adapter consumption follows |
| Launch preferences are saved for supported new opens. Running sessions keep their launch settings. | General hint | no action on save |
| Default agent / Claude Code / Codex CLI (availability check required) | provisional provider selector | integration targets; replaced by provider track |
| Model: MODEL. Additional agent and model selection requires a supported adapter. / Native CLI default | Models | honest unavailable controls |
| Concurrent workers / Fix rounds / Read-only guard caps: N active run, M workers per run. Preferences cannot raise these limits. | Models and capacity | counts from read-only caps |
| Defaults apply to new tasks. System limits and native permissions still apply. | capacity hint | active runs retain settings |
| OS notifications / Include item text | Notifications | independent delivery/text preferences |
| Item text is off by default. Delivery also requires OS permission. Without text opt-in, notifications contain only project name and item kind. Native permission and redaction still apply. | Notifications hint | no implied OS grant |
| Search shortcut / Settings shortcut | Keyboard | bounded remapping; remaining shortcuts unchanged |
| Action STATE · ID · REASON / Retry same action | receipts | reason suffix only when present; retry retains ID |
| The core returned unsupported preferences. Update the desktop and reload. / Preferences could not be read. The saved file was preserved. | read errors | no automatic destructive reset |
| The action outcome is not confirmed. Retry the same action or reload to review. / Save outcome is unconfirmed. Retry the same action or reload to review. | uncertain action | no fresh action ID |
| Preferences changed in another window. Reload and review before saving again. | conflict | no silent retry |
| Action STATE. Reload and review the saved values before another action. / Unsupported preferences response. Reload before another action. | receipt/recovery errors | no new write before review |
| The saved agent or model needs a supported adapter. Choose the native CLI default in Settings before starting. | launcher | prevents unsupported adapter fallback |

Terminal dock (standalone component, `app/ui/src/terminal/`).

| String | Where | Note |
|---|---|---|
| Terminal, Terminals, New terminal, Hide terminal, Terminal here | dock bar and the "Terminal here" action | new |
| The shell ended. / The shell ended with code N. / The shell was stopped. | banner over an ended terminal | new |
| Start a new shell | button on an ended terminal | new |
| No terminals are open. A terminal runs your own shell in this project folder, as you. It is not sandboxed and agents cannot see it. | empty state | new; states plainly that it is outside the guard |
| This folder is not a project or one of its worktrees, so a terminal cannot start there. (and the other open errors) | error state | new; one line per core error code |
| The shell is not reading input right now. What you typed was dropped. | transient note | new |

FG-09 providers in the UI (Settings > Providers, Start, Resume, Run, Crew).

| String | Where | Note |
|---|---|---|
| Default agent / Claude Code / Codex CLI (availability check required) | Models and capacity | removed; the one provider choice is now "Agent for new runs" under Providers |
| Agent for new runs | Settings > Providers | new; replaces "Default agent", scoped global or per project |
| New runs in this scope use this agent. A run keeps the agent it started on, and Resume continues on it. | under the choice | new |
| NAME (not installed) / NAME (not tested with this version) | agent choices | new; not installed cannot be chosen, untested stays selectable |
| Agents on this Mac / Checking the agent CLIs / Could not check the agent CLIs. | provider panel | new |
| This build cannot read the agent check from the core. Update the desktop app. / The agent check answered something this build cannot read. | provider panel errors | new (another schema major, or a malformed answer) |
| Version, Path, Login, Not read, Not on your PATH, Tested, Not tested with this version, Version not compared, Tested with RANGE | provider facts | new |
| Signed in / Signed out / Could not tell / Not installed | login state | new; the CLI's own check |
| Sign in with NAME itself. Vajra never handles the login. | signed out | new |
| KEY is set where Vajra was opened. Vajra does not pass it to NAME; runs use your NAME login. | billing note | new; a variable name only, never a value |
| Available: LIST. / Stop waits for the next tool call. / Stop ends the process. | what a provider can do | new |
| Attach terminal, Transcript, Inspect, Message, Redirect, Stop, Resume, Vajra workflow, Start | disabled controls, with the provider's own reason beside each | new |
| NAME does not support this. | an off control with no reason from the core | new |
| Runs on NAME | Start, Run, Crew | new; shown only for a provider other than Claude |
| NAME is not installed, so this run cannot start. Install it, or choose another agent in Settings, Providers. | Start | new; Start is off, nothing else is tried |
| The saved agent "ID" is not one this build can run. Choose another in Settings, Providers. | Start | new |
| NAME is not installed, so this run cannot be resumed here. | Resume, beside the disabled button | new |
| Model: MODEL. Choosing a model needs a supported adapter. The agent is chosen under Providers. | Models and capacity | changed from "Additional agent and model selection requires a supported adapter." |
| The saved model needs a supported adapter. Choose the native CLI default in Settings before starting. | launcher | changed: an agent other than Claude no longer blocks Start |
| NAME cannot take a message while it runs. / NAME cannot be redirected while it runs. / NAME has no inspect step Vajra can read. / NAME does not write a transcript Vajra can read. / NAME does not run the Vajra workflow, so there is no crew to show. / Stop ends the NAME process. There is no hand-off at the next tool call. / NAME is not installed. / This build cannot run NAME. | limit codes (providers.md stable codes) | new; the core's own sentence shows only for a code not in this list |
| Uses your Claude Code login / Uses your Codex login | login route codes | new |
| The login check did not finish in time, so Vajra cannot tell. / The login check answered something Vajra could not read, so it cannot tell. | login detail codes | new |

Settings keyboard (PR 100 review).

| String | Where | Note |
|---|---|---|
| Use global shortcuts (both) | Settings > Keyboard, project scope | replaces the two per-shortcut "Use global ..." buttons: shortcuts are one override, so the one control says it resets both (PR 100 review) |

Settings General (PR 100 review): the editor and shell choices are hidden until a new terminal or editor reads them.

| String | Where | Note |
|---|---|---|
| Default editor and shell for new terminals and editors. | Settings > General, lede | was "Default editor and shell for supported new opens. Active sessions keep their settings."; the section now shows "Not built yet" |
| Default editor, System default, Editor CLI, Terminal shell, Login shell, /bin/sh, /bin/bash, /bin/zsh, "Launch preferences are saved for supported new opens. Running sessions keep their launch settings." | Settings > General | removed with the rows they labelled |

Settings Notifications (PR 100 review): the section's controls are hidden until notifications are delivered; it shows "Not built yet". Stored values and defaults are unchanged.

| String | Where | Note |
|---|---|---|
| OS notifications, Include item text, "Item text is off by default. Delivery also requires OS permission. Without text opt-in, notifications contain only project name and item kind. Native permission and redaction still apply." | Settings > Notifications | removed with the controls they labelled |

Worker diff count (PR 6c-0, ADR-16 16.10).

| String | Where | Note |
|---|---|---|
| +N −M | worker pane header | exact count |
| ≥ +N lines | worker pane header | a lower bound: some new files were not counted |
| At least N lines added. U new files were not counted (too many or too large). | accessible label of the `≥` form | "1 new file was not counted" for one |
| N lines added, M removed, in F files. | accessible label of the exact form | new |

Settings > Providers > Set up (ADR-17 step 1). Nothing installs in this step: a missing or broken CLI shows the vendor's own commands and docs address as text.

| String | Where | Note |
|---|---|---|
| Set up | panel title | |
| Agent CLIs Vajra knows how to find. Install a missing one with its vendor's own steps, then sign in with the CLI itself. Vajra never handles the login. | panel lede | |
| Looking for agent CLIs / Could not look for the agent CLIs. | loading and error | |
| This build cannot read the setup check from the core. Update the desktop app. / The setup check answered something this build cannot read. | contract errors | |
| Installed / Installed. Vajra can't run it as an agent yet. / Broken / Install it yourself | row state | the second is ADR-16 N5 |
| It is on your PATH but did not report a version. Reinstall it with the vendor's steps. | broken row | Repair in step 1 is the vendor's steps |
| Run one of these in a terminal. One-click install comes after a security review of each source. | install-it-yourself row | |
| Homebrew / npm / Vendor script | command labels | |
| "NAME install steps" | label of the vendor docs address | shown as text, not a link |
| "Sign in to NAME" / Opens the CLI's own sign-in in a terminal tab. | sign-in button and hint | |
| "Sign in: NAME" | terminal tab label | never saved in the tab layout |
| Vajra engine scripts / Installed / Not installed / Run this in a terminal to install them. | engine-scripts row | detection only (ADR-16 N13) |
