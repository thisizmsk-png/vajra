"""Perf gate: read measure.py output and fail on a regression.

usage: gate.py BASELINE_JSON IDLE_JSON [GROWTH_JSON]
Fails closed on an unknown runner identity or a WebKit helper count other than 2 or 3. Otherwise fails if
growth slope > 1 MiB/min, idle > baseline for that (identity, helper count) + max(5 MiB, 3%), or (2-helper
flavour only) idle > the 200 MiB ceiling. In the 3-helper (GPU process) flavour the ceiling is reported as a warning,
not a failure, until the owner rules on it (see README "Owner question").
"""
import json, sys

IDLE_BUDGET, REGRESSION, REGRESSION_PCT, SLOPE_BUDGET = 200.0, 5.0, 0.03, 1.0
FLAVOURS = {2, 3}


def evaluate(baseline, idle, growth):
    """(failures, warnings)."""
    fails, warns = [], []
    v = idle["median_of_medians_mib"]
    n = idle.get("helpers")
    if growth is not None and growth["slope_mib_per_min"] > SLOPE_BUDGET:
        fails.append(f"growth {growth['slope_mib_per_min']} MiB/min exceeds {SLOPE_BUDGET}")
    if v > IDLE_BUDGET:
        fails.append(f"idle {v} MiB exceeds the {IDLE_BUDGET} MiB budget")
    odd = [f for f in idle.get("launch_flavours", []) if f not in FLAVOURS]
    if odd:
        fails.append(f"launch(es) with WebKit helper count {odd}; only 2 or 3 are known flavours (fails closed)")
        return fails, warns
    if n not in FLAVOURS:
        fails.append(f"WebKit helper count is {n}; only 2 or 3 are known flavours (fails closed)")
        return fails, warns
    key = idle.get("identity")
    entry = baseline.get("identities", {}).get(key) if key else None
    if entry is None:
        fails.append(f"no baseline for runner identity {key!r}: measure it with the procedure in app/bench/ci/README.md "
                     "and add it to baseline.json in a reviewed PR (the gate fails closed on an unknown runner)")
        return fails, warns
    flavour = entry.get("flavours", {}).get(str(n))
    if flavour is None:
        fails.append(f"baseline.json has no {n}-helper entry for runner identity {key!r}: add it with the procedure in "
                     "app/bench/ci/README.md (fails closed)")
        return fails, warns
    b = flavour["idle_mib"]
    tol = max(REGRESSION, b * REGRESSION_PCT)
    if v > b + tol:
        msg = f"idle {v} MiB is more than {tol:.1f} MiB over the baseline {b} for runner {key}, {n} helpers"
        if flavour.get("provisional"):
            warns.append(f"{msg} (baseline is provisional, one run: warning only until refined)")
        else:
            fails.append(msg)
    return fails, warns


def check(baseline, idle, growth):
    return evaluate(baseline, idle, growth)[0]


if __name__ == "__main__":
    load = lambda p: json.load(open(p))
    fails, warns = evaluate(load(sys.argv[1]), load(sys.argv[2]), load(sys.argv[3]) if len(sys.argv) > 3 else None)
    for w in warns:
        print(f"::warning::perf gate: {w}")
    print("\n".join(fails) or "perf gate: ok")
    sys.exit(1 if fails else 0)
