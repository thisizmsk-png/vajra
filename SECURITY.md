# Vajra Security Model

The short version:

> The **OS sandbox is the enforced boundary.** Turn it on with `/sandbox` (see
> `config/sandbox.json` and `scripts/enable-sandbox.sh`). Vajra's PreToolUse
> guard is **defense in depth**: it blocks known dangerous commands and protects
> Vajra's own files, but a check over free-form shell can never be complete.

Everything below describes what the code in this repository does today. Each
claim has a test under `tests/`; run `bash tests/run-all.sh`.

## How the hooks are wired

`scripts/install.sh` merges `hooks/hooks.json` into `~/.claude/settings.json`
(backed up first, idempotent, keeps your own hooks). `--unregister-hooks`
removes only Vajra's entries; `--dry-run` shows the change without making it.
`scripts/vajra-doctor.sh` prints PASS/FAIL for every part of the wiring,
including a live smoke test of the guard.

| Event | Script | Behavior |
|---|---|---|
| PreToolUse (Bash, Write, Edit, MultiEdit, NotebookEdit, Read, Grep, Glob, LS) | `hooks/pre-tool-use.sh` | The guard. Exit 2 blocks. Registered with `onFailure: "block"`. |
| PostToolUse, PostToolUseFailure (async) | `hooks/post-tool-use.sh` | Atman practice log (hashes only, no raw text). Never blocks. |
| SessionStart | `hooks/session-start.sh` | Integrity check, active campaign line, post-compaction pointer. |
| PreCompact | `hooks/pre-compact.sh` | Pins a git snapshot at `refs/vajra/compact/<ts>-<session>`. |
| UserPromptSubmit | `hooks/user-prompt-submit.sh` | 0-token router (`scripts/vajra_route.py`). Always exits 0. |

## The guard (`hooks/pre-tool-use.sh`)

**Contract.** Claude Code sends JSON on stdin (`tool_name`, `tool_input`,
`cwd`). Exit 2 blocks the call and shows the reason to Claude; exit 0 means no
decision, so normal permissions still apply. Any other exit code would be a
non-blocking error, so the script never produces one: an EXIT trap converts
every unexpected exit (bad JSON, missing `jq`, a crash) into exit 2, and
`onFailure: "block"` covers timeouts and launch failures. It fails closed.

**What it reads.** Only `tool_input.command` for Bash and the path fields
(`file_path`, `path`, `notebook_path`) for file tools. It never scans file
contents or the command description, so writing a doc that mentions
`rm -rf /` is fine.

**Canonical paths.** Every path-like token is resolved against the session
`cwd` (or a preceding `cd`), `..` is collapsed, symlinks are resolved, globs are
expanded, and comparison is case-insensitive. `../.claude/skills/vajra/x`,
`~/.CLAUDE/Skills/VAJRA/x`, `$HOME//.claude/./skills` and a symlink pointing into
the core all resolve to the same protected path.

**Protected paths.** Writes, deletes, moves and any non-reading command are
blocked for:

- the skill core `~/.claude/skills/vajra/**` (hooks, scripts, config, workflows, everything)
- every Claude settings file: `~/.claude/settings*.json` and any project `.claude/settings*.json`
- `~/.claude/agents/**`, `~/.claude/hooks/**`, `~/.claude/workflows/vajra-*`
- Vajra state in `~/.claude/vajra/`: `.hmac-key`, `manifest.sig`, `.integrity-violation`,
  the logs (`*.jsonl`, `*.log`, `events/`), Atman state (`proposals/`, `patches/`,
  `learned-routes.json`, `atman-history.jsonl`, `practice-archive/`,
  `practice-log.anchor.json`, `practice-log.lock`, `atman.paused`, `atman/`), and
  campaign `checkpoints/`
- the directories above them (`~/.claude`, `~/.claude/skills`, `~/.claude/vajra`,
  campaign dirs), so `rsync -a x/ ~/.claude/` or `tar -C ~/.claude` cannot land
  inside the core without naming it

Reading commands (`cat`, `grep`, `ls`, `jq`, `git status/log/diff`, ...) may look
at protected files; output redirects into them are still blocked.

**The HMAC key** (`~/.claude/vajra/.hmac-key`). The guard blocks these ways of
reading it:

- Read, Grep, Glob or LS with the key as the path.
- Grep whose path, or whose session cwd when no path is given, is the key's
  directory or any ancestor of it (`~/.claude/vajra`, `~/.claude`, HOME, `/`).
  Grep in a project directory is unaffected.
- Any Bash command whose text contains `.hmac-key`, or whose glob expands to it
  (`cat ~/.claude/vajra/.hmac-k?y`, `cat .*` inside the state dir).
