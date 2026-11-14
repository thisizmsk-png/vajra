# Resource profiles (FG-12 S12.1)

Baseline RSS and CPU of Vajra Desktop by component, taken on today's build before any new pane, so later caps and
holds (S12.2 and S12.3) have numbers to start from. Reported, not gated. No caps, holds or enforcement here, and no
process that stays running: a profile is one bounded command.

## What it measures
Per component, RSS (MiB) and CPU (% of one core): the app process, each WebKit helper by type, each run's agent
process tree, each terminal's process tree. The method is `ci/measure.py`'s (summed `ps` RSS; WebKit helpers found by
diffing the `com.apple.WebKit.*` pids before and after launch, then keeping only those macOS names this app as responsible for; new helpers that are not provably ours are counted as `unattributed`, never summed) plus CPU and a visibility guard (load average, frontmost
app and helper count per sample; a sample with extra helpers or load above 6 is left out of the median).

- `profile_run` (`app/core/examples/profile_run.rs`): the real core (App, supervisor, bus, crew tailing, the 1 s change
  loop of `main.rs`, user terminals) driven without a window, one scenario per launch, with a stand-in agent
  (`fake_agent.py`) in a real PTY. Scenarios: `idle`, `run1`, `run4` (4 workers at 20 lines/s each), `term_flood`
  (a terminal printing at full speed, every chunk acknowledged), `term_flood_unacked` (the same with no view reading),
  `terms8` (8 idle terminals, the ADR-15 per-project cap).
- `fg12_profile.py bin`: the shipping binary and its WebKit helpers, idle.
- `test_profile.py`: unit tests for the sampler (`python3 -m unittest app/bench/profile/test_profile.py`).
- `app/core/tests/profile_harness.rs`: builds the harness and starts one run with it for two seconds
  (`cargo test --test profile_harness`), so a change to what a run needs cannot break the load scenarios unnoticed.

## Run it
```
cd app/core && cargo build --release --example profile_run
export FG12_FIXTURE=$PWD/tests/fixtures/transcripts/session/subagents
python3 ../bench/profile/fg12_profile.py core target/release/examples/profile_run run4 out.json 60
# the shipping app (build it as perf.yml does: tauri build --bundles app)
python3 ../bench/profile/fg12_profile.py bin "target/release/bundle/macos/Vajra Desktop.app/Contents/MacOS/vajra-desktop" out.json 60
```
Raw samples for the numbers below are in `results/`.

## Safety
- **Bounded:** seconds clamp to 600, samples to 300; the harness ends itself after its own deadline.
- **Cancellable:** SIGINT/SIGTERM stop sampling and still write what was sampled (`cancelled: true`). The core harness
  handles SIGTERM itself and shuts its terminals and runs down; the sampler then SIGKILLs only descendants it had
  snapshotted (same pid, start time and command) that are still alive, because terminal shells are sessions of their
  own and a process-group kill misses them. The shipping app is quit by pid, never by name. Unit tests cover both.
- **Never in front of the human:** the shipping app is started with `open -g -j` (not activated, window hidden) and
  quit by pid when the run ends. Every launch uses a temp HOME and data dir; the real ones, the installer and the
  manifest generator are not touched.

## Results (2026-11-08, `ao/integration/batch-3` @ ca438c5 and a4efc78, release build, Apple M3 Max, macOS 26.3.1)
The machine was shared (load 4 to 12, other sessions building). Medians of 60 s of 2 s samples after a 25 s settle.

### The shipping app, idle (3 launches, hidden window, no run) against the CI memory gate
| launch | app | GPU | Networking | WebContent | total MiB | CPU % of a core | clean samples |
|---|---|---|---|---|---|---|---|
| 1 | 103.2 | 34.3 | 15.1 | 49.0 | **201.5** | 0.11 | 20 of 29 |
| 2 | 101.5 | 34.0 | 15.0 | 48.4 | **198.9** | 0.17 | 0 of 30 (load up to 11.6) |
| 3 | 103.1 | 34.4 | 14.7 | 48.4 | **200.5** | 0.13 | 30 of 30 |

Median **200.5 MiB**, flavour `GPU+Networking+WebContent`. The CI gate's baseline for that flavour is 201.1 (provisional,
`ci/baseline.json`); the budget is 200 on a real machine (212 on CI only for this flavour). So today's build sits on the
CI baseline and about 0.5 MiB over the real-machine budget, with the window hidden. The 2-helper flavour baseline (181.6)
does not apply here: this machine runs the GPU helper. CPU is effectively zero.

