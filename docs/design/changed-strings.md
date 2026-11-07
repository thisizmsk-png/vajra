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