- Recursive readers (`grep -r/-R`, `rg`, `ag`, `ack`, `ugrep`, `git grep`,
  `diff -r`), `find`/`fd` with `-exec`/`-x`, archivers (`tar` create, `zip`,
  `7z`, `cpio`, `pax`) and copiers (`cp`, `rsync`, `scp`, `ditto`) when a target
  or source is the key's directory or an ancestor of it. Recursive searches,
  and `find -exec` without a path, are also blocked when the cwd is one of
  those directories.
- File names listed from the key's directory, or from a command naming one of
  those directories, piped into `xargs` or `parallel`.

A command that builds the name at run time (`f=.hmac; cat ~/.claude/vajra/$f-key`)
is not visible to the guard. The OS-level guarantee is the sandbox `denyRead`
entry for the key in `config/sandbox.json`; see "What is not guaranteed".

**Vajra's own scripts.** Running `scripts/*.sh`, `scripts/*.py` and
`scripts/*.js` (with `bash`, `python3` or `node`) is allowed, with these
exceptions that only the operator may run, in their own terminal (approve and
reject need a TTY): `install.sh`, `install-skills.sh`, `generate-manifest.sh`,
and `atman.sh approve|reject`. Setting `VAJRA_ATMAN_ASSUME_HUMAN` from a tool call is
blocked. Running hook scripts directly is blocked.

**Dangerous commands.** Recursive deletes of `/`, system roots, HOME or the
working directory (`rm -rf ./node_modules` is fine); force push; `git reset --hard`;
`git clean -f`; world-writable or recursive-root `chmod`/`chown`; raw disk
writes, `mkfs`, `diskutil erase*`; fork bombs; piping into a shell or an
interpreter that reads code from stdin (`curl | sh`, `| bash -s`, `| python3`;
`| python3 -m json.tool` is fine); decode-and-execute and download-and-execute;
interpreter one-liners that decode or download and then execute; network tools
combined with credential paths; env dumps piped to the network; uploads of files
outside the working directory with `curl -d @file`/`-F`/`-T`; DNS lookups of
computed data; `/etc/shadow`.

There is no global "plan phase" gate any more; plan mode is Claude Code's native
plan mode.

**Latency.** About 20 ms per call on an Apple-silicon Mac (one `jq` process,
everything else is bash builtins). The test suite measures and asserts it.

## Integrity (`hooks/session-start.sh`)

- `manifest.json` (shipped) lists the SHA-256 of every shipped file.
- The signature is made **on your machine**: `install.sh` verifies every hash,
  then writes `HMAC-SHA256(your key, sha256(manifest.json))` to
  `~/.claude/vajra/manifest.sig`. No signature is shipped in the repository, so
  no one else's key is involved. If any file differs at install time, install
  refuses to sign and lists the files.
- Each session start re-checks the signature, every file hash, and looks for
  files that are not in the manifest under `hooks/`, `scripts/`, `config/`,
  `workflows/` and `atman/`. On a mismatch it writes
  `~/.claude/vajra/.integrity-violation`; while that flag exists the guard allows
  only reading commands and Vajra's own scripts. Warnings go to stdout, so the
  model sees them and can tell you.
- To accept your own changes, run `bash ~/.claude/skills/vajra/scripts/generate-manifest.sh`
  in your own terminal (it rebuilds and re-signs; `install.sh` refuses to sign a
  tree that differs from the manifest). To discard them, restore the files and
  re-run `install.sh`.

## Keys and signed state

`install.sh` creates `~/.claude/vajra/.hmac-key` with `openssl rand -hex 32`,
mode 600, and replaces an empty or malformed key. It is used by the manifest
signature, the Atman practice-log hash chain (`hooks/post-tool-use.sh`), the
pre-compact event chain, and campaign checkpoints (`scripts/vajra-campaign.sh`,
which reads the key itself so it never enters model context).

## What is not guaranteed

- **The guard is a heuristic over shell.** Commands assembled at run time
  (`$(printf ...)`, variables, `eval` of computed strings, a script written in one
  call and run in the next, `cd ~ && tar -xf x.tar`) can express things it cannot
  see. It closes the known concrete bypasses and is regression-tested, but the
  OS sandbox is what holds against a determined attacker.
- **Same-user trust boundary.** The HMAC key lives under your UID. The guard
  blocks the agent from reading it, and the sandbox `denyRead` entry in
  `config/sandbox.json` enforces that at the OS level; without the sandbox, the
  signatures give tamper evidence, not tamper proofing.
- **Trust on first install.** The local signature records the files as they were
  when you installed. Check what you cloned (`git log`, `git verify-tag`) before
  running `install.sh`.
- **Hard links** to protected files created outside Claude Code are not detected.

## Enabling the sandbox

```bash
bash ~/.claude/skills/vajra/scripts/enable-sandbox.sh   # prints platform-checked steps
```

