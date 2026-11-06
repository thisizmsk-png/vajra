"""Perf gate: read measure.py output and fail on a regression.

usage: gate.py BASELINE_JSON IDLE_JSON [GROWTH_JSON]
Fails if idle > 200 MiB, idle > baseline + 5 MiB (when a baseline exists), or growth slope > 1 MiB/min.
"""
import json, sys

IDLE_BUDGET, REGRESSION, SLOPE_BUDGET = 200.0, 5.0, 1.0


def check(baseline, idle, growth):
    fails = []
    v = idle["median_of_medians_mib"]
    if v > IDLE_BUDGET:
        fails.append(f"idle {v} MiB exceeds the {IDLE_BUDGET} MiB budget")
    b = baseline.get("idle_mib")
    if b is not None and v > b + REGRESSION:
        fails.append(f"idle {v} MiB is more than {REGRESSION} MiB over the baseline {b}")
    if growth is not None and growth["slope_mib_per_min"] > SLOPE_BUDGET:
        fails.append(f"growth {growth['slope_mib_per_min']} MiB/min exceeds {SLOPE_BUDGET}")
    return fails


if __name__ == "__main__":
    load = lambda p: json.load(open(p))
    fails = check(load(sys.argv[1]), load(sys.argv[2]), load(sys.argv[3]) if len(sys.argv) > 3 else None)
    print("\n".join(fails) or "perf gate: ok")
    sys.exit(1 if fails else 0)
