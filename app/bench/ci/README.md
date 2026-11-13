# Per-merge perf check

`.github/workflows/perf.yml` builds the unsigned app on macOS and runs `measure.py` (summed RSS of the app, its PTY children and the WebKit helpers that appeared after launch). `gate.py` fails the check when:

- the runner's identity (macOS build + WebKit version, `idle.json` `identity`) has no entry in `baseline.json` (fails closed), or
- a counted helper has a type other than `Networking`, `WebContent` or `GPU`, or the identity has no entry for the flavour of any launch (fails closed), or
- idle exceeds the budget: **200 MiB** (NFR-PERF-1), or **240 MiB** only when the flavour includes `com.apple.WebKit.GPU` **and** the gate runs on CI (`GITHUB_ACTIONS` set). A real machine keeps 200 in every flavour, or
- idle exceeds that (identity, flavour) baseline `idle_mib` + max(5 MiB, 3%). An entry marked `"provisional": true` (rests on one run) only **warns** on this check until it is refined; non-provisional entries gate. The budget still fails either way, or
- the 10 minute growth slope (push to main only) exceeds 1 MiB/min.

## Why the measurement is built this way
Six CI runs (18 launches) showed two noise sources on the shared macOS runner:
- **Foreign WebKit helpers.** Other processes start WebKit helpers during a launch; a sample then counts them (helper count 4 or 7 instead of 2) and reads 144 to 260 MiB of WebKit instead of about 80. `measure.py` drops samples whose helper count is above the smallest seen in that launch (`launch_summary`, unit-tested) and records the helper counts in `idle.json`.
- **Renderer variance.** On clean launches the app process is steady (89 to 94 MiB) but the WebContent process swings 74 to 90 MiB, so a launch total spans 166 to 183 MiB (mean 174.8, sd 4.9). One run is the median of 5 fresh launches (was 3), which cuts the spread of the reported number to about sd 3. Since the owner's decision of 2026-11-12 a run makes 6 launches: launch 1 is warm-up and is not gated, and the gate judges the median of the other 5. Launch 1 read the highest in almost every CI run (191 to 203 MiB against about 185 for the settled launches). The warm-up launch is still checked for unknown helper types and for an unbaselined flavour, so leaving it out of the median cannot hide a failure. `idle.json` also records each launch's macOS `footprint` (what the app, its PTY children and its WebKit helpers dirty, without shared framework pages) and its median as `footprint_median_mib`; it is shown in the job summary beside the RSS median and is reported only, never gated. The baseline (181.6, so a gate of 187.0 with the 5.45 MiB tolerance) and the tolerance did not change.

The baseline is the pooled clean mean (175.0) and the tolerance stays max(5 MiB, 3%) (5.25 at 175): about 2 sigma of the reported number, so a false failure is rare but not impossible. The 200 MiB budget is the real limit on real machines; CI gets 240 only for the GPU flavour (see the owner ruling below). The runner idles lower than a dev Mac (about 175 vs 190 MiB; 2 helpers vs 3), so compare runner numbers only with runner numbers. If it still flaps on unchanged code, the next lever is more launches, not a wider margin.

## Runner flavours
A launch is classified by the **set of WebKit helper types** it ran, never by their count. The flavour key is the sorted set joined with `+`:
- `Networking+WebContent` (`com.apple.WebKit.Networking` about 22 MiB and `WebContent` about 60 MiB): about 175 MiB idle.
- `GPU+Networking+WebContent` (the same plus `com.apple.WebKit.GPU`, about 32 MiB; WebContent shrinks to about 53 MiB): about 201-205 MiB.

