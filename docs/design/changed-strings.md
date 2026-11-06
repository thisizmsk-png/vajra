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
| Enter the repository folder., Describe what the crew should do., Enter a branch or commit., Fix rounds must be 1, 2 or 3., Use a number from 1 to 16. | inline field errors | each names the field |
| Could not run the checks. / Could not read the campaigns. | load failures | followed by the core's error text |
