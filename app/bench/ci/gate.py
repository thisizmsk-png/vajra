"""Perf gate: read measure.py output and fail on a regression.

usage: gate.py BASELINE_JSON IDLE_JSON [GROWTH_JSON]
Fails if idle > 200 MiB, the growth slope > 1 MiB/min, the runner identity has no baseline (fails closed),
the helper count differs from the one recorded for that identity (our regression), or
idle > that identity's baseline + max(5 MiB, 3%).
"""
import json, sys

IDLE_BUDGET, REGRESSION, REGRESSION_PCT, SLOPE_BUDGET = 200.0, 5.0, 0.03, 1.0


def check(baseline, idle, growth):
    fails = []
    v = idle["median_of_medians_mib"]
    if v > IDLE_BUDGET:
        fails.append(f"idle {v} MiB exceeds the {IDLE_BUDGET} MiB budget")
    if growth is not None and growth["slope_mib_per_min"] > SLOPE_BUDGET:
        fails.append(f"growth {growth['slope_mib_per_min']} MiB/min exceeds {SLOPE_BUDGET}")
    key = idle.get("identity")
    entry = baseline.get("identities", {}).get(key) if key else None
    if entry is None:
        fails.append(f"no baseline for runner identity {key!r}: measure it with the procedure in app/bench/ci/README.md "
                     "and add it to baseline.json in a reviewed PR (the gate fails closed on an unknown runner)")
        return fails
    if idle.get("helpers") != entry["helpers"]:
        fails.append(f"WebKit helper count is {idle.get('helpers')}, baseline for this runner has {entry['helpers']}")
    b = entry["idle_mib"]
    tol = max(REGRESSION, b * REGRESSION_PCT)
    if v > b + tol:
        fails.append(f"idle {v} MiB is more than {tol:.1f} MiB over the baseline {b} for runner {key}")
    return fails


if __name__ == "__main__":
    load = lambda p: json.load(open(p))
    fails = check(load(sys.argv[1]), load(sys.argv[2]), load(sys.argv[3]) if len(sys.argv) > 3 else None)
    print("\n".join(fails) or "perf gate: ok")
    sys.exit(1 if fails else 0)