The shared macOS runner shows both flavours on the **same** identity (same macOS build, WebKit version and image), so identity alone cannot key a baseline (issue #25). Nothing in the UI triggers the GPU process (no canvas, WebGL, filter, `backdrop-filter`, `will-change`, animation or blend mode); it is WebKit's own compositing process, present when the VM exposes a GPU, and every real Mac has it. It therefore stays in the gated total. The runner image version is recorded (`runner.image_version`) but is not part of the key: it rotates weekly without changing WebKit.

**Respawn churn.** The GPU process sometimes respawns on the runner, so a launch can briefly show 5, 7 or 10 helpers (for example the normal three plus two more `GPU`, seen in main run 42344105719). A second helper of a type that is already running is churn, not a new flavour. `measure.py` drops any sample with more helpers than the launch's smallest count or with a duplicate type, and a launch with no clean sample is excluded from the median entirely. The gate prints a warning with the number of churned and excluded launches and still classifies by the type set, so a 3 + 2 duplicate GPU launch is the `GPU+Networking+WebContent` flavour. Only an **unknown helper type** fails closed, and it is collected over every sample before any filtering (the PID diff cannot show that an unknown helper belongs to a foreign app).

`measure.py` records each counted helper's name, RSS, age and arguments in `idle.json` (`runs[].last.helper_detail`) and each sample's `types` counts, so the next surprise is explained by the artifact. It assigns each launch to its own flavour and reports the flavour with the most usable launches (larger type set on a tie), with `median_of_medians_mib` taken over that flavour's launches only (`flavour_launches`; `all_launches_median_mib` is kept for reference). The reported number and flavour therefore always describe the same launches.

`baseline.json` is keyed `identities[<identity>].flavours[<flavour key>]`. Entries marked `"provisional": true` rest on one run and are refined with the procedure below as more runs of that flavour land.

### GPU-flavour ceiling (owner ruling)
The GPU flavour reads about 201-205 MiB on the CI VM, while the dev-Mac H0 total with the GPU process was 189.9 MiB (the shared VM reads about 11 MiB heavier). **Owner ruling: "Allow about 212 MiB on CI only when the GPU process is present. The limit on real machines stays 200."** `gate.py` applied exactly that until run 42466636738, where the GPU flavour read 212.6 MiB on unchanged code (the same job read 182.3 MiB, no GPU helper, on rerun). **Owner decision (raised from 212): the owner is fine with idle above 220 MiB, so the GPU-on-CI ceiling is now 240 MiB.** `gate.py`: `GPU_CI_BUDGET = 240.0` when the flavour includes `GPU` and `GITHUB_ACTIONS` is set, otherwise `IDLE_BUDGET = 200.0`. The no-GPU flavour keeps 200 on CI too, and real machines keep 200. The check against the (identity, flavour) baseline plus max(5 MiB, 3%) is unchanged, and a provisional baseline still only warns. NFR-PERF-1 itself is unchanged for real machines.

## Post-load idle (reported, not gated)
NFR-PERF-1 is also about what the app holds after a long run, not only the first minute. The separate `postload` job builds a second
copy of the app with `VITE_PERF_FIXTURE=1` (the normal bundle has none of it; a CI step greps the shipped bundle). That
build replaces the core with a scripted one: a Crew view with one worker whose Stream tab receives 10,000 lines at 170/s
starting 20 s after launch. `measure.py postload` samples the total before the load, the peak during it, and the idle
total 2 and 10 minutes after the load ends, and writes `postload.json` with `post_load` (pre-load, peak, +2 min, +10 min, retained =
+10 min minus pre-load). `gate.py` prints them in the job summary and as warnings: it never fails on them. The scripted
stream drives the real renderer through a fake transport, so the core's own bus and spill file are not exercised.

It runs on push to main, nightly, on a PR that changes `app/ui/**` or `app/bench/**`, and on a PR labelled `perf`; otherwise the job
skips with a one-line summary note. The per-PR idle gate (`memory` job) is unchanged.

To start gating it, record a baseline with the bump procedure below: set `post_load_mib` on the flavour from the median
`plus10_mib` of at least 3 green runs. Until then a run over the 200 MiB budget or over that baseline only warns.

## Updating the baseline
`baseline.json` is never written by CI. To add or move an entry: open a PR that sets `identities["<identity>"].flavours["<flavour key>"] = {"idle_mib": X, "provisional": false, "measured": "..."}`, where `N` is the flavour key (the helper type set, e.g. `GPU+Networking+WebContent`) and `X` is the median of `median_of_medians_mib` over at least 3 green runs that reported that identity and flavour (list the run IDs in `measured`). With fewer runs, set `"provisional": true` and say so. Say why in the PR. The owner reviews it, because it moves the regression line.
