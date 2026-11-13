# ADE bench rig

A local rig that measures Vajra Desktop the way the public ADE benchmark does, so we can see our own numbers on one machine
before anyone else does. Reported, not gated, except the typing-lag gate below.

```
python3 app/bench/ade/ade_bench.py window APP_BINARY window.json   # cold start (window ready)
python3 app/bench/ade/ade_bench.py weight APP_BINARY weight.json   # at rest (footprint beside RSS), idle CPU
python3 app/bench/ade/ade_bench.py tokens DATA_DIR  tokens.json    # token receipts of finished turns
python3 app/bench/ade/ade_bench.py report window.json weight.json tokens.json report.md
```

**Method.** The memory gate's method (`app/bench/ci/measure.py`, reused unchanged): 1 warm-up launch thrown away, then the
median of 5. Every result records the runner (macOS build, WebKit build). Compare numbers only on the same runner.

**Weight** is `footprint` over the app's process group, reported beside summed RSS. The memory gate's baseline is not touched.

**Not measured, and said so.** New thread, first word, long thread and per-thread weight need a driven UI or a live agent;
the report lists them as `not measured` with the reason. "Window ready" stands in for "typing accepted".

## Typing-lag gate

`app/ui/tests/typingLag.test.tsx` types 60 keystrokes into the Director-General composer while 5 fake agents stream 20 lines a
second each on the change feed, and requires p95 at or under 50 ms. jsdom has no paint, so it measures the part our code owns
(handlers plus the React commit). The perf workflow's `typing-lag` job runs it with `VAJRA_LAG_GATE=1`. **It is report-only for
its first week** (`continue-on-error`); the p50 and p95 print in the job log.

## Token receipt

Each Director-General turn that ran writes a `vajra.token_receipt/1` object (`fresh`, `cache_write`, `cache_read`, `output`,
`side`, `total`) on its `done` ledger line, or on its `failed` line when the provider reported an error (failed turns spend
tokens too, so a goal's sum counts them). `ade_bench.py tokens` counts every ledger line that carries a receipt. It comes from the provider's own `usage` fields. Nothing is estimated and no price is applied.
`side` is Vajra's own off-request spend (titles, branch names): always 0. `ade_bench.py tokens` reads these back; the signed run
record can pick the same object up.

## Typing lag in a real browser (the CI gate)

`app/ui/scripts/typing-lag.mjs` opens the built typing-lag page (`app/ui/lag.html`, the real Director-General composer and
transcript on a scripted core with 5 agents streaming 20 lines/s each) in the installed Chrome and types 120 keys.
Per key it records the time from the key's own timestamp to the first frame after the update; p95 must be at or under
50 ms. A second pass with nothing streaming is the control, and long animation frames over 50 ms are counted.

```
cd app/ui && npx vite build -c vite.lag.config.ts && node scripts/typing-lag.mjs --gate
```

Measured on an Apple M-series laptop, Chrome 157: streaming p50 9.6 ms, p95 16 to 22 ms; idle control p95 16 to 17 ms; no
long frames. The `typing-lag` job in `perf.yml` runs it; it is report-only (`continue-on-error`) until 2026-11-20, then blocks.

Caveat: Chrome's Event Timing API reports a much larger "presentation delay" for the same keys under streaming
(p95 about 440 ms in headless Chrome) while the main thread shows no long frame. That is compositor or GPU presentation in
automation, not page work, and it is rounded and hidden under 16 ms, so the gate does not use it. It is printed as
`event_timing_p95_ms` for anyone who wants to chase it.
