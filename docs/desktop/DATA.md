# What Vajra Desktop reads, writes and sends

Plain answers, checked against the code on `ao/integration/batch-3`. Each claim names the file that does it. Where a claim
is not true yet, it says so. This describes the macOS app in `app/`; the Claude Code skill is covered by
[SECURITY.md](../../SECURITY.md).

## Short version

- It keeps its own files in one folder under your Library, and its agents work in a second one.
- **The app's own code makes one outbound call**, and only if you saved an OpenRouter key (usage meter, below). Everything
  else that leaves your Mac is done by the tools it launches (Claude Code, Codex, `gh`) under your own sign-ins.
- There is no telemetry, analytics, crash upload or auto-update code in the app. See "Nothing else leaves" for what that
  rests on, and what it cannot cover.
- Claude workers the app starts run under the Vajra guard inside Claude Code's OS sandbox, which the app turns on for each
  launch. **Codex seats you drive run with your own settings and are not guarded against key reads**, and the app says so
  on the card.

**Keep this page in step with the code.** It cites batch-3 after #212, #216, #224 and #239. Re-check it when the Claude run
lead (`CLAUDE_LEAD_SPIKED`) or the Claude Code builds Vajra has verified (`CLAUDE_WORKER_PROVEN`) change.

## What it reads

| It reads | Why | Where in the code |
|---|---|---|
| Your project folders (git repos you add) | to show runs, worktrees and diffs | `app/core/src/projects/store.rs` (`add`), `app/core/src/vajra/gitw.rs` |
| `~/.claude/projects/<folder>/<session>/subagents` | Claude Code's worker transcripts, read-only, size-capped, so the app can list workers | `app/core/src/desktop/mod.rs` (`discover_workers`) |
| `~/.claude/vajra/campaigns` and `~/.claude/workflows/vajra-deliver.js` | run progress and a check that the workflow is installed | `app/core/src/vajra/allowlist.rs` |
| A Codex worker's session file under its `CODEX_HOME` | to show which model it really used (the harness's own report, not what was asked for) | `app/core/src/providers/effective.rs` |
| Your login-shell `PATH` | to find `claude`, `codex`, `gh` and `git`. **This runs your login shell, so your shell startup files run and can do whatever they already do.** It has a deadline and runs at most once per refresh gap | `app/core/src/userpath.rs` |
| The results of `claude --version`, the install doctor, the hooks and sandbox status | the first-run checks | `app/core/src/vajra/preflight.rs` |

**What it never reads:** the signing key `~/.claude/vajra/.hmac-key`. The app's file allowlist refuses that name before and
after resolving symlinks, and only stats it to check who owns it (`app/core/src/vajra/allowlist.rs`, `key_path`).
It also never opens a CLI's own sign-in token. A pasted API key goes into the macOS Keychain through the system `security`
tool on stdin, and the app can afterwards only check that an item exists, add one or delete one (`app/core/src/providers/keychain.rs`).
**The key is read back, but not by the app's own process:** when it is used, the system `security` tool reads it
(`find-generic-password ... -w`) and pipes it straight into `curl` for the usage read (`app/core/src/providers/usage.rs`), or
into Codex through the generated auth command for a gateway seat (`app/core/src/providers/codex_home.rs`). So the app's own
process never holds the key, but those two tools do, for the length of the call.

## What it writes

| It writes | Where | Notes |
|---|---|---|
| Its own data | `~/Library/Application Support/VajraDesktop/` (mode 0700; override with `VAJRA_DESKTOP_DATA`) | `app/core/src/paths.rs` |
| An action ledger | `.../control/ledger.jsonl` (0600) | append-only: the file is opened `O_APPEND` and earlier lines are never rewritten. **It is not hash-chained.** (`app/core/src/control/ledger.rs`) |
| The app log | `.../logs/app.log`, rotated at 1 MB, 3 files | `app/core/src/logging.rs` |
| The Director-General's generated settings and MCP config | `.../lead/` | `app/core/src/control/lead_settings.rs` |
| A `CODEX_HOME` per gateway account | under the data folder | `app/core/src/providers/codex_home.rs` |
| Agent worktrees | `~/Library/Application Support/VajraWorktrees/` (0700, a sibling that agents may write) | `app/core/src/vajra/gitw.rs` (`work_root`) |
| A git worktree entry in your repo | `<repo>/.git/worktrees/...`, made with `git worktree add --detach` | removed again on rollback (`gitw.rs`) |
| A pasted provider key | the macOS Keychain, service `vajra.provider.openrouter` (see "What it never reads" for who reads it back) | `app/core/src/providers/keychain.rs` |

