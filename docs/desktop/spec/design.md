# Vajra Desktop: Design (spec Phase 2)

Status: draft, returned as `needs_decision`. Phase 2 of 3. Owner: Architect role. Date: 2026-11-05.
Inputs: `docs/desktop/spec/requirements.md` (J1..J6, US-x.y, AC-x.y.z, NFR-*), the shared brief, and the repo
files cited inline. Prose uses generic role names (Lead, Backend, Verifier, ...). Engine ids appear only in the
mapping table (4.4) and in quoted file contents.

Approach: the smallest design that covers the six jobs. The app reads Vajra's files, launches Claude Code, and
adds one control path. It writes nothing under `~/.claude`. Where Vajra itself has to change, the change goes in
the short "Engine changes" list (1.3). Each item there needs owner approval.

## 0. Open assumptions

| # | Assumption | State |
|---|---|---|
| D1 | Claude Code writes one transcript per session at `~/.claude/projects/<cwd-slug>/<session_id>.jsonl` and a separate transcript per subagent (workflow agent), and each subagent record carries enough to identify it (agent type, the workflow step label or its prompt). Not checked against a live run in this phase. | open, to verify in spike |
| D2 | The hook payload for a workflow subagent's tool call carries the run session's `session_id` and an agent identifier (`agent_id` and/or `agent_type`). `hooks/post-tool-use.sh:4-5` documents `agent_type` in the PostToolUse payload; `agent_id` is unconfirmed. | open, to verify in spike |
| D3 | `hookSpecificOutput.additionalContext` returned from PreToolUse or PostToolUse reaches the subagent that made the call. The repo confirms it only for UserPromptSubmit (`scripts/vajra_route.py:591`). | open, to verify in spike |
| D4 | Crew tree refinement of AC-2.1.1: one agent node per task (owner role + task id), with one child per round (r1..r3). Verifier and reviewer passes sit under one "Verification" node per round. Helper steps (knowledge search, preflight, campaign records, merge, write-back, cleanup) are grouped under "Run steps", collapsed. | open, owner to confirm |
| D5 | Audit linkage (AC-3.4.1): the guard's audit line has no id field (`hooks/pre-tool-use.sh:43-46`: `TS VERDICT tool reason=...`). The design puts the app's `action_id` into `audit.log` through the delivery line that `hooks/desktop-control.sh` writes (E1). The Lead's MCP calls get a guard ALLOW line (E2) that is joined by tool name and time. Reads the human makes directly in the UI (selecting a worker) count as navigation: they are not control actions and get no `audit.log` line. That is a narrower reading of AC-3.4.1. | open, owner to confirm |
| D6 | The Lead chat session runs an inline agent definition (`--agents`) that is built from the Lead's charter, has tools Read, Grep, Glob and the app's MCP tools, and has no Agent or Workflow tool. The installed Lead agent (`bundled-agents/krishna.md:4`) lists only Read, Grep, Glob and Agent(...), so it could not call MCP tools. | open, to verify in spike |
| D7 | The app is distributed outside the Mac App Store (Developer ID, notarized) and is not App Sandbox-ed. It has to spawn `claude`, `git` and `python3`, and read `~/.claude`. | open |
| D8 | App data dir is `~/Library/Application Support/<app>/` where `<app>` is the final bundle name (placeholder `VajraDesktop`). | open |
| D9 | `campaign.json` records no repository path (observed: keys `schema, slug, goal, status, created, updated, featuresDefSha256`). The app keeps its own `runs.jsonl` mapping run to repo. For runs it did not launch, it recovers the repo from the session transcript `cwd`, and otherwise shows "repo unknown". | open |
| D10 | All budgets in section 9 are PROVISIONAL until the stack spike measures them. | **decided:** measured; see the canonical NFR-PERF table |
| D11 | Engine change E4 (persist the delivery report in the campaign) is an open question. The MVP does not depend on it. | **decided:** approved (Q3) |

## 1. Context and constraints

1. **Vajra stays the engine.** Hooks, saved workflows (`workflows/vajra-deliver.js`), agents, campaigns
   (`scripts/vajra-campaign.sh`), the task graph (`scripts/vajra_tasks.py`) and the knowledge ledger
   (`scripts/vajra_kb.py`) remain the source of truth. The app is a view plus a launcher plus one control
   path.
2. **Read-only over `~/.claude`.** The app process never creates, modifies or deletes anything under
   `~/.claude`. Mutations happen only through Vajra's scripts or Claude Code, which the app starts as child
   processes. The app never opens `~/.claude/vajra/.hmac-key` (NFR-SEC-2). Every app-owned file lives in
   the app data dir.
3. **macOS only for MVP** (Apple Silicon first). FSEvents, Unix sockets, `posix_spawn` and the PTY APIs are
   assumed. Nothing in the logical design depends on a particular UI toolkit.
4. **Generic names.** The UI shows role names from 4.4, never engine ids, except the inspector's "engine id"
   field (AC-2.1.3).
5. **Workflow facts the design relies on** (`workflows/vajra-deliver.js`):
   - Args: `ask, run_id, session_id, max_rounds (1..3), base, maxInFlight (1..16, default 4), campaign,
     hints`. A slash launch passes one string, which `parseArgs` JSON-parses (lines 14-20, 50-65).
   - Step labels are code-built: `understand`, `plan.1|2`, `task.<tid>.r<round>`, `verify.r<N>`,
     `verify.r<N>.commands`, `review.<agent>.r<N>`, `kb.search`, `preflight`, `campaign.load`,
     `deploy_plan`, `writeback`, `cleanup` (lines 182-184, 539, 652-668).
   - Task graph ids are `<run_id>.<tid>`, and branches are `vajra/<run_id>/<tid>[-rN]`.
   - The result is `{status, node, branch, commit, report (markdown text), pending_approval, artifacts}`
     (lines 946-956).
   - Progress lines are machine-parsable: `integration <branch> @ <sha> from <sha>`,
     `run <id> round <n>: x/y criteria passing`, `run <id> delivered <branch> @ <sha>`, and
     `pass: <id> (evidence: <cmd> (exit 0))`.
6. **Real state observed** (2026-11-05, read-only, from `~/.claude/vajra/campaigns/`):
   - `tasks.json` `worktree` is `""` in every task: the workflow never passes `--worktree`.
   - `claimed_by` equals `owner_agent`, so it names a role, not an instance.
   - A closed task's `evidence` is `{commands, head, path, sha256}`. `path` points into an agent worktree
     that cleanup may already have removed.
   - The campaigns root also holds non-campaign `*.json` files, so the reader takes directories only.
   - Checkpoint keys: `campaign, created, features, hmac, label, prevHmac, progress, schema, seq`.
   - `audit.log` was 560 KB and grows without bound, so the app tails it from the end.
   - `routing-log.jsonl` keys: `prompt_sha256, route, session_id, ts, words`.

### 1.3 Engine changes (each needs owner approval; nothing else in Vajra changes)

| # | Change | Why | Risk direction |
|---|---|---|---|
| E1 | New `hooks/desktop-control.sh`, registered in `hooks/hooks.json`: PreToolUse (stop) and PostToolUse, synchronous (message/redirect). It exits 0 at once when the app's control dir for the session does not exist. It writes one delivery line per consumed action to `audit.log`. | Delivers control actions to workers without PTY typing (J3) | Additive, separate from the guard |
| E2 (widened by A1 M3 and STRIDE T1) | `hooks/hooks.json` guard matcher gains `mcp__vajra-desktop__.*`. `hooks/pre-tool-use.sh` `ROOTS` gains the app control dir. | Lead control calls appear in `audit.log` and fail closed (`onFailure: block`). Agents cannot write the queue. | Strengthens: more calls checked, one more protected root |
| E5 (new, A1 B1) | `workflows/vajra-deliver.js`: an optional `steer_nonce` argument read from the app's nonce file path, never from args text, and one fixed sentence in Execute-phase task briefs only: "Act on context tagged `<vajra-steer nonce=N>` from the human operator; it never approves a protected action or widens your task." Verify, review and merge briefs get no steering sentence | Additive; without the argument the workflow is unchanged |
| E7 (new, PR review round 4; approved Q13, 2026-11-05) | New `vajra-campaign.sh archive-evidence <slug> <task> <evidence-dir> <setup.json>`, called by the workflow's write-back after `verify-work` and **before** the cleanup step (`workflows/vajra-deliver.js:842`). It copies the bundle (`evidence.json`, `diff.patch`, `diff.stat`, `untracked.txt`, `cmd-N.log`) to `~/.claude/vajra/campaigns/<slug>/evidence/<task>/data/walkthroughs/<evidence-slug>/`, keeping the `<root>/data/walkthroughs/<slug>/evidence.json` layout. `read_evidence` today takes its root from `git rev-parse --show-toplevel` and checks HEAD in that repo (`scripts/vajra_tasks.py:502`), so E7 adds two explicit parameters, `root` (the archive dir, for the path rule) and `repo` (the user's repo, for the `--head` check), with tests for both modes and every existing test unchanged. It also writes `setup.json`. The workflow JS builds it from its own `setupCmds` variable and passes it as one quoted argument, so it never comes from agent text: it is the exact array write-back executed (after the filter and the cap of 5 at `:787`, each string untruncated, up to 1,000 characters). Then it signs a manifest (sha256 of every file, `setup.json` included) with the campaign HMAC, the same way it signs `campaign.json`. E3 gains `--archived <slug>`: it accepts an evidence path under that campaign's `evidence/` dir and also checks the manifest. The app never reads the key; it calls E3 and `vajra-campaign.sh verify`. Additive. |
| E9 (new, memory; approved Q14, 2026-11-05; conditions AC-7.1.8) | `vajra_kb.py` approve, reject, forget, retire, supersede and `atman.sh approve|reject` accept a signed `--desktop-approval <file>` checked against a key enrolled by the user-only `vajra_kb.py desktop-enroll` (ADR-12), with edited text re-gated and used approval ids kept in the HMAC chain; the guard blocks agents from naming `--desktop-approval` or `desktop-enroll` | Human approval from the app without a PTY | bhishma review, PoCs in H5 |
| E10 (new, memory, post-MVP) | `vajra_kb.py` appends `applied` events (record ids, session id, time; never text) when `prime` or `search` returns records, plus a read-only `applied --json` | Rules view "last applied" and "what primed this session" | Read-mostly; tests |
| E11 (new, memory, post-MVP) | Atman reads the app's control ledger (redirect intents, read-only) as a new signal source and proposes rules from repeated redirects | Learning trigger AC-7.4.3 | Proposals only |
| E12 (new, memory, post-MVP) | Atman signal source: verify failures per criterion from the archived evidence (E7: criterion id, command, first error line, run), proposing a pitfall when the same normalized pattern appears in 2 or more runs | Learning trigger AC-7.4.2 | Proposals only |
| E3 | New read-only subcommand `vajra_tasks.py check-evidence <evidence.json> [--head SHA] --json`. It reuses `read_evidence()` (`scripts/vajra_tasks.py:502`), writes nothing and prints `{ok, green, reasons}`. | AC-4.3.3 "unverified evidence" without the app reading the key. Today only `close` and `vajra_kb.py add --source verified` verify evidence, and both mutate state. | Additive, no state change |
| E4 (approved Q3, 2026-11-05) | The workflow's Deliver step writes `report.md` into the campaign through `vajra-campaign.sh` (a new `report` subcommand, signed with the state). | Reports survive without the app having been open | Additive. Approved; until it ships, the app falls back to the transcript copy (ADR-4). |

For E3, the shared green rule raises `Fail(3, reason)` on refusal, so `check-evidence` catches it and returns
`green:false, reasons:[msg]` with exit 0. Exit 1 means unreadable input and exit 2 means usage, matching the
script's convention (`scripts/vajra_tasks.py:45`). One unit test covers each refusal branch of
`read_evidence`.

## 2. Architecture (stack approved; process model in ADR-9)

> **Stack approved** (G2, 2026-11-05, both models): Tauri v2 + Rust core + React/TS + xterm.js. Performance
> approval is withheld until NFR-PERF-1 and NFR-PERF-6 pass on the real build. The process model is ADR-9.
> Detail below this note is owned by the MVP build (vajra-6).

The spike must answer:

| Question | Options |
|---|---|
| Process model | Supervisor in the app process vs a small daemon (launchd agent) that outlives the window. Decided in ADR-9 (section 6): no daemon in the MVP; the Rust core in the app process owns the PTYs and the app stays alive in the menu bar when its window closes. |
| UI toolkit | Tauri v2 (system WKWebView, Rust core) vs Electron vs native (SwiftUI/AppKit) |
| PTY | `portable-pty` / `pty-process` crates (Rust), `node-pty` (Electron), `forkpty` (native). Resize, SIGWINCH, child reaping, no blocking reads on the UI thread. |
| Terminal renderer | xterm.js (WebGL addon) vs a native renderer. Must allow turning OSC 8 and OSC 52 off and must virtualize scrollback. |
| File watching | FSEvents directly vs `notify` crate vs chokidar. Must support coalescing and non-recursive scope. |
| IPC framing | UI <-> core (Tauri commands/events vs Electron IPC) and MCP binary <-> app socket (length-prefixed JSON vs NDJSON) |
| Updater | Tauri updater (signed manifest) vs Sparkle vs Squirrel. Signature verification is mandatory. |
| Signing | Developer ID, hardened runtime, notarization, entitlements (no App Sandbox, D7) |

Evidence required, on this Mac, with the same method for every candidate (NFR-PERF-7):
- idle RSS summed over all app processes (NFR-PERF-1)
- RSS per attached agent stream (NFR-PERF-2)
- cold start to interactive (NFR-PERF-4)
- frame time p95 and p99 with 10 agents at 200 lines/s each (NFR-PERF-3)
- 1 h soak RSS growth (NFR-PERF-6)

Comparison targets: at least two competitors installed locally, Competitor B desktop and Competitor F. Both
are Electron (bundle inspection: `Electron Framework.framework` and Squirrel in `Contents/Frameworks`). One
snapshot from the brief: Competitor B 0.14.10-nightly with sessions, 13 processes, about 1608 MB `ps`
RSS sum (over-counts shared pages; the spike should also report `footprint`/phys_footprint).

The spike also settles every "to verify in spike" item in this document (list in 5.9), plus: how long a run started in a plain terminal takes to show up and how events reach the UI (NFR-PERF-9, POS-2.2).

## 3. Logical components (stack-agnostic)

