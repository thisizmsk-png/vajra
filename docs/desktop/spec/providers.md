# FG-09: provider selection and native login checks (contract)

Status: contract only (first push). The core (detection, login check, start/resume through the chosen adapter) follows in a
second push and must not change anything written here without a new schema major.

Source: `artifacts/vajra-18/spec-open-p0s.md` S9.1 to S9.4.

## Rules that do not bend

- The CLI owns the login. Vajra never reads, stores, forwards or refreshes a token, keychain item or config secret, and has
  no OAuth flow. Billing is always the CLI's own login (`billing.source` is the literal `"login"`).
- **One narrow exception (D3): a pasted API key.** Vajra may write an API key the user pastes into the macOS Keychain, once,
  and never reads it back. It's API keys only, never a CLI's OAuth token or login (H1). The harness reads the key at start
  through its own key command; the core can only tell whether one is saved. See "Stored API keys" below.
- `ENV_ALLOW` (`sessions::ENV_ALLOW`) is unchanged. `billing.key_in_app_env` is an environment variable NAME (for example
  `OPENAI_API_KEY`) that the app saw and did not pass on. It is never a value. A value-shaped string is refused.
- No silent switch. If the chosen provider cannot start, Vajra says so and stops. It never falls back to another provider,
  never pools or rotates accounts, never retries on another paid route.
- "cannot tell" never blocks a start. Only "not installed" does, and only that one start.
- Unknown values read cautiously: an unknown login state is `cannot_tell` (never `signed_in`), an unknown `tested` is
  `unknown`, an unknown stop kind is `other`.

## `provider_list` (`vajra.providers/1`)

Tauri command only (not an MCP tool). Answer, at most 64 KiB, at most 8 providers, ids unique:

```json
{
  "schema": "vajra.providers/1",
  "providers": [{
    "id": "claude",                    // adapter id: ^[a-z0-9_-]{1,32}$
    "name": "Claude Code",
    "installed": true,                 // true  <=> "path" is present <=> login.state != "not_installed"
    "path": "/abs/path/to/cli",        // absolute, from the resolved PATH; null when not installed
    "version": "2.1.0",                // null when unknown
    "tested": "tested",                // tested | untested | unknown  (untested = "Not tested with this version", still selectable)
    "tested_range": "2.0 to 2.1",
    "capabilities": {                  // mirrors adapter::Capabilities
      "pty_attach": true, "transcript": true, "inspect": true, "message": true, "redirect": true,
      "stop": "hook_block",            // hook_block | terminate
      "resume": true, "vajra_workflow": true
    },
    "limits": [{"control": "message", "reason": "Codex cannot take a message mid-run"}],
    "login": {
      "route": "Uses your Claude Code login",   // plain words, never a credential
      "state": "signed_in",                      // signed_in | signed_out | cannot_tell | not_installed
      "checked_by": "cli_status",                // cli_status | none
      "detail": null                             // one plain sentence when it helps, e.g. why we cannot tell
    },
    "billing": {"source": "login", "key_in_app_env": null}
  }]
}
```

**Stable codes (wording stays in the UI).** `limits[].code`, `login.route_code` and `login.detail_code` are stable lowercase
codes (`[a-z0-9_]{1,64}`); the UI maps them to its own approved strings and shows the core's `reason` / `route` / `detail`
only for a code it does not know. All three are optional so older records still parse. Codes in use: limits
`no_message_mid_run`, `no_redirect_mid_run`, `no_inspect`, `no_transcript`, `no_vajra_workflow`, `stop_ends_process`,
`no_resume_session`, `not_installed`, `not_supported`; route `uses_claude_login`, `uses_codex_login`; detail `check_unfinished`,
`check_not_understood`.

**Stop.** A provider whose `capabilities.stop` is `terminate` (Codex) ends the process and has no hook hand-off, so it
carries a `stop` limit (code `stop_ends_process`) and the Stop button can say so. A `hook_block` provider (Claude) has none.

Text fields (`name`, `route`, `reason`, `version`, `tested_range`, `detail`) are non-blank, at most 200 characters, with no
control or invisible characters. `limits` has at most 16 entries; `control` is a label (`message`, `redirect`, `inspect`,
`transcript`, `vajra_workflow`, `resume`, `start`). Every capability that is off has a matching `limits` entry so the UI
shows a disabled control with its reason instead of hiding it.

## `project_state.provider` (additive to `vajra.project_state/1`)

