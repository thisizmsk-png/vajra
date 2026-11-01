# gate-web run evidence (ac-7)

Date: 2026-11-01. Base: 7343d78 (steering.json edit merged). Tree had no source, test or manifest.json edits.

## Setup (not committed)

Playwright and axe-core were installed in a throwaway directory outside the repo (`npm i playwright axe-core`, then `npx playwright install chromium`). No change to package.json or any lockfile. The directory was passed to the suite as `GATE_WEB_DIR`.

## Command

```
env GATE_WEB_DIR=<throwaway dir> bash tests/run-all.sh
```

Exit code: 0 (the tool reported no failure and the runner printed ALL SUITES PASSED).

## gate-web lines (verbatim)

```
=== tests/gate-web/run-gate-web.sh ===
gate-web: 19 passed, 0 failed
```

No `SKIP` line appears in the gate-web section. A first run without `GATE_WEB_DIR` printed `SKIP: gate-web needs playwright, axe-core and a Chromium that launches`, which is the expected skip when the dependencies are not resolvable.

## Totals

```
suites run: 27
ALL SUITES PASSED
```

Every suite reported 0 failed (docs 342 passed, tasks 116, pre-tool-use 239, workflows all scenarios passed, hmac-contract 7 passed). No manifest drift failure from the steering.json edit.

## Caveat

The vitest section printed `SKIP: vitest not installed (run: npm install)`, so the TypeScript unit tests did not run here. That is outside ac-7 (gate-web) but means "full suite" excluded vitest.
