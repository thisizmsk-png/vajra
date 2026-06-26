# Contributing to Vajra

Thanks for considering a contribution. Vajra is built so that most additions are small and self-contained.

## Run the tests first

```bash
bash tests/run-all.sh
```

Everything should be green before you open a pull request.

## How it is laid out

- Each **skill** is one self-contained folder under `bundled-skills/<name>/`.
- Each **agent** is one file under `bundled-agents/`.
- Core harness logic lives in `scripts/` and `config/`.

## Adding a skill or an agent

Copy an existing one as a template, keep it to a single concern, and add a test if it has logic worth checking. If you add or change any file, run `bash scripts/generate-manifest.sh` so the supply-chain integrity manifest stays in sync (otherwise `/vajra verify` will flag the mismatch).

## Style

- No em-dashes or en-dashes in docs. Use a comma, a period, or parentheses.
- Be honest in claims. If a number is not measured, do not state it.
- Match the surrounding code and prose.

## Reporting issues

Open an issue with what you ran, what you expected, and what actually happened. If you found a security bypass, read [SECURITY.md](SECURITY.md) first and report it privately as described there.