The project's chosen provider, so one poll answers "can this project start, and with what?":

```json
{"id": "codex", "name": "Codex CLI", "installed": true, "tested": "untested", "login_state": "cannot_tell",
 "capabilities": { ... as above ... }, "limits": [ ... as above ... ]}
```

Older readers ignore it (unknown fields are ignored). It is absent only when no provider is chosen yet.
`name` and `tested` are there so Crew and Run can say "Codex" and mark an untested version without a second `provider_list`
read.

**No idle work.** `login_state` here is copied from the LAST `provider_list` answer (the core keeps it in memory); reading
`project_state` never runs a CLI. The status commands run only when `provider_list` is called: Settings > Providers open,
Recheck, or before a Start. Each run has a 5 second timeout, and nothing polls. Before the first `provider_list` the login
state reads `cannot_tell`.

**A chosen provider this build has no adapter for** (the preference names one that was removed or is from a newer build) is
present, not absent (absent means "none chosen"): `installed = false`, `login_state = not_installed`, `tested = unknown`, all
capabilities off, and `limits` for `start` and `resume` with code `not_supported`. Fixture:
`project_state.provider.unsupported.v1.json`; the core builds it with `providers::detect::unsupported`.

## `provider_accounts` (`vajra.provider_accounts/1`)

Multi-provider, PR 1 (contract and read path only; no UI, no stored key). Tauri command `provider_accounts`, read-only.
Additive to `vajra.providers/1`, which is unchanged. Same rules as above: at most 64 KiB, unknown fields ignored, another
major reported and not read, ids unique. One record per way of running and paying for agents; today that is each CLI under
its own login (`claude`, `codex`). Stored-key and gateway accounts use the same record in later changes.

```json
{
  "schema": "vajra.provider_accounts/1",
  "accounts": [{
    "id": "claude", "provider": "claude", "harness": "claude",
    "credential": {"source": "cli_login"},
    "billing": {"kind": "subscription", "account_label": "max, s…@example.com", "override_warning": null},
    "label": "Claude Code", "state": "signed_in", "models": []
  }, {
    "id": "codex", "provider": "codex", "harness": "codex",
    "credential": {"source": "cli_login"},
    "billing": {"kind": "unknown", "account_label": null,
                "override_warning": {"name": "OPENAI_API_KEY", "found_in": "cli_config"}},
    "label": "Codex CLI", "state": "cannot_tell", "models": []
  }]
}
```

- **`credential.source`**: `cli_login` (the CLI owns its login), `api_key` (a pasted key in the Keychain) or `gateway` (a
  non-secret base URL plus a Keychain key). `credential.saved` exists for the last two and is the only trace of a key: no key,
  prefix or last four characters. A `cli_login` record has no `saved`; the two others must have it.
- **`billing.kind`**: `subscription`, `api_key`, `gateway` or `unknown` (also the reading of a value this version does not
  know). `account_label` is only what the CLI itself prints, and only for a signed-in CLI: Claude's plan and a masked email
  (`s…@example.com`); Codex a fixed word (`ChatGPT`), because its status line can carry a masked key and is never kept.
- **`billing.override_warning`** is set when the CLI's own config would bill an API key instead of the login. It holds the NAME
  of the setting and where it was found (`cli_env_block`, `cli_config`, `cli_api_key_helper`), never a value. The detector
  reads only the CLIs' own config: Claude's `settings.json` and `settings.local.json` under `~/.claude` (a non-empty
  `apiKeyHelper`, or an `env` block naming `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` with a non-empty value) and Codex's
  `config.toml` under `~/.codex`. The Codex file is parsed as TOML: a key named `OPENAI_API_KEY` or `CODEX_API_KEY`, in any
  table, whose string value is non-blank counts. As for Claude, an empty value is not a key, and the value is only tested for
  being blank, never kept. Comments and the text of multiline strings are not settings, and a file that does not parse as
  TOML finds nothing. It never opens `auth.json`, a keychain item or the app's own environment (`ENV_ALLOW` already scrubs
  that, and `providers[].billing.key_in_app_env` reports it). A plain file only, at most 256 KiB, and no link followed: the
  CLI directory and the file are each opened relative to the one before, and neither may be a link. A file that is missing
  or not understood finds nothing.
- **`state`**: `signed_in`, `signed_out`, `not_installed`, `key_saved`, `key_missing`, `cannot_tell`. An unknown value reads as
  `cannot_tell`, never `signed_in`. `cannot_tell` never blocks a start.