Or interactively: `/sandbox`. The recommended policy is in `config/sandbox.json`.

## Reporting

Run `bash tests/run-all.sh` and `bash scripts/vajra-doctor.sh` before relying on
any layer. Report vulnerabilities privately through GitHub security advisories on
the repository.

## Knowledge ledger (`scripts/vajra_kb.py`)

Vajra's project knowledge lives in `~/.claude/vajra/kb/<project-id>/`
(details in `docs/commands/kb.md`).

- **Signed source of truth.** `events.jsonl` is append-only and HMAC-chained
  with the Vajra key, the same scheme as the practice log. Every append is
  fsynced. Edited, forged, reordered or replayed lines fail verification and
  are ignored; `doctor` reports them and the script refuses to append onto a
  broken chain. `doctor --repair` moves the unverified lines to
  `events.quarantine.jsonl` and leaves the verified chain untouched.
  `kb.sqlite` and `INDEX.md` are derived and rebuilt from the chain.
- **Write gate.** Every write is checked before it is stored. Secrets (cloud
  keys, private keys, GitHub, `sk-` and Slack tokens, JWTs, high-entropy
  strings), prompt-injection patterns, invisible or control characters, URLs
  with query strings (except non-credential queries on a local development
  host) and oversized content are refused with exit 3. A refusal is
  audit-logged as a sha256 of the input, never the content. The gate is a
  pattern filter: it lowers the odds of a secret or an instruction being
  stored, and does not prove that a record is harmless.
- **Trust tiers.**
  - `user` records need `--user-token`, HMAC-SHA256(key,
    `"user\0" + title + "\0" + body`). Only the router mints it, in the
    UserPromptSubmit hook, for `/vajra remember`.
  - `verified` records need a green, signed `vajra-verify-work.sh` evidence
    file and a session id.
  - Everything else goes to `pending/` with a session id and needs the user.
    `approve`, `reject` and `forget` require an interactive TTY, the same
    human gate as Atman (`atman.sh approve kb-<id>` delegates to it).
    Approval re-runs the gate, and ignores the pending file's `applies`
    because pending files are not signed.
  - Proposals and verified writes are refused for sessions that used web or
    MCP content.
  - Only user records, or a proposal the user approves with `--always`, may
    be `applies=always` (shown at every SessionStart).
  - `retire`, `supersede` and `restore` of a user record need a TTY or a user
    token over that operation and record id.
- **Read-back is data.** `prime`, `show`, `search --format full` and the
  router's hits are wrapped in `<untrusted-data source="vajra-kb">`. Record
  text cannot open or close that tag (any case or spacing is escaped), and
  stale facts are not injected. Verified records keep only the evidence
  sha256 and a sanitized slug, never a path or text from the evidence file.
- **Forgetting.** Nothing is deleted silently; `forget` (user only) replaces
  the record's content with a tombstone and re-signs the chain.

What the evidence and token checks guarantee, exactly:

- **Evidence.** A `verified` record (and a task closed by
  `vajra_tasks.py close`) means all of these were true:
  - something that could read the Vajra key wrote this exact
    `evidence.json`, with nothing changed since (the hmac);
  - the file sits at `<repo>/data/walkthroughs/<slug>/evidence.json`;
  - every recorded command exited 0, the tree was stable while they ran, and
    the recorded head is the current HEAD or the commit the caller named;
  - not every command was trivial;
  - the file is at most 48 hours old.

  It does not prove that the commands were meaningful tests: `test -f x`
  passes the trivial-command check. Nor does it prove that the agent did not
  run `vajra-verify-work.sh` itself. Any process that can run the script can
  produce signed evidence, so the hmac proves where the file came from, not
  whose intent it reflects.
- **User token.** A `user` record means a process that could read
  `~/.claude/vajra/.hmac-key` approved that exact title and body. In normal
  operation, that process is the router running `/vajra remember` in the
  UserPromptSubmit hook. Tokens never appear in the model's context. A token
  is bound to its text, so it cannot be reused for other text, but it can be
  replayed for the same text, which only dedupes.

Not guaranteed:

- **The TTY gate is not a boundary on its own.** `approve`, `reject`,
  `forget`, and the user-record `retire`/`supersede`/`restore` check
  `isatty()`. A process that can allocate a pseudo-terminal (`script`,
  `expect`, Python's `pty`) can satisfy that check. Without the OS sandbox
  (`/sandbox`, `config/sandbox.json`), the gate keeps honest agents from
  approving by accident. It does not stop a determined one.
- **The key is the root of both checks.** The guard keeps the agent's own
  tools from reading `.hmac-key`. An agent that can run arbitrary shell
  outside the guard and the sandbox can read the key, mint tokens and sign
  evidence. The guard and the sandbox are the boundary for that, as for
  every other script.