### Core only (no window), by scenario
`idle` and `run4` were taken at `ca438c5`. `run1` and the three terminal scenarios were **rerun on `a4efc78`** (batch-3 after #89,
so terminals start from `UserPathEnv::app(home)`, the environment the app really gives them); the earlier terminal rows
(26.8, 25.5, 45.5 MiB; flood 138% of a core) agree within 1.1 MiB. `run1` ran at load 11.8 (2 clean samples); the three
terminal scenarios at load 3 to 4.6.
| scenario | app process | run tree | each terminal | total MiB | CPU % of a core (whole tree) |
|---|---|---|---|---|---|
| idle | 6.0 | | | **6.0** | 0.00 |
| run1 (one run) | 6.5 | 14.1 | | **20.7** | 0.00 |
| run4 (one run, 4 workers at 20 lines/s) | 8.4 | 14.3 | | **22.7** | 0.84 |
| term_flood (acked, full speed) | 7.1 | 14.3 | 4.3 | **25.7** | **153** (the core 98, `yes` about 55) |
| term_flood_unacked (no view reading) | 6.9 | 14.3 | 4.3 | **25.5** | 0.05 |
| terms8 (8 idle terminals) | 7.5 | 14.3 | 2.9 to 3.0 each | **45.4** | 0.03 |

What it says:
- **Per unit:** a run costs the app +0.5 MiB and four streaming workers +2 MiB more at under 1% of a core. The agent
  here is a 14 MiB stand-in: a real `claude` is 160 to 520 MiB (about 430 median on this Mac), so the agent side of two
  runs and two Lead turns is roughly 1.7 GiB against the app's 200. An idle `zsh -l` terminal is 2.9 MiB.
- **A flooded, watched terminal is expensive in CPU, not memory:** about 100% of a core in the core itself to read, batch
  and acknowledge the output, plus the shell's `yes`. RSS stays within 1.4 MiB of an idle terminal.
- **An unwatched flood is held by the credit window:** with no acknowledgements the CPU falls to 0.03% and memory stays
  at 25.5 MiB. That is the bound the 1 MiB flow window gives; nothing is dropped or killed.
- The webview's own cost for a streaming worker (about +70 to +80 MiB retained after a burst) is the CI `postload` job's
  number, not repeated here.

## Not covered (honest gaps)
- **"3 projects open"** cannot be driven in the core yet: it allows one active run (`run_active`) and holds no project
  records until the FG-01 store lands, so that shape exists only in the webview. Measure it with the shipping binary
  once the shell has projects (S12.6 adds it to the gate).
- A real `claude` run and real workers (needs an account and a model), a terminal inside the real webview (the dock is
  not mounted yet), a quiet machine, a visible and an occluded window, and 30 minute growth. Rerun with the same commands.
- Windows and Linux: macOS only.


## Native process ledgers (macOS)

For an already attributed PID, read RSS and physical footprint separately:

```sh
clang -Wall -Wextra -Werror -O2 app/bench/profile/macos_usage.c -o /tmp/graphite-usage
/tmp/graphite-usage PID [PID ...]
python3 -m unittest discover -s app/bench/profile
```

The helper returns one JSON object per PID: `rssBytes`, `footprintBytes` and `startAbs`, from `proc_pid_rusage`
(`RUSAGE_INFO_V4`). Values are bytes; divide by 1,048,576 for MiB. `startAbs` is an opaque kernel start identity, not
an epoch timestamp or a duration. An unreadable or missing PID has an `error` errno instead of zero memory; the batch
still produces complete JSON and exits `1`. Invalid PID arguments exit `2` before any JSON is written. Success exits `0`.

Use the profile harness's ownership and identity checks to select PIDs. This utility only reads ledgers: it discovers
no owners, sends no signals and uses no coalition query. RSS and physical footprint are different accounting views;
keep the columns separate and label any sums by their process scope. The helper does not alter the RSS CI gate or
its baselines. Its allocation test touches 32 MiB in a temporary child and verifies that both native ledgers increase,
while the process start identity stays the same. No desktop window is opened.

A hidden-window result does not verify the foreground idle target or a foreground 1h soak. Those remain separate
measurements requiring a reserved desktop window; reject a foreground trial if renderer visibility changes or its
UI progress becomes stale. Likewise, never add a new helper by launch timing alone: require attributed ownership.


The macOS CI job runs the full profiling unit suite. Worker sandboxes can deny `ps` and `pgrep`; those process-inspection
checks require CI and must not be reported as locally passed. A regression test injects an inspection denial without
running either command and verifies that the temporary HOME is still removed. M5 foreground curves, the 190 MiB idle
target and a flat foreground 1h soak remain **UNVERIFIED** by this utility or its unit tests.