- **`models`** (`id`, `family`): the models the user enabled on that account. Opaque ids passed straight through; `family`
  picks the icon. Empty for a CLI login.
- **One account per provider (D6).** There is no rotation field and none will be added (H7). No field is named `key`,
  `token` or `secret`.

### Stored API keys (D3)

Multi-provider, PR 2. `app/core/src/providers/keychain.rs`. Tauri commands, each with `account` (and `key` for the two
writes):

| Command | Does | Answer |
|---|---|---|
| `provider_key_saved` | Checks whether an item exists (no password data is asked for) | `{account, saved}` |
| `provider_key_save` | Stores a key the first time; refused with `already_saved` if one is there | `{account, saved: true}` |
| `provider_key_replace` | Deletes the item, then stores the new key | `{account, saved: true}` |
| `provider_key_remove` | Deletes the item; nothing there is not an error | `{account, saved: false}` |

- **The item.** A generic password, service `vajra.provider.<account>`, account `<account>`, access list
  `-T /usr/bin/security` only (never "any app"), so the harness's key command can read it without a prompt.
- **The write.** `/usr/bin/security -i` with a cleared environment; the `add-generic-password` line, key included, goes to its
  **stdin**, never argv. Success is decided by a check afterwards, not by the `-i` exit code.
- **No read-back.** There is no command, method or trait function that returns a stored key. Every answer is the
  `KeyState` `{account, saved}` and nothing else; errors are fixed codes with fixed words that never echo input or
  `security`'s output: `invalid_args`, `already_saved`, `keychain_unavailable` (not run, 30 s timeout, unexpected answer),
  `keychain_write_failed`.
- **Accounts.** Only the fixed list `KEY_ACCOUNTS` (today `openrouter`) takes a key; any other is `invalid_args`.
- **Key shape.** 16 to 256 characters of letters, digits and `. _ ~ + / = -`, not starting with `-`. Anything else
  (whitespace, quotes, backslash, `#`) is refused before any process runs.
- **In memory.** The core wraps the key at once (no `Clone`, no `Display`, a redacting `Debug`) and zeroes its copies after
  the write. Copies the IPC layer makes are out of its reach.

### Gateway launch (not yet wired into start_run)

The building blocks for a Codex-harness run on the `gateway` or `api_key` route (code id `vajra-openrouter`) exist, but `start_run` does not call them yet: no run uses them until a later PR wires them in with account and model selection. When wired, a run starts like this:

- **Own `CODEX_HOME`.** `<data dir>/codex-home/<account>` (0700), holding a generated `config.toml` (0600) that is rewritten
  at every launch. The user's `~/.codex` is never read or written. The file names the provider, a non-secret `https://`
  `base_url`, `wire_api = "responses"` and `[model_providers.vajra-openrouter.auth]` with
  `command = "/usr/bin/security"`, `args = [find-generic-password, -s, vajra.provider.<account>, -a, <account>, -w]`. There is
  no `env_key` and nothing secret in the file: Codex runs that command itself each time it needs the key.
- **argv.** `codex exec --json --sandbox workspace-write -m <model>`, then `-c` overrides: `model_reasoning_effort` (only when
  set), `request_max_retries=0` and `stream_max_retries=0` on the provider (the core owns the retry policy), and
  `sandbox_workspace_write.exclude_slash_tmp=true` / `exclude_tmpdir_env_var=true`. The ask is the last element, after `--`.
  The command line is wrapped in a fixed `/bin/sh -c 'exec "$@" </dev/null'` so stdin is `/dev/null`; the wrapper never
  parses the ask or the model.
- **Environment.** `ENV_ALLOW` plus `CODEX_HOME`, nothing else, and no key in the argv or the environment.

### `preferences.agents` (replaces the `agent` preference)

```json
"agents": {
  "default": {"account": "claude", "model": null, "effort": null},
  "roles": {"docs": {"account": "openrouter", "model": "deepseek/deepseek-v3.2", "effort": null}},
  "allowed": [{"account": "claude", "model": null}, {"account": "openrouter", "model": "deepseek/deepseek-v3.2"}]
}
```

- Set with `act_preferences_update` like any other preference (project override or global, `inherit: ["agents"]` to drop an
  override). `preferences_read` returns the resolved value as `agents` beside `effective`; `sources.agents` says where it came from.