| Component | Responsibility | Inputs | Outputs |
|---|---|---|---|
| Session supervisor | Owns the PTYs of the Run session and the Lead chat session. Spawns `claude` with an argv array. Reads PTYs on dedicated readers that never block. Handles resize, exit and reap. "Take over" toggles human keystroke passthrough. | Start/resume form, approve action | PTY byte streams, exit codes |
| Transcript tailer | Tails the run session jsonl and the subagent transcripts. Parses records into `StreamLine` and `WorkerSeen` events. Checks a schema fingerprint. | Files under `~/.claude/projects/<cwd-slug>/` (allowlist) | Events, captured workflow result |
| Vajra state reader | Reads campaign files, `tasks.json`, `features.json`, `progress.md`, checkpoint names, `.active`, the `audit.log` tail, the `routing-log.jsonl` tail and the knowledge ledger's `audit.jsonl` tail (refusals for POS-3.2; hashes and reasons only). Coalesced watch. Keeps the last good snapshot on parse error. | Allowlisted paths | Snapshots, diffs |
| Script bridge | The only place that executes programs: `vajra-campaign.sh`, `vajra_tasks.py`, `vajra_kb.py`, `vajra-doctor.sh`, `git`. Fixed executable paths, an argv array, never a shell string, timeout, capped output, scrubbed env. | Typed requests | Typed results (exit code, stdout JSON) |
| Control service | Validates control requests, resolves the target worker, writes queue and stop files, watches acks, expires stale entries | UI actions, MCP calls | Queue files, ledger records, events |
| MCP server | `vajra-desktop-mcp` stdio binary launched by the Lead chat session. It forwards tool calls to the Control service over an owner-only Unix socket. It holds no state. | stdio JSON-RPC | Tool results |
| Control ledger | Append-only JSONL in the app data dir, one line per state change of an action | Control service | `control/ledger.jsonl` |
| Event bus | Fan-out from producers to subscribers with a bounded queue per subscriber and drop-to-snapshot | All producers | UI, ledger writer |
| UI | Screens from the brief (Home, Crew, Lead chat, Run, Approvals, History), command palette, status bar | Event bus, snapshots | User intents |

```mermaid
flowchart LR
  subgraph App["Vajra Desktop (one app, process model per spike)"]
    UI[UI] <--> BUS[Event bus<br/>bounded queues]
    SUP[Session supervisor<br/>PTYs] --> BUS
    TT[Transcript tailer] --> BUS
    SR[Vajra state reader] --> BUS
    CS[Control service] --> BUS
    CS --> LED[(Control ledger<br/>ledger.jsonl)]
    CS --> Q[(control/&lt;session&gt;/<br/>queue, stops, acks)]
    UI --> SB[Script bridge<br/>argv only]
    UI --> CS
    UI --> SUP
    SOCK[[Unix socket 0600]] --> CS
  end
  MCP[vajra-desktop-mcp<br/>stdio] --> SOCK
  subgraph CC["Claude Code processes (children of the app)"]
    RUN[Run session<br/>vajra-deliver workflow + workers]
    LEAD[Lead chat session]
  end
  SUP -- PTY --> RUN
  SUP -- PTY --> LEAD
  LEAD -- stdio --> MCP
  RUN -- tool calls --> HOOK[hooks/desktop-control.sh]
  HOOK -- consume + ack --> Q
  HOOK -- delivery line --> AUD[(~/.claude/vajra/audit.log)]
  GUARD[hooks/pre-tool-use.sh] -- ALLOW/BLOCK --> AUD
  LEAD -- mcp tool calls --> GUARD
  RUN -- writes --> TR[(~/.claude/projects/*.jsonl)]
  RUN -- via scripts --> VS[(~/.claude/vajra/campaigns)]
  TT -. read .-> TR
  SR -. read .-> VS
  SR -. read tail .-> AUD
  SB -- spawn --> SCR[vajra-campaign.sh, vajra_tasks.py,<br/>vajra_kb.py, vajra-doctor.sh, git]
```

### 3.1 Data flows

```mermaid
sequenceDiagram
  autonumber
  actor H as Human
  participant UI
  participant SB as Script bridge
  participant SUP as Supervisor
  participant RUN as Run session
  participant LEAD as Lead chat
  participant MCP as MCP server
  participant CS as Control service
  participant HK as desktop-control.sh
  participant AUD as audit.log

  Note over H,AUD: Start run (US-1.1)
  H->>UI: repo, ask, base, max rounds
  UI->>SB: preflight (claude --version, vajra-doctor.sh, git rev-parse)
  UI->>SUP: spawn ["claude","--session-id",uuid,"/vajra-deliver <json args>"] cwd=repo
  SUP->>RUN: PTY (read-only view)
  RUN-->>UI: transcript lines via tailer, tasks.json via state reader

  Note over H,AUD: Ask Lead about a worker (US-3.1)
  H->>LEAD: "what is Backend stuck on?" (typed in Lead chat PTY)
  LEAD->>MCP: list_workers, inspect_worker (guard logs ALLOW, E2)
  MCP->>CS: socket request
  CS-->>MCP: worker summary + last transcript lines (as untrusted data)
  CS->>UI: action cards (read actions, ledger line)

  Note over H,AUD: Message a worker (US-3.2)
  H->>LEAD: "tell Frontend to use the kit Button"
  LEAD->>MCP: message_worker(worker, text)
  MCP->>CS: request
  CS->>CS: resolve worker, write queue/<seq>-<action_id>.json, ledger "queued"
  RUN->>HK: PostToolUse payload (worker's next tool call)
  HK->>HK: match + rename (claim), emit additionalContext
  HK->>AUD: ALLOW desktop-control reason="deliver message id=<action_id>"
  HK->>CS: acks/<action_id>.json (fs event)
  CS->>UI: card "delivered", ledger "delivered"

  Note over H,AUD: Stop a worker (US-3.3)
  H->>UI: Stop -> confirm dialog (always human)
  UI->>CS: stop(worker)
  CS->>CS: write stops/<worker_key>.json, ledger "queued"
  RUN->>HK: PreToolUse payload
  HK-->>RUN: exit 2 "stopped by the human (action <id>)"
  HK->>AUD: BLOCK desktop-control reason="stop id=<action_id>"
  HK->>CS: ack (first block only)

  Note over H,AUD: Approve a plan (US-5.2)
  H->>UI: open plan, read full text, Approve
  UI->>UI: require run status terminal (done/need_input)
  UI->>SUP: write one visible human turn "Execute approved plan <run>.<idx> (<action>)" to Run PTY
  RUN-->>H: Claude Code permission prompt for push/PR/deploy (human answers in terminal)
  UI->>CS: ledger "approved" (plan id, who, time)
```

### 3.2 Positioning additions (POS, from positioning.md)

- **Evidence view is a run's default view** (POS-1.1). Opening a run lands on its criteria. The crew view
  and timeline are one key away. Status values: pass, fail, not run, stale, unverified. An aggregate badge
  never shows green while any criterion is red (POS-1.5).
- **Unverified and stale** (POS-1.3) are two different checks:
  - **Unverified** comes from E3 (`vajra_tasks.py check-evidence <file> --head <recorded head> --json`,
    vajra-6 m1-engine): `green:false` means the proof itself doesn't hold.
  - **Stale** compares the evidence's `git.worktreeTree` with the current whole-tree id of the integration
    worktree. The Rust core computes that read-only, with the same algorithm as `vajra-verify-work.sh`: a
    throwaway `GIT_INDEX_FILE`, `read-tree HEAD`, `add -A`, `write-tree`, and the walkthrough dir excluded.
    It is recomputed when the coalesced file watcher sees a change in that worktree, or when HEAD moves.
  - The cache key is (evidence sha256, current tree id), never the evidence path alone.
  - When the worktree no longer exists, the row shows "no worktree to compare" next to the historical
    proof.
- **Re-run** (POS-1.2) is a human-only action, like Take over. The app shows the recorded setup commands
  and the criterion command verbatim, in run order, and runs them only after the human confirms. It creates
  a fresh detached worktree at the evidence commit under `<repo>/.claude/worktrees/desktop-rerun-<id>`,
  rebuilds the verified tree there (apply the evidence's `diff.patch` with `git apply --binary`, then require
  that the whole-tree id equals the signed `git.worktreeTree`, else refuse with "can't rebuild the verified
  tree"; `diff.patch` isn't covered by the evidence HMAC, only by the E7 manifest, so the tree id is the final check), runs each setup command
  as `bash -c <cmd>` in order (an argument array, never a joined string), then
  `scripts/vajra-verify-work.sh <slug> <command>`, compares exit codes, then removes that worktree.
  - **Where the bundle and setup come from (E7):** cleanup may delete `data/walkthroughs` once tasks close
    (`workflows/vajra-deliver.js:842-865`), so Re-run never depends on the original worktree. It reads the
    archived bundle under `campaigns/<slug>/evidence/<task>/`, verified by E3 `--archived` plus the signed
    manifest. Setup comes only from the archived `setup.json`, the exact list write-back executed (filtered
    and capped at `:787`, untruncated). It never comes from the report's `(setup)` lines, which `oneLine(c,
    300)` truncates (`:923`), and never from `artifacts.verification.setup_cmds`, which still holds filtered-out
    entries. The confirm shows the same array that will run. Without E7, or for deliveries made before it, the
    proof shows "evidence not archived": verification and Re-run are refused, and the row keeps only the
    report text. The control service refuses
  re-run requests that arrive over MCP (NFR-SEC-9).
- **Knowledge approve, reject and forget** (POS-3.1): with E9, through the owner-authenticated approval
  of ADR-12. Without it, they show the exact `vajra_kb.py` command with a Copy
  button and the line "Run this in your own terminal" (kb.md: these are user-only, own-terminal commands).
  The app never runs it, types it or sends it into any PTY, including its own. A process that owns a PTY
  can pass a TTY check (SECURITY.md), so the only honest gate is the human's own terminal. The ledger view
  refreshes when `audit.jsonl` records the approval.
- **Runs the app did not start** (POS-2.2) come from transcripts and campaigns as in D9. Their run session
  is shown read-only with no Take over (the app doesn't own that terminal).
- **Uninstall leaves Vajra whole** (POS-2.3). App files are plain JSONL and markdown (4.3). E4 (persist the
  report in the campaign) becomes the recommended default.
- **No permission-widening flags** (POS-2.4). The session supervisor builds argv from a fixed template.
  A unit test fails if any argv contains `--dangerously-skip-permissions` or a `--permission-mode` other
  than the user's default.

## 4. Data model

App view models. They are derived on read and never written back to Vajra files.

### 4.1 View models and sources

**Run**

| Field | Source |
|---|---|
| `run_id` | App launch args. For runs the app did not launch: `campaign.json.slug` with the `deliver-` prefix removed, or the `vajra/<run_id>/` branch prefix in `tasks.json.tasks[].branch` |
| `campaign` | `deliver-<run_id>` (workflow line 54) or the resumed slug |
| `repo` | App `runs.jsonl`. Fallback: run transcript `cwd` (D9) |
| `session_id` | App-generated UUID passed as `--session-id` and as workflow `session_id` (to verify in spike: hook payload `session_id` equals it) |
| `phase` | Transcript step labels mapped to phases (`understand` -> Understand, `plan.*` -> Plan, `task.*` -> Execute, `verify.*`/`review.*` -> Verify, round > 1 -> Fix, `writeback`/`cleanup` -> Deliver). Native workflow phase records are preferred if present (to verify in spike). |
| `round`, `max_rounds` | Highest `r<N>` in labels. `progress.md` `run <id> round <n>:` lines. Launch args. |
| `status`, `node`, `branch`, `commit`, `report`, `pending_approval` | Workflow result captured from the run transcript (ADR-4). Fallback: `progress.md` `delivered <branch> @ <sha>` |
| `integration` | Last `progress.md` `integration <branch> @ <sha> from <sha>` line |

**WorkerNode**

| Field | Source |
|---|---|
| `key` | Stable app key: `agent_id` if the transcript has one, else `label`, else transcript file name |
| `engine_id` | Subagent `agentType` in the transcript (to verify in spike). For task steps, `tasks.json.tasks[].owner_agent` |
| `role` | Mapping 4.4 |
| `label`, `task_id`, `round` | Step label `task.<tid>.r<N>` (to verify in spike: label in transcript). Fallback: the prompt's first line `You own task <tid> (task graph id <run>.<tid>)` (workflow line 540) |
| `status` | `tasks.json.tasks[].status` for task workers. For others: live (transcript growing), finished (final record), stopped (ack in ledger) |
| `worktree` | Subagent transcript `cwd` (`<repo>/.claude/worktrees/<name>`). `tasks.json.worktree` is empty in practice (1.6) |
| `branch` | `tasks.json.tasks[].branch`, else `git -C <worktree> rev-parse --abbrev-ref HEAD` |
| `current_task_title` | `tasks.json.tasks[].title` |
| `last_report` | `tasks.json.tasks[].report {status, summary}` and `updates[]` |
| `hook_match` | What the hook can match on: `{agent_id}` if present in the hook payload (D2), else `{cwd: worktree}`, else `{agent_type, task_id}` only when unique |

**Task**: `tasks.json.tasks[]` fields `id, title, owner_agent, deps, files, acceptance, status, attempts,
evidence, branch, worktree, claimed_by, report, commit, base_commit, changed, updates`
(`scripts/vajra_tasks.py:7-10`). The edges of the task graph come from `deps`.

**Criterion + Evidence**

| Field | Source |
|---|---|
| `id`, `text`, `passes` | `features.json.features[] {id, description, passes}`. `passes` is trusted only when `vajra-campaign.sh verify <slug>` exits 0 (signed state) |
| `command`, `exit_code`, `notes`, `excerpt` | Captured result `artifacts.verification.results[]` (`criterion_id, status, command, exit_code, evidence, notes`). Fallback: `progress.md` `pass: <id> (evidence: <cmd> (exit 0))` |
| `evidence_file`, `evidence_sha256`, `head` | `tasks.json.tasks[].evidence {path, sha256, head, commands}` for tasks whose `acceptance` contains the id |
| `commit` | Result `commit`, or `evidence.head` |
| `evidence_state` | `verified` (E3 `green:true`), `unverified` (E3 `green:false`, AC-4.3.3), `file_gone` (path removed by cleanup; show the recorded sha256 and do not upgrade), `none` (AC-4.3.2) |

Display rule: "pass" requires a signed `passes:true` OR a verifier result with `status:pass, exit_code:0`.
An `unverified` evidence state downgrades the row to "unverified evidence". `none` never shows pass.

**ApprovalItem** `{id, kind, source_run, age, title, body, link}`

| kind | Source |
|---|---|
| `plan` | Result `pending_approval[] {action, why, plan?}`. `id` = `<run_id>.<index>` |
| `question` | Result `status: need_input`, `question`, `node` |
| `task` | `tasks.json` tasks with status `blocked` or `question`, `report.summary` |
| `permission` | A session PTY waiting on a Claude Code permission prompt (detection to verify in spike: a transcript record or Notification-type signal; the PTY screen heuristic is the last resort) |

**ControlAction** = one ledger record (5.6).

**CampaignSummary**: `slug, goal, status, created, updated` from `campaign.json`. `passing N/M` from
`features.json`. `signature` from `vajra-campaign.sh verify` exit code (0 ok, 3 invalid). `last_integration`
from `progress.md`. `checkpoints` = names in `checkpoints/`.

**KbHit**: one line of `vajra_kb.py search <q> --format ids --k <n>` matching
`^kb-[0-9a-f]{6}\s+(decision|fact|lesson|delivery|pitfall)\s+\S.*$` (the same pattern as workflow `KB_LINE`).
Fields: `id, type, title`. Run with `cwd = repo`, because the project id derives from it.

### 4.2 Crew tree derivation

```
Lead                       the Lead chat session (present when open)
└─ Run <run_id>            the run session (PTY view; status from the result or live)
   ├─ <Role> · <tid>       one per task; role from owner_agent; status from tasks.json
   │   ├─ round 1          subagent with label task.<tid>.r1
   │   └─ round 2..3       fix-round reruns task.<tid>.r<N>
   ├─ Verification r<N>    one per round
   │   ├─ Verifier         verify.r<N> (+ verify.r<N>.commands redo)
   │   └─ <Role> review    review.<agent>.r<N> (Architect, Security)
   ├─ Write-back           writeback (Verifier)
   └─ Run steps            kb.search, preflight, campaign.*, merge, deploy_plan, cleanup (collapsed)
