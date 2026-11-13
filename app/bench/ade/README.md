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

## Request anatomy ("lean first request")

```
python3 app/bench/ade/ade_bench.py anatomy app/bench/ade/anatomy_arms.json anatomy.json   # spends real tokens, about $0.45
python3 app/bench/ade/ade_bench.py report anatomy.json anatomy.md
```

One `hello` turn per arm in an empty directory, read from the CLI's own `usage` and `modelUsage`. The first request is fresh +
cache write + cache read. Tools and MCP servers come from the init event. Side tokens are anything billed to a model other than
the one that answered (a title or summary call would show there). `anatomy_arms.json` mirrors the Director-General's launch flags
(`build_lead_argv`) minus its six control tools, which need the app's MCP server; add an arm for any other harness.

One run each, claude 2.1.296, this machine (the plain arm includes the owner's own plugins, skills and four claude.ai MCP servers,
so it is not comparable to anyone else's bare CLI):

| arm | first request | tools | MCP servers | side tokens |
|---|---|---|---|---|
| plain CLI | 51,660 | 36 | 4 | 0 |
| Director-General flags | 3,126 | 3 | 0 | 0 |

The Director-General's per-turn header, same flags, one `hello` each (first request, tokens): no header 3,085; the old header
(with ask and run ids) 3,153, so +68; the fixed header 3,114, so +29. The ids are gone from the header (nothing read them back
from the model's context), which makes it the same bytes every turn.

Worker sessions run the user's own CLI settings (we add none), so their first request is the plain-CLI figure plus whatever the
engine hooks inject. The hooks add model-visible text only on demand (a relayed steer, or a warning), never per turn.