- **Mapped on read, no data migration.** With no `agents` stored, an old `agent: "codex"` reads as
  `{default: {account: "codex"}, roles: {}, allowed: [{account: "codex"}]}`. A newer write of the old `agent` key drops a stored
  `agents` at that scope; a write of `agents` wins over `agent`.
- **A role with no entry uses `default`.** Roles are the crew role names (lowercase ids, at most 16).
- **`allowed` is a ceiling, enforced on write.** The default and every role must be inside it: an entry matches an account, and
  its `model` of `null` allows every model on it. A write that puts anything outside is refused (`outside_allowed`) and nothing
  is saved; an empty ceiling or a malformed id is `invalid_preferences`. The check is on the effective value, so a project
  override is held to its own ceiling.
- `model` is an opaque id (letters, digits and `-._/:`, at most 80); `effort` is a lowercase level word (at most 16) or `null`
  for the CLI's default. The core checks that an account exists, is installed and has its key when a worker starts, not here.

## `provider_usage` (`vajra.provider_usage/1`)

Multi-provider, PR 7 (usage meters, H5). Tauri command `provider_usage(page_open)`; the same answers ride in `project_state.usage`
as a cache read. One entry per account:

```json
{"schema": "vajra.provider_usage/1", "usage": [
  {"account": "claude", "read_at": null, "source": "none", "windows": [], "amounts": null},
  {"account": "openrouter", "read_at": "2026-11-09T12:00:00Z", "source": "gateway_key",
   "windows": [{"kind": "credit", "used_pct": 84, "resets_at": null, "warn": true}],
   "amounts": {"usage": 8.4, "usage_daily": 1.25, "limit": 10.0, "limit_remaining": 1.6}}]}
```

- **Nothing is estimated.** A window exists only if the account's own source reported it. `windows: []` with `source: "none"`
  is "Not reported". There is no price table, and Claude Code's `total_cost_usd` is ignored. Today the CLI logins report no
  windows, so they read "Not reported"; a CLI that reports windows later fills `kind` (`5h`, `daily`, `weekly`), `used_pct` and
  `resets_at` through the same shape.
- **Warning.** `warn` is true at 80% used or more. The status bar chip shows the most used warned window, with its reset time
  only if the source gave one.