```

Rules:
1. A subagent becomes a node when its transcript appears. A task node exists from `tasks.json` before any
   transcript appears (status pending/ready).
2. Task matching order: label `task.<tid>.r<N>`, then the prompt's first line, then (worktree branch
   `vajra/<run_id>/<tid>`), and finally "Unassigned".
3. Two live workers with the same `engine_id` are always separate nodes. `claimed_by` is never used as an
   identity, because it equals the role.
4. Status shown = task status (text and dot). Overlays: `stopped by the human`, `message pending`,
   `activity`.
5. Unknown labels go to "Run steps" with their title-cased label.

### 4.3 App-owned files (app data dir only)

```
~/Library/Application Support/<app>/
  runs.jsonl                  {run_id, campaign, repo, session_id, lead_session_id, started, args}
  reports/<run_id>.md         captured delivery report (ADR-4)
  results/<run_id>.json       captured workflow result
  spill/<stream_id>.log       ring-buffer spill (bounded, LRU)
  control/ledger.jsonl        control ledger
  control/<session_id>/...    queue, stops, inflight, acks (5.3)
  lead/mcp.json               MCP config for the Lead chat session
  app.sock                    owner-only socket (or $TMPDIR, per spike; path < 104 bytes)
  logs/app.log                rotated, no agent text (NFR-OBS-1)
```

### 4.4 Generic name mapping (engine id -> UI name)

| engine id | UI name |
|---|---|
| krishna | Lead |
| arjuna | Architect |
| bhima | Backend |
| nakula | Frontend |
| vidura | Verifier |
| bhishma | Security |
| shakuni | Red team |
| duryodhana | Critic (pre-mortem) |
| drona | Acceptance |
| draupadi | Product |
| dhrishtadyumna | Release planner |
| yudhishthira | Arbiter |
| abhimanyu | Prototyper |
| karna | Numerics |
| sahadeva | Analyst |
| hanuman | Platform |
| ashwatthama | Incident lead |
| sepoy-backend-1 / sepoy-frontend-1 / sepoy-test-1 | Backend worker 1 / Frontend worker 1 / Test worker 1 |

Unknown ids are title-cased (`foo-bar` -> `Foo Bar`). The table is data in one file, so an engine rename only
touches that file.

## 5. Control channel contract

### 5.1 Principles

- **Never write into another process's PTY on behalf of an agent or a control message.** The app writes to
  a PTY in exactly two human-initiated, visible cases: "Take over terminal" (human keystrokes pass through
  while a banner says "You are controlling the run session"), and Approve (one visible turn, 5.8). Both
  are human-only. The Lead and MCP cannot trigger them.
- Control messages are untrusted data for the worker and are **never approval** for a protected action
  (NFR-SEC-8, AC-3.2.4). Claude Code's permission prompt stays the only approval.
- **Lead chat is a separate session.** deliver.md "Messages sent while it runs": Claude Code relays the most
  recent user message to every workflow agent started afterwards. Chatting in the run session would leak
  into workers, so the human chats only with the Lead chat session (ADR-1, AC-3.1.2).

### 5.2 MCP tools (server name `vajra-desktop`; Claude Code exposes them as `mcp__vajra-desktop__<tool>`, naming to verify in spike)

Every tool result is JSON. Text that comes from agents is wrapped as
`{"untrusted": true, "source": "agent-output", "text": ...}`. Every call writes a ledger line.

```json
{"name":"list_workers","inputSchema":{"type":"object","additionalProperties":false,
 "properties":{"run_id":{"type":"string","pattern":"^[0-9]{8}-[0-9]{4}$"}}}}
```
Returns `{workers:[{worker, role, task_id, title, status, round, live, last_activity}]}`. `worker` is the
app key.

```json
{"name":"inspect_worker","inputSchema":{"type":"object","additionalProperties":false,"required":["worker"],
 "properties":{"worker":{"type":"string","maxLength":128},
   "lines":{"type":"integer","minimum":1,"maximum":200,"default":50}}}}
```
Returns header fields, task (`files, acceptance, attempts, report, updates`), `diffstat`, and the last N
transcript lines as untrusted text.

```json
{"name":"message_worker","inputSchema":{"type":"object","additionalProperties":false,"required":["worker","text"],
 "properties":{"worker":{"type":"string","maxLength":128},"text":{"type":"string","minLength":1,"maxLength":4000}}}}
```
```json
{"name":"redirect_worker","inputSchema":{"type":"object","additionalProperties":false,"required":["worker","text"],
 "properties":{"worker":{"type":"string","maxLength":128},"text":{"type":"string","minLength":1,"maxLength":4000}}}}
```
Both return `{action_id, state:"queued"}`, or an error: `worker_not_found`, `worker_finished`,
`worker_ambiguous`, `control_disabled`. `redirect` differs from `message` only in its framing line
("Change of direction from the human: ...").

```json
{"name":"stop_worker","inputSchema":{"type":"object","additionalProperties":false,"required":["worker"],
 "properties":{"worker":{"type":"string","maxLength":128},"reason":{"type":"string","maxLength":500}}}}
```
Returns `{action_id, state:"awaiting_human_confirm"}`. The UI opens the confirm dialog. Nothing is queued
until the human confirms (AC-3.3.3). The Lead gets the outcome on its next `list_workers`.

The UI calls the same Control service functions directly (AC-3.4.2). Those calls use `actor:"human",
via:"ui"` and `source="human"`.

### 5.3 Queue format and location

Root: `~/Library/Application Support/<app>/control/<session_id>/`. `<session_id>` is the run session id,
validated against `^[A-Za-z0-9-]{8,128}$`. Never under `~/.claude`. Dirs are mode 0700 and files 0600.

```
queue/<seq:06d>-<action_id>.json   message|redirect, consumed once
inflight/<action_id>.<pid>         claimed by a hook, before ack
stops/<worker_key_sha256_16>.json  sticky stop flag
acks/<action_id>.json              written by the hook
```

Queue entry (written atomically by the app: temp file + rename):
```json
{"v":1,"action_id":"01JC...ULID","kind":"message","created":"2026-11-05T10:00:00Z",
 "expires":"2026-11-05T10:30:00Z","source":"human|lead-via-desktop",
 "match":{"agent_id":"<if known>","cwd":"/repo/.claude/worktrees/wf_x","agent_type":"nakula","task_id":"20261105-1000.t2"},
 "text":"use the kit Button"}
```
Stop entry: the same shape with `kind:"stop"`, no `text`, and an optional `reason`. A stop matches only on
`agent_id` or `cwd`, never on `agent_type` alone, so it can't hit the same task's worker in a later fix round
(which runs in a fresh worktree). A stop has no expiry. **Only an explicit human "Resume worker" (a ledger
record) removes it**: not the transcript ending, not handback, not the run ending, not the Lead. A parent can
resume the same agent id with `SendMessage` after handback (steering threat model F2), so the stop must
outlive the worker's turn. Implemented in vajra-6's E1 (`origin/ao/vajra-6/m1-engine`, commit 611d969). The source
is set from the actor (`human` or `lead-via-desktop`), never typed by the caller; the hook rejects any other
value.

**Steer nonce (A1 finding B1, tracer bullet).** Workers ignore plain `<untrusted-data>` context and act only
on a tag carrying the run session's nonce. The nonce:
- is 128 bits from the OS CSPRNG, created per run session (and rotated on resume) by the Control service;
- lives only in `control/<session_id>/nonce` (0600, created with no-follow and exclusive create);
- is never written to the queue, the ledger, acks, `audit.log`, `runs.jsonl`, MCP results or stderr;
- reaches the workers through engine change E5 (below), never through the app's launch arguments.

**Trust model (GPT-2, steering threat model).** The nonce is **not** an authentication secret: each worker
must know N to recognize the tag, so N sits in its brief and its transcript. Trust rests on two things the
worker can't forge:
1. The queue is write-protected: only the app writes it (guard protected root plus, with owner decision
   Q10, a sandbox denyWrite; section 5.4 E2 scope).
2. Hook context is a distinct transcript record (`hook_additional_context`), not tool output.

N is defense in depth. It filters injected text whose author never saw N, such as a web page or a file an
agent fetched.

**Residual risk, stated, not solved.** Text that carries the right N inside a tool result can't be ruled out
at the model level. For example, a worker or a compromised dependency could echo its own brief into a file
another worker reads. Mitigations:
- the PostToolUse hook warns the worker when a tool result contains a `vajra-steer` tag, and audits it;
- the PreToolUse hook blocks any tool input that contains N;
- workers can't read the nonce file or transcripts (E2 read blocks).

We don't promise that the model authenticates the human. A steer is never approval for a protected action and
never widens a task's scope (NFR-SEC-8).

**Rejected forms.** Plain `<untrusted-data>` additional context. The tracer bullet showed workers ignore it as
injection, so it is not a fallback.

### 5.4 `hooks/desktop-control.sh` (E1)

Registration (additive entries in `hooks/hooks.json`):
- PreToolUse, matcher `*`, timeout 5.
- PostToolUse, matcher `*`, timeout 5, **not async**. It must return output, unlike the async Atman hook.

`onFailure` stays default (non-blocking). A broken control hook must not stall every Claude Code session.
The consequence is that stop is best effort, and the UI shows "stop pending" until the ack.

Behaviour:
1. Read the payload (`session_id, hook_event_name, tool_name, cwd, agent_id?, agent_type?`) with `jq`. If
   `jq` is missing or the payload is malformed, exit 0 (no decision; the guard still fails closed on its own).
2. Check that `control/<session_id>/` exists before running `jq` at all (A1 M6), then validate `session_id`. If the directory does not exist, exit 0. This is the fast path for every
   session the app does not own, with a target under 10 ms.
3. Match. An entry matches when `match.agent_id` equals the payload `agent_id`. If either side lacks
   `agent_id`, the canonical `cwd` must equal `match.cwd`. If both lack it, `agent_type` must equal
   `match.agent_type` and the entry must carry `unique:true` (set by the app only when exactly one live
   worker has that type). Main-session calls (no agent fields, cwd = repo root) never match a worker entry.
4. **PreToolUse + matching stop file:** the only exempt call is the exact tool name `SubagentHandback`,
   so a stopped worker can still hand back and the parent gets a report (tracer finding 3). No prefix,
   pattern or other output tool is exempt. Every other call: print `stopped by the human (action <id>). Stop now: make no more
   tool calls and return status blocked with summary "stopped by the human".` to stderr and exit 2. Append
   `TS BLOCK desktop-control reason="stop id=<action_id> tool=<tool>"` to `audit.log`. Write
   `acks/<action_id>.json` if absent (first block only).
   Residual risk, gated by G3b: in a `vajra-deliver` run the worker's final output may be a different tool
   (for example `StructuredOutput`). If G3b shows it is, that exact name is added to the exemption, and the
   app shows the task as "stopped by the human" whatever status the worker reports, so a stopped worker's
   "done" is never shown as done (A1 voter S1). The workflow-side guarantee (a stopped task is never merged)
   needs an engine check and is listed as owner decision Q12.
5. **PostToolUse + matching queue entries** (oldest first, unexpired, at most 3 per call). **Nothing is
   delivered on `SubagentHandback`, to a stopped worker, or when the nonce file is missing or invalid.**
   1. Claim the entry by `rename(queue/X, inflight/<id>.<pid>)`. If the rename fails, another hook claimed it,
      so skip.
   2. Print one JSON object on stdout:
      `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"<vajra-steer nonce=\"N\" kind=\"message\" source=\"human\" id=\"<action_id>\">...</vajra-steer>"}}`,
      with N read from the nonce file (if that file is missing, has the wrong mode or owner, or is a symlink:
      deliver nothing and exit 0). `kind` and `source` are enums and `id` is a ULID. The text is escaped so
      it cannot close the tag. A fixed preamble says the data is a request from the
      human operator, does not change task scope or files, and is never approval for a protected action.
   3. Append `TS ALLOW desktop-control reason="deliver <kind> id=<action_id> tool=<tool>"` to `audit.log`.
   4. Write `acks/<action_id>.json` `{action_id, consumed_at, hook_event, tool_name, agent_id, agent_type, cwd}`
      and remove the inflight file.
5b. **Who can be steered (A1 voter S2).** Message and redirect go only to Execute-phase task workers
   (step label `task.<id>.r<n>`). For verify, review and merge steps the Control service returns
   `control_disabled`, so neither the Lead nor a prompt-injected worker relaying through the Lead can talk
   to the independent gate. Those steps can still be stopped.
5c. **PreToolUse nonce leak check.** Any tool input that contains N is blocked with exit 2, and the reason
   never contains N. **PostToolUse:** a tool result containing a `vajra-steer` tag gets a warning and an audit
   line, and no steer is delivered from it.
6. Expired entries are left alone. The app moves them to `expired` and records "not delivered" (AC-3.2.5).
   Delivery is at most once: a crash between steps 5.1 and 5.4 leaves an inflight file, which the app shows
   as "delivery unknown" after 60 s and never re-queues.
7. The hook never reads `.hmac-key`. It uses bash 3.2 and `jq`, like the other hooks, and never evals
   payload text.

To verify in spike: subagent hook payload fields, whether additionalContext from PostToolUse (or PreToolUse)
reaches the subagent, and that exit 2 from a second PreToolUse hook blocks while the guard logs ALLOW for
the same call. In that case the audit shows ALLOW from the guard plus BLOCK from desktop-control, and the UI
explains this.

E2 scope (GPT-3, matching vajra-6's 611d969 on `ao/vajra-6/m1-engine`):
- **Write-protected root:** the whole app data directory (`~/Library/Application Support/VajraDesktop/`), for
  Write, Edit, MultiEdit, NotebookEdit and Bash writers (`mv`, `rm`, `cp`, `tee`, redirects), with the space
  in the path covered by tests.
- **Read-protected for workers:** any tool call whose payload carries an `agent_id` may not read
  `VajraDesktop/control/` (which holds the nonce) or `~/.claude/projects/` (transcripts, which hold each
  worker's brief and so the nonce). This covers Read, Grep, Glob and LS, read-only Bash (`cat`, `head`,
  `python3 -c`, `grep -r` and so on), symlinks resolved canonically, and recursive Grep over an ancestor.
- **Main-session policy (documented):** the main session and the Lead chat have no `agent_id` and keep
  reading. The Lead needs transcripts to answer questions about workers.
- **Not covered:** a worker that never names those paths can still read its own transcript through Claude
  Code itself. The nonce trust model (5.3) does not depend on hiding N from the worker that holds it.
- **Same-user limitation (stated):** file modes (0600) and the socket's peer-uid check separate users, not
  agents. Every agent runs as the same user, so mode 0600 is never treated as agent authorization.
  Authorization comes from the guard (defense in depth over shell) and, with Q10, the OS sandbox.
- **Sandbox boundary (Q10 approved, 2026-11-05):** Vajra's sandbox profile ships a `denyWrite` for
  `~/Library/Application Support/VajraDesktop/` (the control queue included) as its default. Both
  `scripts/enable-sandbox.sh` and the Desktop first-run setup apply it, and only when the user turns the OS
  sandbox on and approves the settings change shown to them. Nothing edits settings silently (VD-27).
  `vajra-doctor.sh` and preflight report whether it is active. On the owner's Mac the OS sandbox is
  currently off (doctor WARN), so the guard is the active layer until it's enabled. Without it, the guard is the only
  write protection, so the control service follows Q10's working assumption: with `denyWrite` off, control is
  **stop-only**. Message and redirect return `control_disabled: sandbox_off`, nothing is queued, and the UI
  hides them with the reason. Stop still works, because a forged or replayed stop can only halt a worker.

### 5.5 Worker identification

The app learns identity from transcripts (4.1) and writes `match` with every field it knows. The hook
matches on the strongest field both sides have, in this order:
1. `agent_id` (if present in the hook payload)
2. `cwd` -> worktree (every writer, verifier and reviewer runs with `isolation: 'worktree'`)
3. unique `agent_type`

If none is decisive, `message_worker` returns `worker_ambiguous` and nothing is queued. `tasks.json`
`worktree` is empty in practice, so the cwd mapping comes from the subagent transcript cwd, not from the task
graph.

### 5.6 Control ledger record

`control/ledger.jsonl`, append-only. Each state change is one new line; earlier lines are never rewritten.
```json
{"v":1,"ts":"2026-11-05T10:00:00.123Z","action_id":"01JC...","kind":"list|inspect|message|redirect|stop|approve|reject|takeover",
 "state":"requested|awaiting_human_confirm|queued|delivered|not_delivered|delivery_unknown|refused|done",
 "actor":"human|lead","via":"ui|mcp","run_id":"20261105-1000","session_id":"<run session>",
 "worker":"<app key>","engine_id":"nakula","task_id":"20261105-1000.t2",
 "source":"human|lead-via-desktop","text":"<= 4000 chars, as delivered","text_sha256":"...",
 "audit_ref":{"kind":"guard|desktop-control","ts":"...","tool":"..."}}
