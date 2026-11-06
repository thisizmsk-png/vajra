# Per-merge perf check

`.github/workflows/perf.yml` builds the unsigned app on macOS and runs `measure.py` (summed RSS of the app, its PTY children and the WebKit helpers that appeared after launch). `gate.py` fails the check when:

- idle exceeds the 200 MiB ceiling (NFR-PERF-1), or
- idle exceeds `baseline.json` `idle_mib` + 5 MiB, or
- the 10 minute growth slope (push to main only) exceeds 1 MiB/min.

The macOS runner idles lower than a dev Mac (about 174 vs 190 MiB; it reports 2 WebKit helpers) and varies by about 6 MiB between runs (170.9 and 176.4 on the same code), so the baseline is the mean of the runs seen and the +5 MiB rule is applied to that. Compare runner numbers only with runner numbers. If the check flaps on unchanged code, raise the sample (more launches in `measure.py`) before widening the margin.

## Updating the baseline
`baseline.json` is never written by CI. To move it: open a PR that edits `idle_mib` to the `median_of_medians_mib` in the `idle.json` artifact of a green main run, and say why in the PR. The owner reviews it, because it moves the regression line.
