# FG-09: provider selection and native login checks (contract)

Status: contract only (first push). The core (detection, login check, start/resume through the chosen adapter) follows in a
second push and must not change anything written here without a new schema major.

Source: `artifacts/vajra-18/spec-open-p0s.md` S9.1 to S9.4.

## Rules that do not bend

- The CLI owns the login. Vajra never reads, stores, forwards or refreshes a token, keychain item or config secret, and has
  no OAuth flow. Billing is always the CLI's own login (`billing.source` is the literal `"login"`).
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

Text fields (`name`, `route`, `reason`, `version`, `tested_range`, `detail`) are non-blank, at most 200 characters, with no
control or invisible characters. `limits` has at most 16 entries; `control` is a label (`message`, `redirect`, `inspect`,
`transcript`, `vajra_workflow`, `resume`, `start`). Every capability that is off has a matching `limits` entry so the UI
shows a disabled control with its reason instead of hiding it.

## `project_state.provider` (additive to `vajra.project_state/1`)

The project's chosen provider, so one poll answers "can this project start, and with what?":

```json
{"id": "codex", "installed": true, "login_state": "cannot_tell",
 "capabilities": { ... as above ... }, "limits": [ ... as above ... ]}
```

Older readers ignore it (unknown fields are ignored). It is absent only when no provider is chosen yet.

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

## Open for the core push

- The exact read-only status command per CLI must be verified against the installed CLIs (not invented here). If a CLI has
  none, `checked_by = none` and the state is `cannot_tell`.
- The per-project choice is read from the preferences snapshot (key `agent`, values `claude` or `codex`); the spec text for
  S9.3 says `agent.provider`. Asking vajra-20 which spelling is final before the core push.