```
`text` is the exact message the worker receives: typed by the human (`actor:"human"`) or written by the Lead
(`actor:"lead"`, so every Lead instruction is on record). Worker output never goes into the ledger.
Reconciliation (AC-3.4.3):
- every message/redirect/stop `action_id` in state `delivered` has a desktop-control line with the same id;
- every `mcp__vajra-desktop__*` guard line has a ledger `via:"mcp"` record of the same tool within 5 s (D5).

### 5.7 How actions reach `audit.log`, and why E2 only adds logging

- **Lead MCP calls.** E2 adds `mcp__vajra-desktop__.*` to the guard matcher in `hooks/hooks.json`. Matcher
  regex semantics are to verify in spike. The guard's dispatch sends unknown tools to `*) allow ;;`
  (`hooks/pre-tool-use.sh:226`), which audits `ALLOW <tool>` and exits 0 ("no decision"). Effects:
  - every Lead control call gets an audit line;
  - when the guard fails (jq missing, crash, timeout), `onFailure: block` blocks the call;
  - no existing path is allowed that was blocked before; normal permissions still apply after exit 0.
  The change adds checks and logging and removes none.
- **Deliveries and stops.** E1 writes the delivery lines in 5.4. Those lines carry the `action_id` for both
  Lead-initiated and human-initiated actions.
- **Protecting the queue.** E2 adds `"$HOME/Library/Application Support/<app>/control"` to the guard's
  `ROOTS` (`hooks/pre-tool-use.sh:130` area), so every agent write, move or delete there is blocked
  (AC-3.4.4). This is one more protected root and loosens nothing. The guard's canonicalizer must handle the
  space in "Application Support"; a test case is required. The hook and the app are not tool calls, so the
  guard does not apply to them.

### 5.8 Approve and reject

- **Approve** (US-5.2). Enabled only when the full plan text is shown (AC-5.2.1) and the run's workflow has
  ended (status terminal), so the turn cannot be relayed into running workflow agents. The app writes the
  visible turn `Execute approved plan <run>.<idx> (<action>)` to the run session PTY: the plan id plus the approved entry's fixed-vocabulary action, so a planted or mismatched result is visible to the session. The
  plan body is already in that session's context from the result. Claude Code's permission prompt fires for
  each protected command. The app runs no push, PR or deploy. The ledger records `approve`. The audit line
  comes from the guard on the resulting Bash calls.
- **Reject** (US-5.3): Script bridge
  `["bash", "<skills>/vajra/scripts/vajra-campaign.sh", "progress", "rejected plan <id>: <reason>", "--campaign", slug]`.
  The text is one argv element. The script itself strips control characters (`clean_line`).

### 5.9 Items to verify in spike (Claude Code behavior not confirmed from the repo)

1. Transcript layout: project dir slug rule, the session jsonl path, the subagent transcript location and the record shape (type, uuid, parent, agent type, label, cwd, tool_use/tool_result, timestamps).
2. Workflow agent step `label` and `agentType` recorded in subagent transcripts; workflow `phase()` records.
3. The workflow result (`report`, `pending_approval`, `artifacts`) recorded in the run session transcript as the Workflow tool result.
4. `claude --session-id <uuid>` works for interactive sessions, and the hook payload `session_id` for subagents equals it.
5. Positional initial prompt `/vajra-deliver <json>` starts the saved workflow, and the JSON string reaches `parseArgs` unchanged.
6. Hook payload fields for subagent tool calls: `agent_id`, `agent_type`, `cwd`.
7. `additionalContext` from PostToolUse (and PreToolUse) is delivered to the calling subagent.
8. Exit 2 from a non-guard PreToolUse hook blocks a subagent call while other hooks allow it, and the stderr reason reaches the subagent.
9. `--agents` inline agent definition plus `--agent <name>` for the Lead chat, with MCP tool names in its tools list.
10. `--mcp-config <file>` outside `~/.claude`, and the MCP tool naming `mcp__<server>__<tool>`.
11. `hooks.json` matcher is a regex that accepts `mcp__vajra-desktop__.*`.
12. Detecting a session waiting on a permission prompt (transcript record or notification signal) without PTY screen scraping.
13. Transcript schema stability across Claude Code versions (the fingerprint fields for the schema check).
14. All provisional performance numbers (section 9).

## 6. ADRs

**ADR-1 Separate Lead chat session.** Status: accepted 2026-11-05. Decision: the human chats with a second Claude Code session (Lead
agent, read tools plus control tools, no Agent/Workflow tool), never with the run session.
- Options: (a) do nothing, so the human types into the run session; (b) the app answers questions itself
  with no model; (c) separate Lead session (chosen).
- Evidence against: a second session costs tokens and memory, and it sees state only through tools, so it
  can be stale by seconds.
- Why: deliver.md documents that a mid-run user message reached every later workflow agent. (a) breaks scope.
  (b) cannot answer "why is Backend stuck".
- Would change our mind: Claude Code gains per-agent addressing inside a workflow with no relay.

**ADR-2 Control by hook + queue, not PTY injection.** Status: accepted 2026-11-05. Decision: control reaches workers only through E1
reading the app-owned queue.
- Options: (a) do nothing (no steering); (b) PTY injection, as Competitor A's router writes into agent stdin; (c)
  hook + queue (chosen); (d) restart the worker with a new brief.
- Evidence against: delivery waits for the worker's next tool call (a long model turn delays it), and
  several key behaviors are to verify in spike (5.9 items 6-8).
- Why: workers have no PTY (they are subagents of one process). Typing into the run PTY reaches the main
  session and is relayed to all agents. Injection is invisible to `audit.log` and could satisfy TTY gates
  (SECURITY.md: the TTY gate is not a boundary against a process that owns a PTY). (c) is addressed,
  audited, consumed once and wrapped as untrusted data.
- Would change our mind: the spike shows subagent hooks cannot inject context. Fallback then is
  stop-only plus "message on next round" through the fix-loop feedback, and J3 messaging is descoped.

**ADR-3 Read-only over `~/.claude`.** Status: accepted 2026-11-05. Decision: the app reads an allowlist and mutates only through Vajra
scripts and Claude Code. The engine additions it needs are E1 to E8.
- Options: (a) do nothing (app writes campaign files directly); (b) the app links Vajra's logic as a
  library; (c) read-only plus scripts (chosen), with E3 `check-evidence` so evidence can be verified without
  the key or a state change.
- Evidence against: scripts cost a process spawn (tens of ms) per call, and features that need a missing
  script wait for an engine change (E3 today).
- Why: the guard blocks agent writes there, the signatures depend on the scripts being the only writers, and
  reading the key would put it in a second process (NFR-SEC-2).
- Would change our mind: measured script latency breaks NFR-UX-1 on common paths. The answer would still be
  a batch read-only script, not direct writes.

**ADR-4 Capture the delivery report from the transcript.** Status: superseded by E4 (Q3, 2026-11-05);
capture is kept only as the fallback for campaigns delivered before E4. Decision: the tailer captures the workflow result
from the run transcript and stores `reports/<run_id>.md` and `results/<run_id>.json` in the app dir.
- Options: (a) do nothing (report only on screen); (b) parse the PTY screen; (c) transcript capture (chosen);
  (d) E4 engine persistence.
- Evidence against: depends on 5.9 item 3, and runs that happened while the app was closed are captured only
  if their transcript still exists.
- Would change our mind: the owner approves E4. Then the app reads `report.md` and drops capture.

**ADR-5 Worker streams from transcripts, not PTYs.** Status: accepted 2026-11-05. Decision: render worker streams from transcript files
tailed read-only.
- Options: (a) do nothing (one run PTY only); (b) a PTY per worker (needs the engine to spawn processes per
  worker, which breaks the one-process workflow model); (c) transcripts (chosen).
- Evidence against: transcripts are an internal format that may change (to verify in spike), and they show
  tool calls and messages, not live tokens.
- Mitigation: schema fingerprint plus raw view (section 7).
- Would change our mind: Claude Code ships a documented event stream for workflow agents.

**ADR-6 MCP vs CLI for control tools.** Status: accepted 2026-11-05. Decision: a stdio MCP server forwarding to the app socket.
- Options: (a) do nothing (Lead is read-only); (b) a CLI (`vajra-desktop ctl message ...`) called by the
  Lead through Bash; (c) MCP (chosen).
- Evidence against: one more binary, and the MCP naming and config behaviors are to verify in spike.
- Why: (b) puts message text into Bash command lines, which means shell quoting of model text and guard
  parsing of free text, and needs Bash for the Lead. MCP gives typed schemas, no shell, a tool-level guard
  matcher and clean action cards.
- Would change our mind: the spike shows inline agents cannot be given MCP tools.

**ADR-7 No own database in MVP.** Status: accepted 2026-11-05. Decision: derive everything from files on read. App files are
append-only JSONL plus captured reports.
- Options: (a) do nothing (rescan everything on each view); (b) SQLite index of campaigns, runs and
  transcripts; (c) derive from files with in-memory snapshots (chosen).
- Evidence against: History over hundreds of campaigns and large transcripts may be slow to scan at
  startup.
- Would change our mind: measured History open > 300 ms or cold start over NFR-PERF-4. Then add a small
  app-owned index (`index.sqlite` in the app dir, rebuildable, never authoritative).

**ADR-8 Stop is sticky and needs human confirmation.** Status: accepted 2026-11-05. Decision: stop is a flag file that blocks every next
tool call of that worker instance (matched by `agent_id` or worktree `cwd`, 5.3), except the exact tool
`SubagentHandback`. Only a human "Resume worker" clears it (5.3); handback and a later `SendMessage` don't. The
Lead can only request a stop; the human confirms. Messages are not delivered while a worker is stopped.
- Options: (a) do nothing; (b) kill the run session (kills all workers); (c) sticky hook block (chosen).
- Evidence against: the worker may keep reasoning without tools for a turn, and the workflow may treat a
  blocked worker as failed and continue other waves.
- Why: (c) is the only per-worker stop that works inside one process.
- Would change our mind: Claude Code exposes per-subagent cancel.

**ADR-9 No separate daemon in the MVP; runs survive a closed window, not a quit or crash** Status: accepted 2026-11-05. (VD-4, VD-5,
VD-6, POS-2.1). Decided 2026-11-05 by Opus 5.5, accepting the orchestrator's recommendation; GPT 6.1 Sol to
check.
- Decision:
  1. The Rust core owns the PTYs of the run session and the Lead chat inside the app process. There is no
     separate daemon or launchd agent.
  2. Closing the last window does not quit. The app stays alive as a menu bar item showing the number of
     active runs, and its runs and agents keep going. Reopening the window reattaches with nothing lost,
     because the core never stopped.
  3. Cmd-Q with active runs asks first: "Quit and stop 2 runs?" with **Keep running** (closes the window,
     the default) and **Quit** (destructive). With no active runs, quit is immediate.
  4. On a real quit, a crash, logout or shutdown, the run session's PTY closes and its `claude` process
     ends. On the next launch the app offers resume **only** for the sessions that were running and that
     the human did not stop or quit (VD-5). Nothing resumes without a click.
  5. Resume runs `claude --resume <session_id> '/vajra-deliver {"campaign":"<slug>","run_id":"<UTC now>"}'`
     in the repo, as argv with the slug from `runs.jsonl`. Nothing is typed into the PTY. It never uses
     `/vajra continue`, because that resumes Vajra's one global `.active` campaign, which every new run
     rewrites (`vajra-campaign.sh` create; pre-mortem PM-1). A campaign already live in any session is not
     offered.
     The tracer bullet showed that `--resume` plus `SendMessage` continues the same worker id. The Lead chat
     resumes the same way.
  6. Updates (VD-6) install only when no run is active. Otherwise the update waits, and the menu bar says
     "Update ready, installs when runs finish". With one process there is never an older generation left
     running. After 72 h waiting, the menu offers Install now, which stops the runs and offers resume (PM-6).
  6b. Each run session is spawned as the leader of a new process group, and `runs.jsonl` records its pgid
     and the leader's start time (so a reused pid is never mistaken for it). Quit sends SIGTERM to each
     group and waits up to 10 s, then SIGKILL.
  6b'. **Startup cleanup barrier** (PM-8, POS-2.1f). Before the run list loads or any resume is offered, the
     core checks every run in `runs.jsonl` whose session isn't live. For each recorded group that still has
     members, each member's start time must be at or after the run's start (a reused pgid whose leader is gone
     is otherwise indistinguishable); members that fail the check go to the confirm list instead. Groups are
     handled in parallel: SIGTERM, wait up to 10 s, SIGKILL, wait up to 5 s. Then it sweeps for escaped descendants: any
     process of the same user whose cwd (canonicalized with realpath, so `/var` and `/private/var` match) is
     inside that run's worktrees. Those are listed (pid, command, cwd) and killed only after the human
     confirms. The human may instead choose "Keep and resume anyway" (for their own shell or editor), which
     is recorded in the app log. Resume of that campaign stays blocked until each listed process is killed
     or kept. The barrier is bounded at 20 s in total, excluding time spent waiting on that confirm, and its outcome is recorded in the app log. Only after
     it finishes does `vajra-campaign.sh verify` run for each campaign.
  6c. When the last window closes, the webview is destroyed and only the core stays. Reopening rebuilds the
     UI from the core. This keeps a long-lived menu bar app from carrying webview growth (PM-6).
  7. While any run is active the core holds a macOS activity assertion, so App Nap does not throttle the
     windowless app (A1 M5). It is released when the last run ends.
- Options:
  - (a) Do nothing: the window owns everything, and closing it stops runs. Rejected: it fails P2 at the
    most common moment.
  - (b) Menu bar app, no daemon (chosen).
  - (c) A separate Rust daemon (launchd agent) owning the PTYs, with versioned IPC and an update handoff.
  - (d) A detached terminal multiplexer (tmux or dtach) holding each PTY.
  - (e) Claude Code's own background sessions, if they cover workflow runs.
- Evidence for (b):
  - P2 and P3 in `pain-points.md` (vajra-4) are mostly daemon-lifecycle failures in apps that have one:
    old daemon generations left running after an update (a public issue in Competitor F's tracker), every session swept to `exited` after
    a daemon restart (a public issue in Competitor B's tracker), and duplicate sessions after an upgrade (public issues in Competitor H's and Competitor K's trackers). (c)
    takes on exactly that failure class plus a versioned protocol, in exchange for surviving crashes.
  - The spike measured the app core at about 110 MiB RSS (Rust main, flat over an hour). A daemon would
    split that across two processes and add its own baseline against an idle budget we already miss
    (220 vs at most 200 MiB).
  - Vajra already makes interruption recoverable: campaigns are signed files with checkpoints, finished
    worker commits stay on their branches, and `/vajra continue` resumes (CLI only; the app resumes by slug).
- Strongest evidence against (b):
  - A crash or a real quit stops the in-flight run. `deliver.md` says a resumed campaign gets fresh,
    run-namespaced tasks and no "already done" shortcut. So work since the last commit is lost and the
    current wave is redone. That is a real cost on a long delivery.
  - Shutdown and logout can't be blocked.
  - Competitor A and Competitor N survive reboots of their UI, and users praise it (pain-points.md, praise table).
- Consequences:
  - POS-2.1 is narrowed. In the MVP the requirement is that runs survive the window closing and the app
    sleeping. Surviving SIGKILL and Quit moves to v1.1.
  - VD-4 as written (a daemon) is deferred. VD-5 and VD-6 are met as described in the decision above.
  - Runs started in a plain terminal (POS-2.2) are not owned by the app, so they survive anything the
    app does.
- What would change our mind, each measured in beta:
  - Crash-free sessions below 99.5%.
  - More than 1 in 20 deliveries interrupted by a quit or crash.
  - Claude Code documenting background sessions that a workflow run can live in. That is option (e): it
    gives survival without a daemon of our own, so evaluate it first.
  - Vajra adding a resume that keeps a run's tasks. Then (c) or (d) are reconsidered for v1.1.

**ADR-10 One small agent-adapter interface** Status: accepted 2026-11-05. (owner decision, 2026-11-05). Decision: the Rust core talks to
agents through one trait: `spawn`, `stream`, `attach_pty`, `inspect`, `message`, `stop`, `resume`, and a
`capabilities()` set of flags. The UI renders only actions whose flag is true.
- Claude Code adapter: every flag true (the control channel in section 5 is its `message` and `stop`).
- Codex CLI adapter: `spawn`, `stream` (its PTY), `attach_pty` and process-group `stop` (a hard stop, labelled
  so) to start with. `inspect`, `message` and `resume` stay false until G4 proves a supported mechanism, and
  are never emulated by typing into its PTY (ADR-2).
- Options: (a) Claude Code only (the earlier won't-do); (b) a plugin system; (c) one compiled-in interface
  with two adapters (chosen).
- Evidence against: a second adapter adds a conformance suite and a test matrix, and Codex users may expect
  parity.
- Would change our mind: G4 shows Codex can't be streamed or stopped reliably; then the Codex adapter ships
  later and the interface stays.
- Gemini CLI and OpenCode come post-MVP through the same trait.

**ADR-11 Import existing worktrees is read-only discovery** Status: accepted 2026-11-05. (owner decision, 2026-11-05). Decision: discover repos,
worktrees (`git worktree list`) and Claude Code sessions (transcript `cwd`), let the user confirm each, and
record only app-owned entries. Nothing is deleted, moved or rewritten. Each other tool's on-disk layout is read by its own adapter (one adapter per layout), and only once G4 has confirmed that layout; otherwise the import says "not recognised" (AC-1.4.4).
- Options: (a) no import; (b) full import that moves worktrees and converts other tools' state; (c) read-only
  discovery with per-item confirm (chosen).
- Evidence: P6 (13 sources) includes displaced users of a shut-down tool asking for an alternative. (b) risks
  losing someone's work in a tool we don't control.
- Consequences: users keep their worktrees where they are; anything held only by the other tool (its own chat history)
  isn't carried over.
- Would change our mind: G4 finds no stable on-disk layout; then the import is git- and transcript-only.

**Billing hard rules** (owner decision; PRD section 11; status: accepted 2026-11-05; options and evidence in
`vajra-4/billing.md`). The session supervisor builds argv from a fixed
template for the stock binary (no `--bare`, no `-p`, no SDK) and the child environment from an allowlist. It
reports the billing source per session from the presence of `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` or
an `apiKeyHelper` (API key wins), and for Codex from `codex login status`. It never reads a credential value
(NFR-SEC-16). Spawns are staggered by at least 2 s (AC-1.1.5).

**ADR-12 Project memory is a view on the ledger and Atman, with one human-only approval path**
(owner feature, 2026-11-05; E9 approved as Q14 with the conditions in AC-7.1.8). Decision: no new memory store. Inbox, rules, search and triggers read and
mutate only through `vajra_kb.py` and `atman.sh` (US-7.1 to US-7.4).
- **Why not a PTY:** the TTY gate in `human_gate` (`scripts/vajra_kb.py:845`) can't tell the app from an
  agent, because any process that owns a PTY passes it (SECURITY.md). So the app never satisfies it.
- **The approval path (E9, owner approval Q14): signed approvals.** File location is not a boundary: any
  same-user process can write a file, including a detached process or launch agent left by an older
  unsandboxed session, which survives restarts and logins (PR review). So the engine trusts a signature,
  not a path.
  1. **Enrollment, once, human-only.** On first use the core creates a P-256 key in the Secure Enclave
     (non-exportable, access control `userPresence`, so every signature needs Touch ID, a paired Apple
     Watch or the login password; no reuse window). The user enrols its public key by running
     `vajra_kb.py desktop-enroll <pubkey-file>` in their own terminal. That command sits behind the same
     human gate and guard block as `approve`, shows the key's fingerprint and asks the user to compare it
     with the one shown in the app, then appends an HMAC-chained `desktop_key` event. `desktop-revoke`
     removes it the same way. The latest enrolled key is the only one E9 accepts.
  2. **Each approval.** The human confirms in a native sheet. The core asks the Secure Enclave to sign the
     approval payload, which triggers the owner authentication with the action, id and scope in the reason
     string. The payload is canonical JSON: action, store (project id or `global`), record id, sha256 of
     the pending record as read, final `applies` and paths, edited text if any, a random 128-bit approval
     id, and the time. The core writes the payload plus its signature to
     `~/Library/Application Support/VajraDesktop/approvals/<approval id>.json`.
  3. The script bridge runs `vajra_kb.py approve <id> --desktop-approval <file>` (or `reject`, `forget`,
     `retire`, `supersede`; `atman.sh approve|reject <id> --desktop-approval <file> [reason]` for routes,
     where `reject` parses the flag before treating the rest as the reason (today `cmd_reject`,
     `atman/atman.py:988`, takes every remaining argument as the reason and calls its own `human_gate`); for
     a `kb-` id Atman passes the flag through `kb_call`). Atman skill and agent patch proposals stay
     copy-command: the user applies them with `git apply` anyway.
  4. E9 accepts the file only when all of these hold:
     - the signature verifies against the enrolled key (`openssl dgst -sha256 -verify`, already a Vajra
       dependency), and the action and store in the payload match the command;
     - under the store lock: the approval id has not been used (the used ids are recorded in the HMAC-chained
       ledger, not in a file a writer could delete), the pending record's bytes are read once, and their
       sha256 matches;
     - the approval is under 5 minutes old.
     It then builds the record from those same bytes. For Edit then approve, the payload carries the edited
     `title`, `body` and `paths` (all three always present, so an omitted field can't mean "keep the
     original"): the engine replaces exactly those fields on the record built from the hashed bytes and
     keeps everything else from it (id, type, source, provenance, created). The sha256 still binds the
     approval to the original pending record, and the signature binds the edit. The write gate then runs on
     the final record, edit included, not on the original bytes, so a refusal returns the gate's reason
     and nothing is applied. It then applies the final record, records the approval id as used, and audits
     (the audit stores the sha256 of both the original and the final record, never the text). The parser already refuses abbreviations (`allow_abbrev=False`,
     `scripts/vajra_kb.py:1653`), so `--desk` can't stand in for the flag, and the guard matches the full
     name. No other input to these commands changes, so the TTY path stays as it is for the CLI.
- **Why it holds:** a forger needs a signature from a Secure Enclave key that signs only after owner
  authentication, or the Vajra HMAC key to enrol its own key. Neither depends on which processes are
  sandboxed or when they started. Agents can't run `desktop-enroll` (guard), and the app offers no MCP tool
  or IPC command that produces a signature. A synthetic click reaches the sheet, but not the authentication.
- **Residual risk:** a same-user process that can read the Vajra HMAC key can already forge any ledger event
  directly, so E9 is no weaker than the existing kb model. Enrollment is the one step that goes through the
  TTY gate, and the user compares fingerprints there.
- **Options:** (a) keep copy-to-terminal only; (b) the app drives the TTY prompt (rejected: it defeats the
  gate); (c) the app holds the HMAC key and mints user tokens (rejected: the app never reads the key);
  (d) an owner-authenticated approval file trusted for its location (rejected after review: a surviving
  same-user writer can forge it, and no timestamp test excludes one); (e) Secure Enclave signed approvals
  checked against a human-enrolled key (chosen).
- **Evidence against:** it adds a second way into a human gate, and an enrollment step, so it needs
  bhishma's review and red-team PoCs (H5). Would change our mind: a PoC gets an approval accepted without
  owner authentication. Then we fall back to (a).
- **"Applied" stays data:** approved records are primed as `<untrusted-data>` exactly as today. The app adds
  no instruction channel.

**ADR-13 Competitor M: no integration in the MVP; a one-shot read-only import later** (owner request,
2026-11-05). Evidence (GitHub API and Competitor M's docs, 2026-11-05; not installed on the owner's machine):
- **Project:** Competitor M, Apache-2.0, about 113k stars, very active (v15.9.0 on 2026-11-02;
  141 open issues).
- **Storage:** SQLite in the tool's data directory under the home directory (WAL mode, sessions, observations and summaries,
  FTS5) plus a Chroma vector store. Settings are in its settings file.
- **Capture:** six Claude Code hooks (SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Stop,
  SessionEnd) capture what the agent does, compress it with a model, and inject context into later
  sessions. A local Bun worker serves an HTTP API and a viewer, with loopback bind as its only protection
  (its docs say so).
- **Billing and privacy:** compression defaults to provider `claude` with
  its auth-method setting set to `subscription`, which conflicts with our billing rules (no SDK use of plan
  credentials). The installer offers a hosted account (a paid plan with cloud sync) unless opted out. `<private>`
  tags exclude content.
- **Options:**
  - (a) Integrate as a live read-only source. Rejected: it would tie us to an unauthenticated local HTTP
    API and a fast-moving schema, and blur "what shaped this run".
  - (b) A one-shot import into the ledger as `proposed` records. Chosen, post-MVP: the user starts it, it
    reads the SQLite file directly (never the worker API), checks the schema version, and each imported
    observation still passes the write gate and the human approval (US-7.1).
  - (c) Ignore it.
- **Coexistence:** if `vajra-doctor` finds Competitor M's hooks next to Vajra's, preflight warns that two
  memories inject context (P15 context-cost risk). The app never edits either one's settings.
- **Would change our mind:** Competitor M documents a stable export format; then (b) uses that instead.

**ADR-16 Per-worker launch: the core runs non-Claude crew workers as their own sessions (multi-provider D1, PR 6)**

Status: proposed 2026-11-10. This is a design only: arjuna reviewed it and duryodhana ran a pre-mortem, and both are folded in here.

- **Decision it implements:** the owner chose core-launched workers (D1) on 2026-11-09.
- **What it builds on:**
  - PR 3, the gateway adapter (#123);
  - PR 4, the launch receipt (#128);
  - PR 5, provider errors (#126);
  - FG-02, Director-General proposals (#90).
- **Related ADRs:** ADR-14 (approval-only resume) and ADR-15 (projects and the Director-General) are recorded with their FG specs, not in this file.

**Open questions for the owner.** The design below takes the recommended answer to each one. The "Gates" column says which PR waits on it.

| # | Question | Recommendation | Gates |
|---|---|---|---|
| Q1 | Should only non-Claude workers be launched by the core? Claude-account workers would stay subagents inside the run session. | **Yes (hybrid).** A Claude subagent keeps the guard, mid-run messages, redirect, its transcript and the workflow's typed report. A Claude seat still gets a receipt (16.6). | 6a |
| Q2 | Approve engine changes **E13** and **E14**? E13 makes the workflow send a core-seat task through a relay step and adds a `RelayReport` schema. E14 adds a `bundled-agents/vajra-relay.md` definition and a dispatch-only MCP server on the run session. Both ship in 6c and need the owner's install and the manifest regeneration. | **Approve.** 6a and 6b can merge first. Until the installed workflow reports that it supports seats (the handshake in 16.2), every non-Claude seat is refused, so nothing is ever approved for a route that does not exist. | 6c |
| Q3 | The Codex `workspace-write` sandbox stops writes outside the worktree and the shell's network, **but not reads**. A Codex worker can read any file the user can, and what it reads can reach that worker's model provider. Accept that? | **Accept, and disclose it.** The proposal card and the worker card say: "Runs outside the Vajra guard. Can read your files; can write only in its worktree." A project that does not accept this limits `allowed` to Claude. | 6b |
| Q4 | Codex **login** workers use the user's own `~/.codex`: the login, and the config with any MCP servers and `notify` command in it. Those could act outside the guard and outside the sandbox. | **The core switches them off for the worker** (`-c mcp_servers={}`, `-c notify=[]`, spike S3). If Codex cannot do that, workers are **not offered on the login route**; they run on API-key or gateway accounts only. For the receipt, the core reads only the one session file of the thread it started (`~/.codex/sessions/**/rollout-*-<thread_id>.jsonl`, read-only, no-follow), never `auth.json` or `config.toml`. **Security review by bhishma is required.** | 6b |
| Q5 | A seat is an **account for a whole role**: every task the plan gives that role, which can include config or CI edits. The DG's `task` text is a forecast, not a binding. Accept that? | **Accept, with one exception:** a task flagged as security (the workflow's `SEC_RE` or `touches_security`) stays on Claude unless its seat sets `security: true`. The card says "Expected task". The crew tree shows the real task→seat map once the run has planned. | 6a |
| Q6 | No per-worker choice for a run started **from the form** (no proposal) in PR 6? | **Yes.** Only a human-started proposal can put a worker on a non-Claude account. The form follows in PR 8. | — |
| Q7 | What happens when a cap is reached? | The task **waits in its seat's queue** (16.5). It never stops the run and never moves to another account (H7). Per-account caps are an optional setting with **no default**. Because a gateway key's credit limit is the only hard spend stop, Settings suggests setting one, and the proposal card says when a gateway account has none. | 6b |
| Q8 | Work a core worker leaves **uncommitted** after a Stop, a timeout, a crash or an app quit. | **Kept in its worktree, never committed automatically.** The run card lists it with its diff (16.5, "Run end and restart"), and the user decides. | 6b |

**Decision.**
- **Who launches what.** The core spawns each **non-Claude** crew worker as its own PTY session, through that account's adapter (16.8 gives the argv). It supervises it like any other session: the PR 4 receipt, PR 5's typed errors and retry policy, and Stop. Claude workers stay subagents (Q1).
- **How the lead hands off a task.** The run lead (Claude Code, D4) hands a task to a core worker through a **relay step**: a small Claude subagent, inside the guard, in its own worktree. It calls the dispatch-only tools `dispatch_worker` and `await_worker`, commits the task's files, and returns a `RelayReport`.
- **Where the authority sits.** What may run where is fixed by the **human-started proposal** (16.1–16.3), held in a guarded record. The core checks it on every dispatch and never trusts what the run session says.

**Options.**

| Option | Verdict | Why |
|---|---|---|
| (a) Do nothing: every worker is a Claude subagent | Rejected | H2 needs agent, model and effort per worker. |
| (b) A Claude subagent shells out to `codex exec` | Rejected in D1 | The worker would sit outside receipts, typed errors, caps, the session list and Stop, with the subagent's environment instead of `ENV_ALLOW`. |
| (c) The core runs every worker, Claude included | Rejected for now (Q1) | Claude workers would lose mid-run messages and their transcript. |
| (d) Hybrid core launch through a relay | **Chosen** | |
| (e) The core creates the worktree and commits | Rejected | The git bridge is read-only by design (`vajra/bridge.rs`, `GIT_READ_ONLY`). The relay already has a worktree from Claude Code (`isolation: 'worktree'`) and commits under the guard, as every writer does. |
| (f) The worker commits | Rejected | A linked worktree's objects and refs live in the main repo's `.git`, outside the sandbox's writable root. Widening `writable_roots` would let the worker write the user's refs, hooks and config. |

### 16.1 `run_proposal.workers[]` and the hash

`propose_run` gains two optional arguments, `workers` and `max_in_flight`, both in the MCP schema and in the `tools.rs` arg list.

```json
"max_in_flight": 4,
"workers": [
  { "slot": "w1", "role": "backend", "task": "Token bucket middleware", "account": "claude",
    "model": null, "effort": null, "security": false, "why": "Touches auth; needs the guard and mid-run messages" },
  { "slot": "w3", "role": "docs", "task": "Document the limits", "account": "openrouter",
    "model": "deepseek/deepseek-v4-flash", "effort": null, "security": false, "why": "Plain writing, low risk" }
]
```

**Fields.**

| Field | Rule |
|---|---|
| `slot` | `w1` to `w16`, unique. |
| `role` | A key of `crew::roles` (table below). |
| `task` | Plain text, 1 to 200 bytes. |
| `account` | A known account id. |
| `model` | `null` or a `valid_model_id`. |
| `effort` | `null` or 1 to 16 bytes of `[a-z0-9-]`, the same charset rule as `preferences`. It is passed through to a harness that has an effort setting; a harness without one refuses a non-null value. No fixed list (H8). |
| `security` | Default `false`. |
| `why` | Plain text, 0 to 300 bytes. |
| `max_in_flight` | 1 to 16, default `caps::DEFAULT_RUN_WORKERS`. It replaces the constant that `desktop/proposals.rs:93` uses today. |

- **Plain text.** Every text field passes the same `sanitise` check as `ask`.
- **`workers: []` is refused.** Leave the field out instead.
- **Claude seats (a Vajra limit, not a capability check).** A non-null `model` or `effort` on a Claude seat is refused with `not_supported_yet`, because the workflow cannot pass a model to a writer subagent today (writers pin their own, for example `bundled-agents/bhima.md`).
- **`source` is set by the core, once, at proposal.** It is `role_default` when the choice equals `Agents::choice_for(role)`, and `dg` otherwise.
  - It is **stored** on the ledger line and **hashed from the stored value**. Start never re-derives it, so editing a preference cannot turn every waiting proposal into `proposal_changed`.
  - A `source` sent by the DG is refused (`deny_unknown_fields`), so the DG cannot claim `user`.

**Roles (`crew::roles`).** The core maps each role to the workflow's writer ids, as data in one file. A parity test checks the table against the workflow's `ALL_WRITERS`. `Agents.roles` keys are validated against this table too (they are free-form today, in `preferences/mod.rs`).

| Role | Writer ids |
|---|---|
| backend | bhima, sepoy-backend-1 |
| frontend | nakula, sepoy-frontend-1 |
| tests | sepoy-test-1 |
| docs | abhimanyu |
| platform | hanuman |
| numerics | karna |
| analysis | sahadeva |

Architect, verifier, reviewer and merge steps never route to a seat.

**Hash.** It is computed from a typed struct (`ProposalV2`, fields in a fixed order, `null` written out), never from map order.
- **No `workers`:** exactly today's `vajra.run_proposal/1`, so proposals already waiting in a ledger still start.
- **With `workers`:** `vajra.run_proposal/2` over `project, ask, base, rounds, max_in_flight, workers` (in slot order, each with its stored `source`).
- **Any change** to any byte of any worker changes the hash, and Start refuses with `proposal_changed`.

**Ledger and inbox.**
- The `propose` line stores `workers` and `max_in_flight`. `open_proposals` parses them with the same bounds; a line that fails to parse is skipped, never half-read.
- Each inbox item carries `workers`, plus for each worker its `limits` (16.4), `outside_guard: bool` and `route` (16.2).

### 16.2 The H8 checks: what, and when

One function, `providers::launch::launchable(choice, agents, probe, keys, route)`, runs at proposal time and again at start. It returns `Result<(), Refusal{slot, code, detail}>` and checks only these:

| # | Check | Refusal code and detail |
|---|---|---|
| 1 | `agents.permits(choice)` | `outside_allowed` |
| 2 | The harness binary is on the user's PATH (`detect::System::installed`, no process run) | `harness_missing`: "Codex is not installed (looked on your PATH)" |
| 3 | For an `api_key` or `gateway` account, `Keys::saved(account)` | `key_missing`: "No key saved for OpenRouter. Add it in Settings > Providers". If the check itself fails: `key_unreadable`. The key is never read. |
| 4 | **Route handshake** (non-Claude seats only): this build has the core route (6b), **and** the installed `~/.claude/workflows/vajra-deliver.js` declares the literal marker `seats: 'vajra.seats/1'` in its `meta`. The core reads it through the allowlisted reader the preflight already uses. | `core_route_unavailable`: "This Vajra install can't run Codex workers yet. Update Vajra." |

Check 4 also guards against version skew between the app and the installed workflow. Nothing else is checked (H8): login state is advisory, and model ids are opaque. A model the CLI rejects comes back at run time as a typed `bad_request` error (PR 5).

**At proposal.**
- Every worker is checked.
- If any fails, `propose_run` returns `invalid_args` with `refusals: [{slot, code, detail}]` listing **every** failing slot, and nothing is written.

**At start.**
- `start_proposal` re-checks every seat after applying the overrides.
- If any fails, it answers `{"error":"worker_unavailable","refusals":[...]}` before the `requested` line is written. The proposal stays open.

### 16.3 `start_proposal` overrides

The call becomes `start_proposal(id, proposal_sha256, overrides?)`, with `overrides = [{slot, account, model, effort}]`, at most one per slot.

**Each override:**
- names an existing slot (otherwise `unknown_slot`);
- passes `launchable`, so a choice outside `allowed` is refused for that slot with `outside_allowed`;
- is recorded as `source: "user"`.

**What the start records.** Overrides are not part of `proposal_sha256`, which covers what the DG proposed and the human read. Before anything spawns, the start writes:
- `seats` (the final table) and `seats_sha256` on the `approve` `requested` line;
- the guarded record `control/lead/<run>/seats.json`, atomically, next to `PROVIDER_FILE`.

If either write fails, the start is refused.

**`seats.json` is the only authority for dispatch** (16.5). On resume it is carried to the new run (16.5, "Run end and restart").

**The start receipt.** It has one entry per seat: `{slot, role, source, requested{account, harness, model, effort}, route: "subagent"|"core"}`. `route` is the result of check 4, so it never claims a route the install lacks. The per-session launch receipts (PR 4) follow as each worker starts.

### 16.4 Per-worker limits

`Capabilities` (`adapter.rs`) become per worker. Each crew-tree node and each `project_state` worker gains `controls: {message, redirect, stop: "hook_block"|"terminate", reason}`, taken from the seat's engine.

| Seat | Message / Redirect | Stop | Reason shown |
|---|---|---|---|
| Claude (subagent) | yes | sticky hook block (ADR-8) | — |
| Codex (core) | **no** | **ends the process group** (SIGHUP, then SIGKILL after `KILL_GRACE`) | "This worker can't take a message mid-run." / "Stop ends this worker at once; its uncommitted work stays in its worktree." |

**Crew-tree identity.**
- A core worker is a node `core:<ticket>` under its task node, with the relay nested under it, not shown as its own worker.
- `message_worker` and `redirect_worker` refuse every `core:` id with `not_supported_by_harness`, in the service, for the DG too.

**Stop.**
- `stop_worker` on a `core:` node keeps the human confirmation (ADR-8).
- On confirm, the core kills the session (`Supervisor::kill`; `stop_asked` suppresses `provider_error`) and places a sticky stop on the relay.
- The pending `await_worker` answers `stopped`.

### 16.5 Caps, seats as queues, and the dispatch path

**Caps.** The real caps live in `desktop/caps.rs`, not `nap.rs`, and they are not the "1 run, 16 workers" of the PR 6 brief:
- 1 run per project;
- `max_runs` runs in the app (default 2, from 1 to 4);
- a ceiling of 8 workers across concurrent runs, counted at each run's recorded `maxInFlight` (1 to 16).

They stay as they are. A core worker counts against:
1. **its run's `maxInFlight`.** A relay is one of the workflow's parallel steps, and the core also enforces the limit across the run's live core sessions;
2. **the app ceiling**, unchanged, whatever the provider;
3. **an optional per-account cap** (Q7): `agents.account_caps`, `#[serde(default)]`, from 1 to 16 per account.

