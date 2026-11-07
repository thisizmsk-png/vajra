# Per-merge perf check

`.github/workflows/perf.yml` builds the unsigned app on macOS and runs `measure.py` (summed RSS of the app, its PTY children and the WebKit helpers that appeared after launch). `gate.py` fails the check when:

- the runner's identity (macOS build + WebKit version, `idle.json` `identity`) has no entry in `baseline.json` (fails closed), or
- the WebKit helper count is not 2 or 3, or the identity has no entry for that count (fails closed), or
- idle exceeds the 200 MiB ceiling (NFR-PERF-1) in **any** flavour, or
- idle exceeds that (identity, helper count) baseline `idle_mib` + max(5 MiB, 3%). An entry marked `"provisional": true` (rests on one run) only **warns** on this check until it is refined; non-provisional entries gate. The ceiling still fails either way, or
- the 10 minute growth slope (push to main only) exceeds 1 MiB/min.

## Why the measurement is built this way
Six CI runs (18 launches) showed two noise sources on the shared macOS runner:
- **Foreign WebKit helpers.** Other processes start WebKit helpers during a launch; a sample then counts them (helper count 4 or 7 instead of 2) and reads 144 to 260 MiB of WebKit instead of about 80. `measure.py` drops samples whose helper count is above the smallest seen in that launch (`clean_median`, unit-tested) and records the helper counts in `idle.json`.
- **Renderer variance.** On clean launches the app process is steady (89 to 94 MiB) but the WebContent process swings 74 to 90 MiB, so a launch total spans 166 to 183 MiB (mean 174.8, sd 4.9). One run is the median of 5 fresh launches (was 3), which cuts the spread of the reported number to about sd 3.

The baseline is the pooled clean mean (175.0) and the tolerance stays max(5 MiB, 3%) (5.25 at 175): about 2 sigma of the reported number, so a false failure is rare but not impossible. The 200 MiB ceiling is the real budget and is unchanged. The runner idles lower than a dev Mac (about 175 vs 190 MiB; 2 helpers vs 3), so compare runner numbers only with runner numbers. If it still flaps on unchanged code, the next lever is more launches, not a wider margin.

## Runner flavours
The shared macOS runner shows two flavours on the **same** identity (same macOS build, WebKit version and image), so identity alone cannot key a baseline (issue #25):
- **2 helpers** (`com.apple.WebKit.Networking` about 22 MiB and `WebContent` about 60 MiB): about 175 MiB idle. Seen in most runs.
- **3 helpers**: the same two plus `com.apple.WebKit.GPU` (about 32 MiB, started with the app; WebContent shrinks to about 53 MiB): about 201 MiB. Seen in 1 of 5 attempts with the per-helper detail on.

`measure.py` records each counted helper's name, RSS, age and arguments in `idle.json` (`runs[].last.helper_detail`), so the next surprise is explained by the artifact. Nothing in the UI triggers the GPU process (no canvas, WebGL, filter, `backdrop-filter`, `will-change`, animation or blend mode); it is WebKit's own compositing process, present when the VM exposes a GPU, and every real Mac has it. It therefore stays in the gated total. The runner image version is recorded (`runner.image_version`) but is not part of the key: it rotates weekly without changing WebKit.

`measure.py` assigns each launch to the smallest helper count it saw and reports the flavour with the most launches (larger count on a tie), with `median_of_medians_mib` taken over that flavour's launches only (`flavour_launches`; `all_launches_median_mib` is kept for reference). The reported number and flavour therefore always describe the same launches.

`baseline.json` is keyed `identities[<identity>].flavours[<helper count>]`. Entries marked `"provisional": true` rest on one run and are refined with the procedure below as more runs of that flavour land.

### Owner question: GPU-flavour ceiling
The 3-helper flavour reads about 201 MiB on the CI VM, over the 200 MiB ceiling, while the dev-Mac H0 total with 3 helpers was 189.9 MiB (the shared VM reads about 11 MiB heavier). **The ceiling fails for every flavour until the owner rules**; nothing here relaxes NFR-PERF-1. The owner can choose one of these (each is a small reviewed change to `gate.py`):
1. Keep the 200 MiB ceiling on CI for the 3-helper flavour (the check would fail on roughly 1 in 4 green runs).
2. Raise a CI-only ceiling for that flavour to about 212.
3. Make that flavour report-only and rely on the dev-Mac H0 soak.

Until the owner rules, a ceiling failure is a failure: there is no rerun exception. Any retry policy is one of the owner's options above.

## Post-load idle (reported, not gated)
NFR-PERF-1 is also about what the app holds after a long run, not only the first minute. The memory job builds a second
copy of the app with `VITE_PERF_FIXTURE=1` (the normal bundle has none of it; a CI step greps the shipped bundle). That
build replaces the core with a scripted one: a Crew view with one worker whose Stream tab receives 10,000 lines at 170/s
starting 20 s after launch. `measure.py postload` samples the total before the load, the peak during it, and the idle
total 2 and 10 minutes after the load ends, and adds `post_load` to `idle.json` (pre-load, peak, +2 min, +10 min, retained =
+10 min minus pre-load). `gate.py` prints them in the job summary and as warnings: it never fails on them. The scripted
stream drives the real renderer through a fake transport, so the core's own bus and spill file are not exercised.

To start gating it, record a baseline with the bump procedure below: set `post_load_mib` on the flavour from the median
`plus10_mib` of at least 3 green runs. Until then a run over the 200 MiB budget or over that baseline only warns.

## Updating the baseline
`baseline.json` is never written by CI. To add or move an entry: open a PR that sets `identities["<identity>"].flavours["<N>"] = {"idle_mib": X, "provisional": false, "measured": "..."}`, where `N` is the `helpers` count and `X` is the median of `median_of_medians_mib` over at least 3 green runs that reported that identity and count (list the run IDs in `measured`). With fewer runs, set `"provisional": true` and say so. Say why in the PR. The owner reviews it, because it moves the regression line.