The Vajra skill's own state under `~/.claude/vajra/` (the key, the knowledge ledger, campaigns) is written by the skill's
scripts, not by the app.

## What leaves the Mac

**1. The app's one direct call.** If you saved an OpenRouter key, the app asks `https://openrouter.ai/api/v1/key` how much
that key has used. It runs only while Settings > Providers is open or a run is active, at most once a minute per account,
through `/usr/bin/curl` with the key piped on stdin (read by the system `security` tool, not by the app) so it is never in argv,
a log or this process's memory; only four usage numbers are kept. No key saved means no call. (`app/core/src/providers/usage.rs`)

**2. What the tools it launches send, under your sign-ins.** Claude Code and Codex talk to their providers as they always
do; that is the point of the app. Vajra does not add a proxy or forward your keys. Sessions see `PATH, HOME, LANG, TERM, USER,
SHELL` (`app/core/src/sessions/mod.rs`, `ENV_ALLOW`), and a launch may add three more, from a fixed list
(`EXTRA_ENV_ALLOW` in the same file): `CODEX_HOME` (a Vajra-owned one for a gateway seat), `CLAUDE_CONFIG_DIR`, and a
per-ticket `TMPDIR`. None of them is a key. A Codex worker's network follows its seat's setting
(`sandbox_workspace_write.network_access`), and the card says "Internet on for this worker" when it is on
(`app/core/src/adapter/profiles.rs`, `app/core/src/desktop/seat_start.rs`). Three more things leave your Mac that are easy to miss:

- **A Codex gateway seat sends its prompts and code to OpenRouter**, under the key you saved (`app/core/src/providers/codex_home.rs`,
  `DEFAULT_BASE_URL`).
- **The provider checks run each CLI's own login-status command, and that command may contact the provider**
  (`app/core/src/providers/detect.rs`). What it sends is the CLI's business, not the app's.
- **Reading your login-shell `PATH` runs your shell startup files** (`app/core/src/userpath.rs`), which can do anything they
  already do.

**3. A PR comment, only when you click.** "Post proof" runs your own `gh pr comment` for the repo and PR you confirmed. The
text is rebuilt from the record, not taken from the screen. (`app/core/src/desktop/proof_app.rs`, `act_post_proof_comment`)

**Nothing else leaves.** What this rests on:

- The webview's content security policy is `default-src 'self'; connect-src 'self' ipc: http://ipc.localhost`
  (`app/core/tauri.conf.json`), and the window has no shell, fs or http plugin (`app/core/capabilities/default.json`).
- The Rust dependencies include no HTTP client and no updater (`app/core/Cargo.toml`); a search of `app/core/src` and
  `app/ui/src` finds no telemetry, analytics, crash-report, upload or updater code.
- The diagnostics preview is built locally, redacted, and shown for you to copy. Its module has no network or process code
  and a test pins that (`app/core/src/diagnostics.rs`).

What this cannot cover: what Claude Code, Codex, `gh`, your MCP servers or your own hooks do on their own. The app can
disclose that, and does on each seat card, but it does not control it.

## What the guard and the sandboxes block