**A seat is a queue, not a lock.**
- `dispatch_worker` names the **task's owner** (a writer id), not a slot. The core chooses the seat:
  - one with that role, `route: core`, and the task's security flag allowed (16.1);
  - the first free one, or else the one with the shortest queue.
- A seat runs **one task at a time**. A dispatch to a busy seat, or one over a cap, is **queued** (`state: queued`, with its position). It is never refused.
- So three docs tasks on one docs seat run one after another, and the run never asks the user about it.
- A task whose role has no core seat, or a security task, never reaches the relay. The workflow keeps it on a Claude writer.

**The lead→worker path.**

```
workflow (run session, Claude)   [E13]
  └─ for a task whose owner maps to a core seat (seats arg, 16.3):
     step(task.<id>.r<n>, 'vajra-relay', relayPrompt, S.RelayReport, {isolation:'worktree'})
       relay subagent (Claude, inside the guard, own worktree W, model pinned in its definition)
         1. git checkout -B <branchFor(id,n)> <from>; merge deps                (as writers do today)
         2. vajra-dispatch.dispatch_worker {task_id, round, owner, brief, cwd: W}   ──► core → ticket
         3. loop: vajra-dispatch.await_worker {ticket, wait_s ≤ 110}               ──► core
         4. outcome done|progress: git rev-parse --git-dir == gitdir from core; git add -- <task files>;
            git commit (message and trailer built by the core); checkout --detach
         5. return RelayReport {outcome, summary (from the core), branch, commit, files, leftovers}
runTask maps RelayReport: done → ok; progress → continue from commit next round;
  failed → ok:false (rework next round); needs_user|stopped → needsUser
```

