# Contributing to Vajra

Thanks for considering a contribution. Most additions are small and self-contained.

## Develop in a separate clone

Don't develop inside `~/.claude/skills/vajra`. Once Vajra's hooks are
installed, the pre-tool guard protects that directory, so Claude Code can't
edit files or run the test suites there. Work in your own clone instead:

```bash
git clone https://github.com/thisizmsk-png/vajra.git ~/Development/vajra
cd ~/Development/vajra
```

Tests use a temp `HOME`, so they never need the installed copy. To update the
installed copy after your change is merged, run this in your own terminal (not
through Claude Code):

```bash
git -C ~/.claude/skills/vajra pull
```

## Run the tests first

```bash
npm install            # vitest, for the TypeScript tests
bash tests/run-all.sh  # vitest plus every tests/**/run-*.sh suite
bash tests/run-all.sh router   # only suites whose path contains "router"
```

CI (`.github/workflows/ci.yml`) runs the same command on Ubuntu and macOS.
Everything must be green before you open a pull request.

## Layout

- `SKILL.md` is the entry point. Each `/vajra` subcommand has one doc in
  `docs/commands/`.
- `scripts/` and `hooks/` hold the code that enforces behavior; `config/` holds
  the data it reads. `workflows/` holds the saved Claude Code workflows.
- Each skill is one folder under `bundled-skills/<name>/`, and each agent is one
  file under `bundled-agents/`.

## Adding things

- **A test suite:** name it `tests/<area>/run-<name>.sh`, make it exit non-zero
  on failure, and use a temp `HOME` (`HOME=$(mktemp -d)`). Never touch the real
  `~/.claude`. `tests/run-all.sh` picks it up automatically.
- **A route:** add a pattern to `routing.patterns` in `config/default.json`,
  above any broader pattern it overlaps with, and add a case to
  `tests/router/run-router.sh`.
- **A subcommand:** add `docs/commands/<name>.md`, a row in `SKILL.md`, and the
  name in `SUBCOMMANDS` in `scripts/vajra_route.py`. If the word also reads
  naturally at the start of an ask ("status page", "plan and build"), add it to
  `STANDALONE` or `CONJUNCTION_SENSITIVE` there and add router tests for both
  readings.
- **A skill or agent:** copy an existing one, keep it to one concern, and add a
  test if it has logic.

After changing tracked files, run `bash scripts/generate-manifest.sh` in your
own terminal so the integrity manifest stays in sync (like `install.sh`, it is
for you to run, not the model). `vajra-doctor.sh` flags a mismatch.

## Style

- Bash must run on macOS bash 3.2 (no associative arrays, no `mapfile`, no
  `globstar`). Python uses the standard library and runs with `python3 -I`.
- In docs, no em dashes or en dashes; use a comma, a period or parentheses.
- Be honest in claims. A README claim needs code and a test behind it.

## Reporting issues

Open an issue with what you ran, what you expected and what happened. For a
security bypass, read [SECURITY.md](SECURITY.md) first and report it privately.
