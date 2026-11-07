"""H0 measurement: idle summed RSS (app + its WebKit helpers + app PTY children) and a growth slope.

usage: measure.py idle APP_BINARY OUT_JSON      (5 launches, median of medians and spread)
       h0_measure.py growth APP_BINARY OUT_JSON MINUTES

WebKit helpers are XPC services whose parent is launchd, not the app, so they are told apart by diffing the set of
com.apple.WebKit.* pids before and after launch. Another WebKit app starting during a sample would pollute the number;
the load average is recorded with every run for that reason.
"""
import json, os, signal, statistics, subprocess, sys, tempfile, time

KB = 1024.0


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
    parts = {"app": 0.0, "ptys": 0.0, "webkit": 0.0}
    for pid in app:
        parts["app" if pid == proc.pid else "ptys"] += rows[pid][1] / KB
    for pid in helpers:
        parts["webkit"] += rows[pid][1] / KB
    parts["total_mib"] = round(sum(parts.values()), 1)
    parts["helpers"] = len(helpers)
    return {k: round(v, 1) if isinstance(v, float) else v for k, v in parts.items()}


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


def base_helpers(runs):
    return min(min(r["helpers"]) for r in runs)


def runner_identity():
    def sh(*a):
        return subprocess.run(a, capture_output=True, text=True).stdout.strip()
    build = sh("sw_vers", "-buildVersion")
    webkit = sh("defaults", "read", "/System/Library/Frameworks/WebKit.framework/Resources/Info", "CFBundleVersion") \
        or sh("defaults", "read", "/System/Library/Frameworks/WebKit.framework/Versions/A/Resources/Info", "CFBundleVersion")
    return {"key": identity_key(build, webkit), "macos_build": build, "webkit": webkit,
            "macos": sh("sw_vers", "-productVersion"), "image_version": os.environ.get("ImageVersion", "")}


def clean_median(samples):
    """Median total of the samples taken while no foreign WebKit helpers were running.

    Other apps on a shared runner start helpers of their own; a sample then counts them (helper count above the
    smallest seen in this launch) and reads 140 to 260 MiB too high. Those samples are dropped, never averaged in.
    """
    lo = min(s["helpers"] for s in samples)
    kept = [s["total_mib"] for s in samples if s["helpers"] == lo]
    return statistics.median(kept), len(kept)


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
        med, kept = clean_median(samples)
        tot = [s["total_mib"] for s in samples]
        runs.append({"run": n + 1, "median_mib": med, "clean_samples": kept, "min": min(tot), "max": max(tot),
                     "helpers": sorted({s["helpers"] for s in samples}), "last": samples[-1], "load_avg": load()})
        time.sleep(5)
    meds = [r["median_mib"] for r in runs]
    ident = runner_identity()
    json.dump({"identity": ident["key"], "runner": ident, "helpers": base_helpers(runs), "runs": runs,
               "median_of_medians_mib": statistics.median(meds), "spread_mib": [min(meds), max(meds)]}, open(out, "w"), indent=1)


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
    idle(binary, out) if mode == "idle" else growth(binary, out, float(sys.argv[4]))
