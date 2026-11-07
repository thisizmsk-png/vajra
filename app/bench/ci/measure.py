"""H0 measurement: idle summed RSS (app + its WebKit helpers + app PTY children) and a growth slope.

usage: measure.py idle APP_BINARY OUT_JSON      (5 launches, median of medians and spread)
       h0_measure.py growth APP_BINARY OUT_JSON MINUTES

WebKit helpers are XPC services whose parent is launchd, not the app, so they are told apart by diffing the set of
com.apple.WebKit.* pids before and after launch. Another WebKit app starting during a sample would pollute the number;
the load average is recorded with every run for that reason.
"""
import collections, json, os, re, signal, statistics, subprocess, sys, tempfile, time

KB = 1024.0
KNOWN_TYPES = {"Networking", "WebContent", "GPU"}


def ps():
    out = subprocess.run(["ps", "-axo", "pid=,ppid=,rss=,comm="], capture_output=True, text=True).stdout
    rows = {}
    for line in out.splitlines():
        p = line.split(None, 3)
        if len(p) == 4 and p[0].isdigit():
            rows[int(p[0])] = (int(p[1]), int(p[2]), p[3])
    return rows


def webkit_pids(rows):
    return {pid for pid, (_, _, comm) in rows.items() if "com.apple.WebKit." in comm}


def descendants(rows, root):
    seen, stack = {root}, [root]
    while stack:
        cur = stack.pop()
        for pid, (pp, _, _) in rows.items():
            if pp == cur and pid not in seen:
                seen.add(pid)
                stack.append(pid)
    return seen