- **The relay's prompt** is code-built. Its Bash is limited to the fixed git commands above (the agent's tools list, and the guard for `agent_type: vajra-relay`, spike S5).
- **The relay never decides anything.** The summary, the commit message and the trailer `Vajra-Worker: <account> <model>` all come from the core's answer, built in code and wrapped as data. The untrusted worker text never becomes the relay's instructions, and the core removes any `STOPPED_RE` match from it, so a worker cannot fake a Stop.

**What `RelayReport.outcome` means, and what the core state was.**

| Outcome | Core state |
|---|---|
| `done` | `exited` with the task's files changed. |
| `progress` | `partial`: the tree changed, then a retryable error happened (below). |
| `failed` | `failed` (no typed human-facing error), `timed_out`, `abandoned`, `tampered`, or no changes. The workflow reworks the task next round. |
| `needs_user` | A PR 5 `asking_user` event: auth, quota, or a rate limit longer than 15 minutes. |
| `stopped` | The human stopped the worker. |

Only `needs_user` and `stopped` stop the run at `need_input`.

**Transport (`vajra-dispatch`, E14).**
- **The server.** A run with core seats gets `--mcp-config <control>/<session>/dispatch-mcp.json`, an MCP server named **`vajra-dispatch`**, so its tools never collide with the Lead's.
  - It is started with `VAJRA_RUN_DISPATCH=<run_id>` and a per-run random `VAJRA_DISPATCH_TOKEN`. The guarded record keeps only the token's sha256.
  - It serves only `dispatch_worker`, `await_worker` and `cancel_worker`.
  - It handles `tools/call` **concurrently** (one thread per request, replies written under one lock), so parallel relays never queue behind each other's waits.
- **The app side.** The socket sends these tools to a new App-side `Dispatch` handler, **not** to the control `Service`. That handler owns the supervisor calls and parks each `await` on that ticket's condvar, so the Service mutex is never held across a wait. The client's read timeout for `await_worker` is `wait_s + 10` s; the 30 s default stays for every other tool.
- **Who may call.**
  - Only the `vajra-relay` definition lists the tools; every writer definition has an explicit tool list without them.
  - The guard's E2 matcher widens to `mcp__vajra-(desktop|dispatch)__.*` and audit-logs every call.
  - The socket checks the peer uid, as today, and for dispatch tools also the **peer pid** (`LOCAL_PEERPID`). That process must descend from the run session's PTY process. A Codex worker can read `dispatch-mcp.json` (Q3), but it is a child of the core, not of the run session, so it is refused (T3).

**`dispatch_worker` checks, in order.**
1. The token matches the run's record, the peer pid check passes, and the run is live.
2. `task_id` matches `<run>.t<k>`, `round` is 1 to 3, and `owner` maps to a core seat in `seats.json`.
3. **Idempotent:** a second dispatch for the same `(task_id, round)` returns the existing ticket.
4. The `cwd` check (below).
5. `launchable` passes again for the chosen seat, so a key removed mid-run is caught here.
6. If the seat or a cap is busy, the dispatch is queued.

**`brief`.**
- It is plain text, at most 30 KiB (so the brief plus the fixed sentence stays under `argv::MAX_ASK_BYTES`), and redacted with the run's nonce set.
- The core writes it to `<data>/workers/<run>/<task>-r<round>/brief.md` and passes it after `--`. It adds one fixed sentence: "Do not commit, push or switch branches. Leave your changes in the working tree; Vajra commits them."

