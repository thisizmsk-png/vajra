# Per-merge perf check

`.github/workflows/perf.yml` builds the unsigned app on macOS and runs `measure.py` (summed RSS of the app, its PTY children and the WebKit helpers that appeared after launch). `gate.py` fails the check when:

- idle exceeds the 200 MiB ceiling (NFR-PERF-1), or
- idle exceeds `baseline.json` `idle_mib` + max(5 MiB, 3%), or
- the 10 minute growth slope (push to main only) exceeds 1 MiB/min.

## Why the measurement is built this way
Six CI runs (18 launches) showed two noise sources on the shared macOS runner:
- **Foreign WebKit helpers.** Other processes start WebKit helpers during a launch; a sample then counts them (helper count 4 or 7 instead of 2) and reads 144 to 260 MiB of WebKit instead of about 80. `measure.py` drops samples whose helper count is above the smallest seen in that launch (`clean_median`, unit-tested) and records the helper counts in `idle.json`.
- **Renderer variance.** On clean launches the app process is steady (89 to 94 MiB) but the WebContent process swings 74 to 90 MiB, so a launch total spans 166 to 183 MiB (mean 174.8, sd 4.9). One run is the median of 5 fresh launches (was 3), which cuts the spread of the reported number to about sd 3.

The baseline is the pooled clean mean (175.0) and the tolerance stays max(5 MiB, 3%) (5.25 at 175): about 2 sigma of the reported number, so a false failure is rare but not impossible. The 200 MiB ceiling is the real budget and is unchanged. The runner idles lower than a dev Mac (about 175 vs 190 MiB; 2 helpers vs 3), so compare runner numbers only with runner numbers. If it still flaps on unchanged code, the next lever is more launches, not a wider margin.

## Updating the baseline
`baseline.json` is never written by CI. To move it: open a PR that edits `idle_mib` to the `median_of_medians_mib` in the `idle.json` artifact of a green main run, and say why in the PR. The owner reviews it, because it moves the regression line.