- **OpenRouter** (`source: "gateway_key"`): `GET /api/v1/key`. Only `usage`, `usage_daily`, `limit` and `limit_remaining` are
  kept (`amounts`, in the key's own currency). The answer also carries the key's label; it is never deserialized, so it cannot
  reach any reply. `usage` is all-time spend, while `limit` can reset daily, weekly or monthly and `limit_remaining` is what is
  left of the current cap. So the `credit` window is `(limit - limit_remaining) / limit` and exists only when both are reported
  and `limit` is above zero; otherwise there is no percentage (the figures in `amounts` are still shown as reported).
- **When it calls out.** Only while Settings > Providers is open (`page_open`) or a run is active, and at most once a minute per
  account. Otherwise the last answer is returned and no call is made; `project_state` never calls out. When a read changes what is shown (the figures, not `read_at`), the project revisions advance and a `changed` event names them, so the next `project_state` read is a full answer carrying the new `usage`; a read that changes nothing moves nothing. A failed read keeps the
  previous answer (its `read_at` shows its age) and still counts as the minute's attempt.
- **The key never enters the core.** The call is curl with `--proto =https`, a cleared environment and the header read from
  stdin (`-H @-`). A key command (the Keychain read, from the Keychain store) is the producer of that stdin: it prints the key
  straight into curl's pipe, so the key is never in argv, the environment, a file, a log or this process's memory. There is no
  getter for a key. Until the Keychain store lands there is no key command, so no account is metered and no call is made.

## Launch receipt (requested vs effective)

Multi-provider, PR 4 (H3). `app/core/src/providers/effective.rs` (readers) and `control/receipt.rs` (`LaunchReceipt`). One entry
per worker slot, built from what the harness reports about itself once it starts:

```json
{"slot": "w3", "session_id": "s_…", "source": "role_default",
 "requested": {"account": "openrouter", "harness": "codex", "model": "vendor/model-a", "effort": "high"},
 "effective": {"account": "openrouter", "harness": "codex", "model": "vendor/model-a", "effort": "high",
               "sandbox": "workspace-write", "billing": "gateway", "read_from": "harness_session_file"}}
```

- **Codex.** The `--json` stream names a `thread_id` (`thread.started`) but not the model. The model, effort and sandbox come
  from `<CODEX_HOME>/sessions/**/rollout-*-<thread_id>.jsonl` (`session_meta.model_provider`, `turn_context.model`, `.effort`,
  `.sandbox_policy.type`); `read_from` is `harness_session_file`. That is what Codex sent, not proof of what a gateway served.
  A `model_provider` id Vajra did not write gives an `unknown` account.
  Only the id Vajra wrote for its own gateway account names a billing route (`gateway`); `openai` is used with a plan and
  with an API key alike, so its billing is `unknown`.
- **Claude.** The `system`/`init` event of its stream (`read_from: harness_event`). It has no effort setting here, so the
  effective `effort` is `null`, and billing is `unknown`: the stream does not report how the CLI was signed in.
- **Unknown, never copied.** A value the harness does not report is `"unknown"` ("Not reported"); with no session file or
  event every field is `unknown` and `read_from` is `null`. Reported strings are kept only if short printable ASCII.
  Session files are read from the Vajra-owned home only: the walk starts from the held home and each step
  (`sessions`, year, month, day, the file) is a no-follow open relative to the one before, so a link at any of them,
  including the home, is refused; the file must be a regular file; at most 1 MiB is read.
- **Mismatch.** The core never falls back. If `model`, `effort` (when one was asked for) or `account` is reported and differs
  from the request, `launch_mismatch` goes to the Director-General: `{"type": "launch_mismatch", "slot", "session_id",
  "fields": ["model"], "requested", "effective"}`. An `unknown` value is not a mismatch.
- **Diff.** A worker's diff is read from git in its worktree (`git diff --numstat -z <base>` plus `git ls-files --others -z`, through the
  read-only git bridge), never from events: a shell-only edit leaves no edit event. `-z` gives real, unquoted paths; a
  rename or copy is two records and the new path is kept. A new (untracked) file's lines count as added: git counts them
  (`diff --no-index` against `/dev/null`, not following a link); a binary or empty file adds none. Counting is one git call per new file, so it stops
  after 500 files or 2 s, whichever is first; every file is still listed and `partial: true` says `added`/`removed` are a lower bound. `partial` is also set when a
  count could not be had (git failed, timed out, was cut short, or answered in a shape that is not a numstat record).
  It runs once when a run's receipt is built, never on the `project_state` poll path.
- **Provider tag.** Crew-tree nodes and the `workers` of `project_state` carry a `provider` object, or `null` while the
  model is not reported. It is the shape `parseWorkerProvider` (`app/ui/src/workerProvider.ts`) requires;
  `app/core/tests/fixtures/launch/worker-provider.json` pins it, and the core and UI tests both check that file. Nothing in
  it is a credential. Fields:
  - `account`: the account that ran the worker (`claude`, `codex`, `openrouter`), or `unknown`.
  - `label`: the provider's name for display ("Claude Code", "Codex", "OpenRouter"); an account Vajra has no name for shows
    as its own id.
  - `family`: the model family that picks the icon. The vendor part of a `vendor/model` id, `claude` for a `claude*` id,
    otherwise the account.
  - `model`: the model id the harness reported, or `"unknown"` when it did not.
  - `model_name`: a display name for the model. None is known yet, so it is `null`.
  - `billing`: `subscription`, `api_key`, `gateway` or `unknown`. Claude workers read `unknown`, since a transcript does not
    say how the CLI was signed in.
  - `requested`: `{model, model_name, label}` of what was asked for, or `null` where that is not known. A card shows "Ran on X
    (asked for Y)" only when it differs.
  - `error`: the worker's current provider error, `{kind, action, attempt, retry_at}`, or `null` when there is none (always
    `null` until the provider-errors work attaches one).
  Claude workers take the model from their transcript's first assistant record.
- **Not yet wired.** These readers and the receipt exist; nothing yet builds a receipt at worker start, since core-launched
  non-Claude workers arrive with the later launch PRs.

## Versioning

A `provider_list` whose schema is another major is reported as "update needed" and not read further. Adding fields is not a
new major; changing or removing one is.

## Gap matrix (Given / When / Then)

