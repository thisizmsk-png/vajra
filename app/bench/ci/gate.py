"""Perf gate: read measure.py output and fail on a regression.

usage: gate.py BASELINE_JSON IDLE_JSON [GROWTH_JSON]
A launch is classified by the SET of WebKit helper types it ran (Networking, WebContent, GPU), not by their count.
Fails closed on an unknown runner identity, an unknown helper type, or a flavour with no baseline entry. Otherwise fails
if growth slope > 1 MiB/min, idle > the budget, or idle > baseline for that (identity, flavour) + max(5 MiB, 3%).
Budget: 200 MiB; 212 MiB only on CI (GITHUB_ACTIONS set) when the flavour includes the GPU process (owner ruling, see
README "GPU-flavour ceiling").
"""
import json, os, sys

IDLE_BUDGET, GPU_CI_BUDGET, REGRESSION, REGRESSION_PCT, SLOPE_BUDGET = 200.0, 212.0, 5.0, 0.03, 1.0
KNOWN_TYPES = {"Networking", "WebContent", "GPU"}


def on_ci():
    return bool(os.environ.get("GITHUB_ACTIONS"))


def flavour_types(flavour):
    return set(flavour.split("+")) if flavour else set()


def budget_for(flavour, ci):
    return GPU_CI_BUDGET if ci and "GPU" in flavour_types(flavour) else IDLE_BUDGET


def evaluate(baseline, idle, growth, ci=None):
    """(failures, warnings). ci=None reads GITHUB_ACTIONS."""
    ci = on_ci() if ci is None else ci
    fails, warns = [], []
    v = idle.get("median_of_medians_mib")
    flavour = idle.get("flavour")
    if growth is not None and growth["slope_mib_per_min"] > SLOPE_BUDGET:
        fails.append(f"growth {growth['slope_mib_per_min']} MiB/min exceeds {SLOPE_BUDGET}")
    churn = idle.get("churn_launches") or 0
    if churn:
        warns.append(f"{churn} launch(es) saw respawn churn (duplicate helpers of a known type); churned samples and "
                     f"launches are excluded from the median ({idle.get('excluded_launches', 0)} launch(es) excluded entirely)")
    unknown = sorted(set(idle.get("unknown_types") or []))
    if unknown:
        fails.append(f"unknown WebKit helper type(s) {unknown}; only {sorted(KNOWN_TYPES)} are known (fails closed)")
        return fails, warns
    if v is None or not flavour:
        fails.append("no usable launch: every launch was excluded or saw no WebKit helper (fails closed)")
        return fails, warns
    odd = [f for f in idle.get("launch_flavours", []) if not flavour_types(f) or not flavour_types(f) <= KNOWN_TYPES]
    if odd or not flavour_types(flavour) <= KNOWN_TYPES:
        fails.append(f"launch flavour(s) {odd or [flavour]} contain a helper type outside {sorted(KNOWN_TYPES)} (fails closed)")
        return fails, warns
    budget = budget_for(flavour, ci)
    if v > budget:
        fails.append(f"idle {v} MiB exceeds the {budget} MiB budget"
                     + (" (GPU flavour on CI)" if budget == GPU_CI_BUDGET else ""))
    key = idle.get("identity")
    entry = baseline.get("identities", {}).get(key) if key else None
    if entry is None:
        fails.append(f"no baseline for runner identity {key!r}: measure it with the procedure in app/bench/ci/README.md "
                     "and add it to baseline.json in a reviewed PR (the gate fails closed on an unknown runner)")
        return fails, warns
    missing = [f for f in idle.get("launch_flavours", []) if f not in entry.get("flavours", {})]
    if missing:
        fails.append(f"launch flavour(s) {missing} have no baseline entry for runner identity {key!r} in baseline.json (fails closed)")
        return fails, warns
    fl = entry.get("flavours", {}).get(flavour)
    if fl is None:
        fails.append(f"baseline.json has no {flavour!r} entry for runner identity {key!r}: add it with the procedure in "
                     "app/bench/ci/README.md (fails closed)")
        return fails, warns
    warns.extend(post_load_warnings(idle.get("post_load"), fl))
    b = fl["idle_mib"]
    tol = max(REGRESSION, b * REGRESSION_PCT)
    if v > b + tol:
        msg = f"idle {v} MiB is more than {tol:.1f} MiB over the baseline {b} for runner {key}, flavour {flavour}"
        if fl.get("provisional"):
            warns.append(f"{msg} (baseline is provisional, one run: warning only until refined)")
        else:
            fails.append(msg)
    return fails, warns


def post_load_warnings(post, flavour):
    """The post-load idle (+10 min after a scripted 10,000-line stream) is reported, never gated, until a baseline is
    recorded with the bump procedure (`post_load_mib` on the flavour)."""
    if not post:
        return []
    v, out = post["plus10_mib"], []
    if v > IDLE_BUDGET:
        out.append(f"post-load idle {v} MiB is over the {IDLE_BUDGET} MiB budget (reported only)")
    b = flavour.get("post_load_mib")
    if b is None:
        out.append(f"post-load idle {v} MiB (retained {post['retained_mib']} MiB after the load, peak {post['peak_mib']} MiB): no baseline yet, reported only")
    elif v > b + max(REGRESSION, b * REGRESSION_PCT):
        out.append(f"post-load idle {v} MiB is over the baseline {b} MiB (reported only)")
    return out


def summary(idle):
    post = idle.get("post_load")
    if not post:
        return ""
    return ("### Post-load idle (scripted 10,000-line stream, reported only)\n\n| pre-load | peak | +2 min | +10 min | retained |\n|---|---|---|---|---|\n"
            f"| {post['pre_load_mib']} | {post['peak_mib']} | {post['plus2_mib']} | {post['plus10_mib']} | {post['retained_mib']} MiB |\n")


def check(baseline, idle, growth, ci=None):
    return evaluate(baseline, idle, growth, ci)[0]


def post_only(baseline, post_json):
    """(warnings, summary) for a postload.json on its own; looks up the flavour baseline when the identity is known."""
    entry = baseline.get("identities", {}).get(post_json.get("identity", ""), {})
    flavour = entry.get("flavours", {}).get(post_json.get("flavour", ""), {})
    return post_load_warnings(post_json.get("post_load"), flavour), summary(post_json)


if __name__ == "__main__":
    load = lambda p: json.load(open(p))
    if sys.argv[1] == "--post":
        warns, md = post_only(load(sys.argv[2]), load(sys.argv[3]))
        for w in warns:
            print(f"::warning::perf post-load: {w}")
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            open(os.environ["GITHUB_STEP_SUMMARY"], "a").write(md)
        print(md or "no post-load data")
        sys.exit(0)
    fails, warns = evaluate(load(sys.argv[1]), load(sys.argv[2]), load(sys.argv[3]) if len(sys.argv) > 3 else None)
    for w in warns:
        print(f"::warning::perf gate: {w}")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write(summary(load(sys.argv[2])))
    print("\n".join(fails) or "perf gate: ok")
    sys.exit(1 if fails else 0)