**The Director-General (the project's lead).** It runs with Read, Grep, Glob and six control tools, no user settings
(`--setting-sources ""`) and only the MCP servers the app gives it (`app/core/src/sessions/argv.rs`, `build_lead_argv`).
The app writes its settings file, which puts the Vajra guard back and denies reads of: `~/.claude/vajra`, `~/.claude/skills`,
Claude settings and credentials, the app's own data folder, `~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.kube`, `~/.config/gh`,
`~/.docker/config.json`, `.netrc`, `.npmrc`, `.pypirc`, `.pgpass`, `.git-credentials`, plus `.env` and SSH key files in the
repo, and every other project's folder (`app/core/src/control/lead_settings.rs`). It will not start unless the install
doctor passes and reports Claude Code's OS sandbox as active with its write-deny on the app's data folder; otherwise the app
says "Asking the Director-General needs the Claude Code sandbox. Run /sandbox in Claude Code, then retry."
(`app/core/src/desktop/lead.rs`, `doctor_ok`, `SANDBOX_NEEDED`).

**Claude workers.** Two cases:

- **Workers the app starts** run under the Vajra guard inside Claude Code's OS sandbox, which the app switches on for every
  launch (`CLAUDE_WORKER_ALWAYS` in `app/core/src/adapter/profiles.rs`): writes only in the worktree and the ticket's temp
  folder, the signing key's folder and the app's data folder unreadable, no unsandboxed retry, loopback only (so no
  internet; the card says "No internet for this worker yet", `CLAUDE_NO_NETWORK_NOTE`), and the launch is refused if the
  sandbox cannot start. `--setting-sources ""` means none of your Claude Code settings or MCP servers load, and so a hook a
  worker writes in its worktree never runs (`CLAUDE_NOTE` in `app/core/src/desktop/seat_start.rs`). Dispatch is on because
  Claude Code spike S4 passed (`CLAUDE_WORKER_S4_PASSED`), and it happens only for a Claude Code build on the proven list
  with an unchanged flag shape (`CLAUDE_WORKER_PROVEN`); any other build is refused ("Vajra hasn't verified this Claude Code
  version") and Codex workers still run.
- Workers that Claude Code's own campaign starts (the Vajra skill's workflow) run with the Vajra guard, and the OS sandbox
  is the enforced boundary only if you turned it on (`/sandbox` in Claude Code, or `scripts/enable-sandbox.sh`). The
  first-run checks show whether it is on (`preflight.rs`).

Under the guard, a worker is blocked from dangerous or self-modifying commands, writes to the
skill's files and settings, and the ways it knows of to read the signing key. The list is in
[SECURITY.md](../../SECURITY.md) ("The guard"). The guard is defense in depth: a pattern check over free-form shell cannot
be complete.

**Codex seats you drive (workers and verifiers).** Each launch gets a fresh Codex permission profile that replaces your
own sandbox: writes only in the worktree and the ticket's temp folder, and the Vajra state folder and the app's data
folder denied as defense in depth (`CODEX_WORKER_ALWAYS` in `app/core/src/adapter/profiles.rs`). **That is not a claim
that the key is out of reach.** The seat's network follows its switch; web search follows your own Codex setting; and your
MCP servers, hooks, plugins and `notify` hook stay on and run outside that profile, with the worker (`CODEX_WARNING` in
`app/core/src/desktop/seat_start.rs`). The Vajra guard does not run on Codex seats.

**The Codex run lead (the one Codex seat the core runs itself).** This one is isolated: a fresh per-launch profile that
denies the Vajra folders, no write entry, network off, `notify` and web search off, your user config ignored on a login
seat, and `vajra_run` as the only MCP server (`CODEX_LEAD_ALWAYS` in `app/core/src/adapter/profiles.rs`). A run whose lead
is Codex is still not counted as guarded (`CODEX_LEAD_CAUSE`). The Claude run lead stays off until its launch has been
checked against a real Claude Code (`CLAUDE_LEAD_SPIKED` is `false`).

## What is not guarded, and why

- **Codex seats you drive run with your own settings and are not guarded against key reads.** The seat card says: "Runs
  outside the Vajra guard, with your own Codex settings. Your Codex tools (MCP servers, hooks, plugins, notify) run with it,
  outside its sandbox. Its sandbox's internet follows this seat's switch. Web search and your Codex tools (MCP, hooks,
  notify) aren't limited by it. It isn't guarded against reading Vajra's signing key, and with that key it could fake
  Vajra's records and approvals, not only test results." (`app/core/src/desktop/seat_start.rs`, `CODEX_WARNING`). That is the
  owner's decision (13 Nov 2026): a Codex seat keeps your own extras on, and this page makes no claim that the signing key
  is out of reach for it. The permission profile that denies the Vajra folders is defense in depth, not a guarantee.
- **Codex on a ChatGPT login is also a case the app cannot check.** The app cannot see the settings your workspace may
  manage in the cloud, and `codex login status` does not say which plan you are on, so a workspace login cannot be told
  from a personal one. Nothing turns on it today: such a seat launches and carries the same not-guarded card as every
  other Codex seat (owner's decision, 13 Nov 2026), rather than the app refusing every ChatGPT login.
- **The signing key is guarded, not yet sandboxed.** The proof record says so: "Checked by Vajra on this Mac. The signing
  key is guarded, not yet sandboxed." A record whose evidence ran outside the guard says that too, and the app does not
  call any record more than that (`app/core/src/desktop/proof.rs`, `record_note`). The skill's knowledge ledger and practice
  log are hash-chained with that key (`SECURITY.md`); the Desktop action ledger is append-only but not hash-chained.
- **The Claude Code sandbox is opt-in for campaigns Claude Code starts.** Without it, the guard is the only layer on those
  workers, and the guard can be talked around by a command it cannot see (`SECURITY.md`, "What is not guaranteed"). The
  app's own Claude workers do not have this gap: the app turns the sandbox on and refuses the launch if it cannot start.
- **Your own tools run with the agents.** On each seat card the app notes that your Claude Code MCP servers, or your Codex
  MCP servers and `notify` hook, run with it (`CLAUDE_NOTE`, `CODEX_WARNING`).