def launch(binary):
    before = webkit_pids(ps())
    home = tempfile.mkdtemp(prefix="vajra-h0-home-")
    env = dict(os.environ, HOME=home, VAJRA_DESKTOP_DATA=os.path.join(home, "data"))
    proc = subprocess.Popen([binary], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    return proc, before


def sample(proc, before):
    rows = ps()
    if proc.pid not in rows:
        raise SystemExit("app exited")
    app = descendants(rows, proc.pid)
    helpers = webkit_pids(rows) - before
    types = collections.Counter(helper_type(rows[pid][2]) for pid in helpers)
    parts = {"app": 0.0, "ptys": 0.0, "webkit": 0.0}
    for pid in app:
        parts["app" if pid == proc.pid else "ptys"] += rows[pid][1] / KB
    for pid in helpers:
        parts["webkit"] += rows[pid][1] / KB
    parts["total_mib"] = round(sum(parts.values()), 1)
    parts["helpers"] = len(helpers)
    parts["types"] = dict(types)
    parts["helper_detail"] = helper_detail(helpers)
    return {k: round(v, 1) if isinstance(v, float) else v for k, v in parts.items()}


def helper_type(comm):
    """'.../com.apple.WebKit.GPU.xpc/Contents/MacOS/com.apple.WebKit.GPU' -> 'GPU'."""
    return re.split(r"[./ ]", comm.rsplit("com.apple.WebKit.", 1)[-1])[0]


def helper_detail(pids):
    """Name, RSS, age and command line of each counted helper, so a surprising helper count can be explained."""
    if not pids:
        return []
    out = subprocess.run(["ps", "-o", "pid=,rss=,etime=,args=", "-p", ",".join(str(p) for p in sorted(pids))],
                         capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        p = line.split(None, 3)
        if len(p) == 4:
            rows.append({"pid": int(p[0]), "rss_mib": round(int(p[1]) / KB, 1), "etime": p[2],
                         "name": os.path.basename(p[3].split(" ")[0]), "args": p[3][:300]})
    return rows


def stop(proc):
    try:
        os.killpg(proc.pid, signal.SIGTERM)
        proc.wait(timeout=10)
    except Exception:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except Exception:
            pass


def load():
    return os.getloadavg()


LAUNCHES = 5


def identity_key(macos_build, webkit):
    """What decides the helper count and the WebKit footprint. The runner image version is recorded but not part of
    the key: it rotates weekly without changing either, and a key that rotated with it would fail the gate every week."""
    return f"{macos_build}|{webkit}"


def flavour_key(types):
    """The helper TYPE set as a key, e.g. 'GPU+Networking+WebContent'. Counts do not matter: a second GPU process is
    respawn churn, not a new flavour."""
    return "+".join(sorted(types))


def classify(types):
    """(flavour key, churn, unknown types) for one sample's {type: count}."""
    return flavour_key(types), any(c > 1 for c in types.values()), sorted(set(types) - KNOWN_TYPES)


def launch_summary(samples):
    """Summary of one launch.

    A sample is churn when it has more helpers than the launch's smallest count (foreign WebKit apps) or a duplicate of
    a type (respawn). Churned samples never enter the median. The launch's flavour is the type set of its clean samples;
    a launch with no clean sample is excluded entirely (median None) but still reports the type set it saw."""
    lo = min(s["helpers"] for s in samples)
    clean = [s for s in samples if s["helpers"] == lo and not classify(s["types"])[1]]
    churn = len(clean) != len(samples)
    pool = clean or samples
    flavours = collections.Counter(flavour_key(s["types"]) for s in pool)
    flavour = max(flavours, key=lambda k: (flavours[k], len(k), k))
    use = [s for s in clean if flavour_key(s["types"]) == flavour]
    # Unknown types are collected over ALL samples, before any RSS/churn filtering: the PID diff cannot show that an
    # unknown helper belongs to a foreign app, so it must never be filtered away.
    unknown = sorted({t for s in samples for t in classify(s["types"])[2]})
    return {"flavour": flavour, "median": statistics.median(s["total_mib"] for s in use) if use else None,
            "clean_samples": len(use), "churn": churn, "unknown_types": unknown}


def flavour_summary(runs):
    """(flavour key, median of that flavour's launch medians, launches in it, excluded launches).

    The reported number and the reported flavour must describe the same launches. The flavour with the most usable
    launches wins; on a tie the larger type set. Churn-only launches (median None) are excluded and counted."""
    usable = [r for r in runs if r["median_mib"] is not None]
    by = {}
    for r in usable:
        by.setdefault(r["flavour"], []).append(r["median_mib"])
    if not by:
        return None, None, 0, len(runs) - len(usable)
    key = max(by, key=lambda k: (len(by[k]), len(k), k))
    return key, statistics.median(by[key]), len(by[key]), len(runs) - len(usable)


def launch_flavours(runs):
    """Every distinct flavour across all launches (usable or not), so the gate rejects an unbaselined one instead of
    letting the majority flavour hide it."""
    return sorted({r["flavour"] for r in runs})


# The fixture build (VITE_PERF_FIXTURE) starts a scripted 10,000-line stream LOAD_START_S after launch at 170 lines/s and
# opens the worker's Stream tab; it ends at LOAD_END_S. These mirror app/ui/src/perf/fixture.ts.
LOAD_START_S = 20
LOAD_END_S = 20 + 60


def postload_summary(pre, peak, plus2, plus10):
    """Medians of each window's total, the highest sample seen during the load, and what the app kept (+10 min - pre-load)."""
    med = lambda ss: statistics.median(s["total_mib"] for s in ss)
    pre_m, p10 = med(pre), med(plus10)
    return {"pre_load_mib": pre_m, "peak_mib": max(s["total_mib"] for s in peak), "plus2_mib": med(plus2),
            "plus10_mib": p10, "retained_mib": round(p10 - pre_m, 1), "helpers": min(s["helpers"] for s in pre + plus10)}


def window(proc, before, seconds, step=2):
    out, end = [], time.time() + seconds
    while True:
        out.append(sample(proc, before))
        if time.time() + step > end:
            return out
        time.sleep(step)


def postload(binary, out):
    """Launch the fixture build, let it load the scripted stream, and sample idle at +2 and +10 minutes after the load.
    Writes `out` with the runner identity and a "post_load" section; reported only (gate.py --post warns, never fails)."""
    proc, before = launch(binary)
    t0 = time.time()
    at = lambda s: time.sleep(max(0.0, t0 + s - time.time()))
    try:
        at(LOAD_START_S - 10)
        pre = window(proc, before, 8)
        peak = []
        while time.time() < t0 + LOAD_END_S + 5:
            peak.append(sample(proc, before))
            time.sleep(2)
        at(LOAD_END_S + 120 - 10)
        plus2 = window(proc, before, 20)
        at(LOAD_END_S + 600 - 10)
        plus10 = window(proc, before, 20)
    finally:
        stop(proc)
    ident = runner_identity()
    json.dump({"identity": ident["key"], "runner": ident, "helpers": min(s["helpers"] for s in pre + plus10),
               "flavour": launch_summary(pre + plus10)["flavour"],
               "post_load": postload_summary(pre, peak, plus2, plus10)}, open(out, "w"), indent=1)


def runner_identity():
    def sh(*a):
        return subprocess.run(a, capture_output=True, text=True).stdout.strip()
    build = sh("sw_vers", "-buildVersion")
    webkit = sh("defaults", "read", "/System/Library/Frameworks/WebKit.framework/Resources/Info", "CFBundleVersion") \
        or sh("defaults", "read", "/System/Library/Frameworks/WebKit.framework/Versions/A/Resources/Info", "CFBundleVersion")
    return {"key": identity_key(build, webkit), "macos_build": build, "webkit": webkit,
            "macos": sh("sw_vers", "-productVersion"), "image_version": os.environ.get("ImageVersion", "")}


def idle(binary, out):
    runs = []
    for n in range(LAUNCHES):
        proc, before = launch(binary)
        time.sleep(25)  # settle
        samples = []
        for _ in range(15):
            samples.append(sample(proc, before))
            time.sleep(2)
        stop(proc)
        ls = launch_summary(samples)
        tot = [x["total_mib"] for x in samples]
        runs.append({"run": n + 1, "median_mib": ls["median"], "flavour": ls["flavour"], "clean_samples": ls["clean_samples"],
                     "churn": ls["churn"], "unknown_types": ls["unknown_types"],
                     "min": min(tot), "max": max(tot),
                     "helpers": sorted({x["helpers"] for x in samples}), "last": samples[-1], "load_avg": load()})
        time.sleep(5)
    flavour, flavour_median, in_flavour, excluded = flavour_summary(runs)
    meds = [r["median_mib"] for r in runs if r["median_mib"] is not None]
    ident = runner_identity()
    json.dump({"identity": ident["key"], "runner": ident, "flavour": flavour, "launch_flavours": launch_flavours(runs),
               "flavour_launches": in_flavour, "churn_launches": sum(r["churn"] for r in runs), "excluded_launches": excluded,
               "unknown_types": sorted({t for r in runs for t in r["unknown_types"]}), "runs": runs,
               "median_of_medians_mib": flavour_median,
               "all_launches_median_mib": statistics.median(meds) if meds else None,
               "spread_mib": [min(meds), max(meds)] if meds else None}, open(out, "w"), indent=1)


def slope(points):  # least squares, MiB per minute
    n = len(points)
    mx = sum(t for t, _ in points) / n
    my = sum(v for _, v in points) / n
    num = sum((t - mx) * (v - my) for t, v in points)
    den = sum((t - mx) ** 2 for t, _ in points) or 1
    return num / den


def growth(binary, out, minutes):
    proc, before = launch(binary)
    time.sleep(60)
    pts, series = [], []
    t0 = time.time()
    while time.time() - t0 < minutes * 60:
        s = sample(proc, before)
        t = (time.time() - t0) / 60
        pts.append((t, s["total_mib"]))
        series.append({"min": round(t, 1), **s, "load_1m": round(load()[0], 1)})
        json.dump({"partial": True, "series": series}, open(out, "w"))
        time.sleep(30)
    stop(proc)
    vals = [v for _, v in pts]
    json.dump({"minutes": minutes, "slope_mib_per_min": round(slope(pts), 3), "first_mib": vals[0], "last_mib": vals[-1],
               "peak_mib": max(vals), "series": series}, open(out, "w"), indent=1)


if __name__ == "__main__":
    mode, binary, out = sys.argv[1:4]
    if mode == "idle":
        idle(binary, out)
    elif mode == "postload":
        postload(binary, out)
    else:
        growth(binary, out, float(sys.argv[4]))
