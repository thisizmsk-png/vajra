# Preferences contract (FG-08)

Preferences are display and launch defaults, not permission policy. No setting changes a running agent or terminal,
weakens the guard/sandbox, enables automatic delegation, or grants remote access. Native provider authentication
remains outside this store. Settings have no token, environment-value, hook, command-string or approval field.

## Commands

- `preferences_read({ projectId?: string })`
- `act_preferences_update({ envelope: Envelope, patch: Patch, reset: boolean }) -> ActResult`

`Envelope` is the shared FG-11 action envelope: `action_id`, `project_id`, `expect_generation`. For this action,
`project_id: "global"` selects global defaults; otherwise it is the T1 project ID. `expect_generation` is required
and equals the preference document revision. Save and Reset both use this action; the old unreceipted update
is not exposed. Retries reuse the same action ID and identical payload. The typed payload digest binds the
receipt target, so reusing an ID for another intent is rejected. A revision conflict is a rejected receipt with
reason `revision_conflict`. A replay returns its receipt; read again to recover the current snapshot.
`ActResult` has `receipt`, optional `result` (the preference view), optional `error` and `replayed`. An unknown
outcome requires read/review, not a new action ID. The UI shows the receipt state and action ID.

`projectId` is omitted/null for global defaults; otherwise it must pass the existing project-ID contract. IDs are
map keys, never paths. The settings store does not invent a second project registry. The UI supplies a registered
project from the project track. Consumers must validate project existence/ownership through that track before launch.

Responses have `schema: "vajra.preferences/1"`, `rev`, `project_id`, `global`, `overrides`, `effective` and `sources`.
Each source is `global` or `project`. A project inherits every absent override; `patch.inherit` removes selected
project overrides. Reset clears only the selected scope. Global reset preserves other projects' overrides.

Fields: `theme` (system/dark/light), `editor` (system/code), `shell` (login/sh/bash/zsh), `notifications` (boolean, preserved default true), `notifications_text` (boolean, default false),
`agent` (claude/codex), `model` (bounded identifier or empty for native default), `workers` (1–16), `rounds` (1–3),
and `shortcuts` (palette/settings bindings). Provider values are integration targets, not comparative names.
Shortcut choices are deliberately bounded and conflicting/reserved bindings are refused.

An update requires the current whole-document revision. `revision_conflict` means reload and review the new values,
never silently retry over another window's edit. `preferences_busy` means another process holds the bounded
nonblocking file lock. `unsupported_version`, invalid records and I/O failures preserve the current file; there is
no automatic destructive reset. Unknown input fields are refused, including secret/token fields.

## Storage and lifecycle

`<app data>/preferences/settings.json`, versioned JSON, max 128 KiB, max 256 project override entries. Reads are capped,
O_NOFOLLOW and nonblocking and require a regular file owned by the user. Directory operations are relative to held
O_DIRECTORY/O_NOFOLLOW descriptors. Files are atomically replaced from a unique O_EXCL temp file at mode 0600,
inside a mode-0700 directory. Shared reads and exclusive writes use a nonblocking advisory lock on that directory;
the main process additionally serializes commands. A lost reply after rename can mean the write committed: reload
instead of repeating with a fresh revision without review.

Construction stores only a path. No background task, watcher, timer, refresh loop, network request or subprocess.

## Consumer integration

A new launch captures the returned effective settings and revision as an immutable input. Existing sessions keep
their launch settings. Permission enforcement and adapter capability checks remain independent, and system-wide
run/worker caps are not raised by these defaults. An unsupported provider/model stays unavailable.

Theme and keyboard consumers may apply saved preferences immediately. Editor/shell/agent adapters must consume the
snapshot only at an explicit new open/launch; saving preferences alone never executes a program, starts a run,
installs a CLI, changes a provider account, or upgrades an existing session. Browser notifications still require
separate OS permission. `notifications` preserves its existing true default; it is a delivery preference, never an OS grant. Delivery requires both the
effective toggle and granted OS permission. `notifications_text: false` limits delivery to project name and
item kind; no prompt/code/item text. The separate text opt-in never bypasses existing redaction.

## Read-only capabilities and follow-ups

Responses include `caps: { active_runs: 1, workers_per_run: 16, editable: false }`, reflecting current guard
limits. Preferences cannot change these caps. `workers` is a per-run request within that ceiling.
Appearance density, terminal font size, base-ref defaults and migration of UI layout localStorage are follow-ups.
Keyboard remapping retains the already agreed bounded palette/settings choices; other bindings stay read-only.
The agreed v1 uses `vajra.preferences/1` and one atomic document, with explicit global/project source labels.
Unlike the exploratory S8 sketch, unknown fields and corrupt/future files are preserved and refused, never
silently discarded or interpreted as trusted defaults. A separate settings schema/file tree is not introduced.
The contract fixture is `app/core/tests/fixtures/preferences-v1.json`.