**The `cwd` check.**
- `cwd` is opened no-follow and must be a directory below `<project root>/.claude/worktrees/`.
- Its `.git` must be a regular **file** whose `gitdir:` resolves under the project's common dir (`caps::try_key_of_entry`). That rules out the main checkout.
- Its HEAD, read through the bridge, must be on `refs/heads/<branchFor(task, round)>`. That rules out a writer's or the merge step's worktree.
- It must not be the cwd of any other live core session.
- The core records **`dispatch_head`** (HEAD's sha) and a hash of the `.git` file.

**Leases, timeouts and retries.**
- **Lease.** If a ticket gets no `await` for `2 × wait_s + 30` s (the relay died, ran out of plan or context, or stopped polling), the core kills the worker and the ticket reads `abandoned`. The seat frees and the queue moves on.
- **Wall-clock budget.** A worker has `worker_timeout_s` (default 30 min); when it runs out, the worker is killed and reads `timed_out`.
- **Running answers are small.** An `await` that finds the worker still running returns only `{state, attempt, elapsed_s, queue_position}`.
- **Retries** belong to the core (PR 5 `run_with_policy`), and happen **only while the tree has not changed** since `dispatch_head`. That is checked as `git status --porcelain=v1 -z --ignore-submodules=all` being empty, through the bridge.
  - Each retry is a new session on the **same** seat, account, model and cwd.
  - Once the tree has changed, a retryable error ends as `partial`. The relay commits the task's files, and the workflow continues from that commit (its existing `progress` path).
- **Error events.** `ProviderTag` gains `attempt`, and the supervisor's exit report is the only emitter. Today `sessions/mod.rs:599-600` always reports the decision as attempt 1.

**The final `await` answer** `{state, ticket, seat, session_id, exit_code, attempt, summary, receipt, provider_error, diff, gitdir}` is sent only after the process group is gone and these checks pass:
- the `.git` file is unchanged and is still a file;
- HEAD still equals `dispatch_head` (the worker did not commit or switch branches);
- no new nested `.git` entry exists in the tree.

If any check fails, the state is `tampered`.

The fields of that answer:

| Field | What it holds |
|---|---|
| `summary` | Code-built: the state, plus the first 300 characters of the worker's last message, redacted (PR 5 redaction plus the nonce set) and wrapped as untrusted data. |
| `receipt` | The PR 4 `LaunchReceipt`. |
| `diff` | A `Diff` (16.7) of `cwd` **against `dispatch_head`**, so dependency merges are not counted as the worker's. |

The relay stages **only the task's paths** (`git add -- <task files>`). Other changed paths go into `leftovers`, which the run card shows; they are never committed.

**Run end and restart.**
- **When a run ends or is stopped,** the core kills that run's core workers. Pending awaits answer `stopped`, and queued tickets are dropped.
- **Restart.** Runs don't survive an app quit (ADR-9), so after a restart nothing is adopted.
- **The ticket log.** `<data>/workers/<run>/tickets.jsonl` is guarded and append-only. The run card lists every ticket that ended with uncommitted work, with its worktree and diff (Q8).
- **Resume** (`resume_run`) gives the new run the same seats:
  - it copies `seats.json` from the newest run of the same session and campaign that the guarded record vouches for, as `original_workers` does;
  - it writes a new `dispatch-mcp.json` with a **rotated** token;
  - the resume argv (`build_resume_argv`) gains `--mcp-config`, and the workflow gains its `seats` argument.
  - The approval-only resume (ADR-14) gets neither, because it runs no workers.

### 16.6 Receipts for both kinds of seat

- **A core seat** gets PR 4 as built. `effective` comes from the rollout file of the `thread_id` in the worker's `--json` stream:
  - under the Vajra-owned `<data>/codex-home/<account>` for a gateway account;
  - under `~/.codex` for a login account (Q4).
  - It reads `unknown` when the file is not found, and a mismatch emits `launch_mismatch`.
- **A Claude subagent seat:**
  - `effective.model` is `message.model` from the first assistant record in the subagent's transcript, read through the existing bounded, allowlisted reader (`discover_workers`);
  - `read_from` is `harness_transcript`, and billing stays `unknown`;
  - the subagent is matched to its seat by its step label `task.<id>.r<n>` and its owner's role.

### 16.7 The worker diff without one git process per new file (follow-up to #128)

**The problem.** `diff_from_git` runs one `git diff --no-index` per untracked file, at about 19 ms each.
- At the 2 s budget that is about 107 files, so 500 new files leave most lines uncounted and the diff is marked `partial`.
- It also costs CPU while the run is busy.

**The design.** The tracked changes (`git diff --numstat -z <base>`) and the new-file list (`git ls-files --others --exclude-standard -z`) stay one git call each, and gain `--ignore-submodules=all`. **Only the line count of each new file moves into the core,** as a direct read.

**Opening the file.** `count_new_file_at(root: &File, rel: &[u8])` does this:
- It holds the worktree root open with `O_DIRECTORY|O_NOFOLLOW`.
- It opens each directory part with `open_at(.., O_DIRECTORY|O_NOFOLLOW)` (`preferences::open_at`, the same walk as PR 4's `rollout_file`).
- It reads the last part with `fstatat(AT_SYMLINK_NOFOLLOW)`.
- **Paths are bytes.** The `-z` output is split as `&[u8]`, so a non-UTF-8 name is still counted; it is shown lossily only in the UI.
- These parts are refused and the diff marked `partial`: an empty part, `.`, `..`, a NUL, a path longer than `PATH_MAX`, and a trailing `/`. `ls-files` lists a nested repo as `sub/`.

**Counting.** The rules follow `git diff --numstat`:

| Entry | Lines added |
|---|---|
| A symlink | **1**, as git counts its target text. The link is never followed. |
| A regular file | It is opened with `O_RDONLY\|O_NOFOLLOW\|O_NONBLOCK`, `fstat` is checked again, and it is read in 64 KiB chunks. The count is the number of `\n`, plus 1 if the file is non-empty and does not end in `\n`. |
| A file with a NUL in its first 8000 bytes (git's binary test) | 0, like numstat's `-`. |
| An empty file | 0 |
| A FIFO, socket or device | 0, and `partial`. It is never opened. |

**Bounds.**

| Bound | Value | Past it |
|---|---|---|
| `PER_FILE_CAP` | 8 MiB | The lines read so far are counted, and the diff is `partial`. |
| `TOTAL_READ_CAP` | 64 MiB | Counting stops, and the diff is `partial`. |
| `MAX_UNTRACKED` | 5000 files counted | Counting stops, and the diff is `partial`. |
| `UNTRACKED_BUDGET` | 2 s | Counting stops, and the diff is `partial`. |

Every file is still listed. Small files cost tens of µs, so 500 new files take well under 100 ms.

**Known differences from git:**
- a `.gitattributes` `binary`, `-diff` or `working-tree-encoding` mark is not applied, so such a file is counted as text;
- the count is never more than the file's real newlines.

`diff_from_git` keeps its signature, and the bridge stays read-only.

**How `partial` reaches the UI.**
- `Diff` gains `uncounted: u32`: the new files that were listed but not counted.
- The crew-tree worker node and `project_state.workers[]` gain `diff: {files_total, files (first 200), added, removed, partial, uncounted}`. A core seat's diff comes from its final `await`; a subagent seat's from the receipt built at the end of the run.
- `workerProvider.ts` parses it, and the worker card shows:
  - `+N −M` when `partial` is false;
  - **`≥ +N lines`** when it is true, with the accessible label "At least N lines added. U new files were not counted (too many or too large)."
- It is never presented as exact. The strings go in `docs/design/changed-strings.md`.

### 16.8 What runs outside the guard, and each process's environment

| Process | Guard | Sandbox | Environment and argv | Can reach |
|---|---|---|---|---|
| Run session (Claude Code, `/vajra-deliver`) | Vajra guard | Claude Code's own, if enabled | `ENV_ALLOW`. Argv gains `--mcp-config` only when the run has core seats. | Unchanged |
| `vajra-dispatch` MCP server (`vajra --mcp`, started by Claude Code) | none (ours) | none | What Claude Code gives a stdio server, plus `VAJRA_RUN_DISPATCH` and `VAJRA_DISPATCH_TOKEN` | Only the app socket; it forwards requests and holds no state. |
| Relay subagent | **Vajra guard** | as the run | the run's | Its worktree, fixed git commands, and the dispatch tools |
| Codex worker, gateway account | **None**: the Vajra guard is a Claude Code hook | Codex `workspace-write` (Seatbelt): writes only to cwd (`/tmp` and `$TMPDIR` excluded), **reads anywhere**, no shell network | `ENV_ALLOW` + `CODEX_HOME=<data>/codex-home/<account>` (a generated config); stdin `/dev/null`; **no key**. Argv is PR 3's `build_gateway_argv`. | The gateway API through Codex's own process. The key comes from `/usr/bin/security` (`auth.command`), outside the sandbox; a shell inside the sandbox can't read the item (spike). |
| Codex worker, login account | **None** | The same, forced from argv so the user's config can't widen it | `ENV_ALLOW` only; the user's `~/.codex`. **A new `build_login_worker_argv`** (the interactive `Codex::run_argv` is not used): `/bin/sh -c 'exec "$@" </dev/null' sh codex exec --json --sandbox workspace-write [-m <model>] -c sandbox_workspace_write.network_access=false -c 'sandbox_workspace_write.writable_roots=[]' -c sandbox_workspace_write.exclude_slash_tmp=true -c sandbox_workspace_write.exclude_tmpdir_env_var=true -c 'mcp_servers={}' -c 'notify=[]' [-c model_reasoning_effort=…] -- <brief>` | OpenAI, through the user's login |
| `git` from the core | none (ours) | none | The bridge's `BASE_PATH`, `-c core.fsmonitor=`, `--no-ext-diff --no-textconv`, and `--ignore-submodules=all` on `diff` and `status` | Read-only, in W, only after the worker's process group is gone |

**Threats this design names.** Bhishma reviews 6b and 6c.

| # | Threat | Handling |
|---|---|---|
| T1 | A worker reads secrets. | Q3: disclosed on both cards, and the project can limit `allowed`. |
| T2 | A worker plants git config, hooks or a submodule to run code inside a later trusted git call. | The `.git`-file, HEAD and nested-`.git` checks before `exited`; the relay compares `--git-dir` just before committing; `--ignore-submodules=all`; the bridge's no-helper flags. A `.gitattributes` file can only name filter drivers that the user's own trusted config defines. |
| T3 | A forged dispatch. | The token is not enough on its own, because a worker can read files. The peer-pid check refuses any caller outside the run session's process tree. What remains is `seats.json`, one task per seat at a time, the caps and `launchable`: the worst a forgery can do is run an approved seat on its approved account. |
| T4 | A worker outlives Stop. | Killing the group misses a grandchild that calls `setsid`. That grandchild stays in the sandbox, with no network and writes only to W. It could still rewrite `.git` after the check; the relay's `--git-dir` comparison narrows this. **Residual risk**; spike S1 tests `setsid`. |
| T5 | Key exposure. | No key in argv, env, files or logs (PR 2 and PR 3 tests); worker text is redacted (PR 5). |
| T6 | Prompt injection through worker output. | The worker's text reaches only the code-built `summary`, wrapped as data, with `STOPPED_RE` matches removed. The relay's Bash is limited to fixed commands. |

**Evidence against.**
- The relay costs a pinned-model Claude subagent per core task: about 4 tool calls plus one `await` every 110 s.
- A core worker can't take a message mid-run.
- A Codex worker can read the user's files.
- A seat holds a whole role (Q5).

**Would change our mind.**
- Codex gains a hook channel: messages become possible and (c) gets cheaper.
- The Workflow tool gains model-free script steps: the relay becomes a script.
- Codex gains read confinement: T1 closes.

### 16.9 To verify (each item names the PR that needs it)

| # | Item | PR | If false |
|---|---|---|---|
| S1 | Does Seatbelt `workspace-write` keep `<cwd>/.git` read-only, including a linked worktree's `.git` **file**? Does it allow `setsid`, and connecting to a unix socket? | 6b | The core checks still hold. Record the result in T2–T4. |
| S2 | `codex exec` works in a linked worktree with no write access to the common dir: it reads history and runs `git diff`. | 6b | Add `--skip-git-repo-check`, and note that the worker sees no history. |
| S3 | `-c 'mcp_servers={}'`, `-c 'notify=[]'`, `writable_roots=[]` and `network_access=false` override the user's config. `model_providers.openai.request_max_retries=0` and `stream_max_retries=0` apply to the built-in login provider. | 6b | Q4's fallback (no login-route workers). Without the retry override, the core adds **no** retries of its own on the login route. |
| S4 | A Claude Code subagent's MCP tool-call timeout allows a 110 s `await_worker`. | 6c | Lower `wait_s`. |
| S5 | A subagent whose definition lists `mcp__vajra-dispatch__*` can call a server from the parent's `--mcp-config`. Agent `tools` accept Bash command patterns. | 6c | Give the relay its own `mcpServers`, and have the guard enforce the Bash limit. |
| S6 | The 16.7 counter matches `git diff --numstat --no-index` on the fixtures, recording the git version: text, no final newline, CRLF, empty, binary, symlink, nested folders, a newline in a name, a non-UTF-8 name, a nested repo, a FIFO. | 6b | Fix the counter. Parity is a test, not an assumption. |

### 16.10 Split, files and acceptance tests

The work is well over 800 lines, so it ships as **three** PRs. 6a and 6b need no engine change; non-Claude seats stay refused (16.2, check 4) until 6c is installed.

#### 6a: proposal seats and start validation

About 700 lines, half of them tests.

**Files:**
- `control/service/proposals.rs` (`workers`, `max_in_flight`, `ProposalV2` hash, ledger parse);
- `control/service/tools.rs` and `control/mcp.rs` (schema);
- `desktop/proposals.rs` (overrides, re-check, `seats.json`, start receipt, `max_in_flight` instead of the constant);
- `desktop/lead.rs` (`record_run_seats`, `read_run_seats`, copy on resume);
- new `providers/launch.rs` (`launchable`, the handshake);
- new `crew/roles.rs`;
- `preferences/mod.rs` (validate `roles` keys);
- the inbox items;
- `app/ui/src/screens/orch/dgApi.ts` (types only).

**Acceptance tests:**
- **The hash.**
  - It changes when any field of any worker changes, or `max_in_flight` changes, and Start is refused with `proposal_changed`.
  - A v1 proposal (no workers) hashes and starts as before.
  - A preference edit between propose and start does **not** change the hash.
- **Overrides.** An override outside `allowed` is refused for its slot (`outside_allowed`), and every other failing slot is listed too.
- **A missing binary** is refused with `harness_missing` and its detail, both at proposal and at start (after an uninstall in between).
- **Keys.**
  - `key_missing` reads no key: the test uses a fake `Store` that panics on any read.
  - A `Store` error gives `key_unreadable`.
- **The handshake.** A Codex seat with no route handshake is refused with `core_route_unavailable`.
- **Malformed input is refused:**
  - a `source` supplied by the DG;
  - an unknown role or slot;
  - `workers: []`;
  - a model on a Claude seat.
- **The roles table** matches the workflow's `ALL_WRITERS` (parity test).
- **A failed `seats.json` write** refuses the start and leaves the proposal open.

#### 6b: core worker supervision and the dispatch handler

About 950 lines with tests. Split out the 16.7 counter as **6b-1** if the review asks.

**Files:**
- new `desktop/workers.rs`: seat queues, dispatch, await, cancel, lease, timeout, the cwd and `.git` and HEAD checks, `tickets.jsonl`, and killing workers when the run ends;
- `control/socket.rs` (route dispatch tools to the App handler; the peer-pid check; the per-call read timeout);
- `main.rs` (wiring);
- `sessions/mod.rs` (the PR 3 env in `SpawnSpec`; `attempt` in `ProviderTag`);
- `sessions/argv.rs` (`build_login_worker_argv`);
- `control/service/tools.rs` (`not_supported_by_harness` for `core:` ids);
- `desktop/caps.rs` (count live core sessions per run and per account);
- `preferences/mod.rs` (`account_caps`);
- `providers/effective.rs` (the 16.7 counter, `uncounted`, `--ignore-submodules=all`);
- `vajra/bridge.rs` (`--ignore-submodules=all` on `diff` and `status`);
- `desktop/state.rs` and `crew/tree.rs` (`core:` nodes with `controls`, `diff`, `provider`);
- `app/ui/src/workerProvider.ts` and the worker card's diff line.

**Acceptance tests:**
- **Caps and queues.**
  - With 3 docs tasks, 1 docs seat and `maxInFlight` 4, all three run one after another with no refusal.
  - Caps hold across mixed providers, and the app ceiling is unchanged with Codex seats.
- **Controls.**
  - A Codex worker's Message control is disabled with its reason, and `message_worker` on a `core:` id is refused in the service.
  - Stop ends the process group, the `await` answers `stopped`, and no `provider_error` fires.
- **Dispatch refuses** each of these:
  - a bad token, or a caller pid outside the run's process tree;
  - an owner with no core seat;
  - a `cwd` that is the main checkout, outside `.claude/worktrees`, or on the wrong branch.
- **Dispatch is idempotent** for the same (task, round).
- **Tampering.**
  - A worker that rewrites its `.git` file, commits, or creates a nested `.git` gets `tampered`.
  - A symlinked folder in a path is not followed, and a FIFO is not opened.
- **Lease and timeout.**
  - A stub relay that stops polling has its worker killed and marked `abandoned` within the lease.
  - `worker_timeout_s` gives `timed_out`.
- **Retries.**
  - A 504 on an unchanged tree retries twice on the same account (fake clock), then reports `blocked`.
  - A 504 after an edit reports `partial`.
- **The diff** is taken against `dispatch_head`, so a merged dependency's lines are not counted.
- **End to end, with a fake harness binary.** A script on a temp PATH prints spike-shaped `--json` events, writes a rollout file and edits a file. Two core workers on different accounts (Codex login and the gateway) each return a matched launch receipt, a diff and an `await` result. A test-driven relay stands in for the Claude lead: this is the brief's "lead plus two workers".
- **The counter** matches git numstat on the S6 fixtures, and 500 new files finish in under 300 ms with `partial == false`.

#### 6c: the engine route (E13, E14)

About 350 lines. It waits for Q2.

**Files:**
- `workflows/vajra-deliver.js`: the `seats` argument; `meta.seats`; relay routing in `runTask`; the `RelayReport` schema and how its outcomes map; security tasks stay on Claude;
- new `bundled-agents/vajra-relay.md`: Bash limited to the fixed git commands plus the two dispatch tools, a pinned low-cost model, and `maxTurns` bounded;
- `sessions/argv.rs`: `--mcp-config` on the run and resume argv when the run has core seats;
- `desktop/mod.rs`: writes `dispatch-mcp.json`, and generates and rotates the token;
- `control/mcp.rs`: the `vajra-dispatch` server mode with concurrent `tools/call`;
- `tests/workflows`: parity, plus relay routing with a stubbed `agent()`.

**Acceptance tests:**
- A run with no core seats gets a byte-identical argv and workflow args.
- A core-seat task goes through `vajra-relay`, and its commit passes `checkChange`.
- Outcome mapping:
  - `failed` reworks the task next round, without `need_input`;
  - `stopped` holds the task back;
  - `progress` continues from the commit.
- A security task stays on Claude unless its seat sets `security: true`.
- A resumed run with core seats lists the three tools, with a rotated token.
- The run session's dispatch server lists exactly those three tools.

**Reviewers.**
- The standard gate on all three PRs, plus:
  - api-and-interface-design on 6a;
  - bhishma (security-and-hardening) on 6b and 6c;
  - a duryodhana pre-mortem on 6c, because it goes to the owner's install.
- GPT once per PR, on the final head:
  - gpt-6.1-sol **high** on 6b and 6c;
  - gpt-6.1-sol medium on 6a.

## 7. Error handling

| What fails | Detection | Handling | User sees |
|---|---|---|---|
| `claude` missing | `which`/spawn ENOENT in preflight | Start disabled | Preflight row FAIL, hint "Install Claude Code", AC-1.3.2 |
| Vajra hooks not registered / doctor FAIL | `vajra-doctor.sh` exit code and its PASS/FAIL lines | Start disabled; control disabled if E1 is not registered | Row FAIL with `bash ~/.claude/skills/vajra/scripts/install.sh` |
| Workflow file missing | Allowlisted existence check of `~/.claude/workflows/vajra-deliver.js` | Start disabled | AC-1.3.4 text |
| Campaign signature failure | `vajra-campaign.sh verify <slug>` exit 3 | Resume blocked; listed with a badge | "Integrity check failed" plus the restore command (`resume <slug> --from-checkpoint N`), AC-1.2.2 |
| Task graph locked | `vajra_tasks.py` exit 1, message "tasks.json is locked" | The app only reads; reads do not lock. Retry the read with backoff | Nothing, unless it persists > 2 min: "task graph locked by another run" |
| `tasks.json` mid-replace / parse error | JSON error | Keep the last good snapshot and retry on the next event | Stale marker on the graph header |
| Transcript format change | Fingerprint check (expected top-level keys and record types) fails | Stop structured parsing for that file and show the raw view (plain-text lines); crew identity falls back to `tasks.json` | Banner "Transcript format not recognized; showing raw view" |
| Worker gone when message sent | Worker finished or not live at send; or entry expired | Refuse at send; expire later | Card "not delivered: worker finished", AC-3.2.5 |
| Stop races with completion | Worker ends before the next tool call | Stop file removed at run end; ledger `not_delivered` | "Worker finished before stop" |
| Hook claimed but no ack | Inflight file > 60 s | Never re-queue | "delivery unknown" |
| Disk full on spill | ENOSPC on spill write | Stop spilling for that stream and keep the ring buffer; ledger appends are retried and then surfaced | Status bar warning "disk full: older output dropped"; the ledger error is shown, never silent |
| MCP server down / socket refused | The MCP binary cannot connect | Returns MCP error `app_unavailable`; Lead reports it | Lead says control unavailable; UI controls still work |
| App quits mid-run | n/a | Children receive SIGHUP per process model (spike). Queue entries expire. | On restart: "run session ended with the app" or reattach (daemon option) |
| Script bridge timeout | Timer | Kill the process group | Inline error with the script name and exit code |
| Knowledge search fails | Non-zero exit | n/a | "knowledge search unavailable (exit N)"; empty output shows "no results", AC-6.2.2 |
| Evidence check fails | E3 `green:false` | Downgrade the row | "unverified evidence" with reasons, AC-4.3.3 |

## 8. Security considerations (summary; the Security role reviews later)

Assets:
- the user's repos;
- Vajra's signed state and key (never touched);
- the integrity of control actions;
- secrets that may appear in agent output.

Adversary: agent output and repo content, prompt-injected or malicious, plus other local users. Same-user
malware is out of scope, consistent with SECURITY.md "Same-user trust boundary".

- **IPC surface.** One Unix socket, mode 0600, in a 0700 dir. Peer uid checked on connect
  (`getpeereid`/`LOCAL_PEERCRED`); peer pid logged. No TCP or network listener. The UI-core IPC is internal
  to the toolkit and exposes no remote origin. Requests are schema-validated with size limits (64 KB).
- **Command injection.**
  - Agent and user text never reaches a shell. The Script bridge uses argv arrays with fixed executables.
  - Git paths come only from allowlisted worktrees, with `--` before pathspecs.
  - Agent text is rendered as text only (NFR-SEC-3, SEC-4).
- **Terminal.**
  - ANSI sanitizing: only SGR and cursor movement inside the terminal pane. Title-setting, OSC 8
    hyperlinks and OSC 52 clipboard are off by default.
  - No escape-sequence passthrough into non-terminal views; transcripts are rendered as plain text, never
    through a terminal emulator.
- **Filesystem scope.** One `open_allowlisted(path)` function. It canonicalizes with realpath, rejects
  symlinks that escape, and allows only:
  - `~/.claude/vajra/campaigns/<slug>/{campaign.json,features.json,tasks.json,progress.md}`;
  - names in `checkpoints/`, plus `.active`;
  - the tails of `~/.claude/vajra/audit.log` and `routing-log.jsonl`;
  - `~/.claude/projects/<slug>/` transcripts;
  - the existence of `~/.claude/workflows/vajra-deliver.js`;
  - the app data dir;
  - registered repos (for git, through the bridge).

  `.hmac-key` is on an explicit deny list checked first, and a test asserts the app never opens it. No
  recursive watch on `~/.claude/vajra` (NFR-SEC-6). `kb/` is only accessed through `vajra_kb.py`.
- **Writes.** Only the app data dir. A test diffs `~/.claude` FS events by process (AC-1.1.3).
- **Secrets in streams.** Display masking of common token patterns is best effort, and the docs say so.
  Stream data is never sent anywhere (no telemetry of content). Spill files are 0600 and deleted by LRU and
  at run deletion. App logs carry no agent text (NFR-OBS-1).
- **Control integrity.**
  - Messages are wrapped as untrusted data, and the preamble states they are never approval (NFR-SEC-8).
  - Stop needs human confirmation.
  - Approve and Take over are human-only UI actions and are not reachable from MCP.
  - The queue dir is a guard-protected root (E2).
  - The hook validates `session_id` before building paths and never evals payload text.
- **Update channel.** Signed update manifest plus a notarized, hardened-runtime build (NFR-SEC-7). Updates
  are applied only after signature check, over HTTPS, with no downgrade.
- **Supply chain.** Lockfiles committed. `cargo-deny`/`cargo audit` or `npm audit` in CI per the chosen
  stack. SBOM per release. Minimal dependencies (ponytail: platform first).
- **All agent output is untrusted**, including what the Lead says about workers. The Lead's tool results
  mark agent text as untrusted.

## 9. Performance design

- **Readers never block.** Each PTY and transcript has a dedicated reader that writes into a per-stream ring
  buffer (256 KiB, NFR-PERF-2). Overflow appends to `spill/<stream>.log`, with a total spill cap (default 1 GB, LRU by run).
  Scrolling past the ring reads spill pages on demand (AC-2.2.2).
- **Backpressure.** The bus keeps one bounded queue per subscriber (e.g. 1024 events). On overflow it drops
  that subscriber's queue and sends a `snapshot_needed` marker; the subscriber pulls a compact snapshot (last
  N lines plus counters). Producers never wait (NFR-PERF-5).
- **Rendering.** Virtualized lists (crew tree, task graph list, history, ledger) and virtualized terminal
  scrollback. Unselected workers update only a counter and an activity dot (AC-2.2.3). Repaints are batched
  per animation frame.
- **File watching.** FSEvents on:
  - the campaigns root (directory entries only);
  - the active campaign dir;
  - the run's project transcript dir;
  - `audit.log` (tail by size; reopen on inode change).

  Events coalesce for 150 ms, then the reader re-reads only changed files, skipping when size and mtime are
  unchanged. `tasks.json` is small and re-read whole; jsonl and logs are read from the last offset.
- **Script calls.** Cached per input with short TTLs: `verify` per campaign per change of `state.sig`
  mtime, `check-evidence` per (evidence sha256, recorded head); the stale check per (evidence sha256, current tree id), doctor on demand.

Budgets: NFR-PERF-1 to NFR-PERF-11 in [requirements.md](requirements.md#performance), adopted from the
stack spike and reviewed by both models. They replace the provisional table that was here (A1 finding H1).
The retained tail per worker is 256 KiB (NFR-PERF-2), not 2 MB.

## 10. Test strategy and rollback

**Unit.**
- Label parser, crew-tree derivation, the progress-line parsers (`integration`, `round`, `delivered`,
  `pass:`), the KbHit regex, the name mapping and title-casing.
- Allowlist canonicalization (symlink escape, `..`, case, the `.hmac-key` deny), ledger append, queue
  matching rules, ANSI sanitizer (OSC 8/52/title stripped).
- E3: one test per `read_evidence` refusal branch plus green, with no write (directory mtime unchanged).

**Integration.**
- Fixture campaigns under `tests/fixtures/desktop/campaigns/`, copied from the observed shapes in 1.6 and
  anonymized: active, done, abandoned, invalid signature, empty `worktree`, evidence file gone, and stray
  root `*.json`.
- Fixture transcripts (shape from the spike).
- `HOME` points to a temp dir with a test key generated by the scripts themselves, never the real key.
- Script bridge against the real scripts in that temp HOME.
- `hooks/desktop-control.sh` tests in `tests/hooks/`, in the style of `run-pre-tool-use.sh`:
  - fast path;
  - match by `agent_id` / `cwd` / unique type;
  - consumed once under two concurrent hooks;
  - stop blocks with exit 2;
  - expired entries are ignored;
  - malformed payload exits 0;
  - an audit line is written.
- Guard tests for E2: an agent write into the control dir is BLOCKED (path with a space, case variants,
  symlink), and an `mcp__vajra-desktop__x` call audits ALLOW.

**E2E.**
- Playwright on the renderer, with a mocked core for the six jobs and keyboard-only paths (NFR-A11Y-2).
- One real smoke run on a tiny fixture repo with `maxInFlight 2`, checking that message, stop and the
  reconciliation script (AC-3.4.3) agree.

**Load, soak and profiling.**
- Synthetic transcript writer: 10+ agents x 200 lines/s, measuring frame times and RSS (NFR-PERF-2/3).
- 1 h soak with a leak check: heap snapshots or allocation counters at start and end (NFR-PERF-6).
- Flamegraphs of core and renderer under load, attached to the benchmark note (NFR-PERF-7).

**Security.** Red team role tests:
- injection through agent output (ANSI/OSC, fake `<untrusted-data>` close tags, markdown links);
- shell metacharacters in asks, messages and reject reasons;
- socket peer from another uid;
- queue write from an agent;
- `.hmac-key` access attempts (process trace);
- update signature tampering.

**Rollback.** The app is additive.
- Uninstalling it leaves Vajra unchanged except E1-E3.
- E1's registration and E2's matcher entry live in `hooks/hooks.json`, so `install.sh --unregister-hooks`
  (then re-install without them) removes them. E2's `ROOTS` entry is inert without the app.
- E3 is a new subcommand nothing else calls.
- A feature flag `control_channel` (app setting, default on after the spike passes) disables the MCP tools,
  the UI controls and queue writes. With it off the app is a pure viewer and launcher. When the app is not
  running, the hook fast path keeps E1 to a no-op.
- No migrations: app files are JSONL and can be deleted.