| # | Given | When | Then |
|---|-------|------|------|
| G1 | Claude installed and its status command says signed in | `provider_list` | `installed`, `path`, `version`, `login.state = signed_in`, `checked_by = cli_status` |
| G2 | A CLI is not on the resolved PATH | `provider_list` | `installed = false`, no `path`, `login.state = not_installed`; Start for it is off with the reason "not installed"; Resume off with a reason |
| G3 | A CLI is installed and its status command says signed out | `provider_list` | `login.state = signed_out`; the UI says to sign in with the CLI itself; Start is still allowed (the CLI will say so) |
| G4 | A CLI has no read-only status command (Codex today), or it times out or answers something we do not understand | `provider_list` | `login.state = cannot_tell`, `checked_by = none`, a `detail` sentence; Start is NOT blocked |
| G5 | The installed version is outside the tested range | `provider_list` | `tested = untested`; the UI shows "Not tested with this version"; the provider is still selectable |
| G6 | Codex is chosen | the project is shown | controls for message, redirect, inspect, transcript and Vajra workflow are disabled with their `limits` reasons, not hidden |
| G7 | A future core writes `login.state = "expired"` | an older UI reads it | it reads as `cannot_tell`, never `signed_in` |
| G8 | A future core writes `schema = vajra.providers/2` | this UI reads it | "update needed"; nothing is guessed |
| G9 | A provider id appears twice, or `installed` and `path` disagree, or `not_installed` is paired with `installed = true` | the answer is parsed | refused as invalid |
| G10 | The app environment holds `OPENAI_API_KEY` | `provider_list` | `billing.key_in_app_env = "OPENAI_API_KEY"` (a name), the value is not in the answer and is not passed to the CLI |
| G11 | A `key_in_app_env` that looks like a value (`sk-abc123`, lowercase, empty) | the answer is parsed | refused as invalid |
| G12 | The chosen provider is not installed | Start | refused with the reason; no other provider is tried |
| G13 | The answer is over 64 KiB | parsed | refused unread |

G1 to G6, G10, G12 are exercised by the core push. This push covers G7 to G11 and G13 with the shared fixtures
(`providers.v1*.json`, `providers.v2.json`, `providers.invalid.v1.json`, `project_state.provider.v1.json`), parsed by
`app/core/src/providers/contract_tests.rs` and `app/ui/tests/providerContract.test.ts`.

## How the core fills it (verified against the installed CLIs)

- Install: the CLI named `claude` / `codex` on the user's resolved PATH (`userpath`). `--version` gives `version`; the first
  `x.y` token is compared with the adapter's tested range (Claude `2.x`, Codex `0.166`). Outside it is `untested`, a version
  that could not be read is `unknown`. Both stay selectable.
- Login, read-only, 5 second timeout, scrubbed environment (`ENV_ALLOW` unchanged), output dropped after reading one value:
  - Claude: `claude auth status --json`. `loggedIn: true` with exit 0 is `signed_in`, `loggedIn: false` is `signed_out`.
  - Codex: `codex login status`. Exit 0 is `signed_in`; exit 1 with "Not logged in" is `signed_out`.
  - Anything else (a CLI too old to have the command, a timeout, a cut or unparseable answer) is `cannot_tell` with a one
    sentence `detail`, and does not block a start.
- The status output can name an account; it is read for the one value above and never stored, logged or sent.
- Choice: the effective `agent` preference (`claude` | `codex`; the key is `agent`, written with `act_preferences_update` and the
  FG-11 envelope, per vajra-20's settings contract) is read by the UI
  with `preferences_read({projectId})` when a launch form opens and passed as `provider` to `start_run` / `act_start_run`
  / `resume_run`. The core adds no settings store. `resume_targets` returns the `provider` that ran each campaign, and
  Resume passes it back, so a Codex run is never resumed on Claude.
- Refusals: unknown id is `invalid_args` (field `provider`); a CLI that is not installed is `provider_not_installed`. Neither
  tries another provider.
- Resume: Codex has `resume = false` with limit `no_resume_session`. Vajra starts it as `codex -- <ask>` and never learns its session
  id (the app-made id means nothing to `codex resume`), so `resume_targets` omits Codex runs and `resume_run` refuses with
  `provider_cannot_resume`. Capturing Codex's real session id would lift this later.
- Recorded: the run index line carries `provider` (a hint, like the rest of that worker-writable file: only `claude` and
  `codex` are believed, absent reads as `claude`).

## Still to do

- `project_state.provider` (shape above) is not yet filled by the core; it needs the project's effective `agent`, which the
  core only gets from the caller until the preferences store is on this branch.
