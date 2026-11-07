# Per-merge perf check

`.github/workflows/perf.yml` builds the unsigned app on macOS and runs `measure.py` (summed RSS of the app, its PTY children and the WebKit helpers that appeared after launch). `gate.py` fails the check when:

- idle exceeds the 200 MiB ceiling (NFR-PERF-1), or
- the runner's identity (macOS build + WebKit version, `idle.json` `identity`) has no entry in `baseline.json` (fails closed), or
- the WebKit helper count differs from the one recorded for that identity (our regression, not runner noise), or
- idle exceeds that identity's `idle_mib` + max(5 MiB, 3%), or
- the 10 minute growth slope (push to main only) exceeds 1 MiB/min.

## Why the measurement is built this way
Six CI runs (18 launches) showed two noise sources on the shared macOS runner:
- **Foreign WebKit helpers.** Other processes start WebKit helpers during a launch; a sample then counts them (helper count 4 or 7 instead of 2) and reads 144 to 260 MiB of WebKit instead of about 80. `measure.py` drops samples whose helper count is above the smallest seen in that launch (`clean_median`, unit-tested) and records the helper counts in `idle.json`.
- **Renderer variance.** On clean launches the app process is steady (89 to 94 MiB) but the WebContent process swings 74 to 90 MiB, so a launch total spans 166 to 183 MiB (mean 174.8, sd 4.9). One run is the median of 5 fresh launches (was 3), which cuts the spread of the reported number to about sd 3.

The baseline is the pooled clean mean (175.0) and the tolerance stays max(5 MiB, 3%) (5.25 at 175): about 2 sigma of the reported number, so a false failure is rare but not impossible. The 200 MiB ceiling is the real budget and is unchanged. The runner idles lower than a dev Mac (about 175 vs 190 MiB; 2 helpers vs 3), so compare runner numbers only with runner numbers. If it still flaps on unchanged code, the next lever is more launches, not a wider margin.

## Runner flavours
GitHub's `macos-latest` image rotates, and different images ship different WebKit builds: some runs show 2 WebKit helpers (about 175 MiB idle), others 3 (about 192 MiB). One baseline for both made the gate flap on unchanged code (issue #25). `baseline.json` is therefore keyed by the runner identity (`<macOS build>|<WebKit version>`, read by `measure.py`). The runner image version is recorded in `idle.json` (`runner.image_version`) but is not part of the key: it rotates weekly without changing WebKit.

When a run lands on an identity that is not in `baseline.json`, the gate fails closed with that identity in the message. Add it with the procedure below; do not loosen the tolerance.

## Updating the baseline
`baseline.json` is never written by CI. To add or move an identity: open a PR that sets `identities["<identity>"] = {"helpers": N, "idle_mib": X, "measured": "..."}`, where `X` is the median of `median_of_medians_mib` over at least 3 green runs that reported that identity (list the run IDs in `measured`) and `N` is their `helpers`. Say why in the PR. The owner reviews it, because it moves the regression line.
